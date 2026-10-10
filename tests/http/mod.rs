use super::*;
use axum::http::Request;
use clap::Parser;
use tower::ServiceExt;

fn application() -> (Arc<App>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "test").unwrap();
    let mut cfg = Config::parse_from(["test", "--mock"]);
    cfg.static_dir = dir.path().into();
    cfg.auth_file = dir.path().join("auth");
    cfg.ttl_file = dir.path().join("ttl");
    (App::new(cfg).unwrap(), dir)
}
#[tokio::test]
async fn sms_send_segments_acknowledgements_and_storage_validation() {
    let (app, _dir) = application();
    let token = app.auth.create();
    let form = serde_urlencoded::to_string([
        ("action", "send"),
        ("number", "10086"),
        ("message", &"中".repeat(70)),
    ])
    .unwrap();
    let response = call(&app, "/api/sms_data", &form, &token).await;
    assert_eq!(response.status(), 200);
    let data: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(data["segments"], 1);
    assert_eq!(data["number"], "10086");
    let (pdu, len) = sms::submit("10086", "hello", 1).unwrap().remove(0);
    assert!(!pdu.is_empty());
    app.at
        .overrides
        .lock()
        .unwrap()
        .insert(format!("AT+CMGF=0;+CMGS={len}"), "OK".into());
    let response = call(
        &app,
        "/api/sms_data",
        "action=send&number=10086&message=hello",
        &token,
    )
    .await;
    assert_eq!(response.status(), 400);
    app.at.trace.lock().unwrap().clear();
    assert_eq!(
        call(
            &app,
            "/api/sms_data",
            "action=delete_indices&storage=SM&indices=1",
            &token
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        *app.at.trace.lock().unwrap(),
        ["AT+CPMS=\"SM\"", "AT+CMGD=1"]
    );
    app.at.trace.lock().unwrap().clear();
    assert_eq!(
        call(
            &app,
            "/api/sms_data",
            "action=delete_indices&storage=EVIL&indices=1",
            &token
        )
        .await
        .status(),
        400
    );
    assert!(app.at.trace.lock().unwrap().is_empty());
}
#[tokio::test]
async fn disabled_sms_rejects_send_and_delete_without_at_commands() {
    let (app, _dir) = application();
    let token = app.auth.create();
    assert_eq!(
        call(
            &app,
            "/api/sms/settings",
            r#"{"enabled":false,"delete_after_day":false}"#,
            &token
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        call(
            &app,
            "/api/sms_data",
            "action=send&number=%2B1234&message=test",
            &token
        )
        .await
        .status(),
        400
    );
    assert_eq!(
        call(&app, "/api/sms_data", "action=delete_all", &token)
            .await
            .status(),
        400
    );
    assert!(app.at.trace.lock().unwrap().is_empty());
    let response = call(&app, "/api/sms_data", "action=list", &token).await;
    assert_eq!(response.status(), 200);
    let data: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(data["disabled"], true);
}

#[tokio::test]
async fn cell_lock_mutations_require_login_post_and_valid_parameters() {
    let (app, _dir) = application();
    let token = app.auth.create();
    let params = "action=lock_nr_manual&pci=0&earfcn=633984&scs=30&band=78&persistence=persistent&auto_unlock=1";
    assert_eq!(
        call(&app, "/api/network_data", params, "").await.status(),
        401
    );
    let response = app
        .router()
        .oneshot(
            Request::builder()
                .uri(format!("/api/network_data?{params}"))
                .header("cookie", format!("{}={token}", auth::COOKIE))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 405);
    assert_eq!(
        call(
            &app,
            "/api/network_data",
            &params.replace("scs=30", "scs=17"),
            &token
        )
        .await
        .status(),
        400
    );
    assert!(app.at.trace.lock().unwrap().is_empty());
    assert_eq!(
        call(&app, "/api/network_data", params, &token)
            .await
            .status(),
        200
    );
    assert_eq!(app.cell_lock.snapshot()["radios"][1]["persistent"], true);
}
async fn call(app: &Arc<App>, uri: &str, body: &str, token: &str) -> Response {
    app.router()
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("POST")
                .header("host", "localhost")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("cookie", format!("{}={token}", auth::COOKIE))
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn auth_gate_and_password_revocation() {
    let (app, _dir) = application();
    assert_eq!(
        call(&app, "/api/dashboard_data", "", "").await.status(),
        401
    );
    assert_eq!(
        call(&app, "/api/login", "username=admin&password=wrong", "")
            .await
            .status(),
        401
    );
    let response = call(&app, "/api/login", "username=admin&password=admin", "").await;
    assert_eq!(response.status(), 200);
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("HttpOnly")
    );
    let token = app.auth.create();
    let revoked = app.auth.watch(&token).unwrap();
    assert_eq!(
        call(
            &app,
            "/api/set_password",
            "current_password=bad&new_password=next&confirm_password=next",
            &token
        )
        .await
        .status(),
        403
    );
    assert_eq!(
        call(
            &app,
            "/api/set_password",
            "current_password=admin&new_password=next&confirm_password=next",
            &token
        )
        .await
        .status(),
        200
    );
    assert!(*revoked.borrow());
    assert_eq!(
        call(&app, "/api/dashboard_data", "", &token).await.status(),
        401
    );
    assert_eq!(
        call(&app, "/api/login", "username=admin&password=next", "")
            .await
            .status(),
        200
    );
}
#[tokio::test]
async fn cross_origin_mutations_and_public_path_bypass_fail() {
    let (app, _dir) = application();
    let token = app.auth.create();
    let request = Request::builder()
        .uri("/api/set_password")
        .method("POST")
        .header("host", "localhost")
        .header("origin", "http://attacker.invalid")
        .header("cookie", format!("{}={token}", auth::COOKIE))
        .body(Body::from("current_password=admin&new_password=x"))
        .unwrap();
    assert_eq!(app.router().oneshot(request).await.unwrap().status(), 403);
    for uri in [
        "/js/locales.js/../secret",
        "/css/app.css/../../index.html",
        "/api/module_model/../dashboard_data",
        "/cgi-bin/dashboard_data",
    ] {
        let response = call(&app, uri, "", "").await;
        assert!([401, 303].contains(&response.status().as_u16()));
    }
    // The login page's stylesheet loads before signing in.
    let response = call(&app, "/css/app.css", "", "").await;
    assert!(![401, 303].contains(&response.status().as_u16()));
}
#[tokio::test]
async fn root_change_requires_current_password_and_revokes_all() {
    let (app, _dir) = application();
    let token = app.auth.create();
    assert_eq!(
        call(
            &app,
            "/api/set_root_password",
            "current_password=bad&new_password=next&confirm_password=next",
            &token
        )
        .await
        .status(),
        403
    );
    assert_eq!(
        call(
            &app,
            "/api/set_root_password",
            "current_password=admin&new_password=next&confirm_password=next",
            &token
        )
        .await
        .status(),
        200
    );
    assert!(!app.auth.valid(&token, false));
    assert!(app.auth.root_matches("next", true));
    assert!(auth::password_matches(
        "admin",
        &auth::read(&app.auth.path).unwrap().1
    ));
}
#[tokio::test]
async fn form_actions_and_mock_radio_override_work() {
    let (app, _dir) = application();
    let token = app.auth.create();
    for (uri, body) in [
        ("/api/network_data", "action=lock_bands&mode=SA&values=78"),
        (
            "/api/settings_data",
            "action=dns_proxy&family=4&enabled=true",
        ),
        (
            "/api/sms_data",
            "action=send&number=%2B8613800138000&message=test",
        ),
    ] {
        let response = call(&app, uri, body, &token).await;
        assert_eq!(response.status(), 200);
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1048576).await.unwrap())
                .unwrap();
        assert_eq!(value["ok"], true);
    }
    let payload = "+QENG: \"servingcell\",\"NOCONN\",\"NR5G-SA\",\"TDD\",460,01,1234,0,1,633984,78,12,-90,-10,0";
    let body =
        serde_urlencoded::to_string([("action", "set"), ("kind", "qeng"), ("payload", payload)])
            .unwrap();
    assert_eq!(
        call(&app, "/api/mock_at", &body, &token).await.status(),
        200
    );
    let response = call(&app, "/api/dashboard_data", "force=true", &token).await;
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1048576).await.unwrap()).unwrap();
    assert_eq!(value["rsrpLTE"], "-");
    assert_eq!(value["sinrNR"], "0");
}

