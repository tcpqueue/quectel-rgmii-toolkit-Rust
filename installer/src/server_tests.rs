use super::*;
use axum::body::Body;
use http_body_util::BodyExt;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const HOST: &str = "127.0.0.1:4321";

fn app() -> (Arc<App>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(
        TOKEN.into(),
        4321,
        Adb::new(dir.path().join("missing-adb")),
        dir.path().join("development"),
        dir.path().to_owned(),
    );
    (app, dir)
}

async fn send(
    app: &Arc<App>,
    request: axum::http::Request<Body>,
) -> (StatusCode, HeaderMap, Value) {
    let response = app.router().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn get(uri: &str) -> axum::http::request::Builder {
    axum::http::Request::get(uri)
        .header("host", HOST)
        .header("cookie", format!("{COOKIE}={TOKEN}"))
}
fn post(uri: &str) -> axum::http::request::Builder {
    axum::http::Request::post(uri)
        .header("host", HOST)
        .header("cookie", format!("{COOKIE}={TOKEN}"))
        .header("origin", format!("http://{HOST}"))
        .header("content-type", "application/json")
}

#[tokio::test]
async fn launch_token_becomes_strict_cookie_and_other_hosts_are_refused() {
    let (app, _dir) = app();
    let launch = axum::http::Request::get(format!("/?k={TOKEN}"))
        .header("host", HOST)
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = send(&app, launch).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let cookie = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    let wrong = axum::http::Request::get("/?k=wrong")
        .header("host", HOST)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, wrong).await.0, StatusCode::FORBIDDEN);
    let anonymous = axum::http::Request::get("/api/state")
        .header("host", HOST)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, anonymous).await.0, StatusCode::FORBIDDEN);
    // DNS rebinding: a page on attacker.example resolving to 127.0.0.1 sends its own Host.
    let rebound = axum::http::Request::get("/api/state")
        .header("host", "attacker.example:4321")
        .header("cookie", format!("{COOKIE}={TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, rebound).await.0, StatusCode::MISDIRECTED_REQUEST);
    let duplicated = get("/api/state")
        .header("host", "attacker.example:4321")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(&app, duplicated).await.0,
        StatusCode::MISDIRECTED_REQUEST
    );
    let (status, headers, state) = send(&app, get("/api/state").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("default-src 'none'")
    );
    assert_eq!(state["busy"], Value::Null);
    assert_eq!(state["log"]["next"], 0);
}

