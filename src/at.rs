use crate::at_policy as policy;
use anyhow::{Context, Result, bail};
#[cfg(unix)]
use std::io::{Read, Write};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

pub const DASHBOARD: &str = "AT+QSIMSTAT?;+CSQ;+QTEMP;+QUIMSLOT?;+QSPN;+QMAP=\"WWAN\";+QENG=\"servingcell\";+QCAINFO;+QGDNRCNT?;+QGDCNT?;+CGCONTRDP=1;+QRSRP";
pub const SIGNAL: &str = "AT+QTEMP;+QENG=\"servingcell\"";
pub const SMS_LIST: &str =
    "AT+CSMS=1;+CSDH=0;+CNMI=2,1,0,0,0;+CMGF=0;+CPMS=\"ME\",\"ME\",\"ME\";+CMGL=4";
pub fn commands(page: &str) -> Vec<&'static str> {
    match page {
        "dashboard" => vec![DASHBOARD],
        "device" => vec![
            "AT+CGMI;+CGSN;+QGMR;+CIMI;+ICCID;+CNUM",
            "AT+QSIMSTAT?;+CPIN?;+QMAP=\"WWAN\"",
            "AT+QMAP=\"LANIP\"",
        ],
        "network" => vec![
            "AT+QUIMSLOT?;+QNWPREFCFG=\"mode_pref\";+QNWPREFCFG=\"nr5g_disable_mode\";+CGDCONT?;+CGCONTRDP=1;+QNWLOCK=\"common/4g\";+QNWLOCK=\"common/5g\"",
            "AT+QCAINFO",
        ],
        "bands" => vec![
            "AT+QNWPREFCFG=\"lte_band\";+QNWPREFCFG= \"nsa_nr5g_band\";+QNWPREFCFG= \"nr5g_band\"",
        ],
        "settings" => vec![
            "AT+QMAP=\"MPDN_RULE\";+QMAP=\"DHCPV6DNS\";+QCFG=\"usbnet\";+QMAP=\"DMZ\";+QMAP=\"DHCPV4DNS\"",
            "AT+CGSN",
            "AT+QMAP=\"LANIP\"",
        ],
        "sms" => vec![SMS_LIST],
        "model" => vec!["AT+CGMM"],
        _ => vec![],
    }
}
struct Request {
    command: String,
    sms: Option<String>,
    timeout: Option<Duration>,
    reply: oneshot::Sender<Result<String>>,
}
struct Entry {
    updated: Option<Instant>,
    response: Arc<str>,
    running: Option<tokio::sync::watch::Receiver<bool>>,
}
const CACHE_BYTES: usize = 1024 * 1024;

fn trim_cache(cache: &mut HashMap<String, Entry>, keep: &str) {
    while cache.values().map(|e| e.response.len()).sum::<usize>() > CACHE_BYTES {
        let key = cache
            .iter()
            .filter(|(key, entry)| key.as_str() != keep && entry.running.is_none())
            .min_by_key(|(_, entry)| entry.updated)
            .map(|(key, _)| key.clone());
        let Some(key) = key else { break };
        cache.remove(&key);
    }
}

