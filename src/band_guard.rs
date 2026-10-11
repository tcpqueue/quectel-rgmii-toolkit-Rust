//! Band-lock guard: after a band lock, watch for a serving carrier and put the previous band
//! list back when none shows up. Mirrors the three-minute fallback of persistent cell locks,
//! but for the band preference, which the module saves by itself.
use crate::{at::At, events::Events, parser};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// Wait before the first check while the module re-selects a cell.
const SETTLE: Duration = Duration::from_secs(5);
const INTERVAL: Duration = Duration::from_secs(5);
/// Checks after the settle time; any carrier ends the watch. 5 s + 5 x 5 s = 30 s.
const CHECKS: u32 = 5;

#[derive(Default)]
struct State {
    generation: u64,
    status: Value,
}
#[derive(Default)]
pub struct BandGuard {
    state: Mutex<State>,
}

/// Band list in an `AT+QNWPREFCFG="<mode>"` answer, e.g. `1:3:5:8`.
pub fn current_bands(raw: &str, mode: &str) -> Option<String> {
    raw.lines()
        .filter_map(|line| line.trim().strip_prefix("+QNWPREFCFG:"))
        .map(parser::fields)
        .find(|p| p.len() > 1 && p[0].eq_ignore_ascii_case(mode))
        .map(|p| p[1].clone())
        .filter(|v| !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit() || c == b':'))
}
/// Carriers in an `AT+QCAINFO` answer as (primary, NR, band).
fn carriers(raw: &str) -> Vec<(bool, bool, u32)> {
    raw.lines()
        .filter_map(|line| line.trim().strip_prefix("+QCAINFO:"))
        .map(parser::fields)
        .filter(|p| p.len() > 3)
        .filter_map(|p| {
            let name = p[3].to_ascii_uppercase();
            let band = name.rsplit("BAND").next()?.trim().parse().ok()?;
            Some((p[0].eq_ignore_ascii_case("PCC"), name.contains("NR"), band))
        })
        .collect()
}
/// True when `AT+QENG="servingcell"` reports a registered state. A module that loses its
/// usable bands may still camp somewhere in limited service (`LIMSRV`, emergency calls only),
/// which is no connection.
pub fn registered(raw: &str) -> bool {
    raw.lines()
        .filter_map(|line| line.trim().strip_prefix("+QENG:"))
        .map(parser::fields)
        .find(|p| p.first().is_some_and(|k| k == "servingcell"))
        .and_then(|p| p.get(1).cloned())
        .is_some_and(|state| state == "NOCONN" || state == "CONNECT")
}
/// The guard's check: registered, and serving on the locked bands.
pub fn locked_and_registered(raw: &str, mode: &str, locked: &str) -> bool {
    registered(raw) && serving_on(raw, mode, locked)
}
/// True when the module serves on the locked bands, judged from `AT+QCAINFO`.
///
/// Right after a lock the module may still show the old cell for a few seconds, so a carrier
/// of the locked radio counts only on one of the `locked` bands. A lock of a radio that is not
/// in use (SA bands while on LTE or NSA, NSA bands while on SA) leaves the connection alone:
/// any primary carrier then counts.
pub fn serving_on(raw: &str, mode: &str, locked: &str) -> bool {
    let carriers = carriers(raw);
    let Some(&(_, primary_nr, _)) = carriers.iter().find(|c| c.0) else {
        return false;
    };
    let locked: Vec<u32> = locked.split(':').filter_map(|b| b.parse().ok()).collect();
    let relevant: Vec<u32> = carriers
        .iter()
        .filter(|(_, nr, _)| match mode {
            "lte_band" => !nr,
            // SA bands matter only while the primary carrier is NR, NSA bands only under LTE.
            "nr5g_band" => *nr && primary_nr,
            _ => *nr && !primary_nr,
        })
        .map(|c| c.2)
        .collect();
    relevant.is_empty() || relevant.iter().any(|b| locked.contains(b))
}