#[tokio::test]
async fn writes_require_same_origin() {
    let (app, _dir) = app();
    let no_origin = axum::http::Request::post("/api/quit")
        .header("host", HOST)
        .header("cookie", format!("{COOKIE}={TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, no_origin).await.0, StatusCode::FORBIDDEN);
    let foreign = axum::http::Request::post("/api/quit")
        .header("host", HOST)
        .header("cookie", format!("{COOKIE}={TOKEN}"))
        .header("origin", "http://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, foreign).await.0, StatusCode::FORBIDDEN);
    let duplicated = post("/api/quit")
        .header("origin", "http://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, duplicated).await.0, StatusCode::FORBIDDEN);
    let cross_site = post("/api/bye")
        .header("sec-fetch-site", "cross-site")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, cross_site).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        send(&app, post("/api/bye").body(Body::empty()).unwrap())
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn invalid_install_input_is_rejected_before_any_device_access() {
    let (app, _dir) = app();
    app.lock().devices = vec![Device {
        serial: "FAKE1".into(),
        state: "device".into(),
        model: String::new(),
    }];
    for (body, message) in [
        (
            json!({"operation":"install","serial":"FAKE1","http_port":"65536"}),
            "1–65535",
        ),
        (
            json!({"operation":"install","serial":"FAKE1","http_port":"080"}),
            "1–65535",
        ),
        (
            json!({"operation":"install","serial":"FAKE1","web_username":"bad:name","web_password":"secret"}),
            "Web 账号",
        ),
        (
            json!({"operation":"install","serial":"FAKE1","web_username":"owner"}),
            "同时填写",
        ),
        (
            json!({"operation":"install","serial":"FAKE1","root_password":"line\nbreak"}),
            "密码需为",
        ),
        (
            json!({"operation":"install","serial":"OTHER"}),
            "已连接并授权",
        ),
        (
            json!({"operation":"format","serial":"FAKE1"}),
            "unknown operation",
        ),
    ] {
        let (status, _, value) = send(
            &app,
            post("/api/run").body(Body::from(body.to_string())).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            value["error"].as_str().unwrap().contains(message),
            "{value}"
        );
    }
    assert!(app.lock().busy.is_none());
    assert!(app.lock().operation.is_none());
}

#[tokio::test]
async fn busy_operations_block_quit_and_serial_writes_need_identification() {
    let (app, _dir) = app();
    app.lock().busy = Some("install");
    let (status, _, value) = send(&app, post("/api/quit").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(value["error"].as_str().unwrap().contains("安装"));
    app.lock().busy = None;
    for action in ["restart", "factory", "adb"] {
        let body = json!({"port":"COM3","action":action}).to_string();
        let (status, _, _) = send(
            &app,
            post("/api/at/preview").body(Body::from(body)).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{action}");
    }
    assert!(app.lock().pending.is_none());
    // A stale or forged confirmation id never runs anything.
    let body = json!({"id":"nope"}).to_string();
    assert_eq!(
        send(
            &app,
            post("/api/at/confirm").body(Body::from(body)).unwrap()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn identified_module_gets_a_confirmable_plan() {
    let (app, _dir) = app();
    app.lock().identity = Some((
        "COM3".into(),
        ModuleIdentity {
            manufacturer: "Quectel".into(),
            model: "RM520N-EU".into(),
            firmware: "x".into(),
            imei: "123456789012345".into(),
        },
    ));
    let body = json!({"port":"COM3","action":"custom","commands":"AT+QTEMP\nAT+CSQ"}).to_string();
    assert_eq!(
        send(
            &app,
            post("/api/at/preview").body(Body::from(body)).unwrap()
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let (_, _, state) = send(&app, get("/api/state").body(Body::empty()).unwrap()).await;
    assert_eq!(state["pending"]["commands"], json!(["AT+QTEMP", "AT+CSQ"]));
    assert_eq!(state["identity"]["imei"], "•••••••••••2345");
    let blocked = json!({"port":"COM3","action":"custom","commands":"AT+CMGS=\"1\""}).to_string();
    assert_eq!(
        send(
            &app,
            post("/api/at/preview").body(Body::from(blocked)).unwrap()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    send(&app, post("/api/at/dismiss").body(Body::empty()).unwrap()).await;
    assert!(app.lock().pending.is_none());
}

#[tokio::test]
async fn presets_and_logs_stay_on_this_computer() {
    let (app, dir) = app();
    let invalid = json!({"text":"AT;AT"}).to_string();
    assert_eq!(
        send(
            &app,
            post("/api/at/preset").body(Body::from(invalid)).unwrap()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let text = "AT+QTEMP\nAT+QGMR";
    let body = json!({ "text": text }).to_string();
    assert_eq!(
        send(&app, post("/api/at/preset").body(Body::from(body)).unwrap())
            .await
            .0,
        StatusCode::OK
    );
    let (_, _, loaded) = send(&app, get("/api/at/preset").body(Body::empty()).unwrap()).await;
    assert_eq!(loaded["text"], text);
    app.lock().at_log.push("识别结果：RM520N-EU");
    let (_, _, saved) = send(&app, post("/api/at/log").body(Body::empty()).unwrap()).await;
    let path = PathBuf::from(saved["path"].as_str().unwrap());
    assert!(path.starts_with(dir.path()));
    assert!(std::fs::read_to_string(path).unwrap().contains("RM520N-EU"));
}

#[test]
fn log_cursor_reports_resets() {
    let mut log = Log::default();
    for i in 0..LOG_LINES + 10 {
        log.push(&format!("line {i}"));
    }
    assert_eq!(log.since(log.next)["lines"].as_array().unwrap().len(), 0);
    assert_eq!(
        log.since(log.next - 2)["lines"].as_array().unwrap().len(),
        2
    );
    assert_eq!(log.since(0)["reset"], true);
    assert_eq!(log.since(log.next - 2)["reset"], false);
}