fn cacheable(command: &str) -> bool {
    if command == SMS_LIST {
        return true;
    }
    static ALLOWED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let allowed = ALLOWED.get_or_init(|| {
        [
            "dashboard",
            "device",
            "network",
            "bands",
            "settings",
            "model",
        ]
        .iter()
        .flat_map(|page| commands(page))
        .flat_map(split)
        .map(|s| s.replace(' ', "").to_ascii_uppercase())
        .collect()
    });
    split(command)
        .iter()
        .all(|part| allowed.contains(&part.replace(' ', "").to_ascii_uppercase()))
}
#[derive(Clone)]
pub struct At {
    #[cfg(test)]
    pub trace: Arc<Mutex<Vec<String>>>,
    tx: mpsc::Sender<Request>,
    cache: Arc<Mutex<HashMap<String, Entry>>>,
    pub overrides: Arc<Mutex<HashMap<String, String>>>,
    pub mock: bool,
    ready: Instant,
}
impl At {
    pub fn start(mock: bool, devices: Vec<String>) -> Result<Self> {
        let (tx, mut rx) = mpsc::channel::<Request>(16);
        let overrides = Arc::new(Mutex::new(HashMap::<String, String>::new()));
        let mocks = overrides.clone();
        std::thread::Builder::new()
            .name("at-worker".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                let fixtures: HashMap<String, String> =
                    serde_json::from_str(include_str!("../tests/fixtures/mock-at.json")).unwrap();
                let mut port: Option<Port> = None;
                while let Some(request) = rx.blocking_recv() {
                    if request.reply.is_closed() {
                        continue;
                    }
                    let result = if mock {
                        Ok(crate::mock::response(
                            &request.command,
                            &fixtures,
                            &mocks.lock().unwrap(),
                        ))
                    } else {
                        (|| -> Result<String> {
                            if port.is_none() {
                                let mut last = anyhow::anyhow!("no AT device available");
                                for path in &devices {
                                    match Port::open(path) {
                                        Ok(p) => {
                                            port = Some(p);
                                            break;
                                        }
                                        Err(e) => last = e,
                                    }
                                }
                                if port.is_none() {
                                    return Err(last);
                                }
                            }
                            let _lock = global_lock()?;
                            let p = port.as_mut().unwrap();
                            let result = p.execute(
                                &request.command,
                                request.sms.as_deref(),
                                request.timeout,
                            );
                            if result.is_err() {
                                p.drain(Duration::from_millis(1000));
                                if p.disconnected {
                                    port = None;
                                }
                            }
                            result
                        })()
                    };
                    let _ = request.reply.send(result);
                }
            })?;
        let uptime = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            .unwrap_or(35.0);
        let delay = if mock { 0.0 } else { (35.0 - uptime).max(0.0) };
        Ok(Self {
            #[cfg(test)]
            trace: Arc::new(Mutex::new(Vec::new())),
            tx,
            cache: Arc::new(Mutex::new(HashMap::new())),
            overrides,
            mock,
            ready: Instant::now() + Duration::from_secs_f64(delay),
        })
    }
    pub async fn run(&self, command: &str) -> Result<String> {
        self.transaction(command, None).await
    }
    pub async fn transaction(&self, command: &str, sms: Option<String>) -> Result<String> {
        self.transaction_timeout(command, sms, None).await
    }
    pub async fn transaction_timeout(
        &self,
        command: &str,
        sms: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<String> {
        if command.len() > 4096 || command.chars().any(|c| c.is_control()) {
            bail!("invalid AT command")
        }
        #[cfg(test)]
        self.trace.lock().unwrap().push(command.into());
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(Request {
                command: command.into(),
                sms,
                timeout,
                reply,
            })
            .map_err(|_| anyhow::anyhow!("AT queue busy"))?;
        rx.await.context("AT worker stopped")?
    }
    pub async fn fetch(&self, command: &str, force: bool) -> Result<String> {
        self.fetch_wait(command, force, true).await
    }
    pub async fn fetch_wait(&self, command: &str, force: bool, wait: bool) -> Result<String> {
        if policy::action(command) || !cacheable(command) {
            let result = self.run(command).await;
            self.invalidate().await;
            return result;
        }
        let delayed = self.ready > Instant::now() && !policy::immediate(command);
        let (mut completion, old) = {
            let mut cache = self.cache.lock().unwrap();
            if let Some(entry) = cache.get(command)
                && !force
                && entry
                    .updated
                    .is_some_and(|time| time.elapsed() < policy::max_age(command))
            {
                return Ok(entry.response.to_string());
            }
            if !cache.contains_key(command) {
                if cache.len() >= 64 {
                    let key = cache
                        .iter()
                        .filter(|(_, e)| e.running.is_none())
                        .min_by_key(|(_, e)| e.updated)
                        .map(|(k, _)| k.clone())
                        .context("AT cache busy")?;
                    cache.remove(&key);
                }
                cache.insert(
                    command.into(),
                    Entry {
                        updated: None,
                        response: Arc::from(policy::PENDING),
                        running: None,
                    },
                );
            }
            let entry = cache.get_mut(command).unwrap();
            let old = entry.response.clone();
            let receiver = if let Some(receiver) = &entry.running {
                receiver.clone()
            } else {
                let (sender, receiver) = tokio::sync::watch::channel(false);
                entry.running = Some(receiver.clone());
                let this = self.clone();
                let command = command.to_owned();
                tokio::spawn(async move {
                    if !policy::immediate(&command)
                        && let Some(delay) = this.ready.checked_duration_since(Instant::now())
                    {
                        tokio::time::sleep(delay).await
                    }
                    let response = this
                        .run(&command)
                        .await
                        .unwrap_or_else(|e| format!("ERROR: {e}"));
                    let mut cache = this.cache.lock().unwrap();
                    if let Some(entry) = cache.get_mut(&command) {
                        entry.response = Arc::from(response);
                        entry.updated = Some(Instant::now());
                        entry.running = None;
                    }
                    trim_cache(&mut cache, &command);
                    let _ = sender.send(true);
                });
                receiver
            };
            (receiver, old)
        };
        if !wait || delayed {
            return Ok(old.to_string());
        }
        if !*completion.borrow()
            && tokio::time::timeout(
                policy::timeout(command) + Duration::from_secs(2),
                completion.changed(),
            )
            .await
            .is_err()
        {
            return Ok(policy::PENDING.into());
        }
        Ok(self
            .cache
            .lock()
            .unwrap()
            .get(command)
            .map(|e| e.response.to_string())
            .unwrap_or_else(|| policy::PENDING.into()))
    }
    pub async fn page(&self, page: &str, force: bool) -> Result<String> {
        let mut responses = Vec::new();
        for command in commands(page) {
            responses.push(self.fetch(command, force).await?)
        }
        Ok(responses.join("\n"))
    }
    pub async fn dashboard_sample(&self) -> Result<(String, Option<Instant>)> {
        self.fetch(DASHBOARD, true).await?;
        let cache = self.cache.lock().unwrap();
        let entry = cache
            .get(DASHBOARD)
            .context("dashboard sample unavailable")?;
        Ok((entry.response.to_string(), entry.updated))
    }
    pub async fn invalidate(&self) {
        for entry in self.cache.lock().unwrap().values_mut() {
            entry.updated = None;
        }
    }
    pub fn start_refresh(&self) {
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = this.fetch_wait(DASHBOARD, false, false).await;
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            loop {
                tick.tick().await;
                let commands: Vec<_> = this
                    .cache
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(c, e)| {
                        c.as_str() != DASHBOARD
                            && !policy::immediate(c)
                            && !policy::sms(c)
                            && e.running.is_none()
                            && e.updated.is_none_or(|t| t.elapsed() >= policy::max_age(c))
                    })
                    .map(|(c, _)| c.clone())
                    .collect();
                for command in commands {
                    let _ = this.fetch_wait(&command, false, false).await;
                }
            }
        });
    }
}
pub fn split(command: &str) -> Vec<String> {
    let mut quoted = false;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, ch) in command.char_indices() {
        if ch == '"' {
            quoted = !quoted
        }
        if ch == ';' && !quoted {
            parts.push(command[start..i].trim());
            start = i + 1
        }
    }
    parts.push(command[start..].trim());
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            if p.to_ascii_uppercase().starts_with("AT") {
                p.into()
            } else {
                format!("AT{p}")
            }
        })
        .collect()
}
fn terminal(raw: &str) -> bool {
    raw.lines().any(|l| {
        let l = l.trim();
        l == "OK" || l == "ERROR" || l.starts_with("+CME ERROR:") || l.starts_with("+CMS ERROR:")
    })
}

