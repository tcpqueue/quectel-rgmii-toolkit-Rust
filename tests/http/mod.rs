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
        "/api/module_model/../dashboard_data",
        "/cgi-bin/dashboard_data",
    ] {
        let response = call(&app, uri, "", "").await;
        assert!([401, 303].contains(&response.status().as_u16()));
    }
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
    assert_eq!(auth::read(&app.auth.path).unwrap().1, "admin");
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
