use super::*;
use ring::signature::KeyPair;

fn keypair() -> ring::signature::Ed25519KeyPair {
    let pkcs8 =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}
fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
/// Minimal ustar writer for test archives.
fn tar(entries: &[(&str, u8, u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, kind, mode, data) in entries {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
        header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        header[156] = *kind;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|b| *b as u32).sum();
        header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}
fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
    out.extend(0u32.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}
fn package() -> Vec<u8> {
    gzip(&tar(&[
        ("development/", b'5', 0o755, b""),
        ("development/SHA256SUMS", b'0', 0o644, b"sums\n"),
        (
            "development/install_simpleadmin_rust.sh",
            b'0',
            0o755,
            b"#!/bin/bash\n",
        ),
        (
            "development/simpleadmin/simpleadmin-httpd.armv7",
            b'0',
            0o755,
            b"ELF",
        ),
    ]))
}

#[test]
fn versions_compare_numerically_and_reject_other_text() {
    assert!(newer("0.3.10", "0.3.9"));
    assert!(newer("v1.0", "0.9.9"));
    assert!(!newer("0.3.1", "0.3.1"));
    assert!(!newer("0.3.0", "0.3.1"));
    assert!(!newer("0.3.2-beta", "0.3.1"));
    assert!(!newer("latest", "0.3.1"));
    assert!(version("1..2").is_none());
}

#[test]
fn proxy_and_source_settings_are_validated() {
    let url = "https://github.com/o/r/releases/latest/download/simpleadmin-ota.json";
    assert_eq!(proxied("", url), url);
    assert_eq!(
        proxied("https://ghfast.top/", url),
        format!("https://ghfast.top/{url}")
    );
    assert_eq!(
        proxied("https://mirror.test/get?u={url}", url),
        format!("https://mirror.test/get?u={url}")
    );
    let settings = |source: &str, proxy: &str, key: &str| Settings {
        mode: Mode::Auto,
        source: source.into(),
        proxy: proxy.into(),
        public_key: key.into(),
    };
    let ok = settings(" me/fork/ ", " https://ghfast.top ", "")
        .normalize()
        .unwrap();
    assert_eq!(ok.source, "me/fork");
    assert_eq!(ok.proxy, "https://ghfast.top");
    assert_eq!(
        settings("", "", "").normalize().unwrap().source,
        DEFAULT_SOURCE
    );
    assert!(
        settings("https://updates.example/simpleadmin", "", "")
            .normalize()
            .is_ok()
    );
    for (source, proxy, key) in [
        ("me/../x", "", ""),
        ("ftp://host/x", "", ""),
        ("a/b/c", "", ""),
        ("me/fork", "ghfast.top", ""),
        ("me/fork", "", "not-a-key"),
        ("me/fork", "", &b64(&[1; 16])),
    ] {
        assert!(
            settings(source, proxy, key).normalize().is_err(),
            "{source} {proxy} {key}"
        );
    }
    assert!(settings("me/fork", "", &b64(&[1; 32])).normalize().is_ok());
}

#[test]
fn manifests_need_a_trusted_signature() {
    let official = keypair();
    let custom = keypair();
    let key = |k: &ring::signature::Ed25519KeyPair| -> [u8; 32] {
        k.public_key().as_ref().try_into().unwrap()
    };
    let body = br#"{"version":"9.9.9","tag":"v9.9.9","package":"simpleadmin-ota-9.9.9.tar.gz","size":10,"sha256":"00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"}"#;
    let signature = b64(official.sign(body).as_ref());
    let manifest = verify_manifest(body, &signature, &[key(&official)]).unwrap();
    assert_eq!(manifest.version, "9.9.9");
    // Only listed keys are trusted; a custom source adds its key next to the official one.
    assert!(verify_manifest(body, &signature, &[key(&custom)]).is_err());
    let forked = b64(custom.sign(body).as_ref());
    assert!(verify_manifest(body, &forked, &[key(&official), key(&custom)]).is_ok());
    // Any change to the signed bytes, such as a proxy swapping the checksum, is rejected.
    let mut tampered = body.to_vec();
    tampered[20] = b'8';
    assert!(verify_manifest(&tampered, &signature, &[key(&official)]).is_err());
    // A correctly signed manifest still has to name a safe package.
    let bad = br#"{"version":"9.9.9","tag":"v9.9.9","package":"../x","size":10,"sha256":"00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"}"#;
    let signed = b64(official.sign(bad).as_ref());
    assert!(verify_manifest(bad, &signed, &[key(&official)]).is_err());
    assert_eq!(decode_key(OFFICIAL_KEY).map(|k| k.len()), Some(32));
}

#[test]
fn archives_only_unpack_plain_files_inside_development() {
    let entries = gunzip(&package(), 1 << 20).unwrap();
    let listed = untar(&entries).unwrap();
    assert_eq!(listed.len(), 4);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("development/stale")).unwrap();
    extract(&listed, dir.path()).unwrap();
    assert!(!dir.path().join("development/stale").exists());
    assert_eq!(
        std::fs::read(
            dir.path()
                .join("development/simpleadmin/simpleadmin-httpd.armv7")
        )
        .unwrap(),
        b"ELF"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &str| {
            std::fs::metadata(dir.path().join(p))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("development/install_simpleadmin_rust.sh"), 0o755);
        assert_eq!(mode("development/SHA256SUMS"), 0o644);
    }
    let required: [(&str, u8, u32, &[u8]); 3] = [
        ("development/SHA256SUMS", b'0', 0o644, b""),
        ("development/install_simpleadmin_rust.sh", b'0', 0o755, b""),
        (
            "development/simpleadmin/simpleadmin-httpd.armv7",
            b'0',
            0o755,
            b"",
        ),
    ];
    for bad in [
        ("development/../etc/shadow", b'0', 0o644, b"x".as_slice()),
        ("/development/x", b'0', 0o644, b"x"),
        ("other/x", b'0', 0o644, b"x"),
        ("development/link", b'2', 0o777, b""),
        ("development/dev", b'3', 0o644, b""),
    ] {
        let mut list = required.to_vec();
        list.push(bad);
        assert!(untar(&tar(&list)).is_err(), "{}", bad.0);
    }
    assert!(untar(&tar(&required[..2])).is_err());
    let mut corrupt = tar(&required);
    corrupt[10] ^= 1;
    assert!(untar(&corrupt).is_err());
    assert!(gunzip(b"not gzip at all, really", 100).is_err());
    assert!(gunzip(&package(), 100).is_err());
}

