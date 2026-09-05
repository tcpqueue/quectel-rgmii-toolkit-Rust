use crate::{
    Config,
    actions::{self, Params},
    at::At,
    auth::{self, Auth},
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
        Request, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::SinkExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tower_http::services::ServeDir;
#[cfg(test)]
#[path = "../tests/http/mod.rs"]
mod tests;

pub struct App {
    pub config: Config,
    pub at: At,
    pub auth: Auth,
    pub store: Arc<Store>,
    pub monitor: Arc<Monitor>,
    metrics: Metrics,
    pub sockets: Arc<tokio::sync::Semaphore>,
    ttl_lock: tokio::sync::Mutex<()>,
}
pub fn json_response(status: u16, value: Value) -> Response {
    (StatusCode::from_u16(status).unwrap(), axum::Json(value)).into_response()
}
fn error(status: u16, message: impl ToString) -> Response {
    json_response(status, json!({"ok":false,"error":message.to_string()}))
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
    let value = format!(
        "{}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax{}",
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
        Ok(Arc::new(Self {
            config,
            at,
            auth,
            store,
            monitor,
            metrics: Metrics::default(),
            sockets: Arc::new(tokio::sync::Semaphore::new(32)),
            ttl_lock: tokio::sync::Mutex::new(()),
        }))
    }
    pub fn start(self: &Arc<Self>) {
        self.at.start_refresh();
        self.monitor.start(self.at.clone());
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
            .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
            .layer(middleware::from_fn_with_state(self.clone(), gate))
            .with_state(self.clone())
    }
}
async fn gate(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let public = matches!(
        path,
        "/login.html"
            | "/logout.html"
            | "/js/locales.js"
            | "/api/login"
            | "/api/logout"
            | "/api/module_model"
    );
    if !public && !app.auth.valid(&cookie(request.headers()), true) {
        return if path.starts_with("/api/") || path.starts_with("/cgi-bin/") {
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
    if path == "/api/logout" || path == "/logout.html" {
        app.auth.revoke(&token);
        let mut response = if method == "GET" || path == "/logout.html" {
            redirect("/login.html")
        } else {
            json_response(200, json!({"ok":true,"redirect":"/login.html"}))
        };
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
    let body = match to_bytes(request.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return error(413, "request too large"),
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
    api(&app, &method, &uri, &query, body, &token).await
}
pub async fn api(
    app: &Arc<App>,
    method: &str,
    path: &str,
    query: &str,
    body: &str,
    token: &str,
) -> Response {
    if [
        "/api/login",
        "/api/set_password",
        "/api/set_root_password",
        "/api/telemetry/target",
    ]
    .contains(&path)
        && method != "POST"
    {
        return error(405, "method not allowed");
    }
    if path == "/api/telemetry" && method != "GET" {
        return error(405, "method not allowed");
    }
    if ["/api/login", "/api/set_password", "/api/set_root_password"].contains(&path)
        && body.len() > 4096
    {
        return error(413, "request too large");
    }
    if !matches!(method, "GET" | "POST") {
        return error(405, "method not allowed");
    }
    let p = match Params::parse(query, if body.starts_with('{') { "" } else { body }) {
        Ok(p) => p,
        Err(e) => return error(400, e),
    };
    if path == "/api/login" {
        let _guard = app.auth.mutation.lock().await;
        let credentials = match auth::read(&app.auth.path) {
            Ok(c) => c,
            Err(_) => return error(500, "auth config error"),
        };
        if !auth::equal(p.get("username"), &credentials.0)
            || !auth::equal(p.get("password"), &credentials.1)
        {
            return error(401, "invalid credentials");
        }
        let token = app.auth.create();
        let mut response = json_response(200, json!({"ok":true,"redirect":"/"}));
        set_cookie(&mut response, &token, !app.config.no_tls);
        return response;
    }
    if path != "/api/module_model" && !app.auth.valid(token, true) {
        return error(401, "login required");
    }
    if path == "/api/set_password" || path == "/api/set_root_password" {
        return change_password(app, &p, path.ends_with("root_password")).await;
    }
    let force = p.flag("force", false);
    let action = p.get("action");
    let result:Result<Response>=async {
        let value=match path {
            "/api/telemetry"=>app.monitor.snapshot(),
            "/api/telemetry/target"=>{
                #[derive(Deserialize)]#[serde(deny_unknown_fields)]struct Target{target:String}
                if body.len()>1024{bail!("target request too large")}let config:Target=serde_json::from_str(body)?;app.monitor.set_target(&config.target).await?;app.monitor.snapshot()
            },
            "/api/module_model"=>{let raw=app.at.page("model",force).await?;json!({"model":parser::model(&raw),"pending":raw.contains(crate::at_policy::PENDING)})},
            "/api/dashboard_data"=>{
                let raw=app.at.page("dashboard",force).await?;let mut data=parser::dashboard(&raw);if p.flag("debug",false){data["raw"]=json!(raw)}
                data["internetConnection"]=json!(if app.monitor.connected(){"已连接"}else{"未连接"});data["uptimeParts"]=system::uptime(app.config.mock).0;
                data.as_object_mut().unwrap().extend(app.metrics.read(app.config.mock).as_object().unwrap().clone());data["lastUpdate"]=json!(chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string());data
            },
            "/api/device_info_data"=>match action{""|"get"=>{let raw=app.at.page("device",force).await?;let mut data=parser::device(&raw);let model=app.at.page("model",force).await?;let name=parser::model(&model);if name!="-"{data["modelName"]=json!(name)}data["pending"]=json!(raw.contains(crate::at_policy::PENDING)||model.contains(crate::at_policy::PENDING));data},"set_imei"=>run_action(app,&actions::imei(&p)?).await?,_=>bail!("unsupported action")},
            "/api/network_data"=>match action {
                ""|"settings"=>parser::network(&app.at.page("network",force).await?),
                "model"=>{let raw=app.at.page("model",force).await?;json!({"model":parser::model(&raw),"pending":raw.contains(crate::at_policy::PENDING)})},
                "bands"=>{let command=if p.get("mode").is_empty(){crate::at::commands("bands")[0].to_owned()}else{format!("AT+QNWPREFCFG=\"{}\"",actions::band_mode(p.get("mode"))?)};let raw=app.at.fetch_wait(&command,force,p.flag("wait",true)).await?;let mut v=parser::bands(&raw);if raw.contains(crate::at_policy::PENDING){v["pending"]=json!(true)}else if raw.to_ascii_lowercase().contains("error"){v["error"]=json!(raw)}v},
                "scan"=>parser::scan(&app.at.run(actions::scan_mode(p.get("mode"))?).await?),
                _=>run_action(app,&actions::network(&p)?).await?
            },
            "/api/settings_data"=>if action.is_empty()||action=="status"{parser::settings(&app.at.page("settings",force).await?)}else{
                let commands=actions::settings(&p)?;if commands.len()>1{let app=app.clone();tokio::spawn(async move{for command in commands{tokio::time::sleep(Duration::from_secs(1)).await;if run_action(&app,&command).await.ok().is_none_or(|v|v["ok"]!=true){break}}});json!({"ok":true,"response":"设备即将重启","reboot":true,"rebooting":true,"rebootAfterSeconds":1,"rebootCountdownSeconds":40,"message":"设备即将重启，请等待前端倒计时。"})}else{run_action(app,&commands[0]).await?}
            },
            "/api/sms_data"=>match action {
                ""|"list"|"list_meta"=>{let mut data=sms::list(&app.at.page("sms",force).await?);if action=="list_meta"{for value in data["messages"].as_array_mut().unwrap(){value.as_object_mut().unwrap().remove("text");value.as_object_mut().unwrap().remove("textLines");}}data},
                "delete_all"=>run_action(app,"AT+CMGD=,4").await?,
                "delete_indices"=>{let values=p.list("indices",',');if values.is_empty()||values.len()>1024{bail!("missing indices or too many messages")}let mut commands=Vec::new();for v in values{let index=v.parse::<u16>()?;commands.push(format!("+CMGD={index}"))}run_action(app,&format!("AT{}",commands.join(";"))).await?},
                "sim_status"=>{let raw=app.at.run("AT+CPIN?").await?.to_ascii_uppercase();json!({"inserted":!raw.contains("SIM NOT INSERTED")&&!raw.contains("+CME ERROR: 10")})},
                "send"=>send_sms(app,p.get("number"),p.get("message")).await?,_=>bail!("unsupported action")
            },
            "/api/get_atcache"|"/api/get_atcommand"|"/api/user_atcommand"=>{return Ok(text_response(if p.get("atcmd").is_empty(){String::new()}else{app.at.fetch_wait(p.get("atcmd"),force,p.flag("wait",true)).await?}))},
            "/api/get_ping"=>return Ok(text_response(if app.monitor.connected(){"OK"}else{"ERROR"})),
            "/api/get_sms"=>return Ok(text_response(app.at.page("sms",force).await?)),
            "/api/send_sms"=>{let v=send_sms(app,p.get("number"),p.get("msg")).await?;return Ok(text_response(format!("OK segments={} number={}",v["segments"],parser::text(&v,"number"))))},
            "/api/get_uptime"=>return Ok(text_response(system::uptime(app.config.mock).1)),
            "/api/get_ttl_status"=>{let ttl=system::ttl(&app.config.ttl_file);json!({"isEnabled":ttl>0,"ttl":ttl})},
            "/api/set_ttl"=>{let _guard=app.ttl_lock.lock().await;let ttl=p.integer("ttlvalue",0,255)?as u8;json!({"debug_logs":system::set_ttl(ttl,app.config.mock,&app.config.ttl_file,app.store.clone()).await?})},
            "/api/get_language"=>{std::fs::read(app.config.static_dir.join("config/get_language.json")).ok().and_then(|b|serde_json::from_slice::<Value>(&b).ok()).unwrap_or(json!({"language":"zh-CN"}))},
            "/api/set_language"=>{
                let data:Value=serde_json::from_str(body).unwrap_or(Value::Null);let language=if p.get("language").is_empty(){parser::text(&data,"language")}else{p.get("language")};let language=match language.to_ascii_lowercase().as_str(){"zh"|"zh-cn"|"cn"|"chinese"=>"zh-CN","en"|"en-us"|"english"=>"en","ru"|"ru-ru"|"russian"=>"ru","ar"|"ar-sa"|"arabic"=>"ar",_=>bail!("unsupported language")};
                let value=json!({"language":language});let bytes=format!("{value}\n");let path=app.config.static_dir.join("config/get_language.json");let store=app.store.clone();tokio::task::spawn_blocking(move||store.write(&path,bytes.as_bytes(),0o644)).await??;value
            },
            "/api/mock_at"=>{
                if !app.config.mock{return Ok(error(403,"mock mode only"))}
                crate::mock::handle(&app.at,&p).await?
            },_=>return Ok(error(404,"unsupported API endpoint"))
        };Ok(json_response(200,value))
    }.await;
    match result {
        Ok(response) => response,
        Err(e) => error(400, e),
    }
}
async fn run_action(app: &Arc<App>, command: &str) -> Result<Value> {
    let response = app.at.run(command).await?;
    app.at.invalidate().await;
    Ok(json!({"ok":parser::ok(&response),"response":response}))
}
async fn send_sms(app: &Arc<App>, number: &str, message: &str) -> Result<Value> {
    let message = message.trim();
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
        if !parser::ok(&raw) {
            bail!("SMS segment {} failed: {}", i + 1, raw)
        }
    }
    app.at.invalidate().await;
    Ok(json!({"ok":true,"segments":parts.len(),"number":number}))
}
async fn change_password(app: &Arc<App>, p: &Params, root: bool) -> Response {
    let _guard = app.auth.mutation.lock().await;
    let get = |a, b| {
        let v = p.get(a);
        if v.is_empty() { p.get(b) } else { v }
    };
    let current = get("current_password", "currentPassword");
    let next = get("new_password", "newPassword");
    let confirm = get("confirm_password", "confirmPassword");
    if let Err(e) = auth::validate(next) {
        return error(400, e);
    }
    if (root || !confirm.is_empty()) && next != confirm {
        return error(400, "password confirmation mismatch");
    }
    let result = if root {
        let app2 = app.clone();
        let current = current.to_owned();
        let next = next.to_owned();
        match tokio::task::spawn_blocking(move || {
            if !app2.auth.root_matches(&current, app2.config.mock) {
                return Err(anyhow::anyhow!("current root password incorrect"));
            }
            if app2.config.mock {
                app2.auth.mock_change_root(&next);
                Ok(())
            } else {
                auth::change_root(&app2.store, &current, &next, false).map_err(Into::into)
            }
        })
        .await
        {
            Ok(r) => r,
            Err(e) => Err(e.into()),
        }
    } else {
        let (user, password) = match auth::read(&app.auth.path) {
            Ok(c) => c,
            Err(e) => return error(500, e),
        };
        if !auth::equal(current, &password) {
            return error(403, "current password incorrect");
        }
        let path = app.auth.path.clone();
        let store = app.store.clone();
        let data = format!("{user}:{next}\n");
        match tokio::task::spawn_blocking(move || store.write(&path, data.as_bytes(), 0o600)).await
        {
            Ok(r) => r.map_err(Into::into),
            Err(e) => Err(e.into()),
        }
    };
    let committed = result
        .as_ref()
        .err()
        .and_then(|e| e.downcast_ref::<crate::persistence::SavedError>())
        .is_some_and(|e| e.committed);
    if result.is_ok() || committed {
        app.auth.revoke_all()
    }
    let mut response = match result {
        Ok(()) => json_response(200, json!({"ok":true,"redirect":"/login.html"})),
        Err(e) => error(
            if e.to_string().contains("incorrect") {
                403
            } else {
                500
            },
            e,
        ),
    };
    if committed || response.status().is_success() {
        set_cookie(&mut response, "", !app.config.no_tls)
    }
    response
}
async fn websocket(
    State(app): State<Arc<App>>,
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
        .max_message_size(1024 * 1024)
        .max_frame_size(1024 * 1024)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            ws_loop(app, socket, token).await
        })
        .into_response()
}
async fn console_socket(
    State(app): State<Arc<App>>,
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
        .max_message_size(65536)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            crate::console::run(app, socket, token).await
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
async fn ws_loop(app: Arc<App>, mut socket: WebSocket, token: String) {
    let Some(mut revoked) = app.auth.watch(&token) else {
        return;
    };
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _=revoked.changed()=>break,
        _=heartbeat.tick()=>{if !app.auth.valid(&token,false){break}
            if !matches!(tokio::time::timeout(Duration::from_secs(10),socket.send(Message::Ping(Vec::new().into()))).await,Ok(Ok(()))){break}},
            message=socket.recv()=>{
                let payload=match message{Some(Ok(Message::Text(s)))=>s.as_bytes().to_vec(),Some(Ok(Message::Binary(s)))=>s.to_vec(),Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,_=>continue};
                let request=match serde_json::from_slice::<WsRequest>(&payload){Ok(r)=>r,Err(_)=>{let _=socket.send(Message::Text(json!({"id":"","status":400,"body":"","error":"invalid JSON request"}).to_string().into())).await;continue}};
                let method=if request.method.is_empty(){"GET".into()}else{request.method.to_ascii_uppercase()};
                let uri=request.path.parse::<axum::http::Uri>();let mut response=match uri {
                    Ok(uri) if uri.scheme().is_none()&&uri.authority().is_none()&&uri.path().starts_with("/api/")&&!matches!(uri.path(),"/api/ws"|"/api/console/ws"|"/api/login"|"/api/logout")=>api(&app,&method,uri.path(),uri.query().unwrap_or(""),&request.body,&token).await,
                    _=>error(404,"unsupported websocket API endpoint")
                };
                let status=response.status().as_u16();let mut headers=serde_json::Map::new();for(name,value)in response.headers(){headers.insert(name.to_string(),json!([value.to_str().unwrap_or("")]));}
                let body=to_bytes(std::mem::replace(response.body_mut(),Body::empty()),2*1024*1024).await.unwrap_or_default();
                let response=json!({"id":request.id,"status":status,"headers":headers,"body":String::from_utf8_lossy(&body)});
                if !matches!(tokio::time::timeout(Duration::from_secs(10),socket.send(Message::Text(response.to_string().into()))).await,Ok(Ok(()))){break}
                if !app.auth.valid(&token,false){break}
            }
        }
    }
    let _ = socket.close().await;
}
