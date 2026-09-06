use crate::{at::At, parser, persistence::Store};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const NONE: u16 = u16::MAX;
#[derive(Clone, Copy)]
struct Ping {
    time: u64,
    rtt: u16,
    jitter: u16,
    status: u8,
}
fn adjacent_jitter(previous: Option<&Ping>, sample: &Ping) -> u16 {
    match previous {
        Some(prev)
            if prev.rtt != NONE
                && sample.rtt != NONE
                && sample.time > prev.time
                && sample.time - prev.time <= 1500 =>
        {
            sample.rtt.abs_diff(prev.rtt)
        }
        _ => NONE,
    }
}
#[derive(Clone, Copy)]
struct Signal {
    time: u64,
    values: [i16; 5],
}
#[derive(Clone, Copy, Default)]
struct Traffic {
    time: u64,
    received: u64,
    sent: u64,
    elapsed_ms: u32,
}
impl Traffic {
    fn rates(&self) -> (Option<f64>, Option<f64>) {
        if self.elapsed_ms == 0 {
            return (None, None);
        }
        let seconds = self.elapsed_ms as f64 / 1000.0;
        (
            Some(self.received as f64 / seconds),
            Some(self.sent as f64 / seconds),
        )
    }
}
#[derive(Default)]
struct TrafficSampler {
    last: Option<Instant>,
    previous: Option<(Instant, u64, u64)>,
}
impl TrafficSampler {
    fn sample(
        &mut self,
        stamp: Option<Instant>,
        time: u64,
        counters: Option<(u64, u64)>,
    ) -> Option<Traffic> {
        let stamp = stamp?;
        if self.last.is_some_and(|last| stamp <= last) {
            return None;
        }
        self.last = Some(stamp);
        let mut point = Traffic {
            time,
            ..Traffic::default()
        };
        let previous = self.previous.take();
        if let Some((rx, tx)) = counters {
            if let Some((old, old_rx, old_tx)) = previous {
                let elapsed = stamp.duration_since(old).as_millis();
                if (1..=15000).contains(&elapsed) && rx >= old_rx && tx >= old_tx {
                    point.received = rx - old_rx;
                    point.sent = tx - old_tx;
                    point.elapsed_ms = elapsed as u32;
                }
            }
            self.previous = Some((stamp, rx, tx));
        }
        Some(point)
    }
}
struct Ring<T: Copy, const N: usize> {
    values: [T; N],
    next: usize,
    count: usize,
}
impl<T: Copy, const N: usize> Ring<T, N> {
    fn new(empty: T) -> Self {
        Self {
            values: [empty; N],
            next: 0,
            count: 0,
        }
    }
    fn add(&mut self, value: T) {
        self.values[self.next] = value;
        self.next = (self.next + 1) % N;
        self.count = (self.count + 1).min(N)
    }
    fn iter(&self) -> impl Iterator<Item = &T> {
        (0..self.count).map(|i| &self.values[(self.next + N - self.count + i) % N])
    }
    fn last(&self) -> Option<&T> {
        if self.count == 0 {
            None
        } else {
            Some(&self.values[(self.next + N - 1) % N])
        }
    }
}
struct History {
    target: String,
    generation: u64,
    ping: Ring<Ping, 300>,
    signal: Ring<Signal, 60>,
    traffic: Ring<Traffic, 60>,
    traffic_sampler: TrafficSampler,
    ip: String,
}
pub struct Monitor {
    history: Mutex<History>,
    mock: bool,
    path: PathBuf,
    store: Arc<Store>,
    change: tokio::sync::Mutex<()>,
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn metric(v: u16) -> Option<f64> {
    (v != NONE).then_some(v as f64 / 10.0)
}
fn round(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}
pub fn normalize(raw: &str) -> Result<String> {
    let target = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if let Ok(ip) = target.parse::<IpAddr>() {
        if ip.is_unspecified() || ip.is_multicast() || target == "255.255.255.255" {
            bail!("target must be a unicast address")
        }
        return Ok(ip.to_string());
    }
    if target.is_empty() || target.len() > 253 {
        bail!("invalid hostname")
    }
    for label in target.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            bail!("enter a hostname or IP address without a URL, port or path")
        }
    }
    Ok(target)
}
impl Monitor {
    pub fn new(path: PathBuf, mock: bool, store: Arc<Store>) -> Arc<Self> {
        let target = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| normalize(v["target"].as_str().unwrap_or("")).ok())
            .unwrap_or_else(|| "www.baidu.com".into());
        Arc::new(Self {
            history: Mutex::new(History {
                target,
                generation: 0,
                ping: Ring::new(Ping {
                    time: 0,
                    rtt: NONE,
                    jitter: NONE,
                    status: 3,
                }),
                signal: Ring::new(Signal {
                    time: 0,
                    values: [i16::MIN; 5],
                }),
                ip: String::new(),
                traffic: Ring::new(Traffic::default()),
                traffic_sampler: TrafficSampler::default(),
            }),
            mock,
            path,
            store,
            change: tokio::sync::Mutex::new(()),
        })
    }
    pub fn start(self: &Arc<Self>, at: At) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let time = now();
                let mut values = [i16::MIN; 5];
                let mut counters = None;
                let mut stamp = None;
                if let Ok((raw, sampled)) = at.dashboard_sample().await {
                    stamp = sampled;
                    let data = parser::dashboard(&raw);
                    if !raw.contains("ERROR") && parser::text(&data, "nr_rx_human") != "-" {
                        counters = data["nr_rx_bytes"]
                            .as_u64()
                            .zip(data["nr_tx_bytes"].as_u64());
                    }
                    for (i, (key, min, max)) in [
                        ("rsrpLTE", -160.0, -20.0),
                        ("rsrpNR", -160.0, -20.0),
                        ("sinrLTE", -30.0, 60.0),
                        ("sinrNR", -30.0, 60.0),
                        ("temperature", -40.0, 150.0),
                    ]
                    .iter()
                    .enumerate()
                    {
                        if let Ok(v) = parser::text(&data, key).parse::<f64>()
                            && v.is_finite()
                            && v >= *min
                            && v <= *max
                        {
                            values[i] = (v * 10.0).round() as i16
                        }
                    }
                }
                let mut history = this.history.lock().unwrap();
                history.signal.add(Signal { time, values });
                if let Some(point) = history.traffic_sampler.sample(stamp, now(), counters) {
                    history.traffic.add(point);
                }
            }
        });
        let this = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut resolver = crate::resolver::PingResolver::default();
            let mut client4 = None;
            let mut client6 = None;
            let mut seq = 0u16;
            loop {
                tick.tick().await;
                let time = now();
                let (target, generation) = {
                    let h = this.history.lock().unwrap();
                    (h.target.clone(), h.generation)
                };
                let mut sample = Ping {
                    time,
                    rtt: NONE,
                    jitter: NONE,
                    status: 3,
                };
                let mut ip = String::new();
                if this.mock {
                    sample.rtt = ((24.0
                        + 9.0 * (time as f64 / 8000.0).sin()
                        + 4.0 * (time as f64 / 2000.0).sin())
                        * 10.0)
                        .round() as u16;
                    sample.status = 0;
                    ip = "192.0.2.1".into();
                } else {
                    let deadline = tokio::time::Instant::now() + Duration::from_millis(900);
                    let address = resolver.resolve(&target, deadline).await;
                    if let Some(address) = address {
                        ip = address.to_string();
                        let slot = if address.is_ipv4() {
                            &mut client4
                        } else {
                            &mut client6
                        };
                        if slot.is_none() {
                            *slot = surge_ping::Client::new(
                                &surge_ping::Config::builder()
                                    .kind(if address.is_ipv4() {
                                        surge_ping::ICMP::V4
                                    } else {
                                        surge_ping::ICMP::V6
                                    })
                                    .build(),
                            )
                            .ok()
                        }
                        if let Some(client) = slot {
                            let mut pinger = client
                                .pinger(
                                    address,
                                    surge_ping::PingIdentifier(std::process::id() as u16),
                                )
                                .await;
                            pinger.timeout(
                                deadline.saturating_duration_since(tokio::time::Instant::now()),
                            );
                            sample.status = 1;
                            match tokio::time::timeout_at(
                                deadline,
                                pinger.ping(surge_ping::PingSequence(seq), &[0u8; 32]),
                            )
                            .await
                            {
                                Ok(Ok((_, rtt))) => {
                                    sample.status = 0;
                                    sample.rtt =
                                        (rtt.as_secs_f64() * 10000.0).round().min(65534.0) as u16
                                }
                                Ok(Err(surge_ping::SurgeError::Timeout { .. })) | Err(_) => {}
                                Ok(Err(_)) => sample.status = 2,
                            }
                        } else {
                            sample.status = 2
                        }
                    }
                }
                seq = seq.wrapping_add(1);
                let mut h = this.history.lock().unwrap();
                if generation != h.generation {
                    continue;
                }
                sample.jitter = adjacent_jitter(h.ping.last(), &sample);
                h.ip = ip;
                h.ping.add(sample);
            }
        });
    }
    pub fn connected(&self) -> bool {
        self.history
            .lock()
            .unwrap()
            .ping
            .last()
            .is_some_and(|p| p.status == 0 && now().saturating_sub(p.time) < 5000)
    }
    pub fn traffic_rates(&self) -> Value {
        let history = self.history.lock().unwrap();
        let last = history
            .traffic
            .last()
            .filter(|p| now().saturating_sub(p.time) <= 15000);
        let (download, upload) = last.map(Traffic::rates).unwrap_or((None, None));
        let format = |rate: Option<f64>| {
            rate.map(|n| format!("{}/s", parser::human_bytes(n)))
                .unwrap_or_else(|| "-".into())
        };
        json!({"traffic_rates":true,"trafficSampleTime":last.map(|p|p.time),"nr_dl_speed":format(download),"nr_ul_speed":format(upload)})
    }
    pub async fn set_target(&self, raw: &str) -> Result<()> {
        let target = normalize(raw)?;
        let _guard = self.change.lock().await;
        if self.history.lock().unwrap().target == target {
            return Ok(());
        }
        let data = format!("{}\n", json!({"target":target}));
        let store = self.store.clone();
        let path = self.path.clone();
        let result =
            tokio::task::spawn_blocking(move || store.write(&path, data.as_bytes(), 0o600)).await?;
        if result.as_ref().is_ok_and(|_| true) || result.as_ref().is_err_and(|e| e.committed) {
            let mut h = self.history.lock().unwrap();
            h.target = target;
            h.generation += 1;
            h.ping.next = 0;
            h.ping.count = 0;
            h.ip.clear()
        }
        result?;
        Ok(())
    }
    pub fn snapshot(&self) -> Value {
        let time = now();
        let cutoff = time.saturating_sub(300000);
        let h = self.history.lock().unwrap();
        let mut ping = Vec::new();
        let (mut sent, mut received, mut errors, mut sum, mut jitter, mut count) =
            (0, 0, 0, 0.0, 0.0, 0);
        let (mut min, mut max) = (f64::INFINITY, 0.0f64);
        for p in h.ping.iter().filter(|p| p.time > cutoff) {
            let sent_packet = p.status <= 1;
            if sent_packet {
                sent += 1
            } else {
                errors += 1
            }
            if let Some(rtt) = metric(p.rtt) {
                received += 1;
                sum += rtt;
                min = min.min(rtt);
                max = max.max(rtt)
            }
            // The first visible point has no adjacent packet inside this window.
            let point_jitter = if ping.is_empty() {
                None
            } else {
                metric(p.jitter)
            };
            if let Some(v) = point_jitter {
                jitter += v;
                count += 1
            }
            let status = match p.status {
                0 => "ok",
                1 => "timeout",
                2 => "unavailable",
                _ => "dns_error",
            };
            let value = json!({"time":p.time,"rtt":metric(p.rtt),"jitter":point_jitter,"sent":sent_packet,"status":status});
            ping.push(value);
        }
        if !h.ip.is_empty()
            && let Some(last) = ping.last_mut()
        {
            last["ip"] = json!(h.ip);
        }
        let signal:Vec<_>=h.signal.iter().filter(|p|p.time>cutoff).map(|p|{
            let mut value=json!({"time":p.time,"status":if p.values.iter().any(|v|*v!=i16::MIN){"ok"}else{"unavailable"}});
            for(i,key)in ["rsrpLTE","rsrpNR","sinrLTE","sinrNR","temperature"].iter().enumerate(){value[key]=json!((p.values[i]!=i16::MIN).then_some(p.values[i]as f64/10.0))}value
        }).collect();
        let traffic: Vec<_> = h
            .traffic
            .iter()
            .filter(|p| p.time > cutoff)
            .map(|p| {
                let (download, upload) = p.rates();
                json!({"time":p.time,"download":download,"upload":upload})
            })
            .collect();
        let (mut received_bytes, mut sent_bytes) = (0u64, 0u64);
        for p in h
            .traffic
            .iter()
            .filter(|p| p.elapsed_ms > 0 && p.time.saturating_sub(p.elapsed_ms as u64) > cutoff)
        {
            received_bytes = received_bytes.saturating_add(p.received);
            sent_bytes = sent_bytes.saturating_add(p.sent);
        }
        let total = received_bytes as f64 + sent_bytes as f64;
        let download_share = (total > 0.0).then(|| round(100.0 * received_bytes as f64 / total));
        let traffic_summary = json!({"downloadBytes":received_bytes,"uploadBytes":sent_bytes,"downloadShare":download_share,"uploadShare":download_share.map(|p|round(100.0-p))});
        json!({"target":h.target,"generation":h.generation,"serverTime":time,"mock":self.mock,"ping":ping,"signal":signal,"traffic":traffic,"trafficSummary":traffic_summary,"summary":{
            "sent":sent,"received":received,"errors":errors,"loss":(sent>0).then(||round(100.0*(sent-received)as f64/sent as f64)),
            "average":(received>0).then(||round(sum/received as f64)),"minimum":(received>0).then_some(min),"maximum":(received>0).then_some(max),"jitter":(count>0).then(||round(jitter/count as f64))
        }})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traffic_ignores_duplicate_samples_and_preserves_real_zero() {
        let start = Instant::now();
        let mut sampler = TrafficSampler::default();
        let sample = |seconds| Some(start + Duration::from_secs(seconds));
        assert_eq!(
            sampler
                .sample(sample(0), 0, Some((100, 200)))
                .unwrap()
                .rates(),
            (None, None)
        );
        assert_eq!(
            sampler
                .sample(sample(5), 5000, Some((600, 450)))
                .unwrap()
                .rates(),
            (Some(100.0), Some(50.0))
        );
        assert!(sampler.sample(sample(5), 7000, Some((600, 450))).is_none());
        assert!(sampler.sample(sample(3), 8000, Some((400, 300))).is_none());
        assert_eq!(
            sampler
                .sample(sample(10), 10000, Some((600, 450)))
                .unwrap()
                .rates(),
            (Some(0.0), Some(0.0))
        );
        assert_eq!(
            sampler
                .sample(sample(15), 15000, Some((5, 10)))
                .unwrap()
                .rates(),
            (None, None)
        );
        assert_eq!(
            sampler.sample(sample(20), 20000, None).unwrap().rates(),
            (None, None)
        );
        assert_eq!(
            sampler
                .sample(sample(25), 25000, Some((500, 600)))
                .unwrap()
                .rates(),
            (None, None)
        );
        assert_eq!(
            sampler
                .sample(sample(50), 50000, Some((9000, 9000)))
                .unwrap()
                .rates(),
            (None, None)
        );
    }
    #[test]
    fn traffic_window_is_bounded_and_share_uses_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let monitor = Monitor::new(
            dir.path().join("monitor.json"),
            true,
            Arc::new(Store::new(true)),
        );
        let time = now();
        assert!(monitor.snapshot()["trafficSummary"]["downloadShare"].is_null());
        let mut h = monitor.history.lock().unwrap();
        for i in 0..75 {
            h.traffic.add(Traffic {
                time: time - (74 - i) * 5000,
                received: 3000,
                sent: 1000,
                elapsed_ms: 5000,
            });
        }
        drop(h);
        let snapshot = monitor.snapshot();
        assert_eq!(snapshot["traffic"].as_array().unwrap().len(), 60);
        assert_eq!(snapshot["trafficSummary"]["downloadShare"], 75.0);
        assert_eq!(snapshot["trafficSummary"]["uploadShare"], 25.0);
        assert_eq!(snapshot["traffic"][59]["download"], 600.0);
        assert_eq!(monitor.traffic_rates()["nr_dl_speed"], "600 B/s");
    }
    #[test]
    fn jitter_uses_adjacent_packets_inside_five_minute_window() {
        let dir = tempfile::tempdir().unwrap();
        let monitor = Monitor::new(
            dir.path().join("monitor.json"),
            true,
            Arc::new(Store::new(true)),
        );
        let time = now();
        let mut h = monitor.history.lock().unwrap();
        for (age, rtt) in [
            (301000, 100),
            (6000, 200),
            (5000, 500),
            (4000, NONE),
            (3000, 900),
            (1000, 400),
            (0, 600),
        ] {
            let mut p = Ping {
                time: time - age,
                rtt,
                jitter: NONE,
                status: if rtt == NONE { 1 } else { 0 },
            };
            p.jitter = adjacent_jitter(h.ping.last(), &p);
            h.ping.add(p);
        }
        drop(h);
        let result = monitor.snapshot();
        assert_eq!(result["ping"].as_array().unwrap().len(), 6);
        assert_eq!(result["summary"]["jitter"], 25.0);
        for i in [0, 2, 3, 4] {
            assert!(result["ping"][i]["jitter"].is_null());
        }
        assert_eq!(result["summary"]["sent"], 6);
        assert_eq!(result["summary"]["received"], 5);
    }
    #[test]
    fn fixed_history_bound() {
        assert!(
            std::mem::size_of::<[Ping; 300]>()
                + std::mem::size_of::<[Signal; 60]>()
                + std::mem::size_of::<[Traffic; 60]>()
                <= 8160
        );
        let mut h = Ring::<u16, 300>::new(0);
        for i in 0..700 {
            h.add(i)
        }
        assert_eq!(
            h.iter().copied().collect::<Vec<_>>(),
            (400..700).collect::<Vec<_>>()
        );
    }
    #[test]
    fn reject_shell_and_urls() {
        for value in [
            "",
            "https://baidu.com",
            "baidu.com:80",
            "a;id",
            "-a",
            "::",
            "224.0.0.1",
            "255.255.255.255",
        ] {
            assert!(normalize(value).is_err(), "{value}")
        }
        assert_eq!(normalize(" WWW.BAIDU.COM. ").unwrap(), "www.baidu.com");
        assert_eq!(normalize("::1").unwrap(), "::1");
    }
}
