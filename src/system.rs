use crate::{parser::human_bytes, persistence::Store};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path, sync::Mutex, time::Duration};

#[derive(Default)]
pub struct Metrics {
    cpu: Mutex<Option<(u64, u64)>>,
}
impl Metrics {
    pub fn read(&self, mock: bool) -> Value {
        if mock {
            return json!({"cpuUsagePercent":36,"ramUsagePercent":58,"ramUsedHuman":"1.2 GB","ramTotalHuman":"2.0 GB"});
        }
        let mut percent = 0;
        if let Ok(raw) = std::fs::read_to_string("/proc/stat") {
            let values: Vec<u64> = raw
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .skip(1)
                .take(8)
                .filter_map(|v| v.parse().ok())
                .collect();
            if values.len() >= 4 {
                let total = values.iter().sum::<u64>();
                let idle = values[3] + values.get(4).copied().unwrap_or(0);
                let mut prev = self.cpu.lock().unwrap();
                if let Some((old_total, old_idle)) = *prev {
                    let dt = total.saturating_sub(old_total);
                    let di = idle.saturating_sub(old_idle);
                    percent = (100 * dt.saturating_sub(di) + dt / 2)
                        .checked_div(dt)
                        .unwrap_or(0);
                }
                *prev = Some((total, idle));
            }
        }
        let raw = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let mut values = HashMap::new();
        for line in raw.lines() {
            let mut p = line.split_whitespace();
            if let (Some(k), Some(v)) = (p.next(), p.next()) {
                values.insert(k, v.parse::<u64>().unwrap_or(0));
            }
        }
        let n = |k| values.get(k).copied().unwrap_or(0);
        let total = n("MemTotal:");
        let available = values.get("MemAvailable:").copied().unwrap_or_else(|| {
            (n("MemFree:") + n("Buffers:") + n("Cached:") + n("SReclaimable:"))
                .saturating_sub(n("Shmem:"))
        });
        let used = total.saturating_sub(available);
        json!({"cpuUsagePercent":percent.min(100),"ramUsagePercent":(100*used+total/2).checked_div(total).unwrap_or(0),"ramUsedHuman":human_bytes(used as f64*1024.0),"ramTotalHuman":human_bytes(total as f64*1024.0)})
    }
}
pub fn uptime(mock: bool) -> (Value, String) {
    let seconds = if mock {
        93000
    } else {
        std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
            .unwrap_or(0.0) as u64
    };
    let (d, h, m, s) = (
        seconds / 86400,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
    );
    (
        json!({"days":d,"hours":h,"minutes":m,"seconds":s}),
        format!("{d} days, {h} hours, {m} minutes, {s} seconds"),
    )
}
pub async fn command(name: &str, args: &[&str]) -> Result<String> {
    let mut cmd = tokio::process::Command::new(name);
    cmd.args(args)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    let output = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .context("system command timeout")??;
    if !output.status.success() {
        bail!(
            "{name} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
pub fn ttl(path: &Path) -> u8 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}
pub async fn apply_ttl(value: u8, mock: bool) -> Result<Vec<String>> {
    if mock {
        return Ok(vec![format!("TTL {value} applied (mock)")]);
    }
    let mut logs = Vec::new();
    for (name, target, flag) in [
        ("iptables", "TTL", "--ttl-set"),
        ("ip6tables", "HL", "--hl-set"),
    ] {
        let rules = command(name, &["-t", "mangle", "-S", "POSTROUTING"]).await?;
        for line in rules.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let pair = |a, b| fields.windows(2).any(|w| w == [a, b]);
            if fields.len() >= 8
                && fields[0] == "-A"
                && fields[1] == "POSTROUTING"
                && pair("-o", "rmnet+")
                && pair("-j", target)
                && fields.contains(&flag)
            {
                let mut args = vec!["-t", "mangle", "-D"];
                args.extend_from_slice(&fields[1..]);
                command(name, &args).await?;
            }
        }
        if value > 0 {
            command(
                name,
                &[
                    "-t",
                    "mangle",
                    "-I",
                    "POSTROUTING",
                    "-o",
                    "rmnet+",
                    "-j",
                    target,
                    flag,
                    &value.to_string(),
                ],
            )
            .await?;
        }
        logs.push(format!("{name}: TTL {value}"));
    }
    Ok(logs)
}
pub async fn set_ttl(
    value: u8,
    mock: bool,
    path: &Path,
    store: std::sync::Arc<Store>,
) -> Result<Vec<String>> {
    let old = ttl(path);
    let mut logs = match apply_ttl(value, mock).await {
        Ok(logs) => logs,
        Err(e) => {
            let _ = apply_ttl(old, mock).await;
            return Err(e);
        }
    };
    let path = path.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        store.write(&path, format!("{value}\n").as_bytes(), 0o644)
    })
    .await?;
    if let Err(e) = result {
        if !e.committed {
            let _ = apply_ttl(old, mock).await;
        }
        return Err(e.into());
    }
    logs.push("TTL value saved".into());
    Ok(logs)
}
