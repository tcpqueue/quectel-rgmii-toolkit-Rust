//! Online update. A release publishes `simpleadmin-ota.json` (version, package name, size and
//! SHA-256), its Ed25519 signature and the package itself: a gzip ustar archive of the same
//! `development/` tree the Windows assistant uploads. The module checks the signature with a
//! built-in public key (plus an optional key for a custom source), so a GitHub proxy or mirror
//! can neither change the package nor offer an older release. The verified tree is unpacked to
//! `/tmp/development` and installed by the regular install script in a separate systemd unit,
//! because the script restarts this service.
use crate::persistence::Store;
use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

pub const MANIFEST: &str = "simpleadmin-ota.json";
pub const DEFAULT_SOURCE: &str = "tcpqueue/quectel-rgmii-toolkit-Rust";
/// GitHub is often unreachable from Chinese mobile networks, so updates go through this proxy
/// unless the setting is cleared.
pub const DEFAULT_PROXY: &str = "https://ghfast.top/";
const OFFICIAL_KEY: &str = include_str!("ota-public-key.txt");
const MAX_MANIFEST: usize = 16 * 1024;
const MAX_PACKAGE: u64 = 64 * 1024 * 1024;
const MAX_UNPACKED: usize = 128 * 1024 * 1024;
const RESULT: &str = "simpleadmin-ota-result.env";
const LOG: &str = "simpleadmin-ota.log";
const RUNNER: &str = "simpleadmin-ota-run.sh";
const UNIT: &str = "simpleadmin-ota.service";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
/// A failed check (no network yet, GitHub unreachable) is retried sooner than the daily check.
const RETRY_AFTER: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Off,
    /// Check daily and show the result; installing stays manual.
    #[default]
    Check,
    /// Check daily and install newer releases without asking.
    Auto,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    /// `owner/repo` on GitHub, or an http(s) directory holding the manifest and packages.
    pub source: String,
    /// GitHub proxy: a prefix put before the full URL, or a template containing `{url}`.
    pub proxy: String,
    /// Extra trusted Ed25519 public key (base64, 32 bytes) for a fork or a private server.
    pub public_key: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Check,
            source: DEFAULT_SOURCE.into(),
            proxy: DEFAULT_PROXY.into(),
            public_key: String::new(),
        }
    }
}
impl Settings {
    pub fn normalize(mut self) -> Result<Self> {
        self.source = self.source.trim().trim_end_matches('/').to_owned();
        self.proxy = self.proxy.trim().to_owned();
        self.public_key = self.public_key.trim().to_owned();
        if self.source.is_empty() {
            self.source = DEFAULT_SOURCE.into();
        }
        ensure!(
            self.source.len() <= 512 && self.proxy.len() <= 512,
            "update source or proxy is too long"
        );
        if !github_repo(&self.source) {
            let url = reqwest::Url::parse(&self.source)
                .ok()
                .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
            ensure!(
                url.is_some(),
                "update source must be owner/repo or an http(s) address"
            );
        }
        if !self.proxy.is_empty() {
            let sample = proxied(&self.proxy, "https://github.com/a/b");
            let url = reqwest::Url::parse(&sample)
                .ok()
                .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
            ensure!(url.is_some(), "GitHub proxy must be an http(s) address");
        }
        if !self.public_key.is_empty() {
            decode_key(&self.public_key).context("public key must be 32 bytes in base64")?;
        }
        Ok(self)
    }
}
fn github_repo(source: &str) -> bool {
    let mut parts = source.split('/');
    let valid = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
    };
    valid(parts.next()) && valid(parts.next()) && parts.next().is_none()
}
/// Routes a GitHub URL through the configured proxy.
pub fn proxied(proxy: &str, url: &str) -> String {
    if proxy.is_empty() {
        url.into()
    } else if proxy.contains("{url}") {
        proxy.replace("{url}", url)
    } else {
        format!("{}/{url}", proxy.trim_end_matches('/'))
    }
}
fn decode_key(raw: &str) -> Option<[u8; 32]> {
    base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .ok()?
        .try_into()
        .ok()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub tag: String,
    pub package: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
}
impl Manifest {
    fn validate(&self) -> Result<()> {
        ensure!(version(&self.version).is_some(), "invalid release version");
        let name = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && !s.starts_with('.')
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        };
        ensure!(
            name(&self.tag) && name(&self.package),
            "invalid release file name"
        );
        ensure!(
            (1..=MAX_PACKAGE).contains(&self.size),
            "invalid package size"
        );
        ensure!(
            self.sha256.len() == 64 && self.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid package checksum"
        );
        Ok(())
    }
}
/// Release numbers as `major.minor.patch`; anything else is not a release.
pub fn version(raw: &str) -> Option<Vec<u64>> {
    let raw = raw.trim().trim_start_matches('v');
    let parts: Option<Vec<u64>> = raw
        .split('.')
        .map(|p| {
            (!p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
                .then(|| p.parse().ok())
                .flatten()
        })
        .collect();
    parts.filter(|p| (1..=4).contains(&p.len()))
}
pub fn newer(candidate: &str, current: &str) -> bool {
    match (version(candidate), version(current)) {
        (Some(mut a), Some(mut b)) => {
            let n = a.len().max(b.len());
            a.resize(n, 0);
            b.resize(n, 0);
            a > b
        }
        _ => false,
    }
}
/// Checks the manifest signature against the trusted keys and parses it.
pub fn verify_manifest(bytes: &[u8], signature: &str, keys: &[[u8; 32]]) -> Result<Manifest> {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .context("invalid update signature")?;
    let trusted = keys.iter().any(|key| {
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
            .verify(bytes, &signature)
            .is_ok()
    });
    ensure!(trusted, "update signature is not trusted");
    let manifest: Manifest = serde_json::from_slice(bytes).context("invalid update manifest")?;
    manifest.validate()?;
    Ok(manifest)
}

/// Decompresses a gzip member, refusing to grow past `limit` bytes.
pub fn gunzip(data: &[u8], limit: usize) -> Result<Vec<u8>> {
    ensure!(
        data.len() > 18 && data[..3] == [0x1f, 0x8b, 8],
        "package is not gzip"
    );
    let flags = data[3];
    let mut pos = 10;
    let skip_string = |pos: usize| -> Result<usize> {
        let end = data
            .get(pos..)
            .and_then(|rest| rest.iter().position(|b| *b == 0))
            .context("truncated gzip header")?;
        Ok(pos + end + 1)
    };
    if flags & 4 != 0 {
        let extra = data.get(pos..pos + 2).context("truncated gzip header")?;
        pos += 2 + u16::from_le_bytes([extra[0], extra[1]]) as usize;
    }
    if flags & 8 != 0 {
        pos = skip_string(pos)?;
    }
    if flags & 16 != 0 {
        pos = skip_string(pos)?;
    }
    if flags & 2 != 0 {
        pos += 2;
    }
    let body = data
        .get(pos..data.len() - 8)
        .context("truncated gzip data")?;
    let out = miniz_oxide::inflate::decompress_to_vec_with_limit(body, limit)
        .map_err(|_| anyhow::anyhow!("package cannot be decompressed"))?;
    let size = u32::from_le_bytes(data[data.len() - 4..].try_into().unwrap());
    ensure!(
        size == out.len() as u32,
        "package size mismatch after unpacking"
    );
    Ok(out)
}
pub struct Entry<'a> {
    pub path: PathBuf,
    pub dir: bool,
    pub mode: u32,
    pub data: &'a [u8],
}
fn octal(field: &[u8]) -> Result<u64> {
    let text = std::str::from_utf8(field)
        .context("invalid tar header")?
        .trim_matches(|c: char| c == '\0' || c == ' ');
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).context("invalid tar number")
}
fn cstr(field: &[u8]) -> Result<&str> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..end]).context("invalid tar name")
}
/// Lists a ustar archive. Only plain files and directories inside `development/` are accepted:
/// links, devices, absolute paths and `..` are refused because the result is installed as root.
pub fn untar(tar: &[u8]) -> Result<Vec<Entry<'_>>> {
    let mut entries = Vec::new();
    let mut offset = 0;
    while offset + 512 <= tar.len() {
        let header = &tar[offset..offset + 512];
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let expected = octal(&header[148..156])?;
        let sum: u64 = header
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    *b as u64
                }
            })
            .sum();
        ensure!(sum == expected, "tar header checksum mismatch");
        let mut name = cstr(&header[0..100])?.to_owned();
        if &header[257..262] == b"ustar" {
            let prefix = cstr(&header[345..500])?;
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        }
        let size = usize::try_from(octal(&header[124..136])?).context("tar entry too large")?;
        let mode = octal(&header[100..108])? as u32;
        let dir = match header[156] {
            b'0' | 0 => false,
            b'5' => true,
            _ => bail!("package contains an unsupported entry: {name}"),
        };
        let start = offset + 512;
        let data = tar
            .get(start..start.checked_add(size).context("tar entry too large")?)
            .context("truncated tar entry")?;
        offset = start + size.div_ceil(512) * 512;
        ensure!(!name.starts_with('/'), "package path is absolute: {name}");
        let mut path = PathBuf::new();
        for part in name.split('/').filter(|p| !p.is_empty() && *p != ".") {
            ensure!(
                part != ".." && !part.contains('\\'),
                "package path leaves its folder: {name}"
            );
            path.push(part);
        }
        if path.as_os_str().is_empty() {
            continue;
        }
        ensure!(
            path.starts_with("development") && path.components().count() <= 16,
            "package path outside development/: {name}"
        );
        entries.push(Entry {
            path,
            dir,
            mode,
            data,
        });
    }
    for required in [
        "development/SHA256SUMS",
        "development/install_simpleadmin_rust.sh",
        "development/simpleadmin/simpleadmin-httpd.armv7",
    ] {
        ensure!(
            entries
                .iter()
                .any(|e| !e.dir && e.path == Path::new(required)),
            "package is missing {required}"
        );
    }
    Ok(entries)
}
/// Writes the archive below `work`, replacing an older `work/development`.
pub fn extract(entries: &[Entry<'_>], work: &Path) -> Result<()> {
    let root = work.join("development");
    match std::fs::remove_dir_all(&root) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    for entry in entries {
        let target = work.join(&entry.path);
        if entry.dir {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, entry.data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if entry.mode & 0o111 != 0 {
                0o755
            } else {
                0o644
            };
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}
/// Shell script that installs the unpacked tree and records the outcome for the next process.
pub fn runner(work: &Path, version: &str) -> Result<String> {
    let work = work.to_str().context("invalid work directory")?;
    ensure!(
        !work.contains(['\'', '\n']) && crate::ota::version(version).is_some(),
        "invalid update parameters"
    );
    Ok(format!(
        r#"#!/bin/bash
# Written by simpleadmin-httpd for one online update; runs outside the web service.
work='{work}'
result="$work/{RESULT}"
exec >"$work/{LOG}" 2>&1
echo "[信息] 在线更新到 v{version}"
if bash "$work/development/install_simpleadmin_rust.sh"; then status=OK; else status=FAIL; fi
# The installer stops the web service before replacing it; bring it back if it failed later.
if [ "$status" = FAIL ] && command -v systemctl >/dev/null 2>&1 && ! systemctl is-active --quiet simpleadmin-httpd.service; then
    echo "[信息] 安装失败，正在恢复网页服务"
    systemctl daemon-reload || true
    systemctl start simpleadmin-httpd.service || echo "[错误] 网页服务未能恢复，请用设备助手重新安装"
fi
rm -rf "$work/development"
printf 'OTA_STATUS=%s\nOTA_VERSION=%s\nOTA_FINISHED=%s\n' "$status" '{version}' "$(date +%s)" > "$result.tmp"
mv -f "$result.tmp" "$result"
"#
    ))
}
fn read_result(work: &Path) -> Option<Value> {
    let raw = std::fs::read_to_string(work.join(RESULT)).ok()?;
    let field = |key: &str| {
        raw.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or("")
            .to_owned()
    };
    let status = field("OTA_STATUS");
    if status.is_empty() {
        return None;
    }
    let mut value = json!({
        "ok": status == "OK",
        "version": field("OTA_VERSION"),
        "finished": field("OTA_FINISHED").parse::<i64>().ok().map(|s| s * 1000),
    });
    if status != "OK" {
        value["log"] = json!(log_tail(work));
    }
    Some(value)
}
fn log_tail(work: &Path) -> String {
    let log = std::fs::read(work.join(LOG)).unwrap_or_default();
    let text = String::from_utf8_lossy(&log[log.len().saturating_sub(4096)..]).into_owned();
    let lines: Vec<_> = text.lines().collect();
    lines[lines.len().saturating_sub(20)..].join("\n")
}

#[derive(Default)]
struct State {
    phase: &'static str,
    received: u64,
    total: u64,
    error: String,
    latest: Option<Manifest>,
    release_url: Option<String>,
    checked_at: Option<i64>,
    /// When the background task checks next: a day after a success, an hour after a failure.
    next_check: Option<Instant>,
}
pub struct Ota {
    path: PathBuf,
    work: PathBuf,
    store: Arc<Store>,
    mock: bool,
    settings: Mutex<Settings>,
    state: Mutex<State>,
    busy: Arc<tokio::sync::Mutex<()>>,
    client: OnceLock<reqwest::Client>,
}
impl Ota {
    pub fn new(path: PathBuf, work: PathBuf, mock: bool, store: Arc<Store>) -> Arc<Self> {
        let settings = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .and_then(|s| s.normalize().ok())
            .unwrap_or_default();
        Arc::new(Self {
            path,
            work,
            store,
            mock,
            settings: Mutex::new(settings),
            state: Mutex::new(State {
                phase: "idle",
                ..State::default()
            }),
            busy: Arc::new(tokio::sync::Mutex::new(())),
            client: OnceLock::new(),
        })
    }
    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
    pub fn snapshot(&self) -> Value {
        let settings = self.settings();
        let s = self.state.lock().unwrap();
        let current = env!("CARGO_PKG_VERSION");
        let latest = s.latest.as_ref().map(|m| {
            json!({"version":m.version,"tag":m.tag,"published":m.published,"size":m.size,
                "newer":newer(&m.version,current),"url":s.release_url})
        });
        json!({
            "current": current,
            "mock": self.mock,
            "settings": settings,
            "default_source": DEFAULT_SOURCE,
            "default_proxy": DEFAULT_PROXY,
            "phase": s.phase,
            "received": s.received,
            "total": s.total,
            "error": s.error,
            "latest": latest,
            "checked_at": s.checked_at,
            "last_result": read_result(&self.work),
        })
    }
    pub async fn save(&self, body: &str) -> Result<Value> {
        let next = serde_json::from_str::<Settings>(body)?.normalize()?;
        let bytes = format!("{}\n", serde_json::to_string(&next)?);
        let (store, path) = (self.store.clone(), self.path.clone());
        let saved =
            tokio::task::spawn_blocking(move || store.write(&path, bytes.as_bytes(), 0o600))
                .await?;
        if saved.is_ok() || saved.as_ref().is_err_and(|e| e.committed) {
            let changed = {
                let mut current = self.settings.lock().unwrap();
                let changed = current.source != next.source
                    || current.proxy != next.proxy
                    || current.public_key != next.public_key;
                *current = next;
                changed
            };
            if changed {
                // A different source answers for itself on the next check.
                let mut s = self.state.lock().unwrap();
                s.latest = None;
                s.release_url = None;
                s.next_check = None;
                s.checked_at = None;
                s.error.clear();
            }
        }
        saved?;
        Ok(self.snapshot())
    }
    fn client(&self) -> Result<&reqwest::Client> {
        if self.client.get().is_none() {
            let client = reqwest::Client::builder()
                .dns_resolver(Arc::new(crate::resolver::HttpResolver::default()))
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::limited(5))
                .no_proxy()
                .user_agent(concat!("SimpleAdmin-OTA/", env!("CARGO_PKG_VERSION")))
                .pool_max_idle_per_host(1)
                .pool_idle_timeout(Duration::from_secs(30))
                .build()
                .context("HTTP client unavailable")?;
            let _ = self.client.set(client);
        }
        Ok(self.client.get().unwrap())
    }
    /// URLs for the manifest, its signature and a package, plus the release page.
    fn urls(settings: &Settings, manifest: Option<&Manifest>) -> (String, Option<String>) {
        let file = |m: Option<&Manifest>| match m {
            Some(m) => m.package.clone(),
            None => MANIFEST.into(),
        };
        if github_repo(&settings.source) {
            let repo = &settings.source;
            let url = match manifest {
                Some(m) => format!(
                    "https://github.com/{repo}/releases/download/{}/{}",
                    m.tag, m.package
                ),
                None => format!("https://github.com/{repo}/releases/latest/download/{MANIFEST}"),
            };
            let page =
                manifest.map(|m| format!("https://github.com/{repo}/releases/tag/{}", m.tag));
            (proxied(&settings.proxy, &url), page)
        } else {
            (format!("{}/{}", settings.source, file(manifest)), None)
        }
    }
    async fn get(&self, url: &str, limit: u64, timeout: Duration) -> Result<Vec<u8>> {
        let mut response = self
            .client()?
            .get(url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("download failed: {}", short_error(&e)))?;
        let status = response.status();
        ensure!(
            status.is_success(),
            "download failed: HTTP {}",
            status.as_u16()
        );
        if let Some(length) = response.content_length() {
            ensure!(length <= limit, "download is larger than expected");
            self.state.lock().unwrap().total = length;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| anyhow::anyhow!("download interrupted: {}", short_error(&e)))?
        {
            ensure!(
                bytes.len() as u64 + chunk.len() as u64 <= limit,
                "download is larger than expected"
            );
            bytes.extend_from_slice(&chunk);
            self.state.lock().unwrap().received = bytes.len() as u64;
        }
        Ok(bytes)
    }
    fn keys(settings: &Settings) -> Vec<[u8; 32]> {
        decode_key(OFFICIAL_KEY)
            .into_iter()
            .chain(decode_key(&settings.public_key))
            .collect()
    }
    async fn fetch_manifest(&self, settings: &Settings) -> Result<Manifest> {
        let (url, _) = Self::urls(settings, None);
        let bytes = self
            .get(&url, MAX_MANIFEST as u64, Duration::from_secs(30))
            .await?;
        let signature_url = if github_repo(&settings.source) {
            let repo = &settings.source;
            proxied(
                &settings.proxy,
                &format!("https://github.com/{repo}/releases/latest/download/{MANIFEST}.sig"),
            )
        } else {
            format!("{url}.sig")
        };
        let signature = self
            .get(&signature_url, 1024, Duration::from_secs(30))
            .await?;
        verify_manifest(
            &bytes,
            &String::from_utf8_lossy(&signature),
            &Self::keys(settings),
        )
    }
    fn begin(&self, phase: &'static str) {
        let mut s = self.state.lock().unwrap();
        s.phase = phase;
        s.error.clear();
        s.received = 0;
        s.total = 0;
    }
    fn fail(&self, error: &anyhow::Error) {
        let mut s = self.state.lock().unwrap();
        s.phase = "idle";
        s.error = format!("{error:#}");
    }
    async fn check_locked(&self) -> Result<Manifest> {
        let settings = self.settings();
        self.begin("checking");
        let result = self.fetch_manifest(&settings).await;
        let mut s = self.state.lock().unwrap();
        s.phase = "idle";
        s.next_check = Some(
            Instant::now()
                + if result.is_ok() {
                    CHECK_EVERY
                } else {
                    RETRY_AFTER
                },
        );
        s.checked_at = Some(chrono::Utc::now().timestamp_millis());
        match result {
            Ok(manifest) => {
                s.release_url = Self::urls(&settings, Some(&manifest)).1;
                s.latest = Some(manifest.clone());
                Ok(manifest)
            }
            Err(error) => {
                s.error = format!("{error:#}");
                Err(error)
            }
        }
    }
    pub async fn check_now(&self) -> Result<Value> {
        let Ok(_guard) = self.busy.try_lock() else {
            bail!("an update check or installation is already running")
        };
        let _ = self.check_locked().await;
        Ok(self.snapshot())
    }
    /// Starts installing the newest release in the background.
    pub fn install(self: &Arc<Self>) -> Result<Value> {
        let guard =
            self.busy.clone().try_lock_owned().map_err(|_| {
                anyhow::anyhow!("an update check or installation is already running")
            })?;
        // Report progress from the first reply on, before the task gets scheduled.
        self.begin("checking");
        let this = self.clone();
        tokio::spawn(async move {
            let _guard = guard;
            if let Err(error) = this.install_locked().await {
                this.fail(&error);
            }
        });
        Ok(self.snapshot())
    }
    async fn install_locked(&self) -> Result<()> {
        // Fetch a fresh manifest: the one shown on the page may come from another source.
        let manifest = self.check_locked().await?;
        let current = env!("CARGO_PKG_VERSION");
        ensure!(
            newer(&manifest.version, current),
            "v{current} is already the latest release"
        );
        let settings = self.settings();
        self.begin("downloading");
        let (url, _) = Self::urls(&settings, Some(&manifest));
        let package = self
            .get(&url, manifest.size, Duration::from_secs(900))
            .await?;
        ensure!(
            package.len() as u64 == manifest.size,
            "downloaded package is incomplete"
        );
        ensure!(
            hex::encode(Sha256::digest(&package)).eq_ignore_ascii_case(&manifest.sha256),
            "package checksum does not match the signed manifest"
        );
        self.begin("installing");
        let work = self.work.clone();
        let version = manifest.version.clone();
        let mock = self.mock;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let tar = gunzip(&package, MAX_UNPACKED)?;
            let entries = untar(&tar)?;
            std::fs::create_dir_all(&work)?;
            let _ = std::fs::remove_file(work.join(RESULT));
            extract(&entries, &work)?;
            let script = work.join(RUNNER);
            std::fs::write(&script, runner(&work, &version)?)?;
            if mock {
                // Previews unpack and check the package but never run the device installer.
                std::fs::write(
                    work.join(RESULT),
                    format!(
                        "OTA_STATUS=OK\nOTA_VERSION={version}\nOTA_FINISHED={}\n",
                        chrono::Utc::now().timestamp()
                    ),
                )?;
                return Ok(());
            }
            launch(&script)
        })
        .await??;
        if self.mock {
            self.state.lock().unwrap().phase = "idle";
            return Ok(());
        }
        // This service is restarted by the installer; until then report failures it records.
        let deadline = Instant::now() + Duration::from_secs(1200);
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if let Some(result) = read_result(&self.work) {
                if result["ok"] == true {
                    self.state.lock().unwrap().phase = "idle";
                    return Ok(());
                }
                bail!("installation failed; see the log below")
            }
        }
        bail!("installation did not finish within 20 minutes")
    }
    pub fn start(self: &Arc<Self>) {
        if self.mock {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            // Leave time for the modem to register before the first daily check.
            tokio::time::sleep(Duration::from_secs(600)).await;
            loop {
                let settings = this.settings();
                let due = this
                    .state
                    .lock()
                    .unwrap()
                    .next_check
                    .is_none_or(|t| Instant::now() >= t);
                if settings.mode != Mode::Off && due {
                    let newer_release = match this.busy.try_lock() {
                        Ok(_guard) => this
                            .check_locked()
                            .await
                            .is_ok_and(|m| newer(&m.version, env!("CARGO_PKG_VERSION"))),
                        Err(_) => false,
                    };
                    if newer_release && settings.mode == Mode::Auto {
                        let _ = this.install();
                    }
                }
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });
    }
}
fn short_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "timed out".into()
    } else if error.is_connect() {
        "connection failed".into()
    } else if error.is_redirect() {
        "too many redirects".into()
    } else {
        "network or TLS error".into()
    }
}
/// Runs the installer where stopping this service cannot kill it: in the on-demand unit the
/// install script ships, under `systemd-run`, or detached when the service is not in systemd.
#[cfg(unix)]
fn launch(script: &Path) -> Result<()> {
    use std::process::{Command, Stdio};
    let quiet = |command: &mut Command| {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    let systemd = Path::new("/run/systemd/system").is_dir();
    let unit_installed = ["/lib/systemd/system", "/etc/systemd/system"]
        .iter()
        .any(|dir| Path::new(dir).join(UNIT).is_file());
    if systemd && unit_installed && script == Path::new("/tmp").join(RUNNER) {
        let _ = quiet(Command::new("systemctl").args(["reset-failed", UNIT]));
        if quiet(Command::new("systemctl").args(["start", "--no-block", UNIT])) {
            return Ok(());
        }
    }
    if systemd {
        let unit = format!("simpleadmin-ota-{}", chrono::Utc::now().timestamp());
        if quiet(
            Command::new("systemd-run")
                .args(["--unit", &unit, "--quiet", "/bin/bash"])
                .arg(script),
        ) {
            return Ok(());
        }
    }
    let in_service = std::fs::read_to_string("/proc/self/cgroup")
        .is_ok_and(|c| c.contains("simpleadmin-httpd.service"));
    ensure!(
        !in_service,
        "cannot start the installer outside the web service; install this release with the Windows assistant"
    );
    spawn_detached(script)
}
#[cfg(unix)]
pub fn spawn_detached(script: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new("/bin/bash");
    command
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command.spawn().context("cannot start the installer")?;
    Ok(())
}
#[cfg(not(unix))]
fn launch(_: &Path) -> Result<()> {
    bail!("online update requires the module")
}

#[cfg(test)]
#[path = "../tests/ota/mod.rs"]
mod tests;
