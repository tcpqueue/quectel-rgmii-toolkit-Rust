use crate::{
    Config,
    actions::{self, Params},
    at::At,
    auth::{self, Auth},
    forwarding::{Forwarder, Settings as ForwardingSettings},
    parser,
    persistence::Store,
    sms,
    system::{self, Metrics},
    telemetry::Monitor,
};
use anyhow::{Result, bail};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{
        ConnectInfo, FromRequestParts, Request, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, request::Parts},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tower_http::services::ServeDir;
#[cfg(test)]
#[path = "../tests/http/mod.rs"]
mod tests;

/// Requests one WebSocket may run at the same time; extra requests are answered with 429.
const WS_CONCURRENCY: usize = 8;

pub struct App {
    pub config: Config,
    pub at: At,
    pub auth: Auth,
    pub store: Arc<Store>,
    pub monitor: Arc<Monitor>,
    pub forwarding: Arc<Forwarder>,
    metrics: Metrics,
    pub sockets: Arc<tokio::sync::Semaphore>,
    pub consoles: Arc<tokio::sync::Semaphore>,
    ttl_lock: tokio::sync::Mutex<()>,
    pub webui: crate::webui::ListenerState,
    pub cell_lock: Arc<crate::cell_lock::CellLock>,
}
pub fn json_response(status: u16, value: Value) -> Response {
    (StatusCode::from_u16(status).unwrap(), axum::Json(value)).into_response()
}
pub fn error(status: u16, message: impl ToString) -> Response {
    json_response(status, json!({"ok":false,"error":message.to_string()}))
}
/// 429 response for clients locked out by repeated password failures.
pub fn throttled(wait: Duration) -> Response {
    let seconds = wait.as_secs().max(1);
    let mut response = error(
        429,
        format!("too many failed attempts; retry in {seconds} seconds"),
    );
    response
        .headers_mut()
        .insert("retry-after", seconds.to_string().parse().unwrap());
    response
}
fn text_response(value: impl Into<String>) -> Response {
    value.into().into_response()
}
fn redirect(path: &str) -> Response {
    axum::response::Redirect::to(path).into_response()
}
pub fn cookie(headers: &HeaderMap) -> String {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .find_map(|c| c.trim().strip_prefix(&format!("{}=", auth::COOKIE)))
        .unwrap_or("")
        .to_owned()
}
fn set_cookie(response: &mut Response, token: &str, secure: bool) {
    // Strict keeps the session out of every cross-site navigation, not only subresource requests.
    let value = format!(
        "{}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Strict{}",
        auth::COOKIE,
        token,
        if token.is_empty() { 0 } else { 86400 },
        if secure { "; Secure" } else { "" }
    );
    response
        .headers_mut()
        .insert("set-cookie", value.parse().unwrap());
}
pub fn origin_allowed(headers: &HeaderMap, secure: bool) -> bool {
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let scheme = if secure { "https://" } else { "http://" };
    let Some(origin) = origin.strip_prefix(scheme) else {
        return false;
    };
    let normalize = |value: &str| {
        let lower = value.to_ascii_lowercase();
        lower
            .strip_suffix(if secure { ":443" } else { ":80" })
            .unwrap_or(&lower)
            .to_owned()
    };
    !host.is_empty() && normalize(origin) == normalize(host)
}
/// Browsers label every request with its initiator; API calls must come from this site's own pages.
fn fetch_site_allowed(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_none_or(|site| matches!(site, "same-origin" | "none"))
}
/// Client address recorded by the connection loop, used for login backoff.
pub struct Peer(pub Option<IpAddr>);
impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(peer(&parts.extensions)))
    }
}
fn peer(extensions: &axum::http::Extensions) -> Option<IpAddr> {
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
}
impl App {
    pub fn new(config: Config) -> Result<Arc<Self>> {
        if !config.static_dir.join("index.html").is_file() {
            bail!("index.html missing from static directory")
        }
        let store = Arc::new(Store::new(config.mock));
        let auth = Auth::new(config.auth_file.clone(), &store)?;
        let at = At::start(config.mock, config.devices())?;
        let monitor = Monitor::new(
            config.auth_file.with_file_name("monitor.json"),
            config.mock,
            store.clone(),
        );
        let forwarding = Arc::new(Forwarder::new(
            config.auth_file.with_file_name("forwarding.json"),
            store.clone(),
        ));
        let cell_lock = Arc::new(crate::cell_lock::CellLock::new(
            config.auth_file.with_file_name("cell-lock.json"),
            store.clone(),
        ));
        Ok(Arc::new(Self {
            config,
            at,
            auth,
            store,
            monitor,
            forwarding,
            metrics: Metrics::default(),
            sockets: Arc::new(tokio::sync::Semaphore::new(8)),
            consoles: Arc::new(tokio::sync::Semaphore::new(2)),
            ttl_lock: tokio::sync::Mutex::new(()),
            webui: crate::webui::ListenerState::default(),
            cell_lock,
        }))
    }
    pub fn start(self: &Arc<Self>) {
        self.at.start_refresh();
        self.monitor.start(self.at.clone());
        self.forwarding.start(self.at.clone());
        self.cell_lock.start(self.at.clone());
        let app = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tick.tick().await;
                app.auth.prune()
            }
        });
    }
    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/api/ws", get(websocket))
            .route("/api/console/ws", get(console_socket))
            .route(
                "/console",
                get(|| async { axum::response::Html(include_str!("console.html")) }),
            )
            .route(
                "/console/",
                get(|| async { axum::response::Html(include_str!("console.html")) }),
            )
            .fallback(dispatch)
            .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
            .layer(middleware::from_fn_with_state(self.clone(), gate))
            .with_state(self.clone())
    }
}
async fn gate(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let api = path.starts_with("/api/") || path.starts_with("/cgi-bin/");
    let public = matches!(
        path,
        "/login.html"
            | "/logout.html"
            | "/js/locales.js"
            | "/api/login"
            | "/api/logout"
            | "/api/module_model"
    );
    if api && !fetch_site_allowed(request.headers()) {
        return error(403, "cross-site request forbidden");
    }
    if !public && !app.auth.valid(&cookie(request.headers()), true) {
        return if api {
            error(401, "login required")
        } else {
            redirect("/login.html")
        };
    }
    if let Some(origin) = request.headers().get("origin")
        && !origin.is_empty()
        && !origin_allowed(request.headers(), !app.config.no_tls)
    {
        return error(403, "origin forbidden");
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("x-frame-options", "SAMEORIGIN".parse().unwrap());
    response
}
async fn dispatch(State(app): State<Arc<App>>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or("").to_owned();
    let method = request.method().as_str().to_owned();
    let token = cookie(request.headers());
    let peer = peer(request.extensions());
    if path == "/logout.html" || (path == "/api/logout" && method == "GET") {
        // Navigation logout only clears this browser; it cannot revoke a session cross-site.
        let mut response = redirect("/login.html");
        if fetch_site_allowed(request.headers()) {
            app.auth.revoke(&token);
        }
        set_cookie(&mut response, "", !app.config.no_tls);
        return response;
    }
    if path == "/api/logout" {
        if method != "POST" {
            return error(405, "method not allowed");
        }
        app.auth.revoke(&token);
        let mut response = json_response(200, json!({"ok":true,"redirect":"/login.html"}));
        set_cookie(&mut response, "", !app.config.no_tls);
        return response;
    }
    if path == "/login.html" && app.auth.valid(&token, false) {
        return redirect("/");
    }
    if !path.starts_with("/api/") && !path.starts_with("/cgi-bin/") {
        if method != "GET" && method != "HEAD" {
            return error(405, "method not allowed");
        }
        use tower::ServiceExt;
        return match ServeDir::new(&app.config.static_dir).oneshot(request).await {
            Ok(response) => response.map(Body::new),
            Err(_) => error(500, "static file error"),
        };
    }
    if request
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > 64 * 1024)
    {
        return error(413, "request too large");
    }
    let body = match tokio::time::timeout(
        Duration::from_secs(5),
        to_bytes(request.into_body(), 64 * 1024),
    )
    .await
    {
        Ok(Ok(b)) => b,
        Ok(Err(_)) => return error(413, "request too large"),
        Err(_) => return error(408, "request body timeout"),
    };
    let body = match std::str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return error(400, "invalid request encoding"),
    };
    let uri = if let Some(rest) = path.strip_prefix("/cgi-bin/") {
        format!("/api/{rest}")
    } else {
        path
    };
    api(&app, &method, &uri, &query, body, &token, peer).await
}
/// Methods each endpoint accepts. Only side-effect-free reads may use GET, so a link or a
/// cross-site navigation can never change the device even if a session cookie were sent.
fn method_allowed(path: &str, action: &str, method: &str) -> bool {
    let read = match path {
        // Snapshots that are only ever read.
        "/api/telemetry" | "/api/webui_settings" => return method == "GET",
        "/api/forwarding"
        | "/api/module_model"
        | "/api/dashboard_data"
        | "/api/get_ping"
        | "/api/get_uptime"
        | "/api/get_ttl_status"
        | "/api/get_language"
        | "/api/get_sms" => true,
        "/api/device_info_data" => matches!(action, "" | "get"),
        "/api/network_data" => {
            matches!(
                action,
                "" | "settings" | "cell_lock_status" | "model" | "bands"
            )
        }
        "/api/settings_data" => matches!(action, "" | "status"),
        "/api/sms_data" => matches!(action, "" | "list" | "list_meta" | "sim_status"),
        "/api/mock_at" => matches!(action, "" | "show" | "status" | "parse" | "help"),
        _ => false,
    };
    method == "POST" || (read && method == "GET")
}
/// Upper bound for request bodies that carry passwords or small JSON settings.
fn body_limit(path: &str) -> usize {
    match path {
        "/api/login" | "/api/set_password" | "/api/set_root_password" => 4096,
        "/api/sms/settings" | "/api/forwarding/test" => 128,
        "/api/telemetry/target" | "/api/telemetry/schedule" => 1024,
        "/api/forwarding/save" => 16384,
        _ => 64 * 1024,
    }
}
pub async fn api(
    app: &Arc<App>,
    method: &str,
    path: &str,
    query: &str,
    body: &str,
    token: &str,
    peer: Option<IpAddr>,
) -> Response {
    let p = match Params::parse(query, if body.starts_with('{') { "" } else { body }) {
        Ok(p) => p,
        Err(e) => return error(400, e),
    };
    if !method_allowed(path, p.get("action"), method) {
        return error(405, "method not allowed");
    }
    if body.len() > body_limit(path) {
        return error(413, "request too large");
    }
    if path == "/api/login" {
        return login(app, &p, peer).await;
    }
    if path != "/api/module_model" && !app.auth.valid(token, true) {
        return error(401, "login required");
    }
    let result = match path {
        "/api/set_password" => return change_password(app, &p, false, peer).await,
        "/api/set_root_password" => return change_password(app, &p, true, peer).await,
        "/api/webui_settings" => return crate::webui::snapshot(app),
        "/api/set_webui_port" => return crate::webui::change_port(app, &p, peer).await,
        "/api/sms/settings" => sms_settings(app, body).await,
        "/api/forwarding" => Ok(app.forwarding.snapshot()),
        "/api/forwarding/save" => forwarding_save(app, body).await,
        "/api/forwarding/test" => forwarding_test(app, body).await,
        "/api/telemetry" => telemetry(app, &p),
        "/api/telemetry/target" => telemetry_target(app, body).await,
        "/api/telemetry/schedule" => telemetry_schedule(app, body).await,
        "/api/module_model" => module_model(app, &p).await,
        "/api/dashboard_data" => dashboard(app, &p).await,
        "/api/device_info_data" => device_info(app, &p).await,
        "/api/network_data" => network(app, &p).await,
        "/api/settings_data" => settings(app, &p).await,
        "/api/sms_data" => sms_data(app, &p).await,
        "/api/get_atcache" | "/api/get_atcommand" | "/api/user_atcommand" => {
            return text_result(at_command(app, &p).await);
        }
        "/api/get_ping" => return text_result(ping(app).await),
        "/api/get_sms" => return text_result(raw_sms(app, &p).await),
        "/api/send_sms" => return text_result(legacy_send_sms(app, &p).await),
        "/api/get_uptime" => return text_response(system::uptime(app.config.mock).1),
        "/api/get_ttl_status" => {
            let ttl = system::ttl(&app.config.ttl_file);
            Ok(json!({"isEnabled":ttl>0,"ttl":ttl}))
        }
        "/api/set_ttl" => set_ttl(app, &p).await,
        "/api/get_language" => Ok(language(app)),
        "/api/set_language" => set_language(app, &p, body).await,
        "/api/mock_at" => {
            if !app.config.mock {
                return error(403, "mock mode only");
            }
            crate::mock::handle(&app.at, &p).await
        }
        _ => return error(404, "unsupported API endpoint"),
    };
    match result {
        Ok(value) => json_response(200, value),
        Err(e) => error(400, e),
    }
}
fn text_result(result: Result<String>) -> Response {
    match result {
        Ok(text) => text_response(text),
        Err(e) => error(400, e),
    }
}
async fn login(app: &Arc<App>, p: &Params, peer: Option<IpAddr>) -> Response {
    if let Some(wait) = app.auth.throttle.check(peer) {
        return throttled(wait);
    }
    let _guard = app.auth.mutation.lock().await;
    let (user, stored) = match auth::read(&app.auth.path) {
        Ok(c) => c,
        Err(_) => return error(500, "auth config error"),
    };
    let username = p.get("username").to_owned();
    let password = p.get("password").to_owned();
    // Check both fields every time so a wrong username takes as long as a wrong password.
    let valid = tokio::task::spawn_blocking(move || {
        auth::equal(&username, &user) & auth::password_matches(&password, &stored)
    })
    .await
    .unwrap_or(false);
    if !valid {
        app.auth.throttle.failed(peer);
        return error(401, "invalid credentials");
    }
    app.auth.throttle.succeeded(peer);
    let token = app.auth.create();
    let mut response = json_response(200, json!({"ok":true,"redirect":"/"}));
    set_cookie(&mut response, &token, !app.config.no_tls);
    response
}
async fn sms_settings(app: &Arc<App>, body: &str) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SmsSettings {
        enabled: bool,
        delete_after_day: bool,
    }
    let settings: SmsSettings = serde_json::from_str(body)?;
    app.forwarding
        .sms_settings(settings.enabled, settings.delete_after_day)
        .await
}
async fn forwarding_save(app: &Arc<App>, body: &str) -> Result<Value> {
    app.forwarding
        .save(serde_json::from_str::<ForwardingSettings>(body)?)
        .await
}
async fn forwarding_test(app: &Arc<App>, body: &str) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Test {
        platform: String,
    }
    let test: Test = serde_json::from_str(body)?;
    app.forwarding.test(&test.platform).await
}
fn telemetry(app: &Arc<App>, p: &Params) -> Result<Value> {
    let cursor = p.get("cursor");
    if cursor.len() > 512 {
        bail!("invalid telemetry cursor");
    }
    let cursor = serde_json::from_str::<crate::telemetry::Cursor>(cursor).ok();
    Ok(app.monitor.snapshot_since(cursor.as_ref()))
}
async fn telemetry_target(app: &Arc<App>, body: &str) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Target {
        target: String,
    }
    let config: Target = serde_json::from_str(body)?;
    app.monitor.set_target(&config.target).await?;
    Ok(app.monitor.snapshot())
}
async fn telemetry_schedule(app: &Arc<App>, body: &str) -> Result<Value> {
    let schedule: crate::telemetry::Schedule = serde_json::from_str(body)?;
    app.monitor.set_schedule(schedule).await?;
    Ok(app.monitor.snapshot())
}
async fn module_model(app: &Arc<App>, p: &Params) -> Result<Value> {
    let raw = app.at.page("model", p.flag("force", false)).await?;
    Ok(json!({"model":parser::model(&raw),"pending":raw.contains(crate::at_policy::PENDING)}))
}
async fn dashboard(app: &Arc<App>, p: &Params) -> Result<Value> {
    let (raw, stamp) = app
        .at
        .fetch_stamped(crate::at::DASHBOARD, p.flag("force", false))
        .await?;
    // Page refreshes feed the history too, so rates stay live with background sampling off.
    app.monitor.observe(&raw, stamp);
    let mut data = parser::dashboard(&raw);
    if p.flag("debug", false) {
        data["raw"] = json!(raw)
    }
    let connected = app
        .monitor
        .connected()
        .unwrap_or_else(|| crate::cell_lock::dialed(&raw));
    data["internetConnection"] = json!(if connected { "已连接" } else { "未连接" });
    data["uptimeParts"] = system::uptime(app.config.mock).0;
    let object = data.as_object_mut().unwrap();
    for extra in [
        app.metrics.read(app.config.mock),
        app.monitor.traffic_rates(),
    ] {
        if let Value::Object(map) = extra {
            object.extend(map)
        }
    }
    data["lastUpdate"] = json!(chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string());
    Ok(data)
}
async fn device_info(app: &Arc<App>, p: &Params) -> Result<Value> {
    let force = p.flag("force", false);
    match p.get("action") {
        "" | "get" => {
            let raw = app.at.page("device", force).await?;
            let mut data = parser::device(&raw);
            data["appVersion"] = json!(concat!("SimpleAdmin Rust v", env!("CARGO_PKG_VERSION")));
            let model = app.at.page("model", force).await?;
            let name = parser::model(&model);
            if name != "-" {
                data["modelName"] = json!(name)
            }
            data["pending"] = json!(
                raw.contains(crate::at_policy::PENDING)
                    || model.contains(crate::at_policy::PENDING)
            );
            Ok(data)
        }
        "set_imei" => run_action(app, &actions::imei(p)?).await,
        _ => bail!("unsupported action"),
    }
}
async fn network(app: &Arc<App>, p: &Params) -> Result<Value> {
    let force = p.flag("force", false);
    let action = p.get("action");
    match action {
        "" | "settings" => {
            let mut data = parser::network(&app.at.page("network", force).await?);
            data["cell_lock"] = app.cell_lock.snapshot();
            Ok(data)
        }
        "cell_lock_status" => Ok(app.cell_lock.snapshot()),
        "model" => module_model(app, p).await,
        "bands" => {
            let command = if p.get("mode").is_empty() {
                crate::at::commands("bands")[0].to_owned()
            } else {
                format!("AT+QNWPREFCFG=\"{}\"", actions::band_mode(p.get("mode"))?)
            };
            let raw = app
                .at
                .fetch_wait(&command, force, p.flag("wait", true))
                .await?;
            let mut value = parser::bands(&raw);
            if raw.contains(crate::at_policy::PENDING) {
                value["pending"] = json!(true)
            } else if raw.to_ascii_lowercase().contains("error") {
                value["error"] = json!(raw)
            }
            Ok(value)
        }
        "scan" => Ok(parser::scan(
            &app.at.run(actions::scan_mode(p.get("mode"))?).await?,
        )),
        action if crate::cell_lock::CellLock::handles(action) => {
            // Finish the lock and its persistence even if the client disconnects mid-request.
            let lock = app.cell_lock.clone();
            let at = app.at.clone();
            let params = Params(p.0.clone());
            tokio::spawn(async move { lock.apply(&at, &params).await }).await?
        }
        _ => run_action(app, &actions::network(p)?).await,
    }
}
async fn settings(app: &Arc<App>, p: &Params) -> Result<Value> {
    let action = p.get("action");
    if action.is_empty() || action == "status" {
        return Ok(parser::settings(
            &app.at.page("settings", p.flag("force", false)).await?,
        ));
    }
    let commands = actions::settings(p)?;
    if commands.len() == 1 {
        return run_action(app, &commands[0]).await;
    }
    // Multi-step changes end in a reboot; answer first so the page can start its countdown.
    let app = app.clone();
    tokio::spawn(async move {
        for command in commands {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if run_action(&app, &command)
                .await
                .ok()
                .is_none_or(|v| v["ok"] != true)
            {
                break;
            }
        }
    });
    Ok(json!({
        "ok": true,
        "response": "设备即将重启",
        "reboot": true,
        "rebooting": true,
        "rebootAfterSeconds": 1,
        "rebootCountdownSeconds": 40,
        "message": "设备即将重启，请等待前端倒计时。"
    }))
}
async fn sms_data(app: &Arc<App>, p: &Params) -> Result<Value> {
    let action = p.get("action");
    if !app.forwarding.sms_enabled() {
        if matches!(action, "" | "list" | "list_meta") {
            return Ok(json!({"messages":[],"serviceCenters":[],"disabled":true}));
        }
        bail!("SMS service disabled")
    }
    match action {
        "" | "list" | "list_meta" => {
            let mut data = sms::list(&app.at.page("sms", p.flag("force", false)).await?);
            if action == "list_meta" {
                for value in data["messages"].as_array_mut().unwrap() {
                    let message = value.as_object_mut().unwrap();
                    message.remove("text");
                    message.remove("textLines");
                }
            }
            Ok(data)
        }
        "delete_all" => {
            let _guard = app.forwarding.sms_mutation.lock().await;
            let raw = app.at.page("sms", true).await?;
            if !parser::ok(&raw) {
                bail!("SMS read failed")
            }
            let data = sms::list(&raw);
            let entries = data["messages"].as_array().unwrap();
            for storage in [sms::Storage::ME, sms::Storage::SM] {
                if entries.iter().any(|v| sms::Storage::of(v) == storage) {
                    app.at.delete_sms(storage, &[], true).await?;
                }
            }
            Ok(json!({"ok":true}))
        }
        "delete_indices" => {
            let _guard = app.forwarding.sms_mutation.lock().await;
            let values = p.list("indices", ',');
            if values.is_empty() || values.len() > 1024 {
                bail!("missing indices or too many messages")
            }
            let indices = values
                .iter()
                .map(|v| v.parse::<u16>())
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let storage = sms::Storage::parse(p.get("storage"))?;
            let raw = app.at.delete_sms(storage, &indices, false).await?;
            Ok(json!({"ok":parser::ok(&raw),"response":raw}))
        }
        "sim_status" => {
            let raw = app.at.run("AT+CPIN?").await?.to_ascii_uppercase();
            Ok(
                json!({"inserted":!raw.contains("SIM NOT INSERTED")&&!raw.contains("+CME ERROR: 10")}),
            )
        }
        "send" => send_sms(app, p.get("number"), p.get("message")).await,
        _ => bail!("unsupported action"),
    }
}
async fn at_command(app: &Arc<App>, p: &Params) -> Result<String> {
    let _guard = app.forwarding.sms_mutation.lock().await;
    if p.get("atcmd").is_empty() {
        return Ok(String::new());
    }
    app.at
        .fetch_wait(p.get("atcmd"), p.flag("force", false), p.flag("wait", true))
        .await
}
async fn ping(app: &Arc<App>) -> Result<String> {
    let connected = match app.monitor.connected() {
        Some(connected) => connected,
        None => crate::cell_lock::dialed(&app.at.fetch(crate::at::DASHBOARD, false).await?),
    };
    Ok(if connected { "OK" } else { "ERROR" }.into())
}
async fn raw_sms(app: &Arc<App>, p: &Params) -> Result<String> {
    if !app.forwarding.sms_enabled() {
        bail!("SMS service disabled")
    }
    app.at.page("sms", p.flag("force", false)).await
}
async fn legacy_send_sms(app: &Arc<App>, p: &Params) -> Result<String> {
    let v = send_sms(app, p.get("number"), p.get("msg")).await?;
    Ok(format!(
        "OK segments={} number={}",
        v["segments"],
        parser::text(&v, "number")
    ))
}
async fn set_ttl(app: &Arc<App>, p: &Params) -> Result<Value> {
    let _guard = app.ttl_lock.lock().await;
    let ttl = p.integer("ttlvalue", 0, 255)? as u8;
    let logs = system::set_ttl(
        ttl,
        app.config.mock,
        &app.config.ttl_file,
        app.store.clone(),
    )
    .await?;
    Ok(json!({ "debug_logs": logs }))
}
fn language(app: &Arc<App>) -> Value {
    std::fs::read(app.config.static_dir.join("config/get_language.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .unwrap_or(json!({"language":"zh-CN"}))
}
async fn set_language(app: &Arc<App>, p: &Params, body: &str) -> Result<Value> {
    let data: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let language = if p.get("language").is_empty() {
        parser::text(&data, "language")
    } else {
        p.get("language")
    };
    let language = match language.to_ascii_lowercase().as_str() {
        "zh" | "zh-cn" | "cn" | "chinese" => "zh-CN",
        "en" | "en-us" | "english" => "en",
        "ru" | "ru-ru" | "russian" => "ru",
        "ar" | "ar-sa" | "arabic" => "ar",
        _ => bail!("unsupported language"),
    };
    let value = json!({ "language": language });
    let bytes = format!("{value}\n");
    let path = app.config.static_dir.join("config/get_language.json");
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || store.write(&path, bytes.as_bytes(), 0o644)).await??;
    Ok(value)
}
async fn run_action(app: &Arc<App>, command: &str) -> Result<Value> {
    let _sms_guard = if command.to_ascii_uppercase().contains("+CMGD") {
        Some(app.forwarding.sms_mutation.lock().await)
    } else {
        None
    };
    if _sms_guard.is_some() && !app.forwarding.sms_enabled() {
        bail!("SMS service disabled")
    }
    let response = app.at.run(command).await?;
    app.at.invalidate().await;
    Ok(json!({"ok":parser::ok(&response),"response":response}))
}
async fn send_sms(app: &Arc<App>, number: &str, message: &str) -> Result<Value> {
    let _guard = app.forwarding.sms_mutation.lock().await;
    if !app.forwarding.sms_enabled() {
        bail!("SMS service disabled")
    }
    if message.is_empty() || message.len() > 65536 {
        bail!("missing or oversized message")
    }
    let number = sms::normalize_number(number, &app.at.fetch("AT+CIMI", false).await?)?;
    let parts = sms::submit(&number, message, rand::random::<u8>())?;
    for (i, (pdu, len)) in parts.iter().enumerate() {
        let raw = app
            .at
            .transaction(&format!("AT+CMGF=0;+CMGS={len}"), Some(pdu.clone()))
            .await?;
        if !sms::sent(&raw) {
            bail!("SMS segment {} failed: {}", i + 1, raw)
        }
        if i + 1 < parts.len() {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    app.at.invalidate().await;
    Ok(json!({"ok":true,"segments":parts.len(),"number":number}))
}
async fn change_password(app: &Arc<App>, p: &Params, root: bool, peer: Option<IpAddr>) -> Response {
    if let Some(wait) = app.auth.throttle.check(peer) {
        return throttled(wait);
    }
    let _guard = app.auth.mutation.lock().await;
    let get = |a, b| {
        let v = p.get(a);
        if v.is_empty() { p.get(b) } else { v }
    };
    let current = get("current_password", "currentPassword");
    let next = get("new_password", "newPassword");
    let confirm = get("confirm_password", "confirmPassword");
    let username = p.get("new_username");
    if !root && !username.is_empty() {
        let options = crate::install_credentials::Credentials {
            web_username: Some(username.into()),
            web_password: Some(if next.is_empty() { current } else { next }.into()),
            root_password: None,
        };
        if let Err(e) = options.validate() {
            return error(400, e);
        }
    }
    let next = if !root && !username.is_empty() && next.is_empty() {
        current
    } else {
        next
    };
    if let Err(e) = auth::validate(next) {
        return error(400, e);
    }
    if (root || !confirm.is_empty()) && next != confirm {
        return error(400, "password confirmation mismatch");
    }
    let app2 = app.clone();
    let current = current.to_owned();
    let next = next.to_owned();
    let username = username.to_owned();
    let result = tokio::task::spawn_blocking(move || -> Result<()> {
        if root {
            if !app2.auth.root_matches(&current, app2.config.mock) {
                bail!("current root password incorrect");
            }
            if app2.config.mock {
                app2.auth.mock_change_root(&next);
                return Ok(());
            }
            return auth::change_root(&app2.store, &current, &next, false).map_err(Into::into);
        }
        let (user, stored) = auth::read(&app2.auth.path)?;
        if !auth::password_matches(&current, &stored) {
            bail!("current password incorrect");
        }
        let user = if username.is_empty() { user } else { username };
        let line = auth::record(&user, &next)?;
        app2.store
            .write(&app2.auth.path, line.as_bytes(), 0o600)
            .map_err(Into::into)
    })
    .await
    .unwrap_or_else(|e| Err(e.into()));
    let committed = result
        .as_ref()
        .err()
        .and_then(|e| e.downcast_ref::<crate::persistence::SavedError>())
        .is_some_and(|e| e.committed);
    if result.is_ok() || committed {
        app.auth.revoke_all()
    }
    let mut response = match result {
        Ok(()) => {
            app.auth.throttle.succeeded(peer);
            json_response(200, json!({"ok":true,"redirect":"/login.html"}))
        }
        Err(e) if e.to_string().contains("incorrect") => {
            app.auth.throttle.failed(peer);
            error(403, e)
        }
        Err(e) => error(500, e),
    };
    if committed || response.status().is_success() {
        set_cookie(&mut response, "", !app.config.no_tls)
    }
    response
}
async fn websocket(
    State(app): State<Arc<App>>,
    Peer(peer): Peer,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers, !app.config.no_tls) {
        return error(403, "websocket origin forbidden");
    }
    let token = cookie(&headers);
    let permit = match app.sockets.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return error(503, "too many connections"),
    };
    upgrade
        .max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            ws_loop(app, socket, token, peer).await
        })
        .into_response()
}
async fn console_socket(
    State(app): State<Arc<App>>,
    Peer(peer): Peer,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers, !app.config.no_tls) {
        return error(403, "websocket origin forbidden");
    }
    let token = cookie(&headers);
    let permit = match app.consoles.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return error(503, "too many connections"),
    };
    upgrade
        .max_message_size(16 * 1024)
        .max_frame_size(16 * 1024)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            crate::console::run(app, socket, token, peer).await
        })
        .into_response()
}
#[derive(Deserialize)]
struct WsRequest {
    #[serde(default)]
    id: String,
    #[serde(default)]
    method: String,
    path: String,
    #[serde(default)]
    body: String,
}
fn ws_message(id: &str, status: u16, error: &str) -> String {
    json!({"id":id,"status":status,"body":"","error":error}).to_string()
}
/// Runs one WebSocket API request and serializes the reply envelope.
async fn ws_reply(app: &Arc<App>, request: WsRequest, token: &str, peer: Option<IpAddr>) -> String {
    let method = if request.method.is_empty() {
        "GET".into()
    } else {
        request.method.to_ascii_uppercase()
    };
    let mut response = match request.path.parse::<axum::http::Uri>() {
        Ok(uri)
            if uri.scheme().is_none()
                && uri.authority().is_none()
                && uri.path().starts_with("/api/")
                && !matches!(
                    uri.path(),
                    "/api/ws" | "/api/console/ws" | "/api/login" | "/api/logout"
                ) =>
        {
            api(
                app,
                &method,
                uri.path(),
                uri.query().unwrap_or(""),
                &request.body,
                token,
                peer,
            )
            .await
        }
        _ => error(404, "unsupported websocket API endpoint"),
    };
    let status = response.status().as_u16();
    let mut headers = serde_json::Map::new();
    for (name, value) in response.headers() {
        headers.insert(name.to_string(), json!([value.to_str().unwrap_or("")]));
    }
    let body = to_bytes(
        std::mem::replace(response.body_mut(), Body::empty()),
        2 * 1024 * 1024,
    )
    .await
    .unwrap_or_default();
    json!({"id":request.id,"status":status,"headers":headers,"body":String::from_utf8_lossy(&body)})
        .to_string()
}
async fn ws_loop(app: Arc<App>, mut socket: WebSocket, token: String, peer: Option<IpAddr>) {
    let Some(mut revoked) = app.auth.watch(&token) else {
        return;
    };
    // Requests run concurrently so a cell scan or SMS send never stalls the rest of the page.
    let permits = Arc::new(tokio::sync::Semaphore::new(WS_CONCURRENCY));
    let (replies, mut finished) = tokio::sync::mpsc::channel::<String>(WS_CONCURRENCY);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    loop {
        let outgoing = tokio::select! {
            _ = revoked.changed() => break,
            _ = heartbeat.tick() => {
                if !app.auth.valid(&token, false) {
                    break;
                }
                Message::Ping(Vec::new().into())
            }
            Some(reply) = finished.recv() => Message::Text(reply.into()),
            message = socket.recv() => {
                let payload = match message {
                    Some(Ok(Message::Text(s))) => s.as_bytes().to_vec(),
                    Some(Ok(Message::Binary(s))) => s.to_vec(),
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => continue,
                };
                match serde_json::from_slice::<WsRequest>(&payload) {
                    Err(_) => Message::Text(ws_message("", 400, "invalid JSON request").into()),
                    Ok(request) => match permits.clone().try_acquire_owned() {
                        Err(_) => Message::Text(
                            ws_message(&request.id, 429, "too many concurrent requests").into(),
                        ),
                        Ok(permit) => {
                            let (app, token, replies) = (app.clone(), token.clone(), replies.clone());
                            tokio::spawn(async move {
                                let reply = ws_reply(&app, request, &token, peer).await;
                                drop(permit);
                                let _ = replies.send(reply).await;
                            });
                            continue;
                        }
                    },
                }
            }
        };
        if !matches!(
            tokio::time::timeout(Duration::from_secs(10), socket.send(outgoing)).await,
            Ok(Ok(()))
        ) || !app.auth.valid(&token, false)
        {
            break;
        }
    }
    let _ = socket.close().await;
}
