//! Cell scans as background jobs. A full scan (`AT+QSCAN`) takes minutes, so the page starts
//! it and polls; leaving the page does not stop it. The latest result of each kind stays in
//! memory for ten minutes so the page can show it again without scanning.
use serde_json::{Value, json};
use std::sync::Mutex;

/// Results older than this are dropped.
pub const KEEP_MS: u64 = 10 * 60 * 1000;
pub const KINDS: [&str; 2] = ["neighbour", "full"];

#[derive(Clone)]
struct Done {
    mode: String,
    started: u64,
    finished: u64,
    cells: Value,
    error: String,
}
#[derive(Clone)]
struct Running {
    kind: &'static str,
    mode: String,
    started: u64,
}
#[derive(Default)]
struct State {
    running: Option<Running>,
    results: [Option<Done>; 2],
}
#[derive(Default)]
pub struct Scans {
    state: Mutex<State>,
}
fn slot(kind: &str) -> Option<usize> {
    KINDS.iter().position(|k| *k == kind)
}
impl Scans {
    /// Marks a scan as started; false when one is already running (they share the AT port).
    pub fn begin(&self, kind: &str, mode: &str, now: u64) -> bool {
        let mut s = self.state.lock().unwrap();
        let Some(i) = slot(kind) else { return false };
        if s.running.is_some() {
            return false;
        }
        s.running = Some(Running {
            kind: KINDS[i],
            mode: mode.into(),
            started: now,
        });
        true
    }
    /// Stores the result of the running scan: cells on success, the error text otherwise.
    pub fn finish(&self, result: Result<Value, String>, now: u64) {
        let mut s = self.state.lock().unwrap();
        let Some(run) = s.running.take() else { return };
        let (cells, error) = match result {
            Ok(cells) => (cells, String::new()),
            Err(e) => (Value::Null, e),
        };
        s.results[slot(run.kind).unwrap()] = Some(Done {
            mode: run.mode,
            started: run.started,
            finished: now,
            cells,
            error,
        });
    }
    pub fn snapshot(&self, now: u64) -> Value {
        let mut s = self.state.lock().unwrap();
        for result in &mut s.results {
            if result
                .as_ref()
                .is_some_and(|r| now.saturating_sub(r.finished) > KEEP_MS)
            {
                *result = None;
            }
        }
        let results: serde_json::Map<String, Value> = KINDS
            .iter()
            .zip(&s.results)
            .map(|(kind, r)| {
                (
                    (*kind).to_owned(),
                    r.as_ref().map_or(Value::Null, |r| {
                        json!({"mode":r.mode,"started":r.started,"finished":r.finished,"cells":r.cells,"error":r.error,"expires":r.finished + KEEP_MS})
                    }),
                )
            })
            .collect();
        json!({
            "running": s.running.as_ref().map(|r| json!({"kind":r.kind,"mode":r.mode,"started":r.started})),
            "results": results,
            "now": now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_scan_at_a_time_and_results_expire() {
        let scans = Scans::default();
        assert!(scans.begin("full", "LTE Only", 1_000));
        assert!(!scans.begin("neighbour", "", 2_000), "the AT port is busy");
        assert_eq!(scans.snapshot(2_000)["running"]["kind"], "full");
        scans.finish(Ok(json!({"lte_cells_parsed":[1]})), 80_000);
        let v = scans.snapshot(90_000);
        assert!(v["running"].is_null());
        assert_eq!(v["results"]["full"]["mode"], "LTE Only");
        assert_eq!(v["results"]["full"]["cells"]["lte_cells_parsed"][0], 1);
        assert!(v["results"]["neighbour"].is_null());
        assert!(scans.begin("neighbour", "", 91_000));
        scans.finish(Err("timeout".into()), 92_000);
        assert_eq!(
            scans.snapshot(93_000)["results"]["neighbour"]["error"],
            "timeout"
        );
        let later = scans.snapshot(80_000 + KEEP_MS + 1);
        assert!(later["results"]["full"].is_null(), "kept ten minutes");
        assert!(!later["results"]["neighbour"].is_null());
        assert!(!scans.begin("other", "", 0));
    }
}
