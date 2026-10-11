//! Recent network events: radio changes seen in the dashboard samples, internet outages seen
//! by the latency probe, and actions taken by the band-lock guard and the watchdog. Kept in
//! memory only; a restart starts an empty list.
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Mutex};

const CAPACITY: usize = 200;

/// Radio state compared between consecutive dashboard samples.
#[derive(Clone, Default, PartialEq, Debug)]
struct Radio {
    /// "LTE", "NR5G-NSA", "NR5G-SA", "nosim" or "" when not registered.
    mode: String,
    pci: String,
    /// Primary band first, then secondary carriers, e.g. "N78 / B3".
    bands: String,
}
impl Radio {
    fn from_dashboard(data: &Value) -> Self {
        let text = |key: &str| {
            let v = data[key].as_str().unwrap_or("-");
            if v == "-" {
                String::new()
            } else {
                v.to_owned()
            }
        };
        let raw_mode = text("network_mode");
        let mode = if raw_mode == "未插卡" {
            "nosim".to_owned()
        } else {
            // "NR5G-SA FDD" -> "NR5G-SA"
            raw_mode.split_whitespace().next().unwrap_or("").to_owned()
        };
        Self {
            mode,
            pci: text("pcc_pci"),
            bands: text("bands"),
        }
    }
    fn primary_band(&self) -> &str {
        self.bands.split(" / ").next().unwrap_or("")
    }
}

struct State {
    list: VecDeque<Value>,
    next_id: u64,
    radio: Option<Radio>,
    /// Wall-clock time of the first failed probe of the current outage.
    outage_since: Option<u64>,
    outage_reported: bool,
}

pub struct Events {
    state: Mutex<State>,
    restored: tokio::sync::broadcast::Sender<Outage>,
}

/// A finished internet outage, in wall-clock milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outage {
    pub lost: u64,
    pub restored: u64,
}

/// An outage is reported once the probe has failed for this long.
pub const OUTAGE_REPORT_MS: u64 = 10_000;

impl Default for Events {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                list: VecDeque::with_capacity(CAPACITY),
                next_id: 1,
                radio: None,
                outage_since: None,
                outage_reported: false,
            }),
            restored: tokio::sync::broadcast::channel(8).0,
        }
    }
}

