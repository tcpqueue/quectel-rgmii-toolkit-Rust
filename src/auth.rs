use crate::persistence::{SavedError, Store};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::watch;

pub const COOKIE: &str = "zbims_session";
const TTL: Duration = Duration::from_secs(86400);
struct Session {
    expires: Instant,
    revoked: watch::Sender<bool>,
}
pub struct Auth {
    pub path: PathBuf,
    sessions: Mutex<HashMap<String, Session>>,
    pub mutation: tokio::sync::Mutex<()>,
    pub throttle: Throttle,
    mock_root: Mutex<String>,
}
/// Failed password attempts allowed before each further failure locks the client out.
const FREE_FAILURES: u32 = 5;
const MAX_LOCKOUT: Duration = Duration::from_secs(900);
const FAILURE_MEMORY: Duration = Duration::from_secs(3600);
const MAX_CLIENTS: usize = 1024;
struct Failure {
    count: u32,
    until: Instant,
    last: Instant,
}
/// Per-client password failure backoff shared by Web login, password changes and the root console.
#[derive(Default)]
pub struct Throttle {
    clients: Mutex<HashMap<[u8; 8], Failure>>,
}
fn client_key(ip: Option<IpAddr>) -> [u8; 8] {
    match ip.map(|ip| ip.to_canonical()) {
        Some(IpAddr::V4(ip)) => {
            let mut key = [0; 8];
            key[..4].copy_from_slice(&ip.octets());
            key
        }
        // One IPv6 host normally controls a whole /64.
        Some(IpAddr::V6(ip)) => ip.octets()[..8].try_into().unwrap(),
        None => [0xff; 8],
    }
}
impl Throttle {
    /// Returns the remaining lockout when the client must wait before another attempt.
    pub fn check(&self, ip: Option<IpAddr>) -> Option<Duration> {
        let clients = self.clients.lock().unwrap();
        let now = Instant::now();
        clients
            .get(&client_key(ip))
            .filter(|f| f.until > now)
            .map(|f| f.until - now)
    }
    pub fn failed(&self, ip: Option<IpAddr>) {
        let mut clients = self.clients.lock().unwrap();
        let now = Instant::now();
        clients.retain(|_, f| now.duration_since(f.last) < FAILURE_MEMORY);
        if clients.len() >= MAX_CLIENTS
            && let Some(oldest) = clients.iter().min_by_key(|(_, f)| f.last).map(|(k, _)| *k)
        {
            clients.remove(&oldest);
        }
        let entry = clients.entry(client_key(ip)).or_insert(Failure {
            count: 0,
            until: now,
            last: now,
        });
        entry.count = entry.count.saturating_add(1);
        entry.last = now;
        if entry.count > FREE_FAILURES {
            let exponent = (entry.count - FREE_FAILURES - 1).min(16);
            entry.until = now + Duration::from_secs(1 << exponent).min(MAX_LOCKOUT);
        }
    }
    pub fn succeeded(&self, ip: Option<IpAddr>) {
        self.clients.lock().unwrap().remove(&client_key(ip));
    }
}
pub fn hashed(stored: &str) -> bool {
    stored.starts_with("$6$") || stored.starts_with("$5$")
}
pub fn hash_password(password: &str) -> Result<String> {
    validate(password)?;
    let params = sha_crypt::Sha512Params::new(5000).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    sha_crypt::sha512_simple(password, &params).map_err(|e| anyhow::anyhow!("{e:?}"))
}
/// Accepts the current SHA-crypt format and legacy plaintext auth files.
pub fn password_matches(input: &str, stored: &str) -> bool {
    if hashed(stored) {
        verify_hash(input, stored)
    } else {
        equal(input, stored)
    }
}
/// One auth-file line with a freshly salted password hash.
pub fn record(user: &str, password: &str) -> Result<String> {
    Ok(format!("{user}:{}\n", hash_password(password)?))
}
pub fn equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
pub fn validate(password: &str) -> Result<()> {
    if password.is_empty() || password.len() > 128 || password.contains(['\r', '\n', '\0']) {
        bail!("password must contain 1 to 128 bytes and no line breaks or NUL")
    }
    Ok(())
}
pub fn read(path: &Path) -> Result<(String, String)> {
    let data = std::fs::read_to_string(path)?;
    let line = data
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .context("empty auth file")?;
    let (user, pass) = line.split_once(':').context("invalid auth file")?;
    if user.is_empty() {
        bail!("empty username")
    }
    Ok((user.into(), pass.into()))
}
impl Auth {
    pub fn new(path: PathBuf, store: &Store) -> Result<Self> {
        if !path.exists() || std::fs::metadata(&path)?.len() == 0 {
            store.write(&path, record("admin", "admin")?.as_bytes(), 0o600)?
        }
        let (user, password) = read(&path)?;
        if !hashed(&password) {
            // Upgrade plaintext files written by older releases, installers and helper scripts.
            match record(&user, &password) {
                Ok(line) => {
                    if let Err(error) = store.write(&path, line.as_bytes(), 0o600) {
                        eprintln!("cannot hash stored Web password: {error}")
                    }
                }
                Err(error) => eprintln!("cannot hash stored Web password: {error}"),
            }
        }
        Ok(Self {
            path,
            sessions: Mutex::new(HashMap::new()),
            mutation: tokio::sync::Mutex::new(()),
            throttle: Throttle::default(),
            mock_root: Mutex::new("admin".into()),
        })
    }
    /// Queues one password check. Clients locked out by earlier failures get the remaining
    /// wait instead; the check repeats after the queue, so a parallel burst cannot skip it.
    pub async fn password_attempt(
        &self,
        ip: Option<IpAddr>,
    ) -> std::result::Result<tokio::sync::MutexGuard<'_, ()>, Duration> {
        if let Some(wait) = self.throttle.check(ip) {
            return Err(wait);
        }
        let guard = self.mutation.lock().await;
        match self.throttle.check(ip) {
            Some(wait) => Err(wait),
            None => Ok(guard),
        }
    }
    pub fn create(&self) -> String {
        use rand::RngCore;
        let mut bytes = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let mut sessions = self.sessions.lock().unwrap();
        let now = Instant::now();
        sessions.retain(|_, s| {
            if s.expires <= now {
                let _ = s.revoked.send(true);
                false
            } else {
                true
            }
        });
        if sessions.len() >= 128
            && let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, s)| s.expires)
                .map(|(k, _)| k.clone())
            && let Some(s) = sessions.remove(&oldest)
        {
            let _ = s.revoked.send(true);
        }
        let (tx, _) = watch::channel(false);
        sessions.insert(
            token.clone(),
            Session {
                expires: now + TTL,
                revoked: tx,
            },
        );
        token
    }
    pub fn valid(&self, token: &str, touch: bool) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        let now = Instant::now();
        if let Some(s) = sessions.get_mut(token)
            && s.expires > now
        {
            if touch {
                s.expires = now + TTL
            }
            return true;
        }
        if let Some(s) = sessions.remove(token) {
            let _ = s.revoked.send(true);
        }
        false
    }
    pub fn watch(&self, token: &str) -> Option<watch::Receiver<bool>> {
        self.sessions
            .lock()
            .unwrap()
            .get(token)
            .map(|s| s.revoked.subscribe())
    }
    pub fn revoke(&self, token: &str) {
        if let Some(s) = self.sessions.lock().unwrap().remove(token) {
            let _ = s.revoked.send(true);
        }
    }
    pub fn revoke_all(&self) {
        for (_, s) in self.sessions.lock().unwrap().drain() {
            let _ = s.revoked.send(true);
        }
    }
    pub fn prune(&self) {
        let mut s = self.sessions.lock().unwrap();
        let now = Instant::now();
        s.retain(|_, s| {
            if s.expires <= now {
                let _ = s.revoked.send(true);
                false
            } else {
                true
            }
        });
    }
    pub fn root_matches(&self, password: &str, mock: bool) -> bool {
        if mock {
            equal(password, &self.mock_root.lock().unwrap())
        } else {
            std::fs::read_to_string("/etc/shadow")
                .ok()
                .and_then(|s| root_hash(&s).map(str::to_owned))
                .is_some_and(|h| verify_hash(password, &h))
        }
    }
    pub fn mock_change_root(&self, next: &str) {
        *self.mock_root.lock().unwrap() = next.into();
    }
}
fn root_hash(raw: &str) -> Option<&str> {
    raw.lines().find_map(|line| {
        line.strip_prefix("root:")
            .and_then(|rest| rest.split(':').next())
    })
}
pub fn verify_hash(password: &str, hash: &str) -> bool {
    if validate(password).is_err() {
        return false;
    }
    if hash.starts_with("$6$") {
        return sha_crypt::sha512_check(password, hash).is_ok();
    }
    if hash.starts_with("$5$") {
        return sha_crypt::sha256_check(password, hash).is_ok();
    }
    // Older firmware may have an MD5 shadow hash; upgrade it on the next change.
    if hash.starts_with("$1$") {
        use std::io::Write;
        let salt = hash.split('$').nth(2).unwrap_or("");
        if let Ok(mut child) = std::process::Command::new("openssl")
            .args(["passwd", "-1", "-salt", salt, "-stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(format!("{password}\n").as_bytes());
            }
            if let Ok(output) = child.wait_with_output() {
                return output.status.success()
                    && equal(String::from_utf8_lossy(&output.stdout).trim(), hash);
            }
        }
    }
    false
}
pub fn change_root(
    store: &Store,
    current: &str,
    next: &str,
    initialize: bool,
) -> std::result::Result<(), SavedError> {
    store.update(Path::new("/etc/shadow"), 0o600, |bytes| {
        validate(next)?;
        let old = std::str::from_utf8(bytes)?;
        let hash = root_hash(old).context("root account unavailable")?;
        if !initialize && !verify_hash(current, hash) {
            bail!("current root password incorrect")
        }
        if verify_hash(next, hash) {
            return Ok(bytes.to_vec());
        }
        let hash = hash_password(next)?;
        let mut result = String::new();
        for line in old.lines() {
            if line.starts_with("root:") {
                let mut fields: Vec<_> = line.split(':').map(str::to_owned).collect();
                fields[1] = hash.clone();
                if fields.len() > 2 {
                    fields[2] = (crate::telemetry::now() / 86400000).to_string()
                }
                result.push_str(&fields.join(":"))
            } else {
                result.push_str(line)
            }
            result.push('\n')
        }
        Ok(result.into_bytes())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revoked_sessions_notify_sockets() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::new(dir.path().join("auth"), &Store::new(true)).unwrap();
        let token = auth.create();
        let receiver = auth.watch(&token).unwrap();
        assert!(auth.valid(&token, false));
        auth.revoke_all();
        assert!(*receiver.borrow());
        assert!(!auth.valid(&token, false));
    }
    #[test]
    fn plaintext_auth_files_are_upgraded_to_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth");
        std::fs::write(&path, "owner:legacy:secret\n").unwrap();
        Auth::new(path.clone(), &Store::new(true)).unwrap();
        let (user, stored) = read(&path).unwrap();
        assert_eq!(user, "owner");
        assert!(stored.starts_with("$6$"));
        assert!(password_matches("legacy:secret", &stored));
        assert!(!password_matches("legacy", &stored));
        assert!(password_matches("plain", "plain"));
        assert!(!password_matches("$6$x", "plain"));
    }
    #[test]
    fn repeated_failures_lock_out_one_client_only() {
        let throttle = Throttle::default();
        let attacker = Some("192.0.2.7".parse().unwrap());
        let neighbour = Some("2001:db8::1".parse().unwrap());
        for _ in 0..FREE_FAILURES {
            assert!(throttle.check(attacker).is_none());
            throttle.failed(attacker);
        }
        assert!(throttle.check(attacker).is_none());
        throttle.failed(attacker);
        assert!(throttle.check(attacker).is_some());
        // Addresses inside the same IPv6 /64 share a bucket; other clients are unaffected.
        assert!(throttle.check(neighbour).is_none());
        assert!(
            throttle
                .check(Some("::ffff:192.0.2.7".parse().unwrap()))
                .is_some()
        );
        throttle.succeeded(attacker);
        assert!(throttle.check(attacker).is_none());
    }
}
