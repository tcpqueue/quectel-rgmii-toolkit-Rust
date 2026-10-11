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
/// True when `AT+QCAINFO` lists a primary carrier, i.e. the module is camped on a cell.
pub fn has_carrier(raw: &str) -> bool {
    raw.lines()
        .filter_map(|line| line.trim().strip_prefix("+QCAINFO:"))
        .map(parser::fields)
        .any(|p| p.first().is_some_and(|k| k.eq_ignore_ascii_case("PCC")))
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
                    .run("AT+QCAINFO")
                    .await
                    .is_ok_and(|raw| has_carrier(&raw))
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
        assert!(has_carrier(
            "+QCAINFO: \"PCC\",428910,6,\"NR5G BAND 1\",751\r\nOK\r\n"
        ));
        assert!(!has_carrier("OK\r\n"));
        assert!(!has_carrier("+CME ERROR: 3\r\n"));
    }
}
