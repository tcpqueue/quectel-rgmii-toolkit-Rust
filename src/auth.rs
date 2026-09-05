use crate::persistence::{SavedError, Store};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
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
    mock_root: Mutex<String>,
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
            store.write(&path, b"admin:admin\n", 0o600)?
        }
        read(&path)?;
        Ok(Self {
            path,
            sessions: Mutex::new(HashMap::new()),
            mutation: tokio::sync::Mutex::new(()),
            mock_root: Mutex::new("admin".into()),
        })
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
        let params = sha_crypt::Sha512Params::new(5000).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let hash = sha_crypt::sha512_simple(next, &params).map_err(|e| anyhow::anyhow!("{e:?}"))?;
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
}
