use crate::{
    at::At,
    cleanup::{Cleanup, Part, Receipt},
    parser,
    persistence::Store,
    sms,
};
use anyhow::{Context, Result, bail};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const RETRY_WINDOW: Duration = Duration::from_secs(180);
const MAX_JOBS: usize = 64;
const MAX_QUEUED_BYTES: usize = 256 * 1024;
const MAX_SEEN: usize = 4096;
const PLATFORMS: [&str; 5] = ["serverchan", "wecom", "dingtalk", "feishu", "webhook"];

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Channel {
    pub platform: String,
    pub enabled: bool,
    pub url: String,
    pub token: String,
    pub secret: String,
    #[serde(skip_serializing)]
    pub clear: bool,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub sms_enabled: bool,
    pub delete_after_day: bool,
    pub enabled: bool,
    pub device_name: String,
    pub channels: Vec<Channel>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            sms_enabled: true,
            delete_after_day: false,
            enabled: false,
            device_name: "RM520N-EU".into(),
            channels: PLATFORMS
                .iter()
                .map(|p| Channel {
                    platform: (*p).into(),
                    ..Channel::default()
                })
                .collect(),
        }
    }
}
impl Settings {
    fn validate(&self) -> Result<()> {
        if self.device_name.trim().is_empty()
            || self.device_name.chars().count() > 64
            || self.device_name.chars().any(char::is_control)
        {
            bail!("invalid device name")
        }
        if self.channels.len() != PLATFORMS.len() {
            bail!("five platform entries required")
        }
        let mut seen = HashSet::new();
        for c in &self.channels {
            if !PLATFORMS.contains(&c.platform.as_str()) || !seen.insert(&c.platform) {
                bail!("invalid or duplicate platform")
            }
            if c.url.len() > 2048 || c.token.len() > 512 || c.secret.len() > 256 {
                bail!("credentials too long")
            }
            if c.token.contains(['\r', '\n']) || c.secret.contains(['\r', '\n']) {
                bail!("invalid credentials")
            }
            if !c.enabled {
                continue;
            }
            if c.platform == "serverchan" {
                if !c.token.starts_with("SCT")
                    || c.token.len() < 8
                    || !c.token.bytes().all(|b| b.is_ascii_alphanumeric())
                {
                    bail!("invalid ServerChan Turbo SendKey")
                }
            } else {
                let url = reqwest::Url::parse(&c.url).context("invalid webhook URL")?;
                if !matches!(url.scheme(), "https" | "http")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.fragment().is_some()
                {
                    bail!("invalid webhook URL")
                }
                let expected = match c.platform.as_str() {
                    "wecom" => Some(("qyapi.weixin.qq.com", "/cgi-bin/webhook/send")),
                    "dingtalk" => Some(("oapi.dingtalk.com", "/robot/send")),
                    "feishu" => Some(("open.feishu.cn", "/open-apis/bot/v2/hook/")),
                    _ => None,
                };
                if let Some((host, path)) = expected {
                    if url.scheme() != "https"
                        || url.host_str() != Some(host)
                        || url.port().is_some_and(|p| p != 443)
                        || !(if c.platform == "feishu" {
                            url.path().starts_with(path) && url.path().len() > path.len()
                        } else {
                            url.path() == path
                        })
                    {
                        bail!("webhook does not match selected platform")
                    }
                    if c.platform != "feishu"
                        && !url.query_pairs().any(|(k, v)| {
                            k == if c.platform == "wecom" {
                                "key"
                            } else {
                                "access_token"
                            } && !v.is_empty()
                        })
                    {
                        bail!("webhook token missing")
                    }
                }
                if c.platform == "webhook" && !c.token.is_empty() {
                    reqwest::header::HeaderValue::from_str(&c.token)
                        .context("invalid Authorization header")?;
                }
            }
        }
        if self.enabled && !self.channels.iter().any(|c| c.enabled) {
            bail!("enable at least one channel")
        }
        Ok(())
    }
}

