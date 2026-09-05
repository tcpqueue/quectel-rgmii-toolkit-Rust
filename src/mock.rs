use crate::{
    actions::Params,
    at::{At, DASHBOARD},
    parser,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
pub fn kind(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "at" | "all" | "full" | "dashboard" | "at-test" | "at_test_payload" => Some("dashboard"),
        "qca" | "qcainfo" | "qca-test" | "qcainfo-test" | "qcainfo_test_payload" => Some("qcainfo"),
        "qeng" | "qeng-test" | "qeng_test_payload" => Some("qeng"),
        "sms" => Some("sms"),
        _ => None,
    }
}
pub fn response(
    command: &str,
    fixtures: &HashMap<String, String>,
    overrides: &HashMap<String, String>,
) -> String {
    let source = if command == crate::at::SIGNAL {
        DASHBOARD
    } else {
        command
    };
    let mut value = fixtures
        .get(source)
        .cloned()
        .unwrap_or_else(|| "\r\nOK\r\n".into());
    if source == crate::at::SMS_LIST
        && let Some(sms) = overrides.get("sms")
    {
        return envelope(command, sms);
    }
    if let Some(dashboard) = overrides.get("dashboard").filter(|s| !s.is_empty())
        && source == DASHBOARD
    {
        return envelope(command, dashboard);
    }
    for (kind, prefix) in [("qcainfo", "+QCAINFO:"), ("qeng", "+QENG:")] {
        if let Some(payload) = overrides.get(kind).filter(|s| !s.is_empty())
            && (source == DASHBOARD
                || command
                    .to_ascii_uppercase()
                    .contains(prefix.trim_end_matches(':')))
        {
            let mut inserted = false;
            let mut lines = Vec::new();
            for line in value.lines() {
                if line.trim().to_ascii_uppercase().starts_with(prefix) {
                    if !inserted {
                        lines.push(payload.as_str());
                        inserted = true;
                    }
                    continue;
                }
                if !inserted && line.trim() == "OK" {
                    lines.push(payload);
                    inserted = true;
                }
                lines.push(line)
            }
            if !inserted {
                lines.push(payload)
            }
            value = lines.join("\n");
        }
    }
    if source == DASHBOARD {
        static START: std::sync::LazyLock<std::time::Instant> =
            std::sync::LazyLock::new(std::time::Instant::now);
        let seconds = START.elapsed().as_secs_f64();
        let rx = 536870912.0 + 2097152.0 * seconds + 8388608.0 * (1.0 - (seconds / 12.0).cos());
        let tx = 67108864.0 + 262144.0 * seconds + 1048576.0 * (1.0 - (seconds / 10.0).cos());
        value = value
            .lines()
            .map(|line| {
                if line.trim().starts_with("+QGDNRCNT:") {
                    format!("+QGDNRCNT: {},{}", rx as u64, tx as u64)
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    envelope(command, &value)
}
fn envelope(command: &str, raw: &str) -> String {
    let mut raw = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_owned();
    if !raw.to_ascii_uppercase().starts_with("AT") {
        raw = format!("{command}\n{raw}")
    }
    if !raw.lines().any(|l| l.trim() == "OK" || l.contains("ERROR")) {
        raw.push_str("\nOK")
    }
    raw.replace('\n', "\r\n") + "\r\n"
}
pub async fn handle(at: &At, p: &Params) -> Result<Value> {
    if !at.mock {
        bail!("mock mode only")
    }
    let action = p.get("action");
    match action {
        "set" | "save" => {
            let key = kind(p.get("kind"))
                .ok_or_else(|| anyhow::anyhow!("unknown mock AT payload kind"))?;
            at.overrides.lock().unwrap().insert(
                key.into(),
                p.get("payload")
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .trim()
                    .into(),
            );
        }
        "clear" | "reset" => {
            let mut map = at.overrides.lock().unwrap();
            if p.get("kind").is_empty() {
                map.clear()
            } else {
                let key = kind(p.get("kind"))
                    .ok_or_else(|| anyhow::anyhow!("unknown mock AT payload kind"))?;
                map.remove(key);
            }
        }
        "" | "show" | "status" | "parse" | "help" => {}
        _ => bail!("unsupported mock AT action"),
    }
    at.invalidate().await;
    let mut status = json!({});
    {
        let map = at.overrides.lock().unwrap();
        for key in ["dashboard", "qcainfo", "qeng", "sms"] {
            status[key] = json!(if map.get(key).is_some_and(|v| !v.is_empty()) {
                "manual"
            } else {
                "default"
            })
        }
    }
    let mut result = if action == "parse" {
        let raw = at.run(DASHBOARD).await?;
        let mut result = parser::dashboard(&raw);
        if p.flag("debug", false) {
            result["raw"] = json!(raw)
        }
        result
    } else {
        json!({})
    };
    result["ok"] = json!(true);
    result["status"] = status;
    if action == "set" || action == "save" {
        result["kind"] = json!(kind(p.get("kind")).unwrap())
    }
    if action == "help" {
        result["commands"] = json!([
            "SimpleAdmin.MockAT.at(`AT response`)",
            "SimpleAdmin.MockAT.qca(`QCAINFO response`)",
            "SimpleAdmin.MockAT.qeng(`QENG response`)",
            "SimpleAdmin.MockAT.parse()",
            "SimpleAdmin.MockAT.show()",
            "SimpleAdmin.MockAT.clear()"
        ])
    }
    Ok(result)
}
