// Engine scenarios against a scripted adb, mirroring the device states the installer must handle.
// The fake adb is a shell script, so these run on Unix development hosts.
#![cfg(unix)]

use super::*;
use std::{fs, net::TcpListener, os::unix::fs::PermissionsExt};

const FAKE_ADB: &str = r#"#!/bin/bash
dir="__DIR__"; mode="__CASE__"
call="$*"
printf '%s\n' "$call" >> "$dir/calls"
case "$call" in
  *"cat > /tmp/development/install-credentials.json"*)
    json="$(cat)"
    [[ "$json" == *web_username* && "$json" == *root_password* ]] || exit 1
    printf '%s' "$json" > "$dir/credential-input"
    [ "$mode" = credential-transfer-failed ] && exit 1
    exit 0;;
  *"sha256sum /tmp/development/install-credentials.json"*)
    echo "$(sha256sum < "$dir/credential-input" | cut -d' ' -f1)  /tmp/development/install-credentials.json"; exit 0;;
  *SIMPLEADMIN_PREFLIGHT=1*)
    [ "$mode" = probe-failed ] && exit 1
    echo SIMPLEADMIN_PREFLIGHT=1
    echo SA_SYSTEM=Linux
    if [ "$mode" = emulator ]; then echo SA_ARCH=x86_64; else echo SA_ARCH=armv7l; fi
    case "$mode" in phone*|emulator) echo SA_MODULE=0;; *) echo SA_MODULE=1;; esac
    if [ "$mode" = no-root ]; then echo SA_UID=2000; else echo SA_UID=0; fi
    if [ "$mode" = no-bash ]; then echo SA_BASH=0; else echo SA_BASH=1; fi
    case "$mode" in readonly-tmp*|missing-tmp) echo SA_TMP=0;; *) echo SA_TMP=1;; esac
    echo SIMPLEADMIN_PREFLIGHT_DONE=1; exit 0;;
  devices|"devices -l")
    echo "List of devices attached"
    [ "$mode" = none ] && exit 0
    serial=FAKE1; [ "$mode" = forwarded ] && serial=127.0.0.1:21503
    state=device; [ "$mode" = unauthorized ] && state=unauthorized
    printf '%s\t%s\n' "$serial" "$state"
    [ "$mode" = multiple ] && printf 'FAKE2\tdevice\n'
    exit 0;;
  *"push "*)
    if [ "$mode" = push-failed ]; then echo "simulated push failure" >&2; exit 1; fi; exit 0;;
  *"bash /tmp/development/install_simpleadmin_rust.sh"*)
    [[ "$call" == *SIMPLEADMIN_HTTP_PORT=8080* ]] && printf 8080 > "$dir/device-http-port"
    if [ "$mode" = install-failed ]; then echo "simulated installation failure" >&2; exit 1; fi
    echo "simulated installation"; exit 0;;
  *"cat /tmp/simpleadmin-install-result.env"*)
    if [ "$mode" = missing-result ]; then echo REBOOT_REQUIRED=0; else echo INSTALL_STATUS=OK; echo REBOOT_REQUIRED=0; fi; exit 0;;
  *"if [ -e /usrdata/simpleadmin/http_port ]"*)
    [ "$mode" = port-read-failed ] && exit 1
    if [ "$mode" = invalid-saved-port ]; then echo bad; else cat "$dir/device-http-port"; echo; fi; exit 0;;
  *"forward tcp:0 tcp:"*)
    [[ "$call" == *"tcp:$(cat "$dir/device-http-port")" ]] || exit 1
    cat "$dir/port"; echo; exit 0;;
esac
exit 0
"#;

#[derive(Default)]
struct Recorder {
    lines: Vec<String>,
    stages: Vec<Stage>,
}
impl Sink for Recorder {
    fn line(&mut self, text: &str) {
        self.lines.push(text.to_owned())
    }
    fn stage(&mut self, stage: Stage) {
        self.stages.push(stage)
    }
}

struct Scenario {
    outcome: Outcome,
    calls: String,
    log: String,
    stages: Vec<Stage>,
    dir: tempfile::TempDir,
}

