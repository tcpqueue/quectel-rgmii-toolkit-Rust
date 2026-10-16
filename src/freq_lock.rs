//! NR5G frequency lock (`AT+QNWCFG="nr5g_earfcn_lock"`): the module only uses the listed
//! NR-ARFCNs, somewhere between a band lock and a cell lock.
//!
//! Only NR is offered. On an RM520N-EU (RM520NEUDAR03A08M4G) the LTE variant crashed the
//! modem firmware when it was applied with an NSA 5G leg up, even for the serving EARFCN.
//! A wrong SCS can crash the module too, so every entry is checked against the bands the
//! carrier configuration and the current band selection allow, and against the SCS those
//! bands use. The lock cannot be combined with an NR cell lock (`AT+QNWLOCK="common/5g"`).
use crate::{actions, at::At, events::Events, parser, persistence::Store};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const QUERY: &str = "AT+QNWCFG=\"nr5g_earfcn_lock\"";
pub const UNLOCK: &str = "AT+QNWCFG=\"nr5g_earfcn_lock\",0";
/// The module accepts at most 32 entries.
pub const MAX_ENTRIES: usize = 32;

/// Parses `arfcn:scs` entries separated by commas, e.g. `504990:30,513000:30`.
pub fn parse(list: &str) -> Result<Vec<(u32, u32)>> {
    let mut entries = vec![];
    for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some((arfcn, scs)) = item.split_once(':') else {
            bail!("each entry needs an NR-ARFCN and an SCS")
        };
        let arfcn: u32 = arfcn
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid NR-ARFCN"))?;
        let scs: u32 = scs
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid SCS"))?;
        if arfcn > 3279165 {
            bail!("invalid NR-ARFCN")
        }
        if entries.iter().any(|(a, _)| *a == arfcn) {
            bail!("NR-ARFCN {arfcn} is listed twice")
        }
        entries.push((arfcn, scs));
    }
    if entries.is_empty() || entries.len() > MAX_ENTRIES {
        bail!("enter 1 to {MAX_ENTRIES} frequencies")
    }
    Ok(entries)
}
pub fn command(entries: &[(u32, u32)]) -> String {
    let list: Vec<String> = entries.iter().map(|(a, s)| format!("{a}:{s}")).collect();
    format!(
        "AT+QNWCFG=\"nr5g_earfcn_lock\",{},{}",
        entries.len(),
        list.join(":")
    )
}
fn band_list(raw: &str, keys: &[&str]) -> Vec<u32> {
    let mut bands = vec![];
    for p in parser::lines(raw)
        .filter_map(|l| l.trim().strip_prefix("+QNWPREFCFG:"))
        .map(parser::fields)
    {
        if p.len() > 1 && keys.contains(&p[0].as_str()) {
            for band in p[1].split(':').filter_map(|b| b.parse().ok()) {
                if !bands.contains(&band) {
                    bands.push(band);
                }
            }
        }
    }
    bands
}
/// NR bands the lock may use: the carrier configuration's (`policy`, from
/// `AT+QNWPREFCFG="policy_band"`, skipped when unknown) within the current NSA and SA band
/// selection (`selected`).
pub fn allowed_bands(policy: &str, selected: &str) -> Result<Vec<u32>> {
    const KEYS: [&str; 2] = ["nsa_nr5g_band", "nr5g_band"];
    let selected = band_list(selected, &KEYS);
    if selected.is_empty() {
        bail!("cannot read the current NR band selection")
    }
    let policy = band_list(policy, &KEYS);
    Ok(selected
        .into_iter()
        .filter(|b| policy.is_empty() || policy.contains(b))
        .collect())
}
/// TDD bands among those `parser::nr_bands` knows; the others are FDD.
fn nr_tdd(band: u32) -> bool {
    matches!(band, 34 | 38 | 39 | 40 | 41 | 46 | 48 | 77 | 78 | 79 | 90)
}
/// Checks each entry: its frequency must lie in an allowed band, must not also lie in a
/// disallowed band of the other duplex mode, and the SCS must be one every allowed band uses.
pub fn check(entries: &[(u32, u32)], allowed: &[u32]) -> Result<()> {
    for &(arfcn, scs) in entries {
        let candidates = parser::nr_bands(arfcn);
        if candidates.is_empty() {
            bail!("NR-ARFCN {arfcn} is not in a supported FR1 band")
        }
        let usable: Vec<u32> = candidates
            .iter()
            .copied()
            .filter(|b| allowed.contains(b))
            .collect();
        let names = |bands: &[u32]| {
            bands
                .iter()
                .map(|b| format!("n{b}"))
                .collect::<Vec<_>>()
                .join("/")
        };
        if usable.is_empty() {
            bail!(
                "NR-ARFCN {arfcn} ({}) is outside the allowed bands {}",
                names(&candidates),
                names(allowed)
            )
        }
        // Where a TDD band overlaps an FDD band (n41 and n7 around 2.62-2.69 GHz), the modem may
        // treat the frequency as the other band. On an RM520N-EU, locking 524910 (n41, also n7)
        // crashed the modem: "CAL data not available ... frequency 2624820".
        if let Some(other) = candidates
            .iter()
            .find(|b| !allowed.contains(b) && usable.iter().any(|u| nr_tdd(*u) != nr_tdd(**b)))
        {
            bail!(
                "NR-ARFCN {arfcn} also lies in n{other}, which is not allowed here; locking such a frequency crashed the modem"
            )
        }
        if let Some(band) = usable.iter().find(|b| !actions::nr_scs_valid(**b, scs)) {
            let expected = actions::nr_band_scs(*band)
                .map(|s| s.iter().map(u32::to_string).collect::<Vec<_>>().join(" / "))
                .unwrap_or_else(|| "15 / 30".into());
            bail!("SCS {scs} kHz is not valid for n{band} (NR-ARFCN {arfcn}); use {expected} kHz")
        }
    }
    Ok(())
}

