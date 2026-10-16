//! Connection watchdog. When the latency probe has failed for the configured time, it walks
//! through the recovery steps the user allowed, in order, and stops at the first one after
//! which the probe succeeds again:
//!
//! 1. re-register on the network (`AT+COPS=2`, then `AT+COPS=0`);
//! 2. switch the radio off and on (`AT+CFUN=0`, `AT+CFUN=1`), putting cell and frequency
//!    locks back if the module dropped them;
//! 3. switch to the other SIM slot, and back again when that does not help;
//! 4. restart the module (`AT+CFUN=1,1`), at most `max_reboots` times until the connection
//!    has been seen working again.
//!
//! Off by default. Needs the latency probe; without it the watchdog only reports that.
use crate::{at::At, events::Events, parser, persistence::Store, telemetry::Monitor};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    /// Seconds of failed probes before recovery starts.
    pub outage_seconds: u32,
    /// Minutes to wait after a recovery attempt before another one may start.
    pub cooldown_minutes: u32,
    pub reregister: bool,
    pub radio: bool,
    pub sim_switch: bool,
    pub reboot: bool,
    /// Restarts allowed until the connection is seen working again.
    pub max_reboots: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            outage_seconds: 120,
            cooldown_minutes: 15,
            reregister: true,
            radio: true,
            sim_switch: false,
            reboot: false,
            max_reboots: 3,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !(30..=3600).contains(&self.outage_seconds) {
            bail!("outage time must be 30-3600 seconds")
        }
        if !(5..=240).contains(&self.cooldown_minutes) {
            bail!("cooldown must be 5-240 minutes")
        }
        if !(1..=10).contains(&self.max_reboots) {
            bail!("restart limit must be 1-10")
        }
        if self.enabled && self.steps().is_empty() {
            bail!("enable at least one recovery step")
        }
        Ok(())
    }
    /// Recovery steps in order, without the ones the user skipped.
    pub fn steps(&self) -> Vec<Step> {
        [
            (self.reregister, Step::Reregister),
            (self.radio, Step::Radio),
            (self.sim_switch, Step::SimSwitch),
            (self.reboot, Step::Reboot),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, step)| step)
        .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Reregister,
    Radio,
    SimSwitch,
    Reboot,
}
impl Step {
    pub fn tier(self) -> u32 {
        match self {
            Step::Reregister => 1,
            Step::Radio => 2,
            Step::SimSwitch => 3,
            Step::Reboot => 4,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Step::Reregister => "reregister",
            Step::Radio => "radio",
            Step::SimSwitch => "sim_switch",
            Step::Reboot => "reboot",
        }
    }
}