/// Serves `/`, `/login.html` and `/js/locales.js` like SimpleAdmin, or a foreign app.
fn web_fixture(wrong_app: bool) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut raw = Vec::new();
            let mut chunk = [0u8; 512];
            while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => raw.extend_from_slice(&chunk[..n]),
                }
            }
            let request = String::from_utf8_lossy(&raw).into_owned();
            let body = if wrong_app {
                "factory web"
            } else if request.contains("locales.js") {
                "root.Lang"
            } else if request.contains("login.html") {
                "loginLanguage"
            } else {
                "SimpleAdminSpaMode"
            };
            let status = if request.starts_with("GET / HTTP/") && !wrong_app {
                "303 See Other\r\nLocation: /login.html"
            } else {
                "200 OK"
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}

fn scenario(case: &str, operation: Operation, device_port: &str) -> Scenario {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("development")).unwrap();
    fs::write(root.join("development/SHA256SUMS"), "fixture").unwrap();
    fs::write(
        root.join("development/diagnose_simpleadmin.sh"),
        "echo diag",
    )
    .unwrap();
    fs::write(
        root.join("port"),
        web_fixture(case == "wrong-app").to_string(),
    )
    .unwrap();
    fs::write(root.join("device-http-port"), device_port).unwrap();
    fs::write(root.join("calls"), "").unwrap();
    let adb_path = root.join("adb");
    fs::write(
        &adb_path,
        FAKE_ADB
            .replace("__DIR__", &root.to_string_lossy())
            .replace("__CASE__", case),
    )
    .unwrap();
    fs::set_permissions(&adb_path, fs::Permissions::from_mode(0o755)).unwrap();
    let serial = if case == "forwarded" {
        "127.0.0.1:21503"
    } else {
        "FAKE1"
    };
    let mut recorder = Recorder::default();
    let outcome = run(
        &operation,
        &Adb::new(&adb_path),
        &root.join("development"),
        serial,
        &root.join("reports"),
        &mut recorder,
    );
    Scenario {
        outcome,
        calls: fs::read_to_string(root.join("calls")).unwrap(),
        log: recorder.lines.join("\n"),
        stages: recorder.stages,
        dir,
    }
}
fn install() -> Operation {
    Operation::Install {
        http_port: None,
        credentials: None,
    }
}

#[test]
fn successful_install_keeps_tunnel_and_cleans_staging() {
    let s = scenario("success", install(), "80");
    assert!(s.outcome.ok, "{}", s.log);
    let url = s.outcome.url.as_deref().unwrap();
    assert!(url.starts_with("http://127.0.0.1:"));
    assert_eq!(s.outcome.http_port, Some(80));
    assert_eq!(
        s.stages,
        [Stage::Device, Stage::Upload, Stage::Install, Stage::Verify]
    );
    assert!(
        !s.calls.contains("SIMPLEADMIN_HTTP_PORT="),
        "upgrade keeps the port"
    );
    assert!(!s.calls.contains("forward --remove"));
    assert!(
        s.calls
            .trim_end()
            .ends_with("shell rm -rf /tmp/development")
    );
    let report = fs::read_to_string(s.outcome.report.unwrap()).unwrap();
    assert!(report.contains("simulated installation"));
    assert!(report.contains("PC_HTTP_CHECK=OK"));
}

#[test]
fn device_selection_failures_never_touch_a_device() {
    for case in ["none", "unauthorized"] {
        let s = scenario(case, install(), "80");
        assert!(!s.outcome.ok, "{case}");
        assert!(
            !s.calls.contains("shell") && !s.calls.contains("push"),
            "{case}: {}",
            s.calls
        );
    }
    // Several devices are fine when the chosen one is ready.
    let s = scenario("multiple", install(), "80");
    assert!(s.outcome.ok, "{}", s.log);
    assert!(
        s.calls
            .lines()
            .filter(|l| l.contains("push"))
            .all(|l| l.starts_with("-s FAKE1 "))
    );
}