/// Bands such as n41 allow 15 and 30 kHz, but a real cell uses one. When the module has seen
/// a cell on an entry's frequency (`observed`: serving cell and recent scans, as arfcn/SCS),
/// the entry must use that cell's SCS.
pub fn check_observed(entries: &[(u32, u32)], observed: &[(u32, u32)]) -> Result<()> {
    for &(arfcn, scs) in entries {
        if let Some((_, seen)) = observed.iter().find(|(a, s)| *a == arfcn && *s != scs) {
            bail!("the cell on NR-ARFCN {arfcn} uses {seen} kHz, not {scs} kHz")
        }
    }
    Ok(())
}
/// NR cells with a known SCS in a scan or neighbour result (`nr5g_cells_parsed`).
pub fn observed_cells(cells: &Value) -> Vec<(u32, u32)> {
    cells["nr5g_cells_parsed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some((
                c["freq"].as_str()?.parse().ok()?,
                u32::try_from(c["scs"].as_u64()?).ok()?,
            ))
        })
        .collect()
}
/// The module forgets the lock when it restarts; a persistent lock is sent again at startup.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
struct Saved {
    entries: Vec<(u32, u32)>,
}
#[derive(Default)]
struct State {
    saved: Saved,
    /// False when the file could not be loaded; the next change rewrites it.
    stored: bool,
    /// "", "temporary", "persistent", "unlocked" or "restore_failed".
    phase: &'static str,
    /// Guard after a lock: "idle", "checking", "ok", "reverted" or "failed".
    guard: &'static str,
    generation: u64,
    error: String,
}
pub struct FreqLock {
    state: Mutex<State>,
    mutation: tokio::sync::Mutex<()>,
    path: PathBuf,
    store: Arc<Store>,
    timing: (Duration, Duration),
}
impl FreqLock {
    pub fn new(path: PathBuf, store: Arc<Store>, mock: bool) -> Arc<Self> {
        let loaded = std::fs::read(&path).ok().map(|b| {
            serde_json::from_slice::<Saved>(&b)
                .ok()
                .filter(|s| s.entries.len() <= MAX_ENTRIES)
        });
        let (saved, stored) = match loaded {
            None => (Saved::default(), true),
            Some(Some(saved)) => (saved, true),
            Some(None) => (Saved::default(), false),
        };
        Arc::new(Self {
            state: Mutex::new(State {
                saved,
                stored,
                guard: "idle",
                ..State::default()
            }),
            mutation: tokio::sync::Mutex::new(()),
            path,
            store,
            // Settle, then interval between checks; the mock answers at once.
            timing: if mock {
                (Duration::from_millis(300), Duration::from_millis(300))
            } else {
                (Duration::from_secs(5), Duration::from_secs(5))
            },
        })
    }
    pub fn persistent(&self) -> bool {
        !self.state.lock().unwrap().saved.entries.is_empty()
    }
    /// `module` is the lock the module reports right now.
    pub fn snapshot(&self, module: Option<Vec<(u32, u32)>>) -> Value {
        let s = self.state.lock().unwrap();
        json!({
            "module": module,
            "saved": if s.saved.entries.is_empty() { Value::Null } else { json!(s.saved.entries) },
            "phase": s.phase,
            "guard": s.guard,
            "error": s.error,
        })
    }
    async fn persist(&self, saved: Saved) -> Result<()> {
        {
            let s = self.state.lock().unwrap();
            if s.stored && s.saved == saved {
                return Ok(());
            }
        }
        let bytes = serde_json::to_vec(&saved)?;
        let (path, store) = (self.path.clone(), self.store.clone());
        let result = tokio::task::spawn_blocking(move || store.write(&path, &bytes, 0o600)).await?;
        if result.is_ok() || result.as_ref().is_err_and(|e| e.committed) {
            let mut s = self.state.lock().unwrap();
            s.saved = saved;
            s.stored = true;
        }
        match result {
            Err(e) if !e.committed => Err(e.error),
            _ => Ok(()),
        }
    }
    /// Locks to `entries` (already checked). `previous` is what the module had before, put
    /// back by the guard when the module finds no service within 30 seconds.
    pub async fn lock(
        self: &Arc<Self>,
        at: &At,
        events: &Arc<Events>,
        entries: Vec<(u32, u32)>,
        previous: Vec<(u32, u32)>,
        persistent: bool,
        guard: bool,
    ) -> Result<Value> {
        let _guard = self.mutation.lock().await;
        let response = at.run(&command(&entries)).await?;
        at.invalidate().await;
        if !parser::ok(&response) {
            bail!("frequency lock rejected: {}", response.trim())
        }
        let before = self.state.lock().unwrap().saved.clone();
        let next = Saved {
            entries: if persistent { entries.clone() } else { vec![] },
        };
        let mut warning = None;
        if let Err(e) = self.persist(next).await {
            warning = Some(format!("cannot save the lock for restarts: {e}"));
        }
        let generation = {
            let mut s = self.state.lock().unwrap();
            s.phase = if persistent {
                "persistent"
            } else {
                "temporary"
            };
            s.error.clear();
            s.generation += 1;
            s.guard = if guard { "checking" } else { "idle" };
            s.generation
        };
        if guard {
            self.watch(
                at.clone(),
                events.clone(),
                entries,
                previous,
                before,
                generation,
            );
        }
        Ok(json!({"ok":true,"response":response,"warning":warning}))
    }
    /// Clears the lock in the module and the saved one. The module answers ERROR when nothing
    /// is locked, which counts as done once the query shows no lock.
    pub async fn unlock(&self, at: &At) -> Result<Value> {
        let _guard = self.mutation.lock().await;
        {
            let mut s = self.state.lock().unwrap();
            s.generation += 1;
            if s.guard == "checking" {
                s.guard = "idle";
            }
        }
        let response = unlock(at).await?;
        self.persist(Saved::default()).await?;
        let mut s = self.state.lock().unwrap();
        s.phase = "unlocked";
        s.error.clear();
        Ok(json!({"ok":true,"response":response}))
    }
    /// Sends a saved lock again after the program starts (the module forgets it on restart),
    /// guarded like a new lock: no service within 30 s removes it and the saved copy.
    pub fn start(self: &Arc<Self>, at: At, events: Arc<Events>) {
        let this = self.clone();
        tokio::spawn(async move {
            at.wait_ready().await;
            let entries = this.state.lock().unwrap().saved.entries.clone();
            if entries.is_empty() {
                return;
            }
            let _guard = this.mutation.lock().await;
            let result = at.run(&command(&entries)).await;
            at.invalidate().await;
            let generation = {
                let mut s = this.state.lock().unwrap();
                s.generation += 1;
                if result.as_ref().is_ok_and(|r| parser::ok(r)) {
                    s.phase = "persistent";
                    s.guard = "checking";
                } else {
                    s.phase = "restore_failed";
                    s.error = "Persistent frequency lock restore failed".into();
                    return;
                }
                s.generation
            };
            this.watch(at, events, entries, vec![], Saved::default(), generation);
        });
    }
    /// Up to five checks after the settle time; true once the module serves on `entries`.
    /// False also when a newer lock or unlock replaced this one (`generation` moved on).
    async fn confirm(&self, at: &At, entries: &[(u32, u32)], generation: u64) -> Option<bool> {
        let locked: Vec<u32> = entries.iter().map(|e| e.0).collect();
        tokio::time::sleep(self.timing.0).await;
        for check in 0..5 {
            if check > 0 {
                tokio::time::sleep(self.timing.1).await;
            }
            if self.state.lock().unwrap().generation != generation {
                return None;
            }
            if at
                .run("AT+QENG=\"servingcell\";+QCAINFO")
                .await
                .is_ok_and(|raw| crate::band_guard::nr_frequency_ok(&raw, &locked))
            {
                return Some(true);
            }
        }
        Some(false)
    }
    /// Watches a new lock. Without service after 30 s the previous lock comes back; when that
    /// one finds no service either, the lock is removed.
    fn watch(
        self: &Arc<Self>,
        at: At,
        events: Arc<Events>,
        entries: Vec<(u32, u32)>,
        previous: Vec<(u32, u32)>,
        saved_before: Saved,
        generation: u64,
    ) {
        let this = self.clone();
        tokio::spawn(async move {
            match this.confirm(&at, &entries, generation).await {
                None => return,
                Some(true) => {
                    let mut s = this.state.lock().unwrap();
                    if s.generation == generation {
                        s.guard = "ok";
                    }
                    return;
                }
                Some(false) => {}
            }
            let restore = |list: &[(u32, u32)]| {
                if list.is_empty() {
                    UNLOCK.to_owned()
                } else {
                    command(list)
                }
            };
            let mut back = previous.clone();
            let mut restored = {
                let _guard = this.mutation.lock().await;
                if this.state.lock().unwrap().generation != generation {
                    return;
                }
                let ok = if back.is_empty() {
                    unlock(&at).await.is_ok()
                } else {
                    at.run(&restore(&back)).await.is_ok_and(|r| parser::ok(&r))
                };
                at.invalidate().await;
                ok
            };
            if restored
                && !back.is_empty()
                && this.confirm(&at, &back, generation).await == Some(false)
            {
                // The previous lock finds no cell either (e.g. a wrong SCS): drop the lock.
                let _guard = this.mutation.lock().await;
                if this.state.lock().unwrap().generation != generation {
                    return;
                }
                back.clear();
                restored = unlock(&at).await.is_ok();
            }
            let _guard = this.mutation.lock().await;
            // A lock or unlock made meanwhile saved its own state; keep it.
            if this.state.lock().unwrap().generation != generation {
                return;
            }
            // The failed lock must not come back after a restart.
            let mut saved_before = saved_before;
            if back.is_empty() {
                saved_before = Saved::default();
            }
            let saved = this.persist(saved_before).await.is_ok();
            {
                let mut s = this.state.lock().unwrap();
                if s.generation == generation {
                    s.guard = if restored && saved {
                        "reverted"
                    } else {
                        "failed"
                    };
                    s.phase = if back.is_empty() {
                        "unlocked"
                    } else {
                        "temporary"
                    };
                }
            }
            let list = |e: &[(u32, u32)]| {
                e.iter()
                    .map(|(a, s)| format!("{a}:{s}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            events.push(
                if restored {
                    "freq_reverted"
                } else {
                    "freq_revert_failed"
                },
                if restored { "warn" } else { "bad" },
                json!({"locked":list(&entries),"previous":list(&back)}),
            );
        });
    }
}
/// Sends the unlock; an ERROR answer is fine when the module then reports no lock.
pub async fn unlock(at: &At) -> Result<String> {
    let response = at.run(UNLOCK).await?;
    at.invalidate().await;
    if parser::ok(&response) {
        return Ok(response);
    }
    let state = at.run(QUERY).await?;
    if parser::nr_freq_lock(&state).is_some_and(|e| e.is_empty()) {
        return Ok(state);
    }
    bail!("frequency unlock rejected: {}", response.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    const POLICY: &str = "+QNWPREFCFG: \"gw_band\",1:8\r\n+QNWPREFCFG: \"lte_band\",1:3:5:8:34:38:39:40:41\r\n+QNWPREFCFG: \"nsa_nr5g_band\",41\r\n+QNWPREFCFG: \"nr5g_band\",28:41\r\nOK\r\n";
    const SELECTED: &str = "+QNWPREFCFG: \"nsa_nr5g_band\",1:28:41:78\r\n+QNWPREFCFG: \"nr5g_band\",1:28:41:78\r\nOK\r\n";

    #[test]
    fn parses_and_formats_entries() {
        let entries = parse("504990:30, 513000:30").unwrap();
        assert_eq!(entries, [(504990, 30), (513000, 30)]);
        assert_eq!(
            command(&entries),
            "AT+QNWCFG=\"nr5g_earfcn_lock\",2,504990:30:513000:30"
        );
        for bad in [
            "",
            "504990",
            "504990:x",
            "504990:30,504990:15",
            "9999999:30",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..33).map(|i| format!("{}:30", 504990 + i)).collect();
        assert!(parse(&many.join(",")).is_err());
    }
    #[test]
    fn reads_the_module_lock() {
        assert_eq!(
            parser::nr_freq_lock("+QNWCFG: \"nr5g_earfcn_lock\",2,504990:30:513000:30\r\nOK\r\n"),
            Some(vec![(504990, 30), (513000, 30)])
        );
        assert_eq!(
            parser::nr_freq_lock("+QNWCFG: \"nr5g_earfcn_lock\",0\r\nOK\r\n"),
            Some(vec![])
        );
        assert_eq!(parser::nr_freq_lock("ERROR\r\n"), None);
    }
    #[test]
    fn only_allowed_bands_and_their_scs_pass() {
        let allowed = allowed_bands(POLICY, SELECTED).unwrap();
        assert_eq!(allowed, [28, 41], "policy limits the selection");
        assert!(check(&[(504990, 30)], &allowed).is_ok());
        let error = check(&[(524910, 30)], &allowed).unwrap_err().to_string();
        assert!(error.contains("also lies in n7"), "{error}");
        assert!(
            check(&[(504990, 15)], &allowed).is_ok(),
            "n41 also uses 15 kHz"
        );
        assert!(check(&[(152650, 15)], &allowed).is_ok(), "n28");
        assert!(
            check(&[(152650, 30)], &allowed).is_err(),
            "n28 is 15 kHz only"
        );
        assert!(
            check(&[(633984, 30)], &allowed).is_err(),
            "n78 not allowed by policy"
        );
        assert!(check(&[(504990, 60)], &allowed).is_err());
        assert!(check(&[(2079165, 120)], &allowed).is_err(), "FR2");
        let all = allowed_bands("", SELECTED).unwrap();
        assert!(check(&[(633984, 30)], &all).is_ok(), "unknown policy");
        assert!(
            check(&[(633984, 30)], &[78]).is_ok(),
            "n77 overlaps n78, but both are TDD"
        );
        assert!(
            check(&[(428910, 15)], &[1]).is_ok(),
            "n1 and n66 are both FDD"
        );
        assert!(check(&[(633984, 15)], &all).is_err(), "n78 is 30 kHz only");
        assert!(allowed_bands(POLICY, "ERROR").is_err());
    }
    #[tokio::test]
    async fn a_lock_made_while_the_guard_reverts_is_kept() {
        let at = At::start(true, vec![]).unwrap();
        // No service on any lock: the guard puts the previous lock back and checks it too.
        at.overrides.lock().unwrap().insert(
            "AT+QENG=\"servingcell\";+QCAINFO".into(),
            "+QENG: \"servingcell\",\"SEARCH\"".into(),
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("freq-lock.json");
        let lock = FreqLock::new(path.clone(), Arc::new(Store::new(true)), true);
        let events = Arc::new(Events::default());
        lock.lock(
            &at,
            &events,
            vec![(504990, 30)],
            vec![(513000, 30)],
            true,
            true,
        )
        .await
        .unwrap();
        let reverted = async {
            while parser::nr_freq_lock(&at.run(QUERY).await.unwrap()) != Some(vec![(513000, 30)]) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), reverted)
            .await
            .unwrap();
        // While the guard checks the previous lock, a new persistent lock replaces it.
        lock.lock(&at, &events, vec![(520000, 30)], vec![], true, false)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let saved: Saved = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.entries, [(520000, 30)]);
        assert_eq!(lock.snapshot(None)["phase"], "persistent");
        assert_eq!(
            parser::nr_freq_lock(&at.run(QUERY).await.unwrap()),
            Some(vec![(520000, 30)])
        );
    }
    #[test]
    fn a_seen_cell_decides_the_scs() {
        let cells = json!({"nr5g_cells_parsed":[
            {"freq":"504990","scs":30},{"freq":"513000","scs":null},{"freq":"x","scs":15}
        ]});
        let observed = observed_cells(&cells);
        assert_eq!(observed, [(504990, 30)]);
        assert!(check_observed(&[(504990, 30), (513000, 15)], &observed).is_ok());
        let error = check_observed(&[(504990, 15)], &observed).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the cell on NR-ARFCN 504990 uses 30 kHz, not 15 kHz"
        );
    }
}