#[derive(Clone, Serialize)]
struct Notification {
    id: String,
    device: String,
    sender: String,
    received_at: String,
    text: String,
    #[serde(skip)]
    parts: Vec<Part>,
}
#[derive(Default)]
struct Detector {
    initialized: bool,
    seen: HashSet<[u8; 16]>,
}
pub(crate) fn identity(v: &Value) -> [u8; 16] {
    let digest = Sha256::digest(
        serde_json::to_vec(&json!([
            v["sender"],
            v["date"],
            v["text"],
            v["indices"],
            v["concatRef"],
            v["concatSeq"]
        ]))
        .unwrap(),
    );
    digest[..16].try_into().unwrap()
}
impl Detector {
    fn remember(&mut self, id: [u8; 16]) {
        self.seen.insert(id);
    }
    fn scan(&mut self, entries: Vec<Value>, device: &str) -> Result<Vec<Notification>> {
        if entries.len() > MAX_SEEN {
            bail!("SMS inbox exceeds forwarding capacity")
        }
        let present: HashSet<_> = entries.iter().map(identity).collect();
        self.seen.retain(|id| present.contains(id));
        if !self.initialized {
            for v in &entries {
                self.remember(identity(v));
            }
            self.initialized = true;
            return Ok(Vec::new());
        }
        let mut groups: HashMap<String, Vec<Value>> = HashMap::new();
        for v in entries {
            let key = if parser::text(&v, "concatRef").is_empty() {
                hex::encode(identity(&v))
            } else {
                parser::text(&v, "concatRef").to_owned()
            };
            groups.entry(key).or_default().push(v);
        }
        let mut result = Vec::new();
        for mut group in groups.into_values().flat_map(split_fragments) {
            let ids: Vec<_> = group.iter().map(identity).collect();
            let old = ids.iter().any(|id| self.seen.contains(id));
            let total = group[0]["concatTotal"].as_u64().unwrap_or(1) as usize;
            if total > 1 {
                let seq: HashSet<_> = group
                    .iter()
                    .filter_map(|v| v["concatSeq"].as_u64())
                    .collect();
                if total > 255
                    || group.len() != total
                    || seq.len() != total
                    || !(1..=total as u64).all(|s| seq.contains(&s))
                {
                    continue;
                }
            }
            for id in &ids {
                self.remember(*id);
            }
            if old {
                continue;
            }
            group.sort_by_key(|v| v["concatSeq"].as_u64().unwrap_or(0));
            let text: String = group.iter().map(|v| parser::text(v, "text")).collect();
            let first = &group[0];
            if text.is_empty() || text.len() > 65536 || parser::text(first, "sender").is_empty() {
                continue;
            }
            result.push(Notification {
                id: hex::encode(identity(first)),
                device: device.into(),
                sender: parser::text(first, "sender").into(),
                received_at: parser::text(first, "date").into(),
                text,
                parts: group
                    .iter()
                    .filter_map(|v| {
                        Some(Part {
                            index: u16::try_from(v["indices"][0].as_u64()?).ok()?,
                            fingerprint: identity(v),
                        })
                    })
                    .collect(),
            });
        }
        Ok(result)
    }
}
fn split_fragments(mut entries: Vec<Value>) -> Vec<Vec<Value>> {
    entries.sort_by(|a, b| {
        parser::text(a, "date")
            .cmp(parser::text(b, "date"))
            .then_with(|| a["indices"][0].as_u64().cmp(&b["indices"][0].as_u64()))
    });
    let timestamp = |v: &Value| {
        chrono::NaiveDateTime::parse_from_str(
            parser::text(v, "date").get(..17).unwrap_or_default(),
            "%y/%m/%d,%H:%M:%S",
        )
        .ok()
    };
    let mut groups: Vec<Vec<Value>> = Vec::new();
    for entry in entries {
        let candidate = groups.iter_mut().find(|group| {
            let first = &group[0];
            first["concatTotal"] == entry["concatTotal"]
                && !group.iter().any(|v| v["concatSeq"] == entry["concatSeq"])
                && match (timestamp(first), timestamp(&entry)) {
                    (Some(a), Some(b)) => (b - a).num_seconds().abs() <= 300,
                    _ => parser::text(first, "date") == parser::text(&entry, "date"),
                }
        });
        if let Some(group) = candidate {
            group.push(entry)
        } else {
            groups.push(vec![entry])
        }
    }
    groups
}
struct Job {
    notification: Arc<Notification>,
    channel: usize,
    part: usize,
    attempts: u32,
    created: Instant,
    next: Instant,
}
#[derive(Serialize)]
struct Record {
    time: i64,
    platform: String,
    sender: String,
    status: String,
    detail: String,
}
struct State {
    settings: Settings,
    generation: u64,
    detector: Detector,
    queue: VecDeque<Job>,
    records: VecDeque<Record>,
    cooldown: [Option<Instant>; 5],
    poll_error: String,
    test_at: Option<Instant>,
    outcomes: HashMap<String, (usize, bool)>,
}
pub struct Forwarder {
    pub sms_mutation: tokio::sync::Mutex<()>,
    pub cleanup: Cleanup,
    state: Mutex<State>,
    mutation: tokio::sync::Mutex<()>,
    client: OnceLock<reqwest::Client>,
    path: PathBuf,
    store: Arc<Store>,
}
impl Forwarder {
    pub fn new(path: PathBuf, store: Arc<Store>) -> Self {
        let (settings, poll_error) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes)
                .ok()
                .filter(|s| s.validate().is_ok())
            {
                Some(s) => (s, String::new()),
                None => (
                    Settings::default(),
                    "invalid saved forwarding settings".into(),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (Settings::default(), String::new())
            }
            Err(_) => (
                Settings::default(),
                "cannot read forwarding settings".into(),
            ),
        };
        Self {
            sms_mutation: tokio::sync::Mutex::new(()),
            cleanup: Cleanup::new(
                path.with_file_name("sms-delete-receipts.json"),
                store.clone(),
            ),
            state: Mutex::new(State {
                settings,
                generation: 0,
                detector: Detector::default(),
                queue: VecDeque::new(),
                records: VecDeque::new(),
                cooldown: [None; 5],
                poll_error,
                test_at: None,
                outcomes: HashMap::new(),
            }),
            mutation: tokio::sync::Mutex::new(()),
            client: OnceLock::new(),
            path,
            store,
        }
    }
    pub fn snapshot(&self) -> Value {
        let s = self.state.lock().unwrap();
        let channels: Vec<_> = s.settings.channels.iter().map(|c|json!({"platform":c.platform,"enabled":c.enabled,"has_url":!c.url.is_empty(),"has_token":!c.token.is_empty(),"has_secret":!c.secret.is_empty()})).collect();
        json!({"sms_enabled":s.settings.sms_enabled,"delete_after_day":s.settings.delete_after_day,"cleanup":self.cleanup.status(),"enabled":s.settings.enabled,"device_name":s.settings.device_name,"channels":channels,"queued":s.queue.len(),"ready":s.detector.initialized,"error":s.poll_error,"records":s.records,"retry_seconds":180})
    }
    fn merge_settings(&self, mut next: Settings) -> Result<Settings> {
        let state = self.state.lock().unwrap();
        next.device_name = next.device_name.trim().into();
        for c in &mut next.channels {
            if c.clear {
                c.url.clear();
                c.token.clear();
                c.secret.clear();
                c.enabled = false;
                c.clear = false;
                continue;
            }
            if let Some(old) = state
                .settings
                .channels
                .iter()
                .find(|old| old.platform == c.platform)
            {
                if c.url.is_empty() {
                    c.url = old.url.clone();
                }
                if c.token.is_empty() {
                    c.token = old.token.clone();
                }
                if c.secret.is_empty() {
                    c.secret = old.secret.clone();
                }
            }
        }
        next.validate()?;
        Ok(next)
    }
    pub async fn save(&self, next: Settings) -> Result<Value> {
        let _sms_guard = self.sms_mutation.lock().await;
        let _guard = self.mutation.lock().await;
        let next = self.merge_settings(next)?;
        self.persist(next).await
    }
    async fn persist(&self, next: Settings) -> Result<Value> {
        if self.state.lock().unwrap().settings == next {
            return Ok(self.snapshot());
        }
        let bytes = serde_json::to_vec(&next)?;
        let path = self.path.clone();
        let store = self.store.clone();
        let saved = tokio::task::spawn_blocking(move || store.write(&path, &bytes, 0o600)).await?;
        if saved.is_ok() || saved.as_ref().is_err_and(|e| e.committed) {
            let mut s = self.state.lock().unwrap();
            let reset = s.settings.enabled != next.enabled
                || s.settings.sms_enabled != next.sms_enabled
                || s.settings.channels != next.channels;
            s.settings = next;
            if reset {
                s.generation += 1;
                s.detector = Detector::default();
                s.queue.clear();
                s.outcomes.clear();
            }
            if !s.settings.delete_after_day {
                self.cleanup.cancel();
            }
            s.cooldown = [None; 5];
            s.poll_error.clear();
        }
        saved?;
        Ok(self.snapshot())
    }
    fn record(s: &mut State, platform: &str, sender: &str, status: &str, detail: &str) {
        s.records.push_front(Record {
            time: chrono::Utc::now().timestamp_millis(),
            platform: platform.into(),
            sender: sender.into(),
            status: status.into(),
            detail: detail.into(),
        });
        s.records.truncate(50);
    }
    pub fn start(self: &Arc<Self>, at: At) {
        let cleanup_self = self.clone();
        let cleanup_at = at.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                cleanup_self.cleanup.flush().await;
                let _guard = cleanup_self.sms_mutation.lock().await;
                let enabled = {
                    let s = cleanup_self.state.lock().unwrap();
                    s.settings.sms_enabled && s.settings.delete_after_day
                };
                if enabled {
                    let _ = cleanup_self
                        .cleanup
                        .delete_due(&cleanup_at, chrono::Utc::now().timestamp())
                        .await;
                    cleanup_self.cleanup.flush().await;
                }
            }
        });
        let this = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(10));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let (enabled, generation, device) = {
                    let s = this.state.lock().unwrap();
                    (
                        s.settings.enabled && s.settings.sms_enabled,
                        s.generation,
                        s.settings.device_name.clone(),
                    )
                };
                if !enabled {
                    continue;
                }
                let raw = at.page("sms", false).await;
                let mut s = this.state.lock().unwrap();
                if s.generation != generation {
                    continue;
                }
                match raw {
                    Ok(raw) if parser::ok(&raw) => {
                        match s.detector.scan(sms::received(&raw), &device) {
                            Ok(messages) => {
                                s.poll_error.clear();
                                for message in messages {
                                    let notification = Arc::new(message);
                                    let mut pending = 0;
                                    let mut failed = false;
                                    for channel in 0..s.settings.channels.len() {
                                        if !s.settings.channels[channel].enabled {
                                            continue;
                                        }
                                        let bytes: usize =
                                            s.queue.iter().map(|j| j.notification.text.len()).sum();
                                        if s.queue.len() >= MAX_JOBS
                                            || bytes + notification.text.len() > MAX_QUEUED_BYTES
                                        {
                                            let platform =
                                                s.settings.channels[channel].platform.clone();
                                            Self::record(
                                                &mut s,
                                                &platform,
                                                &notification.sender,
                                                "failed",
                                                "queue capacity exceeded",
                                            );
                                            failed = true;
                                            continue;
                                        }
                                        let now = Instant::now();
                                        s.queue.push_back(Job {
                                            notification: notification.clone(),
                                            channel,
                                            part: 0,
                                            attempts: 0,
                                            created: now,
                                            next: now,
                                        });
                                        pending += 1;
                                    }
                                    if pending > 0 {
                                        s.outcomes
                                            .insert(notification.id.clone(), (pending, failed));
                                    }
                                }
                            }
                            Err(_) => s.poll_error = "SMS inbox exceeds forwarding capacity".into(),
                        }
                    }
                    _ => s.poll_error = "SMS read failed".into(),
                }
            }
        });
        let this = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                this.deliver_next().await;
            }
        });
    }
    async fn deliver_next(&self) {
        let work = {
            let mut s = self.state.lock().unwrap();
            let now = Instant::now();
            let mut i = 0;
            while i < s.queue.len() {
                if now.duration_since(s.queue[i].created) >= RETRY_WINDOW {
                    let job = s.queue.remove(i).unwrap();
                    self.completed(&mut s, &job.notification, false);
                    let platform = s.settings.channels[job.channel].platform.clone();
                    Self::record(
                        &mut s,
                        &platform,
                        &job.notification.sender,
                        "failed",
                        "retry window expired",
                    );
                } else {
                    i += 1
                }
            }
            let index = s.queue.iter().position(|job| {
                job.next <= now && s.cooldown[job.channel].is_none_or(|t| t <= now)
            });
            index.map(|i| {
                let job = s.queue.remove(i).unwrap();
                let channel = s.settings.channels[job.channel].clone();
                s.cooldown[job.channel] = Some(now + Duration::from_secs(4));
                (job, channel, s.generation)
            })
        };
        let Some((mut job, channel, generation)) = work else {
            return;
        };
        let parts = chunks(&job.notification.text, 1200);
        let count = parts.len();
        let result = self
            .send(
                &channel,
                &job.notification,
                parts[job.part],
                job.part,
                count,
                RETRY_WINDOW.saturating_sub(job.created.elapsed()),
            )
            .await;
        let mut s = self.state.lock().unwrap();
        if s.generation != generation {
            return;
        }
        match result {
            Ok(()) => {
                job.part += 1;
                job.attempts = 0;
                if job.part == count {
                    self.completed(&mut s, &job.notification, true);
                    Self::record(
                        &mut s,
                        &channel.platform,
                        &job.notification.sender,
                        "sent",
                        "",
                    );
                } else {
                    job.next = Instant::now() + Duration::from_secs(4);
                    s.queue.push_back(job);
                }
            }
            Err((message, retry)) => {
                job.attempts += 1;
                Self::record(
                    &mut s,
                    &channel.platform,
                    &job.notification.sender,
                    if retry { "retrying" } else { "failed" },
                    &message,
                );
                if retry {
                    job.next =
                        Instant::now() + Duration::from_secs((5u64 << job.attempts.min(4)).min(60));
                    s.queue.push_back(job);
                } else {
                    self.completed(&mut s, &job.notification, false);
                }
            }
        }
    }
    pub async fn test(&self, platform: &str) -> Result<Value> {
        let (channel, device) = {
            let mut s = self.state.lock().unwrap();
            if s.test_at
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
            {
                bail!("wait before testing again")
            }
            let channel = s
                .settings
                .channels
                .iter()
                .find(|c| c.platform == platform)
                .context("unknown platform")?
                .clone();
            let mut settings = s.settings.clone();
            for c in &mut settings.channels {
                c.enabled = c.platform == platform;
            }
            settings.validate()?;
            s.test_at = Some(Instant::now());
            (channel, s.settings.device_name.clone())
        };
        let note = Notification {
            id: format!("test-{}", chrono::Utc::now().timestamp_millis()),
            device,
            sender: "SimpleAdmin".into(),
            received_at: chrono::Utc::now().to_rfc3339(),
            text: "SMS forwarding test".into(),
            parts: Vec::new(),
        };
        let result = self
            .send(&channel, &note, &note.text, 0, 1, Duration::from_secs(15))
            .await;
        let mut s = self.state.lock().unwrap();
        match result {
            Ok(()) => {
                Self::record(&mut s, platform, "SimpleAdmin", "sent", "test");
                Ok(json!({"ok":true}))
            }
            Err((message, _)) => {
                Self::record(&mut s, platform, "SimpleAdmin", "failed", &message);
                bail!("{message}")
            }
        }
    }
    fn completed(&self, s: &mut State, n: &Notification, success: bool) {
        if let Some((pending, failed)) = s.outcomes.get_mut(&n.id) {
            *pending = pending.saturating_sub(1);
            *failed |= !success;
            if *pending == 0 {
                if !*failed && s.settings.delete_after_day {
                    self.cleanup.schedule(Receipt {
                        id: n.id.clone(),
                        delivered: chrono::Utc::now().timestamp(),
                        parts: n.parts.clone(),
                    });
                }
                s.outcomes.remove(&n.id);
            }
        }
    }
    pub fn sms_enabled(&self) -> bool {
        self.state.lock().unwrap().settings.sms_enabled
    }
    pub async fn sms_settings(&self, enabled: bool, delete_after_day: bool) -> Result<Value> {
        let _guard = self.sms_mutation.lock().await;
        let _mutation = self.mutation.lock().await;
        let mut next = self.state.lock().unwrap().settings.clone();
        next.sms_enabled = enabled;
        next.delete_after_day = delete_after_day;
        self.persist(next).await
    }
    async fn send(
        &self,
        c: &Channel,
        n: &Notification,
        part: &str,
        index: usize,
        total: usize,
        remaining: Duration,
    ) -> std::result::Result<(), (String, bool)> {
        if self.client.get().is_none() {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .pool_max_idle_per_host(1)
                .pool_idle_timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| ("HTTP client unavailable".into(), true))?;
            let _ = self.client.set(client);
        }
        let client = self.client.get().unwrap();
        let (url, body) = payload(c, n, part, index, total, chrono::Utc::now().timestamp())
            .map_err(|_| ("invalid channel configuration".into(), false))?;
        let mut request = client
            .post(url)
            .timeout(remaining.min(Duration::from_secs(15)));
        if c.platform == "serverchan" {
            request = request.form(&[
                ("title", body["title"].as_str().unwrap()),
                ("desp", body["desp"].as_str().unwrap()),
            ]);
        } else {
            request = request.json(&body);
        }
        if c.platform == "webhook" {
            request = request.header("Idempotency-Key", format!("{}-{}", n.id, index + 1));
            if !c.token.is_empty() {
                request = request.header("Authorization", &c.token);
            }
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| ("network or TLS request failed".into(), true))?;
        let status = response.status();
        if !status.is_success() {
            return Err((
                format!("HTTP {}", status.as_u16()),
                status.is_server_error() || matches!(status.as_u16(), 408 | 429),
            ));
        }
        if c.platform == "webhook" {
            return Ok(());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ("provider response interrupted".into(), true))?
        {
            if bytes.len() + chunk.len() > 16384 {
                return Err(("provider response too large".into(), false));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ("invalid provider response".into(), true))?;
        let code = match c.platform.as_str() {
            "serverchan" | "feishu" => value.get("code").or_else(|| value.get("StatusCode")),
            _ => value.get("errcode"),
        }
        .and_then(Value::as_i64);
        match code {
            Some(0) => Ok(()),
            Some(code) => Err((format!("provider error {code}"), true)),
            None => Err(("provider success code missing".into(), true)),
        }
    }
}
fn chunks(text: &str, max: usize) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (offset, c) in text.char_indices() {
        if offset + c.len_utf8() - start > max {
            parts.push(&text[start..offset]);
            start = offset;
        }
    }
    parts.push(&text[start..]);
    parts
}
fn sign(key: &str, message: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(message.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}
fn payload(
    c: &Channel,
    n: &Notification,
    part: &str,
    index: usize,
    total: usize,
    seconds: i64,
) -> Result<(reqwest::Url, Value)> {
    let content = format!(
        "[{}] SMS ({}/{})\nFrom: {}\nTime: {}\n\n{}",
        n.device,
        index + 1,
        total,
        n.sender,
        n.received_at,
        part
    );
    let mut url = reqwest::Url::parse(if c.platform == "serverchan" {
        "https://sctapi.ftqq.com"
    } else {
        &c.url
    })?;
    let body = match c.platform.as_str() {
        "serverchan" => {
            url.set_path(&format!("/{}.send", c.token));
            json!({"title":format!("{} SMS",n.device).chars().take(32).collect::<String>(),"desp":content})
        }
        "wecom" => json!({"msgtype":"text","text":{"content":content}}),
        "dingtalk" => {
            if !c.secret.is_empty() {
                let timestamp = (seconds * 1000).to_string();
                let signature = sign(&c.secret, &format!("{timestamp}\n{}", c.secret));
                url.query_pairs_mut()
                    .append_pair("timestamp", &timestamp)
                    .append_pair("sign", &signature);
            }
            json!({"msgtype":"text","text":{"content":content}})
        }
        "feishu" => {
            let mut value = json!({"msg_type":"text","content":{"text":content}});
            if !c.secret.is_empty() {
                value["timestamp"] = json!(seconds.to_string());
                value["sign"] = json!(sign(&format!("{seconds}\n{}", c.secret), ""));
            }
            value
        }
        "webhook" => {
            json!({"event":"sms.received","id":n.id,"device":n.device,"sender":n.sender,"received_at":n.received_at,"text":part,"part":index+1,"parts":total})
        }
        _ => bail!("unknown platform"),
    };
    Ok((url, body))
}

#[cfg(test)]
#[path = "../tests/forwarding/mod.rs"]
mod tests;