#[tokio::test]
async fn web_username_can_change_without_resetting_password() {
    let (app, _dir) = application();
    let token = app.auth.create();
    let response = call(
        &app,
        "/api/set_password",
        "current_password=wrong&new_username=owner",
        &token,
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(auth::read(&app.auth.path).unwrap().0, "admin");
    let response = call(
        &app,
        "/api/set_password",
        "current_password=admin&new_username=bad%3Aname",
        &token,
    )
    .await;
    assert_eq!(response.status(), 400);
    let response = call(
        &app,
        "/api/set_password",
        "current_password=admin&new_username=owner",
        &token,
    )
    .await;
    assert_eq!(response.status(), 200);
    let (user, stored) = auth::read(&app.auth.path).unwrap();
    assert_eq!(user, "owner");
    assert!(auth::password_matches("admin", &stored));
    assert!(!app.auth.valid(&token, false));
}

#[tokio::test]
async fn web_port_rebinds_and_rejects_conflicts_before_saving() {
    let (app, dir) = application();
    let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old = first.local_addr().unwrap();
    let instance = app.clone();
    let server = tokio::spawn(async move { crate::webui::serve(instance, first).await.unwrap() });
    tokio::task::yield_now().await;
    let token = app.auth.create();
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = occupied.local_addr().unwrap().port();
    let data = format!("current_password=admin&http_port={port}");
    assert_eq!(
        call(&app, "/api/set_webui_port", &data, &token)
            .await
            .status(),
        409
    );
    assert!(!dir.path().join("http_port").exists());
    assert_eq!(
        call(
            &app,
            "/api/set_webui_port",
            "current_password=wrong&http_port=12345",
            &token
        )
        .await
        .status(),
        403
    );
    for value in ["0", "65536", "080", "-1", "80%3Breboot"] {
        let data = format!("current_password=admin&http_port={value}");
        assert_eq!(
            call(&app, "/api/set_webui_port", &data, &token)
                .await
                .status(),
            400
        );
    }
    drop(occupied);
    assert_eq!(
        call(&app, "/api/set_webui_port", &data, &token)
            .await
            .status(),
        200
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("http_port")).unwrap(),
        format!("{port}\n")
    );
    tokio::task::yield_now().await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let response = client
        .get(format!("http://127.0.0.1:{port}/api/webui_settings"))
        .header("Cookie", format!("{}={token}", auth::COOKIE))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let json: Value = response.json().await.unwrap();
    assert_eq!(json["http_port"], port);
    assert_eq!(json["username"], "admin");
    assert!(json.get("password").is_none());
    assert!(tokio::net::TcpStream::connect(old).await.is_err());
    server.abort();
}