struct Port {
    #[cfg(unix)]
    path: String,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    #[cfg(unix)]
    tty: Option<std::fs::File>,
    #[cfg(unix)]
    _owner: Option<std::fs::File>,
    disconnected: bool,
}
impl Port {
    #[cfg(unix)]
    fn open(path: &str) -> Result<Self> {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        let smd = std::path::Path::new(path)
            .file_name()
            .is_some_and(|s| s.to_string_lossy().starts_with("smd"));
        // A transaction lock alone cannot prevent two persistent readers stealing replies.
        let owner = if smd { Some(reader_lock()?) } else { None };
        let mut options = std::fs::OpenOptions::new();
        options
            .read(true)
            .write(!smd)
            .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC);
        let mut reader = options
            .open(path)
            .with_context(|| format!("open AT device {path}"))?;
        if !smd {
            unsafe {
                let mut term = std::mem::zeroed();
                if libc::tcgetattr(reader.as_raw_fd(), &mut term) == 0 {
                    libc::cfmakeraw(&mut term);
                    libc::cfsetispeed(&mut term, libc::B115200);
                    libc::cfsetospeed(&mut term, libc::B115200);
                    libc::tcsetattr(reader.as_raw_fd(), libc::TCSANOW, &term);
                }
            }
        }
        let tty = if smd { None } else { Some(reader.try_clone()?) };
        let (tx, rx) = std::sync::mpsc::sync_channel(16);
        std::thread::Builder::new()
            .name("at-reader".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            path: path.into(),
            rx,
            tty,
            _owner: owner,
            disconnected: false,
        })
    }
    #[cfg(not(unix))]
    fn open(_: &str) -> Result<Self> {
        bail!("native AT requires Linux; use --mock on Windows")
    }
    fn drain(&self, max: Duration) {
        let deadline = Instant::now() + max;
        while Instant::now() < deadline {
            if self.rx.recv_timeout(Duration::from_millis(30)).is_err() {
                break;
            }
        }
    }
    fn write(&mut self, data: &[u8]) -> Result<()> {
        #[cfg(unix)]
        {
            if let Some(file) = &mut self.tty {
                file.write_all(data)?;
                return Ok(());
            }
            // Match the O_WRONLY|O_TRUNC open used by shell redirection on SMD.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path)?;
            file.write_all(data)?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = data;
            bail!("native AT requires Linux")
        }
    }
    fn receive(&mut self, timeout: Duration, prompt: bool) -> Result<String> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(remaining) {
                Ok(chunk) => {
                    if out.len() + chunk.len() > 512 * 1024 {
                        bail!("AT response too large")
                    }
                    out.extend(chunk);
                    let raw = String::from_utf8_lossy(&out);
                    if (terminal(&raw) && (!prompt || raw.contains("ERROR")))
                        || (prompt && raw.trim_end().ends_with('>'))
                    {
                        return Ok(raw.into_owned());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    self.disconnected = true;
                    break;
                }
                Err(_) => break,
            }
        }
        bail!("AT response timed out")
    }
    fn execute(
        &mut self,
        command: &str,
        sms: Option<&str>,
        timeout: Option<Duration>,
    ) -> Result<String> {
        self.drain(Duration::from_millis(150));
        self.write(format!("{command}\r\n").as_bytes())?;
        let mut response = self.receive(
            timeout.unwrap_or_else(|| {
                if sms.is_some() {
                    Duration::from_secs(3)
                } else {
                    policy::timeout(command)
                }
            }),
            sms.is_some(),
        )?;
        if let Some(pdu) = sms
            && response.trim_end().ends_with('>')
        {
            self.write(format!("{pdu}\x1a").as_bytes())?;
            response.push_str(&self.receive(Duration::from_secs(60), false)?);
        }
        Ok(response)
    }
}
#[cfg(unix)]
fn reader_lock() -> Result<std::fs::File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/tmp/simpleadmin-rust-at-reader.lock")?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!(
            "AT reader already active; use the WebUI AT terminal or stop the service before using the standalone AT command"
        )
    }
    Ok(file)
}
#[cfg(unix)]
fn global_lock() -> Result<std::fs::File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let path = std::env::var("SIMPLEADMIN_AT_LOCK_FILE")
        .unwrap_or_else(|_| "/tmp/simpleadmin-go-at.lock".into());
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let deadline = Instant::now() + Duration::from_secs(125);
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        if Instant::now() >= deadline {
            bail!("AT port busy")
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[cfg(not(unix))]
fn global_lock() -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_queries_are_refreshed_and_cache_bytes_are_bounded() {
        assert!(cacheable(DASHBOARD));
        assert!(cacheable(SIGNAL));
        assert!(cacheable(SMS_LIST));
        assert!(cacheable("AT+QNWPREFCFG=\"nr5g_band\""));
        assert!(!cacheable("AT+QUIMSLOT=2"));
        assert!(!cacheable("AT+CSQ;+QCFG=\"unknown\",1"));
        let mut cache = HashMap::new();
        for i in 0..4 {
            cache.insert(
                i.to_string(),
                Entry {
                    updated: Some(Instant::now()),
                    response: Arc::from("x".repeat(512 * 1024)),
                    running: None,
                },
            );
            trim_cache(&mut cache, &i.to_string());
        }
        assert!(cache.values().map(|e| e.response.len()).sum::<usize>() <= CACHE_BYTES);
        assert!(cache.contains_key("3"));
    }
    #[test]
    #[cfg(unix)]
    fn pty_preserves_grouped_commands_and_sms_prompt() {
        use std::os::fd::FromRawFd;
        let (mut master, mut slave) = (0, 0);
        let mut name = [0 as libc::c_char; 128];
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    name.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            },
            0
        );
        let path = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }
            .to_str()
            .unwrap()
            .to_owned();
        let mut master = unsafe { std::fs::File::from_raw_fd(master) };
        let _slave = unsafe { std::fs::File::from_raw_fd(slave) };
        let mut port = Port::open(&path).unwrap();
        let modem = std::thread::spawn(move || {
            fn read_until(file: &mut std::fs::File, end: u8) -> Vec<u8> {
                let mut data = Vec::new();
                loop {
                    let mut byte = [0];
                    file.read_exact(&mut byte).unwrap();
                    data.push(byte[0]);
                    if byte[0] == end {
                        return data;
                    }
                }
            }
            assert_eq!(read_until(&mut master, b'\n'), b"AT+CGMM;+CSQ\r\n");
            master.write_all(b"\r\n+QSPN: \"TOKYO\"\r\n").unwrap();
            std::thread::sleep(Duration::from_millis(20));
            master
                .write_all(b"RM520N-EU\r\n+CSQ: 20,99\r\nOK\r\n")
                .unwrap();
            assert_eq!(read_until(&mut master, b'\n'), b"AT+CMGF=0;+CMGS=3\r\n");
            master.write_all(b"\r\nOK\r\n").unwrap();
            std::thread::sleep(Duration::from_millis(20));
            master.write_all(b"> ").unwrap();
            assert_eq!(read_until(&mut master, 26), b"001122\x1a");
            master.write_all(b"\r\n+CMGS: 1\r\nOK\r\n").unwrap();
            std::thread::sleep(Duration::from_millis(50));
        });
        let response = port.execute("AT+CGMM;+CSQ", None, None).unwrap();
        assert!(response.contains("+CSQ: 20,99"));
        let response = port
            .execute("AT+CMGF=0;+CMGS=3", Some("001122"), None)
            .unwrap();
        assert!(response.contains("+CMGS: 1"));
        modem.join().unwrap();
    }
    #[tokio::test]
    async fn concurrent_readers_share_cache() {
        let at = At::start(true, vec![]).unwrap();
        let (a, b) = tokio::join!(at.fetch(DASHBOARD, true), at.fetch(DASHBOARD, true));
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(at.cache.lock().unwrap().len(), 1);
    }
    #[test]
    fn split_quoted_commands() {
        assert_eq!(
            split("AT+CGDCONT=1,\"IP\",\"a;b\";+CFUN=1"),
            ["AT+CGDCONT=1,\"IP\",\"a;b\"", "AT+CFUN=1"]
        );
    }
    #[test]
    fn final_line_only() {
        assert!(!terminal("+QSPN: \"TOKYO\""));
        assert!(terminal("\r\nOK\r\n"));
    }
}