/// Waits of the real module; the mock server shortens them so tests finish quickly.
struct Timing {
    tick: Duration,
    startup: Duration,
    outage_unit: Duration,
    cooldown_unit: Duration,
    pause: Duration,
    reregister: Duration,
    radio: Duration,
    sim: Duration,
}
impl Timing {
    fn new(mock: bool) -> Self {
        if mock {
            Self {
                tick: Duration::from_millis(500),
                startup: Duration::from_secs(1),
                outage_unit: Duration::from_millis(50),
                cooldown_unit: Duration::from_secs(1),
                pause: Duration::ZERO,
                reregister: Duration::from_secs(2),
                radio: Duration::from_secs(2),
                sim: Duration::from_secs(2),
            }
        } else {
            Self {
                tick: Duration::from_secs(5),
                // Leave room for registration and dialing after boot.
                startup: Duration::from_secs(180),
                outage_unit: Duration::from_secs(1),
                cooldown_unit: Duration::from_secs(60),
                pause: Duration::from_secs(3),
                reregister: Duration::from_secs(45),
                radio: Duration::from_secs(60),
                sim: Duration::from_secs(90),
            }
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    reboots: u32,
    /// Start of the outage that led to a module restart, so the outage that continues after
    /// the restart keeps its start time (and its notification its full length).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outage_since: Option<u64>,
}

struct Runtime {
    phase: &'static str,
    step: Option<Step>,
    outage_seconds: u64,
    cooldown_until: Option<Instant>,
    reboots: u32,
    outage_since: Option<u64>,
}

pub struct Watchdog {
    settings: tokio::sync::watch::Sender<Settings>,
    runtime: Mutex<Runtime>,
    path: PathBuf,
    state_path: PathBuf,
    store: Arc<Store>,
    timing: Timing,
    change: tokio::sync::Mutex<()>,
    started: Instant,
}

impl Watchdog {
    pub fn new(path: PathBuf, mock: bool, store: Arc<Store>) -> Arc<Self> {
        let settings = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .filter(|s| s.validate().is_ok())
            .unwrap_or_default();
        let state_path = path.with_file_name("watchdog-state.json");
        let saved = std::fs::read(&state_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
            .unwrap_or_default();
        Arc::new(Self {
            settings: tokio::sync::watch::Sender::new(settings),
            runtime: Mutex::new(Runtime {
                phase: "off",
                step: None,
                outage_seconds: 0,
                cooldown_until: None,
                reboots: saved.reboots,
                outage_since: saved.outage_since,
            }),
            path,
            state_path,
            store,
            timing: Timing::new(mock),
            change: tokio::sync::Mutex::new(()),
            started: Instant::now(),
        })
    }
    pub fn settings(&self) -> Settings {
        *self.settings.borrow()
    }
    pub fn snapshot(&self) -> Value {
        let r = self.runtime.lock().unwrap();
        json!({
            "settings": self.settings(),
            "phase": r.phase,
            "tier": r.step.map(Step::tier),
            "step": r.step.map(Step::name),
            "outage_seconds": r.outage_seconds,
            "cooldown_seconds": r.cooldown_until.map(|t| t.saturating_duration_since(Instant::now()).as_secs()),
            "reboots": r.reboots,
        })
    }
    async fn write(&self, path: PathBuf, data: String) -> Result<()> {
        let store = self.store.clone();
        let result =
            tokio::task::spawn_blocking(move || store.write(&path, data.as_bytes(), 0o600)).await?;
        // A write that reached the disk but failed to sync still counts.
        match result {
            Err(e) if !e.committed => Err(e.into()),
            _ => Ok(()),
        }
    }
    pub async fn save(&self, settings: Settings) -> Result<()> {
        settings.validate()?;
        let _guard = self.change.lock().await;
        if self.settings() == settings {
            return Ok(());
        }
        self.write(
            self.path.clone(),
            format!("{}\n", serde_json::to_string(&settings)?),
        )
        .await?;
        self.settings.send_replace(settings);
        Ok(())
    }
    async fn set_reboots(&self, reboots: u32, outage_since: Option<u64>) {
        let changed = {
            let mut r = self.runtime.lock().unwrap();
            let changed = r.reboots != reboots || r.outage_since != outage_since;
            r.reboots = reboots;
            r.outage_since = outage_since;
            changed
        };
        if changed {
            let saved = Saved {
                reboots,
                outage_since,
            };
            let data = format!("{}\n", serde_json::to_string(&saved).unwrap_or_default());
            let _ = self.write(self.state_path.clone(), data).await;
        }
    }
    fn set_phase(&self, phase: &'static str, step: Option<Step>, outage_ms: u64) {
        let mut r = self.runtime.lock().unwrap();
        r.phase = phase;
        r.step = step;
        r.outage_seconds = outage_ms / 1000;
    }
    pub fn start(self: &Arc<Self>, at: At, monitor: Arc<Monitor>, events: Arc<Events>) {
        // An outage that led to a restart goes on until the probe succeeds again. A record left
        // while the watchdog was switched off afterwards is stale.
        let resumed = self.runtime.lock().unwrap().outage_since;
        if let Some(since) = resumed
            && self.settings().enabled
        {
            events.resume_outage(since);
        }
        let this = self.clone();
        tokio::spawn(async move {
            let mut changes = this.settings.subscribe();
            loop {
                let settings = *changes.borrow_and_update();
                let tick = this.timing.tick;
                // A working connection ends the restart series, during the startup wait too.
                if monitor.connected() == Some(true) {
                    this.set_reboots(0, None).await;
                }
                if !settings.enabled {
                    this.set_phase("off", None, 0);
                    // Switched off: no restart series continues.
                    this.set_reboots(0, None).await;
                    if changes.changed().await.is_err() {
                        return;
                    }
                    continue;
                }
                let cooling = {
                    let mut r = this.runtime.lock().unwrap();
                    if r.cooldown_until.is_some_and(|t| t <= Instant::now()) {
                        r.cooldown_until = None;
                    }
                    r.cooldown_until.is_some()
                };
                match monitor.outage_ms() {
                    _ if this.started.elapsed() < this.timing.startup => {
                        this.set_phase("startup", None, 0)
                    }
                    None => this.set_phase("no_probe", None, 0),
                    Some(0) => {
                        this.set_phase(if cooling { "cooldown" } else { "monitoring" }, None, 0)
                    }
                    Some(ms) if cooling => this.set_phase("cooldown", None, ms),
                    Some(ms)
                        if ms
                            < this.timing.outage_unit.as_millis() as u64
                                * u64::from(settings.outage_seconds) =>
                    {
                        this.set_phase("outage", None, ms)
                    }
                    Some(ms) => {
                        this.recover(&at, &monitor, &events, &changes, ms).await;
                        let mut r = this.runtime.lock().unwrap();
                        r.cooldown_until = Some(
                            Instant::now() + this.timing.cooldown_unit * settings.cooldown_minutes,
                        );
                        r.phase = "cooldown";
                        r.step = None;
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(tick) => {}
                    changed = changes.changed() => if changed.is_err() { return },
                }
            }
        });
    }
    fn connected(monitor: &Monitor) -> bool {
        monitor.outage_ms() == Some(0)
    }
    /// Waits up to `limit`, returning early once the probe succeeds again.
    async fn wait_for_connection(&self, monitor: &Monitor, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if Self::connected(monitor) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(self.timing.tick.min(deadline - Instant::now())).await;
        }
    }
    async fn recover(
        &self,
        at: &At,
        monitor: &Monitor,
        events: &Events,
        changes: &tokio::sync::watch::Receiver<Settings>,
        outage_ms: u64,
    ) {
        let settings = *changes.borrow();
        events.push(
            "watchdog_start",
            "warn",
            json!({"seconds": outage_ms / 1000}),
        );
        for step in settings.steps() {
            // Switched off or changed while recovering: stop here.
            if *changes.borrow() != settings {
                return;
            }
            self.set_phase("recovering", Some(step), monitor.outage_ms().unwrap_or(0));
            let (wait, result) = match step {
                Step::Reregister => (self.timing.reregister, self.reregister(at).await),
                Step::Radio => (self.timing.radio, self.radio(at, events).await),
                Step::SimSwitch => (self.timing.sim, self.switch_sim(at).await.map(|_| ())),
                Step::Reboot => {
                    let reboots = self.runtime.lock().unwrap().reboots;
                    if reboots >= settings.max_reboots {
                        events.push(
                            "watchdog_reboot_limit",
                            "bad",
                            json!({"limit": settings.max_reboots}),
                        );
                        continue;
                    }
                    self.set_reboots(reboots + 1, events.outage_since()).await;
                    events.push(
                        "watchdog_step",
                        "bad",
                        json!({"tier":step.tier(),"step":step.name(),"count":reboots + 1}),
                    );
                    let _ = at.run("AT+CFUN=1,1").await;
                    // The module restarts and takes this process with it.
                    tokio::time::sleep(Duration::from_secs(120)).await;
                    return;
                }
            };
            at.invalidate().await;
            match result {
                Ok(()) => events.push(
                    "watchdog_step",
                    "warn",
                    json!({"tier":step.tier(),"step":step.name()}),
                ),
                Err(e) => events.push(
                    "watchdog_step_failed",
                    "bad",
                    json!({"tier":step.tier(),"step":step.name(),"error":e.to_string()}),
                ),
            }
            if self.wait_for_connection(monitor, wait).await {
                events.push(
                    "watchdog_recovered",
                    "info",
                    json!({"tier":step.tier(),"step":step.name()}),
                );
                return;
            }
            if step == Step::SimSwitch {
                // The other slot did not help either: go back to where we started.
                if let Ok(slot) = self.switch_sim(at).await {
                    events.push("watchdog_sim_back", "warn", json!({"slot":slot}));
                }
                at.invalidate().await;
                tokio::time::sleep(self.timing.pause).await;
            }
        }
        events.push("watchdog_gave_up", "bad", json!({}));
    }
    async fn reregister(&self, at: &At) -> Result<()> {
        ok(at.run("AT+COPS=2").await?)?;
        tokio::time::sleep(self.timing.pause).await;
        ok(at.run("AT+COPS=0").await?)
    }
    /// Radio off and on. Cell locks are not saved in the module (save_ctrl 0) and may be
    /// dropped; any that disappeared are sent again with the values read before.
    async fn radio(&self, at: &At, events: &Events) -> Result<()> {
        const QUERY: &str = "AT+QNWLOCK=\"common/4g\";+QNWLOCK=\"common/5g\"";
        let before_raw = at.run(QUERY).await.unwrap_or_default();
        let before = parser::cell_locks(&before_raw);
        let before_freq =
            parser::nr_freq_lock(&at.run(crate::freq_lock::QUERY).await.unwrap_or_default())
                .unwrap_or_default();
        ok(at.run("AT+CFUN=0").await?)?;
        tokio::time::sleep(self.timing.pause + self.timing.pause).await;
        // The radio is off now; giving up after one failed answer would leave it off.
        let mut on = Err(anyhow::anyhow!("radio not switched on"));
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(self.timing.pause).await;
            }
            on = match at.run("AT+CFUN=1").await {
                Ok(raw) => ok(raw),
                Err(e) => Err(e),
            };
            if on.is_ok() {
                break;
            }
        }
        on?;
        tokio::time::sleep(self.timing.pause).await;
        if !before_freq.is_empty()
            && parser::nr_freq_lock(&at.run(crate::freq_lock::QUERY).await.unwrap_or_default())
                .is_some_and(|e| e.is_empty())
        {
            let restored = at
                .run(&crate::freq_lock::command(&before_freq))
                .await
                .is_ok_and(|raw| parser::ok(&raw));
            events.push(
                "watchdog_lock_restored",
                if restored { "info" } else { "bad" },
                json!({"radio":"nr_freq","ok":restored}),
            );
        }
        let after = parser::cell_locks(&at.run(QUERY).await.unwrap_or_default());
        for (radio, key) in [("lte", "common/4g"), ("nr", "common/5g")] {
            if let Some(values) = before[radio].as_array()
                && after[radio].is_null()
            {
                let list: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                let restored = at
                    .run(&format!("AT+QNWLOCK=\"{key}\",{}", list.join(",")))
                    .await
                    .is_ok_and(|raw| parser::ok(&raw));
                events.push(
                    "watchdog_lock_restored",
                    if restored { "info" } else { "bad" },
                    json!({"radio":radio,"ok":restored}),
                );
            }
        }
        Ok(())
    }
    /// Switches to the other SIM slot and returns the new slot.
    async fn switch_sim(&self, at: &At) -> Result<u32> {
        let raw = at.run("AT+QUIMSLOT?").await?;
        let slot = parser::lines(&raw)
            .filter_map(|l| l.trim().strip_prefix("+QUIMSLOT:"))
            .find_map(|v| v.trim().parse::<u32>().ok());
        let other = match slot {
            Some(1) => 2,
            Some(2) => 1,
            _ => bail!("cannot read the SIM slot"),
        };
        ok(at.run(&format!("AT+QUIMSLOT={other}")).await?)?;
        Ok(other)
    }
}
fn ok(raw: String) -> Result<()> {
    if parser::ok(&raw) {
        Ok(())
    } else {
        bail!("{}", raw.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn steps_follow_the_switches_in_order() {
        let mut s = Settings::default();
        assert!(!s.enabled, "off by default");
        assert_eq!(s.steps(), [Step::Reregister, Step::Radio]);
        s.reregister = false;
        s.sim_switch = true;
        s.reboot = true;
        assert_eq!(s.steps(), [Step::Radio, Step::SimSwitch, Step::Reboot]);
        assert_eq!(
            s.steps().iter().map(|s| s.tier()).collect::<Vec<_>>(),
            [2, 3, 4]
        );
    }
    #[tokio::test]
    async fn the_radio_is_switched_on_again_after_a_failed_answer() {
        let dir = tempfile::tempdir().unwrap();
        let watchdog = Watchdog::new(
            dir.path().join("watchdog.json"),
            true,
            Arc::new(Store::new(true)),
        );
        let at = At::start(true, vec![]).unwrap();
        at.overrides
            .lock()
            .unwrap()
            .insert("AT+CFUN=1".into(), "ERROR".into());
        assert!(watchdog.radio(&at, &Events::default()).await.is_err());
        let tries = at
            .trace
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "AT+CFUN=1")
            .count();
        assert_eq!(tries, 3);
    }
    #[tokio::test]
    async fn a_restart_record_is_dropped_while_switched_off() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("watchdog-state.json");
        std::fs::write(&state, r#"{"reboots":2,"outage_since":5000}"#).unwrap();
        let store = Arc::new(Store::new(true));
        let watchdog = Watchdog::new(dir.path().join("watchdog.json"), true, store.clone());
        let events = Arc::new(Events::default());
        let monitor = Monitor::new(dir.path().join("monitor.json"), true, store, events.clone());
        let at = At::start(true, vec![]).unwrap();
        watchdog.start(at, monitor, events.clone());
        assert_eq!(events.outage_since(), None, "a stale outage is not resumed");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let saved: Saved = serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
        assert_eq!((saved.reboots, saved.outage_since), (0, None));
    }
    #[test]
    fn settings_are_validated() {
        let mut s = Settings {
            enabled: true,
            ..Settings::default()
        };
        assert!(s.validate().is_ok());
        s.outage_seconds = 10;
        assert!(s.validate().is_err());
        s.outage_seconds = 120;
        s.max_reboots = 0;
        assert!(s.validate().is_err());
        s.max_reboots = 3;
        s.reregister = false;
        s.radio = false;
        assert!(s.validate().is_err(), "an enabled watchdog needs a step");
        s.enabled = false;
        assert!(s.validate().is_ok());
        assert!(
            serde_json::from_str::<Settings>(r#"{"enabled":true,"extra":1}"#).is_err(),
            "unknown fields are rejected"
        );
    }
}