fn get_request(uri: &str, token: &str) -> axum::http::request::Builder {
    Request::builder()
        .uri(uri)
        .header("host", "localhost")
        .header("cookie", format!("{}={token}", auth::COOKIE))
}

#[tokio::test]
async fn state_changes_reject_get_and_cross_site_requests() {
    let (app, _dir) = application();
    let token = app.auth.create();
    // A link or cross-site navigation must not run AT commands, change TTL or reboot.
    for uri in [
        "/api/settings_data?action=manual_at&command=AT%2BCFUN%3D0",
        "/api/settings_data?action=reboot",
        "/cgi-bin/get_atcommand?atcmd=ATI",
        "/api/set_ttl?ttlvalue=64",
        "/api/sms_data?action=delete_all",
        "/api/network_data?action=scan&mode=Full%20Scan",
        "/api/set_language?language=en",
    ] {
        let response = app
            .router()
            .oneshot(get_request(uri, &token).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 405, "{uri}");
    }
    assert!(app.at.trace.lock().unwrap().is_empty());
    // Reads keep working over GET for scripts and the existing pages.
    for uri in [
        "/api/get_ttl_status",
        "/api/dashboard_data",
        "/api/settings_data",
        "/api/sms_data?action=list_meta",
    ] {
        let response = app
            .router()
            .oneshot(get_request(uri, &token).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{uri}");
    }
    let cross_site = app
        .router()
        .oneshot(
            get_request("/api/settings_data", &token)
                .method("POST")
                .header("sec-fetch-site", "cross-site")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("action=reboot"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cross_site.status(), 403);
    let response = call(&app, "/api/login", "username=admin&password=admin", "").await;
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("SameSite=Strict")
    );
}

#[tokio::test]
async fn repeated_login_failures_are_throttled_per_client() {
    let (app, _dir) = application();
    let attacker = Some("192.0.2.10".parse().unwrap());
    let mut statuses = Vec::new();
    for _ in 0..7 {
        let p = Params::parse("", "username=admin&password=wrong").unwrap();
        statuses.push(login(&app, &p, attacker).await.status().as_u16());
    }
    assert_eq!(statuses[..6], [401; 6]);
    assert_eq!(statuses[6], 429);
    // Even the right password waits out the lockout, while other clients are unaffected.
    let good = Params::parse("", "username=admin&password=admin").unwrap();
    let locked = login(&app, &good, attacker).await;
    assert_eq!(locked.status(), 429);
    assert!(locked.headers().contains_key("retry-after"));
    let other = Some("192.0.2.11".parse().unwrap());
    assert_eq!(login(&app, &good, other).await.status(), 200);
}

#[tokio::test]
async fn telemetry_schedule_is_validated_persisted_and_reported() {
    let (app, dir) = application();
    let token = app.auth.create();
    for body in [
        r#"{"ping_enabled":true,"ping_interval":0,"sample_enabled":true,"sample_interval":5}"#,
        r#"{"ping_enabled":true,"ping_interval":1,"sample_enabled":true,"sample_interval":1}"#,
        r#"{"ping_enabled":true,"unknown":1}"#,
    ] {
        assert_eq!(
            call(&app, "/api/telemetry/schedule", body, &token)
                .await
                .status(),
            400,
            "{body}"
        );
    }
    let body =
        r#"{"ping_enabled":false,"ping_interval":30,"sample_enabled":false,"sample_interval":60}"#;
    let response = call(&app, "/api/telemetry/schedule", body, &token).await;
    assert_eq!(response.status(), 200);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1048576).await.unwrap()).unwrap();
    assert_eq!(value["schedule"]["ping_enabled"], false);
    assert_eq!(value["schedule"]["sample_interval"], 60);
    assert_eq!(app.monitor.connected(), None);
    let saved: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("monitor.json")).unwrap()).unwrap();
    assert_eq!(saved["ping_interval"], 30);
    assert_eq!(saved["target"], "www.baidu.com");
    // With the latency probe off, connectivity falls back to the modem's data session.
    let response = call(&app, "/api/get_ping", "", &token).await;
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn websocket_requests_do_not_wait_for_slow_ones() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message as WsMessage, client::IntoClientRequest};
    let (app, _dir) = application();
    app.at
        .overrides
        .lock()
        .unwrap()
        .insert("scan_delay_ms".into(), "1500".into());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let instance = app.clone();
    let server = tokio::spawn(async move { crate::webui::serve(instance, listener).await });
    let token = app.auth.create();
    let mut request = format!("ws://{address}/api/ws")
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("origin", format!("http://{address}").parse().unwrap());
    headers.insert(
        "cookie",
        format!("{}={token}", auth::COOKIE).parse().unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    for (id, method, path, body) in [
        (
            "scan",
            "POST",
            "/api/network_data",
            "action=scan&mode=Full+Scan",
        ),
        ("uptime", "GET", "/api/get_uptime", ""),
    ] {
        let message = json!({"id":id,"method":method,"path":path,"body":body}).to_string();
        socket.send(WsMessage::Text(message.into())).await.unwrap();
    }
    let started = std::time::Instant::now();
    let mut order = Vec::new();
    while order.len() < 2 {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let WsMessage::Text(text) = message {
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["status"], 200, "{value}");
            order.push((value["id"].as_str().unwrap().to_owned(), started.elapsed()));
        }
    }
    assert_eq!(order[0].0, "uptime");
    assert!(order[0].1 < Duration::from_millis(1000), "{order:?}");
    assert_eq!(order[1].0, "scan");
    server.abort();
}
