//! The AT worker's queue as the AT terminal page shows it: the command on the port now, the
//! ones waiting behind it, and the ones finished in the last 30 seconds with their responses.
//! A copy is kept in a file under /tmp so it can also be read over adb; nothing survives a reboot.
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Finished commands older than this are dropped.
pub const KEEP_MS: u64 = 30_000;
/// Bounds memory if something floods the port within the window.
const MAX_DONE: usize = 100;
/// Longer responses (an SMS list) are cut; the page only needs to show what came back.
const RESPONSE_BYTES: usize = 16 * 1024;
pub const FILE: &str = "/tmp/simpleadmin-at-queue.json";

/// A moment as the wall clock shows it and as a monotonic clock measures it. Durations and
/// expiry use the latter, so a clock step (NTP after boot) never shows a negative time.
#[derive(Clone, Copy)]
pub struct Stamp {
    pub wall: u64,
    pub mono: Instant,
}
impl Stamp {
    pub fn now() -> Self {
        Self {
            wall: crate::telemetry::now(),
            mono: Instant::now(),
        }
    }
}
fn millis(from: Instant, to: Instant) -> u64 {
    to.saturating_duration_since(from).as_millis() as u64
}

#[derive(Clone, Serialize)]
pub struct Item {
    pub id: u64,
    pub command: String,
    pub queued_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// Time spent waiting for the port; still counting while it waits.
    pub wait_ms: u64,
    /// Time the modem took; still counting while it runs, absent if it never ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_ms: Option<u64>,
    /// ok, error (the modem answered ERROR), failed (timeout, port error) or cancelled
    /// (the caller gave up before the command reached the port).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(skip)]
    queued: Instant,
    #[serde(skip)]
    started: Option<Instant>,
    #[serde(skip)]
    finished: Option<Instant>,
}
#[derive(Default)]
struct State {
    next: u64,
    waiting: VecDeque<Item>,
    running: Option<Item>,
    done: VecDeque<Item>,
}
#[derive(Default)]
pub struct AtLog {
    state: Mutex<State>,
    changed: tokio::sync::Notify,
}
fn status(result: &anyhow::Result<String>) -> &'static str {
    match result {
        Ok(raw)
            if raw.lines().any(|l| {
                let l = l.trim();
                l == "ERROR" || l.starts_with("+CME ERROR") || l.starts_with("+CMS ERROR")
            }) =>
        {
            "error"
        }
        Ok(_) => "ok",
        Err(_) => "failed",
    }
}
fn cut(mut text: String) -> (String, bool) {
    if text.len() <= RESPONSE_BYTES {
        return (text, false);
    }
    let mut end = RESPONSE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1
    }
    text.truncate(end);
    (text, true)
}
impl AtLog {
    /// A request entered the worker's queue; the id follows it to the port and back.
    pub fn queued(&self, command: &str, at: Stamp) -> u64 {
        let mut s = self.state.lock().unwrap();
        s.next += 1;
        let id = s.next;
        s.waiting.push_back(Item {
            id,
            command: command.into(),
            queued_at: at.wall,
            finished_at: None,
            wait_ms: 0,
            run_ms: None,
            status: None,
            response: None,
            truncated: false,
            queued: at.mono,
            started: None,
            finished: None,
        });
        drop(s);
        self.changed.notify_one();
        id
    }
    pub fn started(&self, id: u64, at: Stamp) {
        let mut s = self.state.lock().unwrap();
        if let Some(i) = s.waiting.iter().position(|item| item.id == id) {
            let mut item = s.waiting.remove(i).unwrap();
            item.wait_ms = millis(item.queued, at.mono);
            item.started = Some(at.mono);
            // The worker runs one request at a time; anything left here never finished.
            if let Some(previous) = s.running.replace(item) {
                s.waiting.push_front(previous)
            }
        }
        drop(s);
        self.changed.notify_one();
    }
    pub fn finished(&self, id: u64, result: &anyhow::Result<String>, at: Stamp) {
        let text = match result {
            Ok(raw) => raw.clone(),
            Err(error) => format!("{error:#}"),
        };
        self.finish(id, status(result), text, at)
    }
    /// The caller stopped waiting before the worker reached the request, so it was skipped.
    pub fn cancelled(&self, id: u64, at: Stamp) {
        self.finish(id, "cancelled", String::new(), at)
    }
    fn finish(&self, id: u64, status: &'static str, text: String, at: Stamp) {
        let mut s = self.state.lock().unwrap();
        let item = if s.running.as_ref().is_some_and(|item| item.id == id) {
            s.running.take()
        } else {
            s.waiting
                .iter()
                .position(|item| item.id == id)
                .and_then(|i| s.waiting.remove(i))
        };
        if let Some(mut item) = item {
            match item.started {
                Some(started) => item.run_ms = Some(millis(started, at.mono)),
                None => item.wait_ms = millis(item.queued, at.mono),
            }
            let (response, truncated) = cut(text);
            item.finished_at = Some(at.wall);
            item.finished = Some(at.mono);
            item.status = Some(status);
            item.response = Some(response);
            item.truncated = truncated;
            s.done.push_back(item);
            while s.done.len() > MAX_DONE {
                s.done.pop_front();
            }
        }
        drop(s);
        self.changed.notify_one();
    }
    pub fn snapshot(&self, at: Stamp) -> Value {
        let mut s = self.state.lock().unwrap();
        let keep = Duration::from_millis(KEEP_MS);
        while s.done.front().is_some_and(|item| {
            item.finished
                .is_none_or(|t| at.mono.saturating_duration_since(t) > keep)
        }) {
            s.done.pop_front();
        }
        let running = s.running.clone().map(|mut item| {
            item.run_ms = item.started.map(|t| millis(t, at.mono));
            item
        });
        let waiting: Vec<_> = s
            .waiting
            .iter()
            .cloned()
            .map(|mut item| {
                item.wait_ms = millis(item.queued, at.mono);
                item
            })
            .collect();
        json!({
            "now": at.wall,
            "keep_ms": KEEP_MS,
            "running": running,
            "waiting": waiting,
            "done": s.done.iter().rev().collect::<Vec<_>>(),
        })
    }
    /// Keeps `path` in step with the queue: after a change (batched for 300 ms) and every few
    /// seconds, so finished commands also leave the file once they are older than 30 s.
    pub fn start_writer(self: &Arc<Self>, path: PathBuf) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut last = String::new();
            loop {
                let _ = tokio::time::timeout(Duration::from_secs(5), this.changed.notified()).await;
                tokio::time::sleep(Duration::from_millis(300)).await;
                let text = this.snapshot(Stamp::now()).to_string();
                if text != last && write(&path, &text).is_ok() {
                    last = text;
                }
            }
        });
    }
}
/// Replaces the file atomically; it can hold SMS contents, so only the owner may read it.
fn write(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let temp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&temp)?.write_all(text.as_bytes())?;
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ms` after `base` on both clocks.
    fn after(base: Stamp, ms: u64) -> Stamp {
        Stamp {
            wall: base.wall + ms,
            mono: base.mono + Duration::from_millis(ms),
        }
    }

    #[test]
    fn requests_move_from_waiting_to_running_to_done() {
        let log = AtLog::default();
        let t = Stamp::now();
        let a = log.queued("AT+CSQ", t);
        let b = log.queued("AT+QENG=\"servingcell\"", after(t, 1));
        let s = log.snapshot(after(t, 2));
        assert_eq!(s["waiting"].as_array().unwrap().len(), 2);
        assert_eq!(s["waiting"][0]["wait_ms"], 2);
        assert!(s["running"].is_null());

        log.started(a, after(t, 10));
        let s = log.snapshot(after(t, 40));
        assert_eq!(s["running"]["command"], "AT+CSQ");
        assert_eq!(s["running"]["wait_ms"], 10);
        assert_eq!(s["running"]["run_ms"], 30, "still counting while it runs");
        assert_eq!(s["waiting"][0]["id"], b);
        assert_eq!(s["waiting"][0]["wait_ms"], 39);

        log.finished(a, &Ok("+CSQ: 20,99\r\nOK\r\n".into()), after(t, 100));
        log.started(b, after(t, 100));
        log.finished(b, &Ok("+CME ERROR: 10\r\n".into()), after(t, 200));
        let s = log.snapshot(after(t, 300));
        assert!(s["running"].is_null());
        assert!(s["waiting"].as_array().unwrap().is_empty());
        let done = s["done"].as_array().unwrap();
        assert_eq!(done[0]["id"], b, "newest first");
        assert_eq!(done[0]["status"], "error");
        assert_eq!(done[0]["wait_ms"], 99);
        assert_eq!(done[0]["run_ms"], 100);
        assert_eq!(done[1]["status"], "ok");
        assert_eq!(done[1]["response"], "+CSQ: 20,99\r\nOK\r\n");
        assert_eq!(done[1]["finished_at"], t.wall + 100);
        assert_eq!(done[1]["run_ms"], 90);
    }
    #[test]
    fn a_wall_clock_step_does_not_change_durations() {
        let log = AtLog::default();
        let t = Stamp::now();
        let id = log.queued("AT+CSQ", t);
        // NTP sets the clock back an hour while the command waits.
        let mut later = after(t, 500);
        later.wall -= 3_600_000;
        log.started(id, later);
        log.finished(id, &Ok("OK".into()), after(later, 20));
        let s = log.snapshot(after(later, 30));
        assert_eq!(s["done"][0]["wait_ms"], 500);
        assert_eq!(s["done"][0]["run_ms"], 20);
    }
    #[test]
    fn failures_and_skipped_requests_are_shown() {
        let log = AtLog::default();
        let t = Stamp::now();
        let a = log.queued("AT+CGMI;+CGSN;+QGMR", t);
        let b = log.queued("AT+CSQ", t);
        log.started(a, t);
        log.finished(
            a,
            &Err(anyhow::anyhow!("AT response timed out")),
            after(t, 3000),
        );
        log.cancelled(b, after(t, 3000));
        let s = log.snapshot(after(t, 3000));
        assert_eq!(s["done"][0]["status"], "cancelled");
        assert_eq!(s["done"][0]["wait_ms"], 3000);
        assert!(s["done"][0].get("run_ms").is_none(), "it never ran");
        assert_eq!(s["done"][1]["status"], "failed");
        assert_eq!(s["done"][1]["response"], "AT response timed out");
    }
    #[test]
    fn finished_commands_expire_after_30_seconds_and_long_responses_are_cut() {
        let log = AtLog::default();
        let t = Stamp::now();
        let id = log.queued("AT+CMGF=0;+CNMI=2,1,0,0,0;+CMGL=4", t);
        log.started(id, t);
        log.finished(id, &Ok("é".repeat(RESPONSE_BYTES)), after(t, 1000));
        let s = log.snapshot(after(t, 1000 + KEEP_MS));
        assert_eq!(s["done"][0]["truncated"], true);
        assert!(s["done"][0]["response"].as_str().unwrap().len() <= RESPONSE_BYTES);
        let s = log.snapshot(after(t, 1001 + KEEP_MS));
        assert!(s["done"].as_array().unwrap().is_empty());
        for i in 0..MAX_DONE as u64 + 5 {
            let id = log.queued("AT+CSQ", after(t, i));
            log.started(id, after(t, i));
            log.finished(id, &Ok("OK".into()), after(t, i));
        }
        let s = log.snapshot(after(t, 200));
        assert_eq!(s["done"].as_array().unwrap().len(), MAX_DONE);
    }
    #[tokio::test]
    async fn writer_mirrors_the_queue_to_a_file() {
        let dir = std::env::temp_dir().join(format!("at-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("at-queue.json");
        let log = Arc::new(AtLog::default());
        log.start_writer(path.clone());
        let id = log.queued("AT+CSQ", Stamp::now());
        log.started(id, Stamp::now());
        log.finished(id, &Ok("+CSQ: 20,99\r\nOK".into()), Stamp::now());
        let mut text = String::new();
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.contains("+CSQ: 20,99") {
                break;
            }
        }
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved["done"][0]["command"], "AT+CSQ");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