#[test]
#[cfg(unix)]
fn system_tar_packages_are_readable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("development");
    std::fs::create_dir_all(root.join("simpleadmin/www/js/pages")).unwrap();
    std::fs::write(root.join("SHA256SUMS"), "sums").unwrap();
    std::fs::write(root.join("install_simpleadmin_rust.sh"), "#!/bin/bash\n").unwrap();
    std::fs::write(root.join("simpleadmin/simpleadmin-httpd.armv7"), "ELF").unwrap();
    let long = "a-rather-long-file-name-for-the-ustar-prefix-field-check.js";
    std::fs::write(root.join("simpleadmin/www/js/pages").join(long), "x").unwrap();
    let archive = dir.path().join("p.tar.gz");
    let status = std::process::Command::new("tar")
        .current_dir(dir.path())
        .args(["--format=ustar", "-czf"])
        .arg(&archive)
        .arg("development")
        .status();
    let Ok(status) = status else { return };
    assert!(status.success());
    let tar = gunzip(&std::fs::read(&archive).unwrap(), 1 << 20).unwrap();
    let entries = untar(&tar).unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e.path.ends_with(long) && e.data == b"x")
    );
}

/// Runs the generated runner with `script` as the installer and a stand-in `systemctl` that
/// records its calls and reports the web service as running or stopped.
#[cfg(unix)]
fn run_runner(script: &str, service_running: bool) -> (Value, String, bool) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("development");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("install_simpleadmin_rust.sh"), script).unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let calls = dir.path().join("calls");
    std::fs::write(
        bin.join("systemctl"),
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\n[ \"$1\" != is-active ] || exit {}\n",
            calls.display(),
            if service_running { 0 } else { 3 }
        ),
    )
    .unwrap();
    std::fs::set_permissions(
        bin.join("systemctl"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let runner_path = dir.path().join("run.sh");
    std::fs::write(&runner_path, runner(dir.path(), "9.9.9").unwrap()).unwrap();
    let status = std::process::Command::new("bash")
        .arg(&runner_path)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .status()
        .unwrap();
    assert!(status.success());
    (
        read_result(dir.path()).unwrap(),
        std::fs::read_to_string(&calls).unwrap_or_default(),
        tree.exists(),
    )
}

#[test]
#[cfg(unix)]
fn runner_records_the_install_outcome() {
    for (script, ok) in [("echo installed", true), ("echo broken; exit 3", false)] {
        let (result, _, tree_left) = run_runner(script, true);
        assert_eq!(result["ok"], ok);
        assert_eq!(result["version"], "9.9.9");
        assert!(!tree_left, "unpacked tree is removed afterwards");
        if !ok {
            assert!(result["log"].as_str().unwrap().contains("broken"));
        }
    }
    assert!(runner(Path::new("/tmp/x'y"), "1.0.0").is_err());
    assert!(runner(Path::new("/tmp"), "1.0.0;reboot").is_err());
}

#[test]
#[cfg(unix)]
fn failed_install_restarts_a_stopped_web_service() {
    // Success, or a failure that left the service running: nothing to restart.
    for (script, running) in [("exit 0", false), ("exit 1", true)] {
        let (_, calls, _) = run_runner(script, running);
        assert!(!calls.contains("start"), "{calls}");
    }
    let (result, calls, _) = run_runner("exit 1", false);
    assert_eq!(result["ok"], false);
    assert!(calls.contains("start simpleadmin-httpd.service"), "{calls}");
    assert!(result["log"].as_str().unwrap().contains("正在恢复网页服务"));
}

#[tokio::test]
async fn failed_check_is_retried_within_the_hour() {
    let dir = tempfile::tempdir().unwrap();
    let ota = Ota::new(
        dir.path().join("ota-settings.json"),
        dir.path().join("work"),
        false,
        Arc::new(Store::new(true)),
    );
    // Nothing listens on the discard port, so the check fails at once.
    *ota.settings.lock().unwrap() = Settings {
        source: "http://127.0.0.1:9".into(),
        ..Settings::default()
    };
    let started = Instant::now();
    assert!(ota.check_locked().await.is_err());
    let next = ota.state.lock().unwrap().next_check.unwrap();
    assert!(next >= started + RETRY_AFTER && next <= Instant::now() + RETRY_AFTER);
}

#[tokio::test]
async fn check_and_install_from_a_custom_source() {
    let signer = keypair();
    let package = package();
    let manifest = serde_json::to_vec(&json!({
        "version": "9.9.9",
        "tag": "v9.9.9",
        "package": "simpleadmin-ota-9.9.9.tar.gz",
        "size": package.len(),
        "sha256": hex::encode(Sha256::digest(&package)),
    }))
    .unwrap();
    let signature = b64(signer.sign(&manifest).as_ref());
    let tampered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let served = tampered.clone();
    let router = axum::Router::new()
        .route(
            "/ota/simpleadmin-ota.json",
            axum::routing::get(move || {
                let manifest = manifest.clone();
                async move { manifest }
            }),
        )
        .route(
            "/ota/simpleadmin-ota.json.sig",
            axum::routing::get(move || {
                let signature = signature.clone();
                async move { signature }
            }),
        )
        .route(
            "/ota/simpleadmin-ota-9.9.9.tar.gz",
            axum::routing::get(move || {
                let mut package = package.clone();
                if served.load(std::sync::atomic::Ordering::SeqCst) {
                    let last = package.len() - 9;
                    package[last] ^= 1;
                }
                async move { package }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });

    let dir = tempfile::tempdir().unwrap();
    let ota = Ota::new(
        dir.path().join("ota-settings.json"),
        dir.path().join("work"),
        true,
        Arc::new(Store::new(true)),
    );
    // The official key alone does not trust this server.
    let source = json!({"mode":"check","source":format!("http://{address}/ota/"),"proxy":"","public_key":""});
    ota.save(&source.to_string()).await.unwrap();
    let snapshot = ota.check_now().await.unwrap();
    assert!(snapshot["latest"].is_null());
    assert!(snapshot["error"].as_str().unwrap().contains("not trusted"));
    let mut trusted = source.clone();
    trusted["public_key"] = json!(b64(signer.public_key().as_ref()));
    let saved = ota.save(&trusted.to_string()).await.unwrap();
    assert_eq!(saved["settings"]["source"], format!("http://{address}/ota"));
    assert!(
        std::fs::read_to_string(dir.path().join("ota-settings.json"))
            .unwrap()
            .contains("public_key")
    );
    let snapshot = ota.check_now().await.unwrap();
    assert_eq!(snapshot["latest"]["version"], "9.9.9");
    assert_eq!(snapshot["latest"]["newer"], true);
    assert_eq!(snapshot["error"], "");

    let wait = || async {
        for _ in 0..200 {
            if ota.snapshot()["phase"] == "idle" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        ota.snapshot()
    };
    // A package that does not match the signed checksum is never unpacked.
    tampered.store(true, std::sync::atomic::Ordering::SeqCst);
    ota.install().unwrap();
    let failed = wait().await;
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("checksum does not match")
    );
    assert!(!dir.path().join("work/development").exists());
    tampered.store(false, std::sync::atomic::Ordering::SeqCst);
    ota.install().unwrap();
    assert!(ota.install().is_err(), "one installation at a time");
    let done = wait().await;
    assert_eq!(done["error"], "");
    assert_eq!(done["last_result"]["ok"], true);
    assert_eq!(done["last_result"]["version"], "9.9.9");
    assert!(
        dir.path()
            .join("work/development/install_simpleadmin_rust.sh")
            .is_file()
    );
    assert!(
        std::fs::read_to_string(dir.path().join("work").join(RUNNER))
            .unwrap()
            .contains("install_simpleadmin_rust.sh")
    );
    // Settings survive a restart.
    let reloaded = Ota::new(
        dir.path().join("ota-settings.json"),
        dir.path().join("work"),
        true,
        Arc::new(Store::new(true)),
    );
    assert_eq!(reloaded.settings().public_key, trusted["public_key"]);
}
