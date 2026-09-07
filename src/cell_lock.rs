use crate::{
    actions::{self, Params},
    at::At,
    parser,
    persistence::Store,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const GUARD: Duration = Duration::from_secs(180);
const PREFIXES: [&str; 2] = ["AT+QNWLOCK=\"common/4g\",", "AT+QNWLOCK=\"common/5g\","];

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Rule {
    values: Vec<u32>,
    auto_unlock: bool,
    enabled: bool,
}
impl Rule {
    fn command(&self, radio: usize) -> Result<String> {
        let v = &self.values;
        let valid = if radio == 0 {
            v.first()
                .is_some_and(|n| (1..=10).contains(n) && v.len() == 1 + *n as usize * 2)
                && v[1..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|p| p[0] <= 262143 && p[1] <= 503)
        } else {
            v.len() == 4
                && v[0] <= 1007
                && v[1] <= 3279165
                && [15, 30, 60, 120, 240].contains(&v[2])
                && (1..=1024).contains(&v[3])
        };
        if !valid {
            bail!("invalid saved cell lock")
        }
        Ok(format!(
            "{}{}",
            PREFIXES[radio],
            v.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
        ))
    }
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    rules: [Option<Rule>; 2],
}
#[derive(Default)]
struct Runtime {
    phase: &'static str,
    error: String,
    deadline: Option<Instant>,
    touched: bool,
}
struct State {
    settings: Settings,
    runtime: [Runtime; 2],
}
pub struct CellLock {
    state: Mutex<State>,
    mutation: tokio::sync::Mutex<()>,
    changed: tokio::sync::Notify,
    path: PathBuf,
    store: Arc<Store>,
}
impl CellLock {
    pub fn new(path: PathBuf, store: Arc<Store>) -> Self {
        let loaded = (|| -> Result<Settings> {
            if !path.exists() {
                return Ok(Settings::default());
            }
            let bytes = std::fs::read(&path)?;
            if bytes.len() > 4096 {
                bail!("cell lock file too large");
            }
            let settings: Settings = serde_json::from_slice(&bytes)?;
            for (radio, rule) in settings.rules.iter().enumerate() {
                if let Some(rule) = rule {
                    rule.command(radio)?;
                }
            }
            Ok(settings)
        })();
        let error = loaded
            .as_ref()
            .err()
            .map(|_| "Cannot load persistent cell locks".to_owned())
            .unwrap_or_default();
        Self {
            state: Mutex::new(State {
                settings: loaded.unwrap_or_default(),
                runtime: std::array::from_fn(|_| Runtime {
                    error: error.clone(),
                    ..Runtime::default()
                }),
            }),
            mutation: tokio::sync::Mutex::new(()),
            changed: tokio::sync::Notify::new(),
            path,
            store,
        }
    }
    pub fn snapshot(&self) -> Value {
        let state = self.state.lock().unwrap();
        json!({"radios": (0..2).map(|i| {
            let r = &state.runtime[i];
            json!({"radio":if i==0 {"lte"} else {"nr"}, "persistent":state.settings.rules[i].as_ref().is_some_and(|r| r.enabled), "auto_unlock":state.settings.rules[i].as_ref().is_some_and(|r| r.auto_unlock), "phase":r.phase, "error":r.error, "remaining_seconds":r.deadline.map(|t| t.saturating_duration_since(Instant::now()).as_secs()), "values":state.settings.rules[i].as_ref().map(|r| &r.values)})
        }).collect::<Vec<_>>()})
    }
    async fn persist(&self, settings: Settings) -> Result<()> {
        if self.state.lock().unwrap().settings == settings {
            return Ok(());
        }
        let path = self.path.clone();
        let store = self.store.clone();
        let bytes = serde_json::to_vec(&settings)?;
        let result = tokio::task::spawn_blocking(move || store.write(&path, &bytes, 0o600)).await?;
        if result.is_ok() || result.as_ref().is_err_and(|e| e.committed) {
            self.state.lock().unwrap().settings = settings;
        }
        result?;
        Ok(())
    }
    pub fn handles(action: &str) -> bool {
        matches!(
            action,
            "lock_lte_manual"
                | "lock_nr_manual"
                | "lock_scanned_cells"
                | "unlock_lte"
                | "unlock_nr"
        )
    }
    pub async fn apply(&self, at: &At, p: &Params) -> Result<Value> {
        let command = actions::network(p)?;
        let radio = PREFIXES
            .iter()
            .position(|prefix| command.starts_with(prefix))
            .ok_or_else(|| anyhow::anyhow!("unsupported cell lock"))?;
        let unlock = matches!(p.get("action"), "unlock_lte" | "unlock_nr");
        let persistent = match p.get("persistence") {
            "" | "temporary" => false,
            "persistent" => true,
            _ => bail!("invalid lock persistence"),
        };
        let rule = if unlock {
            None
        } else {
            let values = command[PREFIXES[radio].len()..]
                .split(',')
                .map(str::parse)
                .collect::<std::result::Result<Vec<u32>, _>>()?;
            let r = Rule {
                values,
                auto_unlock: persistent && p.flag("auto_unlock", false),
                enabled: true,
            };
            r.command(radio)?;
            Some(r)
        };
        let _guard = self.mutation.lock().await;
        let previous = self.state.lock().unwrap().settings.clone();
        let response = at.run(&command).await?;
        if !parser::ok(&response) {
            bail!("cell lock rejected: {response}");
        }
        let mut next = previous.clone();
        next.rules[radio] = if persistent { rule.clone() } else { None };
        let mut warning = None;
        if let Err(error) = self.persist(next).await {
            if error
                .downcast_ref::<crate::persistence::SavedError>()
                .is_some_and(|e| e.committed)
            {
                warning = Some(error.to_string());
            } else {
                let restore = previous.rules[radio]
                    .as_ref()
                    .filter(|r| r.enabled)
                    .map(|r| r.command(radio))
                    .transpose()?
                    .unwrap_or_else(|| format!("{}0", PREFIXES[radio]));
                let _ = at.run(&restore).await;
                at.invalidate().await;
                bail!("cell lock settings save failed: {error}");
            }
        }
        {
            let mut state = self.state.lock().unwrap();
            state.runtime[radio] = Runtime {
                phase: if unlock {
                    "unlocked"
                } else if persistent {
                    "persistent"
                } else {
                    "temporary"
                },
                deadline: rule
                    .filter(|r| r.auto_unlock)
                    .map(|_| Instant::now() + GUARD),
                touched: true,
                error: String::new(),
            };
        }
        at.invalidate().await;
        self.changed.notify_one();
        Ok(json!({"ok":true,"response":response,"warning":warning,"cell_lock":self.snapshot()}))
    }
    pub fn start(self: &Arc<Self>, at: At) {
        let this = self.clone();
        tokio::spawn(async move {
            at.wait_ready().await;
            this.restore(&at).await;
            loop {
                let active = this
                    .state
                    .lock()
                    .unwrap()
                    .runtime
                    .iter()
                    .any(|r| r.deadline.is_some());
                if active {
                    tokio::select! { _ = tokio::time::sleep(Duration::from_secs(5)) => (), _ = this.changed.notified() => () }
                } else {
                    this.changed.notified().await;
                }
                this.check(&at).await;
            }
        });
    }
    async fn restore(&self, at: &At) {
        let _guard = self.mutation.lock().await;
        for radio in 0..2 {
            let rule = {
                let state = self.state.lock().unwrap();
                if state.runtime[radio].touched {
                    continue;
                }
                state.settings.rules[radio].clone()
            };
            let Some(rule) = rule.filter(|r| r.enabled) else {
                continue;
            };
            let result = match rule.command(radio) {
                Ok(command) => at.run(&command).await,
                Err(e) => Err(e),
            };
            let error = match result {
                Ok(raw) if parser::ok(&raw) => String::new(),
                _ => "Persistent cell lock restore failed".into(),
            };
            self.state.lock().unwrap().runtime[radio] = Runtime {
                phase: "persistent",
                error,
                touched: true,
                deadline: rule.auto_unlock.then(|| Instant::now() + GUARD),
            };
        }
        at.invalidate().await;
    }
    async fn check(&self, at: &At) {
        let _guard = self.mutation.lock().await;
        if !self
            .state
            .lock()
            .unwrap()
            .runtime
            .iter()
            .any(|r| r.deadline.is_some())
        {
            return;
        }
        // Query the modem's data session directly; DNS or an unreachable ping target is not a dial failure.
        let connected = at
            .run("AT+QMAP=\"WWAN\"")
            .await
            .ok()
            .is_some_and(|raw| dialed(&raw));
        for (radio, prefix) in PREFIXES.iter().enumerate() {
            let deadline = self.state.lock().unwrap().runtime[radio].deadline;
            let Some(deadline) = deadline else {
                continue;
            };
            let recovering = self.state.lock().unwrap().runtime[radio].phase == "fallback_error";
            if connected
                && !recovering
                && Instant::now() + GUARD - Duration::from_secs(15) >= deadline
            {
                let mut state = self.state.lock().unwrap();
                state.runtime[radio].deadline = None;
                state.runtime[radio].phase = "connected";
            } else if Instant::now() >= deadline {
                // Disable reboot restoration first. An interrupted unlock will be retried while running.
                let mut next = self.state.lock().unwrap().settings.clone();
                if let Some(rule) = &mut next.rules[radio] {
                    rule.enabled = false;
                }
                let saved = self.persist(next).await;
                let unlocked = at
                    .run(&format!("{prefix}0"))
                    .await
                    .ok()
                    .is_some_and(|raw| parser::ok(&raw));
                at.invalidate().await;
                let mut state = self.state.lock().unwrap();
                let r = &mut state.runtime[radio];
                if saved.is_ok() && unlocked {
                    r.deadline = None;
                    r.phase = "fallback";
                    r.error.clear();
                } else {
                    r.phase = "fallback_error";
                    r.error = if saved.is_err() {
                        "Cannot disable persistent cell lock"
                    } else {
                        "Cell unlock failed; retrying"
                    }
                    .into();
                }
            }
        }
    }
}
fn dialed(raw: &str) -> bool {
    parser::ok(raw)
        && raw
            .lines()
            .filter_map(|line| line.trim().strip_prefix("+QMAP:"))
            .any(|line| {
                let fields = parser::fields(line);
                fields.len() >= 5
                    && fields[0].eq_ignore_ascii_case("WWAN")
                    && matches!(fields[3].as_str(), "IPV4" | "IPV6")
                    && fields[4].parse::<IpAddr>().is_ok_and(|ip| {
                        !ip.is_unspecified()
                            && !ip.is_loopback()
                            && !ip.is_multicast()
                            && match ip {
                                IpAddr::V4(ip) => !ip.is_link_local(),
                                IpAddr::V6(ip) => !ip.is_unicast_link_local(),
                            }
                    })
            })
}

#[cfg(test)]
#[path = "../tests/cell_lock/mod.rs"]
mod tests;