impl Events {
    /// Adds one event. `kind` selects the page text; `level` is "info", "warn" or "bad".
    pub fn push(&self, kind: &str, level: &str, detail: Value) {
        let mut state = self.state.lock().unwrap();
        Self::add(&mut state, kind, level, detail, crate::telemetry::now());
    }
    fn add(state: &mut State, kind: &str, level: &str, detail: Value, time: u64) {
        if state.list.len() == CAPACITY {
            state.list.pop_front();
        }
        let id = state.next_id;
        state.next_id += 1;
        state
            .list
            .push_back(json!({"id":id,"time":time,"kind":kind,"level":level,"detail":detail}));
    }
    /// Compares a dashboard sample with the previous one. The first sample only sets the baseline.
    pub fn observe_dashboard(&self, data: &Value) {
        self.observe_radio(Radio::from_dashboard(data), crate::telemetry::now())
    }
    fn observe_radio(&self, radio: Radio, time: u64) {
        let mut state = self.state.lock().unwrap();
        let Some(old) = state.radio.replace(radio.clone()) else {
            return;
        };
        if old == radio {
            return;
        }
        if old.mode != radio.mode {
            let (kind, level) = match (old.mode.as_str(), radio.mode.as_str()) {
                ("nosim", _) => ("sim_inserted", "info"),
                (_, "nosim") => ("sim_removed", "bad"),
                (_, "") => ("signal_lost", "bad"),
                ("", _) => ("signal_restored", "info"),
                _ => ("mode_change", "warn"),
            };
            Self::add(
                &mut state,
                kind,
                level,
                json!({"from":old.mode,"to":radio.mode,"bands":radio.bands,"pci":radio.pci}),
                time,
            );
            return;
        }
        if radio.mode.is_empty() || radio.mode == "nosim" {
            return;
        }
        if old.pci != radio.pci && !old.pci.is_empty() && !radio.pci.is_empty() {
            Self::add(
                &mut state,
                "cell_change",
                "info",
                json!({"from":old.pci,"to":radio.pci,"band":radio.primary_band(),"mode":radio.mode}),
                time,
            );
            return;
        }
        if old.bands != radio.bands && !radio.bands.is_empty() {
            let kind = if old.primary_band() != radio.primary_band() {
                "band_change"
            } else {
                "ca_change"
            };
            Self::add(
                &mut state,
                kind,
                "info",
                json!({"from":old.bands,"to":radio.bands}),
                time,
            );
        }
    }
    /// Feeds one latency probe result: reports an outage after `OUTAGE_REPORT_MS` of failures
    /// and its end with the duration.
    pub fn observe_probe(&self, ok: bool, time: u64) {
        let mut state = self.state.lock().unwrap();
        if ok {
            if let Some(since) = state.outage_since.take()
                && std::mem::take(&mut state.outage_reported)
            {
                let seconds = time.saturating_sub(since) / 1000;
                Self::add(
                    &mut state,
                    "internet_restored",
                    "info",
                    json!({"seconds":seconds}),
                    time,
                );
                let _ = self.restored.send(Outage {
                    lost: since,
                    restored: time,
                });
            }
            return;
        }
        let since = *state.outage_since.get_or_insert(time);
        if !state.outage_reported && time.saturating_sub(since) >= OUTAGE_REPORT_MS {
            state.outage_reported = true;
            Self::add(&mut state, "internet_lost", "bad", json!({}), since);
        }
    }
    /// Each reported outage once it ends (outages shorter than `OUTAGE_REPORT_MS` are not sent).
    pub fn outages(&self) -> tokio::sync::broadcast::Receiver<Outage> {
        self.restored.subscribe()
    }
    /// Start of the outage in progress once it has been reported, for the watchdog to keep
    /// across a module restart.
    pub fn outage_since(&self) -> Option<u64> {
        let state = self.state.lock().unwrap();
        state.outage_since.filter(|_| state.outage_reported)
    }
    /// Continues an outage that started before this program did (the watchdog restarted the
    /// module): it is already reported and ends with the next successful probe.
    pub fn resume_outage(&self, since: u64) {
        let mut state = self.state.lock().unwrap();
        state.outage_since = Some(since);
        state.outage_reported = true;
    }
    /// Forgets the outage in progress, e.g. when the probe target changes.
    pub fn reset_probe(&self) {
        let mut state = self.state.lock().unwrap();
        state.outage_since = None;
        state.outage_reported = false;
    }
    /// Newest first by event time. An outage is dated from its first failed probe but only
    /// recorded 10 s later, so insertion order is not time order.
    pub fn snapshot(&self) -> Value {
        let state = self.state.lock().unwrap();
        let mut list: Vec<Value> = state.list.iter().rev().cloned().collect();
        list.sort_by_key(|e| std::cmp::Reverse(e["time"].as_u64().unwrap_or(0)));
        json!({ "events": list })
    }
    pub fn clear(&self) {
        self.state.lock().unwrap().list.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn radio(mode: &str, pci: &str, bands: &str) -> Radio {
        Radio {
            mode: mode.into(),
            pci: pci.into(),
            bands: bands.into(),
        }
    }
    fn kinds(events: &Events) -> Vec<String> {
        events.snapshot()["events"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .map(|e| e["kind"].as_str().unwrap().to_owned())
            .collect()
    }
    #[test]
    fn radio_changes_become_events() {
        let e = Events::default();
        e.observe_radio(radio("NR5G-SA", "249", "N78"), 1);
        assert!(kinds(&e).is_empty(), "the first sample is the baseline");
        e.observe_radio(radio("NR5G-SA", "249", "N78"), 2);
        e.observe_radio(radio("NR5G-SA", "751", "N1"), 3);
        e.observe_radio(radio("NR5G-SA", "751", "N1 / N28"), 4);
        e.observe_radio(radio("NR5G-SA", "751", "N28 / N1"), 5);
        e.observe_radio(radio("LTE", "345", "B3"), 6);
        e.observe_radio(radio("", "", ""), 7);
        e.observe_radio(radio("LTE", "345", "B3"), 8);
        e.observe_radio(radio("nosim", "", ""), 9);
        e.observe_radio(radio("LTE", "345", "B3"), 10);
        assert_eq!(
            kinds(&e),
            [
                "cell_change",
                "ca_change",
                "band_change",
                "mode_change",
                "signal_lost",
                "signal_restored",
                "sim_removed",
                "sim_inserted"
            ]
        );
        let first = &e.snapshot()["events"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(first["detail"]["from"], "249");
        assert_eq!(first["detail"]["band"], "N1");
    }
    #[test]
    fn short_probe_failures_are_not_outages() {
        let e = Events::default();
        e.observe_probe(false, 1_000);
        e.observe_probe(false, 9_000);
        e.observe_probe(true, 10_000);
        assert!(kinds(&e).is_empty());
        e.observe_probe(false, 20_000);
        e.observe_probe(false, 30_000);
        e.observe_probe(false, 31_000);
        e.observe_probe(true, 65_000);
        assert_eq!(kinds(&e), ["internet_lost", "internet_restored"]);
        let list = e.snapshot()["events"].clone();
        assert_eq!(list[0]["detail"]["seconds"], 45);
        assert_eq!(
            list[1]["time"], 20_000,
            "the outage is dated from its first failure"
        );
    }
    #[test]
    fn finished_outages_are_broadcast() {
        let e = Events::default();
        let mut outages = e.outages();
        e.observe_probe(false, 20_000);
        e.observe_probe(false, 31_000);
        assert_eq!(e.outage_since(), Some(20_000));
        e.observe_probe(true, 65_000);
        assert_eq!(
            outages.try_recv().unwrap(),
            Outage {
                lost: 20_000,
                restored: 65_000
            }
        );
        e.resume_outage(100_000);
        e.observe_probe(true, 190_000);
        assert_eq!(outages.try_recv().unwrap().lost, 100_000);
        assert_eq!(kinds(&e).last().unwrap(), "internet_restored");
        assert!(outages.try_recv().is_err());
    }
    #[test]
    fn events_are_listed_by_time() {
        let e = Events::default();
        e.observe_radio(radio("LTE", "1", "B3"), 1_000);
        e.observe_probe(false, 20_000);
        e.observe_radio(radio("", "", ""), 26_000);
        e.observe_probe(false, 31_000);
        let times: Vec<u64> = e.snapshot()["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["time"].as_u64().unwrap())
            .collect();
        assert_eq!(
            times,
            [26_000, 20_000],
            "the outage sorts before the later signal loss"
        );
    }
    #[test]
    fn the_list_keeps_the_newest_events() {
        let e = Events::default();
        for i in 0..(CAPACITY as u64 + 5) {
            e.push("test", "info", json!({"i":i}));
        }
        let list = e.snapshot()["events"].as_array().unwrap().clone();
        assert_eq!(list.len(), CAPACITY);
        assert_eq!(list[0]["detail"]["i"], CAPACITY as u64 + 4);
    }
}
