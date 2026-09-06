use crate::{at::At, forwarding::identity, parser, persistence::Store, sms};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const DAY: Duration = Duration::from_secs(86400);
const VALID_CLOCK: i64 = 1577836800; // 2020-01-01

#[derive(Clone, Deserialize, Serialize)]
pub struct Part {
    pub index: u16,
    pub fingerprint: [u8; 16],
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Receipt {
    pub id: String,
    pub delivered: i64,
    pub parts: Vec<Part>,
}
struct State {
    receipts: Vec<Receipt>,
    generation: u64,
    saved: u64,
    error: String,
    ages: HashMap<String, Instant>,
    clock: (i64, Instant),
    clock_warning: bool,
}
pub struct Cleanup {
    state: Mutex<State>,
    path: PathBuf,
    store: Arc<Store>,
}
impl Cleanup {
    pub fn new(path: PathBuf, store: Arc<Store>) -> Self {
        let loaded = (|| -> Result<Vec<Receipt>> {
            match std::fs::metadata(&path) {
                Ok(m) if m.len() > 65536 => bail!("receipt file too large"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(e) => return Err(e.into()),
                _ => {}
            }
            let receipts: Vec<Receipt> = serde_json::from_slice(&std::fs::read(&path)?)?;
            if receipts.len() > 64
                || receipts.iter().map(|r| r.parts.len()).sum::<usize>() > 512
                || receipts
                    .iter()
                    .any(|r| r.parts.is_empty() || r.delivered <= 0 || r.id.len() > 64)
            {
                bail!("invalid receipts")
            }
            Ok(receipts)
        })();
        let (receipts, error) = match loaded {
            Ok(r) => (r, String::new()),
            Err(_) => (Vec::new(), "cannot read deletion receipts".into()),
        };
        Self {
            state: Mutex::new(State {
                ages: receipts
                    .iter()
                    .map(|r| (r.id.clone(), Instant::now()))
                    .collect(),
                receipts,
                generation: 0,
                saved: 0,
                error,
                clock: (chrono::Utc::now().timestamp(), Instant::now()),
                clock_warning: false,
            }),
            path,
            store,
        }
    }
    pub fn status(&self) -> Value {
        let s = self.state.lock().unwrap();
        serde_json::json!({"pending":s.receipts.len(),"clock_paused":s.clock_warning,"error":if s.clock_warning { "system clock changed or unavailable; deletion waits for 24 hours of continuous uptime" } else { &s.error }})
    }
    pub fn schedule(&self, receipt: Receipt) {
        let mut s = self.state.lock().unwrap();
        if s.receipts.iter().any(|r| r.id == receipt.id) {
            return;
        }
        if s.receipts.len() >= 64
            || s.receipts.iter().map(|r| r.parts.len()).sum::<usize>() + receipt.parts.len() > 512
            || receipt.parts.is_empty()
        {
            s.error = "deletion receipt capacity exceeded".into();
            return;
        }
        s.ages.insert(receipt.id.clone(), Instant::now());
        s.receipts.push(receipt);
        s.generation += 1;
    }
    pub fn cancel(&self) {
        let mut s = self.state.lock().unwrap();
        if !s.receipts.is_empty() {
            s.receipts.clear();
            s.ages.clear();
            s.generation += 1;
        }
    }
    pub async fn flush(&self) {
        let snapshot = {
            let s = self.state.lock().unwrap();
            if s.generation == s.saved {
                return;
            }
            (s.generation, serde_json::to_vec(&s.receipts).unwrap())
        };
        let store = self.store.clone();
        let path = self.path.clone();
        let bytes = snapshot.1;
        let result = tokio::task::spawn_blocking(move || store.write(&path, &bytes, 0o600)).await;
        let mut s = self.state.lock().unwrap();
        match result {
            Ok(Ok(())) => {
                s.saved = snapshot.0;
                s.error.clear();
            }
            _ => s.error = "cannot save deletion receipts".into(),
        }
    }
    pub async fn delete_due(&self, at: &At, now: i64) -> Result<()> {
        // Only receipts already committed to disk may authorize deletion.
        let due = {
            let mut s = self.state.lock().unwrap();
            let stamp = Instant::now();
            let elapsed = stamp.duration_since(s.clock.1).as_secs() as i64;
            if now < VALID_CLOCK
                || now
                    .saturating_sub(s.clock.0)
                    .saturating_sub(elapsed)
                    .unsigned_abs()
                    > 120
            {
                for age in s.ages.values_mut() {
                    *age = stamp;
                }
                s.clock_warning = true;
            }
            s.clock = (now, stamp);
            if now < VALID_CLOCK {
                return Ok(());
            }
            if s.ages.values().all(|age| stamp.duration_since(*age) >= DAY) {
                s.clock_warning = false;
            }
            if s.saved != s.generation {
                return Ok(());
            }
            s.receipts
                .iter()
                .filter(|r| {
                    now.saturating_sub(r.delivered) >= 86400
                        && s.ages
                            .get(&r.id)
                            .is_some_and(|age| stamp.duration_since(*age) >= DAY)
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        if due.is_empty() {
            return Ok(());
        }
        let raw = at.page("sms", true).await?;
        if !parser::ok(&raw) {
            bail!("SMS read failed")
        }
        let entries = sms::received(&raw);
        for receipt in due {
            let mut remaining = receipt.parts.clone();
            let verified = receipt.parts.iter().all(|part| {
                entries.iter().any(|v| {
                    v["indices"][0].as_u64() == Some(part.index as u64)
                        && identity(v) == part.fingerprint
                })
            });
            let mut failed = false;
            if verified {
                for part in &receipt.parts {
                    let raw = at.run(&format!("AT+CMGD={}", part.index)).await?;
                    if !parser::ok(&raw) {
                        failed = true;
                        break;
                    }
                    remaining.retain(|p| p.index != part.index);
                }
                at.invalidate().await;
            } else {
                remaining.clear();
            }
            let mut s = self.state.lock().unwrap();
            if let Some(index) = s.receipts.iter().position(|r| r.id == receipt.id) {
                if remaining.is_empty() {
                    s.receipts.remove(index);
                    s.ages.remove(&receipt.id);
                } else {
                    s.receipts[index].parts = remaining;
                }
                s.generation += 1;
            }
            if failed {
                s.error = "SMS deletion failed".into();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn age(c: &Cleanup, now: i64) {
        let mut s = c.state.lock().unwrap();
        s.clock = (now, Instant::now());
        for stamp in s.ages.values_mut() {
            *stamp = Instant::now() - DAY;
        }
    }
    #[tokio::test]
    async fn time_jumps_and_restart_cannot_authorize_early_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::new(true));
        let path = dir.path().join("receipts.json");
        let c = Cleanup::new(path.clone(), store.clone());
        let raw = "+CMGL: 1,\"REC READ\",\"10086\",,\"26/09/06,12:00:00+32\"\nTest\nOK";
        let mut r = receipt(raw);
        r.delivered = 1000;
        c.schedule(r);
        c.flush().await;
        let at = At::start(true, vec![]).unwrap();
        c.delete_due(&at, VALID_CLOCK + 999999).await.unwrap();
        assert!(at.trace.lock().unwrap().is_empty());
        assert!(c.status()["error"].as_str().unwrap().contains("clock"));
        let restarted = Cleanup::new(path, store);
        restarted
            .delete_due(&at, chrono::Utc::now().timestamp())
            .await
            .unwrap();
        assert!(at.trace.lock().unwrap().is_empty());
        age(&c, VALID_CLOCK + 999999);
        c.delete_due(&at, VALID_CLOCK + 1999999).await.unwrap();
        assert!(at.trace.lock().unwrap().is_empty());
        c.delete_due(&at, 1000).await.unwrap();
        assert!(at.trace.lock().unwrap().is_empty());
    }
    fn receipt(raw: &str) -> Receipt {
        let v = &sms::received(raw)[0];
        Receipt {
            id: "test".into(),
            delivered: VALID_CLOCK + 1000,
            parts: vec![Part {
                index: 1,
                fingerprint: identity(v),
            }],
        }
    }
    #[tokio::test]
    async fn receipts_survive_restart_and_require_24_hours() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::new(true));
        let path = dir.path().join("receipts.json");
        let c = Cleanup::new(path.clone(), store.clone());
        let raw = "+CMGL: 1,\"REC READ\",\"10086\",,\"26/09/06,12:00:00+32\"\nTest\nOK";
        c.schedule(receipt(raw));
        c.flush().await;
        let c = Cleanup::new(path.clone(), store);
        let at = At::start(true, vec![]).unwrap();
        at.overrides
            .lock()
            .unwrap()
            .insert("sms".into(), raw.into());
        age(&c, VALID_CLOCK + 87399);
        c.delete_due(&at, VALID_CLOCK + 87399).await.unwrap();
        assert_eq!(c.status()["pending"], 1);
        assert!(at.trace.lock().unwrap().is_empty());
        c.delete_due(&at, VALID_CLOCK + 87400).await.unwrap();
        assert_eq!(c.status()["pending"], 0);
        assert_eq!(
            at.trace
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.starts_with("AT+CMGD="))
                .cloned()
                .collect::<Vec<_>>(),
            vec!["AT+CMGD=1"]
        );
        c.flush().await;
        assert_eq!(std::fs::read_to_string(path).unwrap(), "[]");
    }
    #[tokio::test]
    async fn reused_indices_cancel_deletion_and_disabled_option_clears_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cleanup::new(dir.path().join("receipts.json"), Arc::new(Store::new(true)));
        let raw = "+CMGL: 1,\"REC READ\",\"10086\",,\"26/09/06,12:00:00+32\"\nOriginal\nOK";
        let at = At::start(true, vec![]).unwrap();
        at.overrides
            .lock()
            .unwrap()
            .insert("sms".into(), raw.replace("Original", "Replacement"));
        c.schedule(receipt(raw));
        c.flush().await;
        age(&c, VALID_CLOCK + 87400);
        c.delete_due(&at, VALID_CLOCK + 87400).await.unwrap();
        assert_eq!(c.status()["pending"], 0);
        assert!(at.overrides.lock().unwrap()["sms"].contains("Replacement"));
        assert!(
            !at.trace
                .lock()
                .unwrap()
                .iter()
                .any(|s| s.contains("+CMGD") || s.contains("+CMGS"))
        );
        c.schedule(receipt(raw));
        c.cancel();
        assert_eq!(c.status()["pending"], 0);
    }
}