impl BandGuard {
    pub fn snapshot(&self) -> Value {
        let status = self.state.lock().unwrap().status.clone();
        if status.is_null() {
            json!({"phase":"idle"})
        } else {
            status
        }
    }
    /// A new band change replaces any watch in progress.
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        state.generation += 1;
        if state.status["phase"] == "checking" {
            state.status["phase"] = json!("cancelled");
        }
    }
    /// Reads the current list for `mode` (an `AT+QNWPREFCFG` key) before a lock, so the
    /// guard knows what to restore.
    pub async fn previous(at: &At, mode: &str) -> Result<String> {
        let raw = at.run(&format!("AT+QNWPREFCFG=\"{mode}\"")).await?;
        match current_bands(&raw, mode) {
            Some(bands) => Ok(bands),
            None => bail!("cannot read the current {mode} setting"),
        }
    }
    /// Starts watching after a successful lock of `mode` to `locked`; restores `previous`
    /// when no carrier appears within 30 s.
    pub fn watch(
        self: &Arc<Self>,
        at: At,
        events: Arc<Events>,
        mode: &'static str,
        locked: String,
        previous: String,
    ) {
        let generation = {
            let mut state = self.state.lock().unwrap();
            state.generation += 1;
            state.status = json!({"phase":"checking","mode":mode,"locked":locked,"previous":previous,"started":crate::telemetry::now()});
            state.generation
        };
        let this = self.clone();
        tokio::spawn(async move {
            let current = || this.state.lock().unwrap().generation == generation;
            tokio::time::sleep(SETTLE).await;
            for check in 0..CHECKS {
                if check > 0 {
                    tokio::time::sleep(INTERVAL).await;
                }
                if !current() {
                    return;
                }
                if at
                    .run("AT+QENG=\"servingcell\";+QCAINFO")
                    .await
                    .is_ok_and(|raw| locked_and_registered(&raw, mode, &locked))
                {
                    let mut state = this.state.lock().unwrap();
                    if state.generation == generation {
                        state.status["phase"] = json!("ok");
                    }
                    return;
                }
            }
            if !current() {
                return;
            }
            let restored = at
                .run(&format!("AT+QNWPREFCFG=\"{mode}\",{previous}"))
                .await
                .is_ok_and(|raw| parser::ok(&raw));
            at.invalidate().await;
            let phase = if restored { "reverted" } else { "failed" };
            {
                let mut state = this.state.lock().unwrap();
                if state.generation == generation {
                    state.status["phase"] = json!(phase);
                }
            }
            events.push(
                if restored {
                    "band_reverted"
                } else {
                    "band_revert_failed"
                },
                if restored { "warn" } else { "bad" },
                json!({"mode":mode,"locked":locked,"previous":previous}),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_band_lists_and_carriers() {
        let raw = "+QNWPREFCFG: \"nr5g_band\",1:28:41:78\r\nOK\r\n";
        assert_eq!(
            current_bands(raw, "nr5g_band").as_deref(),
            Some("1:28:41:78")
        );
        assert_eq!(current_bands(raw, "lte_band"), None);
        assert_eq!(current_bands("ERROR", "nr5g_band"), None);
        assert!(!serving_on("OK\r\n", "lte_band", "3"));
        assert!(!serving_on("+CME ERROR: 3\r\n", "lte_band", "3"));
    }
    #[test]
    fn only_a_carrier_on_the_locked_bands_counts() {
        let sa = "+QCAINFO: \"PCC\",428910,6,\"NR5G BAND 1\",751\r\nOK\r\n";
        assert!(serving_on(sa, "nr5g_band", "1:28"));
        assert!(
            !serving_on(sa, "nr5g_band", "28:41"),
            "still on the old band"
        );
        assert!(
            serving_on(sa, "lte_band", "5"),
            "an LTE lock leaves SA alone"
        );
        assert!(
            serving_on(sa, "nsa_nr5g_band", "41"),
            "an NSA lock leaves SA alone"
        );
        let nsa = "+QCAINFO: \"PCC\",1300,100,\"LTE BAND 3\",262,-93,-13,-60,15\r\n\
                   +QCAINFO: \"SCC\",504990,12,\"NR5G BAND 41\",108\r\nOK\r\n";
        assert!(serving_on(nsa, "lte_band", "3:8"));
        assert!(
            !serving_on(nsa, "lte_band", "5"),
            "the old LTE cell is still shown"
        );
        assert!(serving_on(nsa, "nsa_nr5g_band", "41"));
        assert!(!serving_on(nsa, "nsa_nr5g_band", "78"));
        assert!(
            serving_on(nsa, "nr5g_band", "28"),
            "an SA lock leaves NSA alone"
        );
        let lte_only = "+QCAINFO: \"PCC\",1300,100,\"LTE BAND 3\",262\r\nOK\r\n";
        assert!(
            serving_on(lte_only, "nsa_nr5g_band", "78"),
            "no NR leg yet is not an outage"
        );
    }
    #[test]
    fn limited_service_is_no_connection() {
        // Captured on an RM520N-EU after locking LTE to B5, which China Mobile does not use:
        // the module camps on SA n41 in limited service.
        let limited = "+QENG: \"servingcell\",\"LIMSRV\",\"NR5G-SA\",\"TDD\",460,00,22382F001,108,58D006,504990,41,12,-85,-10,-20,0,0\r\nOK\r\n\
                       +QCAINFO: \"PCC\",504990,12,\"NR5G BAND 41\",108\r\nOK\r\n";
        assert!(!registered(limited));
        assert!(!locked_and_registered(limited, "lte_band", "5"));
        let nsa = "+QENG: \"servingcell\",\"NOCONN\"\r\n\
                   +QENG: \"LTE\",\"FDD\",460,00,F581B57,262,1300,3,5,5,99D2,-96,-14,-60,14,14,180,-\r\nOK\r\n\
                   +QCAINFO: \"PCC\",1300,100,\"LTE BAND 3\",1,262,-95,-12,-58,4\r\n\
                   +QCAINFO: \"SCC\",504990,12,\"NR5G BAND 41\",108\r\nOK\r\n";
        assert!(registered(nsa));
        assert!(locked_and_registered(nsa, "lte_band", "3:8"));
        assert!(
            !locked_and_registered(nsa, "lte_band", "5"),
            "still on the old B3 cell"
        );
    }
}