#[test]
fn preflight_failures_stop_before_writing() {
    for case in [
        "phone",
        "emulator",
        "no-root",
        "no-bash",
        "readonly-tmp",
        "missing-tmp",
        "probe-failed",
    ] {
        let s = scenario(case, install(), "80");
        assert!(!s.outcome.ok, "{case}");
        assert!(
            !s.calls.contains("push ")
                && !s.calls.contains("rm -")
                && !s.calls.contains("shell bash "),
            "{case} wrote to the device:\n{}",
            s.calls
        );
        assert!(s.outcome.error.is_some());
    }
    // Opening the web page only needs a module, not root or a writable /tmp.
    let s = scenario("no-root", Operation::OpenWeb, "80");
    assert!(s.outcome.ok, "{}", s.log);
    let s = scenario("phone", Operation::Diagnose, "80");
    assert!(!s.outcome.ok);
}

#[test]
fn failed_installs_are_reported_and_diagnosed() {
    for case in [
        "push-failed",
        "install-failed",
        "missing-result",
        "wrong-app",
    ] {
        let s = scenario(case, install(), "80");
        assert!(!s.outcome.ok, "{case}");
        assert!(
            !s.log.contains("安装和网页检查通过"),
            "{case}: false success"
        );
        assert!(
            s.calls.contains("simpleadmin-diagnose-"),
            "{case}: no diagnostics"
        );
        assert!(s.outcome.report.is_some());
    }
    let s = scenario("wrong-app", install(), "80");
    assert!(
        s.calls.contains("forward --remove"),
        "failed tunnel left open"
    );
}

#[test]
fn http_port_choices() {
    let custom = Operation::Install {
        http_port: Some(8080),
        credentials: None,
    };
    let s = scenario("custom-port", custom, "80");
    assert!(s.outcome.ok, "{}", s.log);
    assert!(s.calls.contains("SIMPLEADMIN_HTTP_PORT=8080 bash"));
    assert!(s.calls.contains("forward tcp:0 tcp:8080"));
    assert_eq!(s.outcome.http_port, Some(8080));
    let s = scenario("preserve-port", install(), "8080");
    assert!(s.outcome.ok);
    assert!(s.calls.contains("forward tcp:0 tcp:8080"));
    assert!(!s.calls.contains("SIMPLEADMIN_HTTP_PORT="));
    for case in ["port-read-failed", "invalid-saved-port"] {
        assert!(!scenario(case, install(), "80").outcome.ok, "{case}");
    }
}

#[test]
fn diagnose_and_open_web_do_not_change_the_device() {
    let s = scenario("diagnose", Operation::Diagnose, "8080");
    assert!(s.outcome.ok, "{}", s.log);
    assert!(s.outcome.url.is_none(), "diagnostics close their tunnel");
    assert!(s.calls.contains("forward --remove"));
    for forbidden in [
        "install_simpleadmin",
        "remount",
        "reboot",
        "AT+",
        "sms",
        "passwd",
    ] {
        assert!(!s.calls.contains(forbidden), "diagnose ran {forbidden}");
    }
    let s = scenario("web", Operation::OpenWeb, "80");
    assert!(s.outcome.ok);
    assert!(s.outcome.url.is_some());
    for forbidden in [
        "push",
        "install_simpleadmin",
        "remount",
        "reboot",
        "AT+",
        "passwd",
    ] {
        assert!(!s.calls.contains(forbidden), "open web ran {forbidden}");
    }
    let s = scenario("forwarded", Operation::OpenWeb, "80");
    assert!(s.outcome.ok, "{}", s.log);
}

#[test]
fn credentials_travel_over_stdin_only() {
    let json = crate::credentials::build(
        Some(("owner", "web-secret:\"$value")),
        Some("root-secret:$value"),
    )
    .unwrap();
    let operation = Operation::Install {
        http_port: None,
        credentials: json.clone(),
    };
    let s = scenario("credentials", operation.clone(), "80");
    assert!(s.outcome.ok, "{}", s.log);
    let received = fs::read_to_string(s.dir.path().join("credential-input")).unwrap();
    assert_eq!(Some(received), json);
    let report = fs::read_to_string(s.outcome.report.unwrap()).unwrap();
    for secret in ["web-secret", "root-secret"] {
        assert!(!s.calls.contains(secret) && !report.contains(secret) && !s.log.contains(secret));
    }
    let s = scenario("credential-transfer-failed", operation, "80");
    assert!(!s.outcome.ok);
    assert!(
        !s.calls
            .contains("bash /tmp/development/install_simpleadmin_rust.sh")
    );
}
