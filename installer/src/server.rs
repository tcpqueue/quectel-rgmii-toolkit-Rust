//! Local web UI backend. It listens on 127.0.0.1 only; the first page load exchanges the random
//! launch token for a SameSite=Strict cookie, and every request must also name this exact host so
//! other websites (including DNS-rebinding pages) cannot drive the installer.

use crate::{
    adb::{Adb, Device},
    credentials,
    engine::{self, Operation, Outcome, Stage},
    qualcomm::{self, AtLink, Cancel, ModuleIdentity, NetworkProfile, UsbProfile},
};
use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const COOKIE: &str = "sa_setup";
const LOG_LINES: usize = 4000;
/// The page polls every second while visible; background tabs may be throttled to once a minute.
const IDLE_EXIT: Duration = Duration::from_secs(180);
/// After the page reports it is closing, wait briefly in case it was only reloaded.
const CLOSE_GRACE: Duration = Duration::from_secs(15);

#[derive(Default, Serialize)]
struct Log {
    #[serde(skip)]
    lines: VecDeque<(u64, String)>,
    next: u64,
}
impl Log {
    fn push(&mut self, text: &str) {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let stamped = format!("[{}] {line}", chrono::Local::now().format("%H:%M:%S"));
            self.lines.push_back((self.next, stamped));
            self.next += 1;
            if self.lines.len() > LOG_LINES {
                self.lines.pop_front();
            }
        }
    }
    fn since(&self, cursor: u64) -> Value {
        let lines: Vec<&str> = self
            .lines
            .iter()
            .filter(|(seq, _)| *seq >= cursor)
            .map(|(_, text)| text.as_str())
            .collect();
        // A cursor older than the buffer means the client must replace its copy.
        let reset =
            cursor < self.lines.front().map_or(self.next, |(seq, _)| *seq) || cursor > self.next;
        json!({"lines": lines, "next": self.next, "reset": reset})
    }
    fn text(&self) -> String {
        self.lines
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Serialize)]
struct Status {
    title: String,
    message: String,
    /// info, success, warning or error.
    severity: &'static str,
}
fn status(title: &str, message: &str, severity: &'static str) -> Status {
    Status {
        title: title.into(),
        message: message.into(),
        severity,
    }
}

#[derive(Clone, Serialize)]
struct Pending {
    id: String,
    title: String,
    description: String,
    commands: Vec<String>,
    destructive: bool,
    #[serde(skip)]
    port: String,
    #[serde(skip)]
    kind: PendingKind,
}
#[derive(Clone)]
enum PendingKind {
    Adb(UsbProfile),
    Commands {
        disconnect: bool,
        recheck: Option<(Option<String>, NetworkProfile, bool)>,
    },
}

#[derive(Serialize)]
struct InfoItem {
    label: &'static str,
    value: String,
}

#[derive(Default)]
struct Inner {
    busy: Option<&'static str>,
    devices: Vec<Device>,
    devices_error: Option<String>,
    devices_scanned: bool,
    operation: Option<&'static str>,
    stage: Option<Stage>,
    outcome: Option<Outcome>,
    log: Log,
    ports: Vec<qualcomm::AtPort>,
    ports_error: Option<String>,
    identity: Option<(String, ModuleIdentity)>,
    at_log: Log,
    prepare: Option<Status>,
    pending: Option<Pending>,
    info: Vec<InfoItem>,
    closing: Option<Instant>,
}

pub struct App {
    pub token: String,
    pub port: u16,
    pub adb: Adb,
    pub development: PathBuf,
    pub data_dir: PathBuf,
    inner: Mutex<Inner>,
    cancel: Mutex<Cancel>,
    scanning: std::sync::atomic::AtomicBool,
    last_seen: Mutex<Instant>,
    /// Last authorized request; `None` until a page has opened.
    page_seen: Mutex<Option<Instant>>,
    pub shutdown: tokio::sync::Notify,
}

impl App {
    pub fn new(
        token: String,
        port: u16,
        adb: Adb,
        development: PathBuf,
        data_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            token,
            port,
            adb,
            development,
            data_dir,
            inner: Mutex::default(),
            cancel: Mutex::default(),
            scanning: std::sync::atomic::AtomicBool::new(false),
            last_seen: Mutex::new(Instant::now()),
            page_seen: Mutex::new(None),
            shutdown: tokio::sync::Notify::new(),
        })
    }
    pub fn reports(&self) -> PathBuf {
        self.data_dir.join("Reports")
    }
    /// For the native window: the running operation and whether a page is open. An open page
    /// polls at least every 5 seconds, also in a background tab.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn summary(&self) -> (Option<&'static str>, bool) {
        let seen = *self.page_seen.lock().unwrap();
        let inner = self.lock();
        let open = seen.is_some_and(|seen| {
            seen.elapsed() < Duration::from_secs(8) && !inner.closing.is_some_and(|at| at >= seen)
        });
        (inner.busy.map(label), open)
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Claims the single operation slot shared by ADB and serial work.
    fn begin(&self, name: &'static str) -> Result<(), Box<Response>> {
        let mut inner = self.lock();
        if let Some(current) = inner.busy {
            return Err(Box::new(failure(
                StatusCode::CONFLICT,
                &format!("当前操作（{}）尚未结束，请等待结果。", label(current)),
            )));
        }
        inner.busy = Some(name);
        *self.cancel.lock().unwrap() = Cancel::default();
        Ok(())
    }
    fn finish(&self) {
        self.lock().busy = None;
    }
    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/", get(index))
            .route(
                "/app.js",
                get(|| async {
                    asset(
                        "text/javascript; charset=utf-8",
                        include_str!("../web/app.js"),
                    )
                }),
            )
            .route(
                "/app.css",
                get(|| async { asset("text/css; charset=utf-8", include_str!("../web/app.css")) }),
            )
            .route(
                "/select.svg",
                get(|| async { asset("image/svg+xml", include_str!("../web/select.svg")) }),
            )
            .route(
                "/quectel-logo.svg",
                get(|| async { asset("image/svg+xml", include_str!("../web/quectel-logo.svg")) }),
            )
            .route("/api/state", get(state))
            .route("/api/devices", post(refresh_devices))
            .route("/api/run", post(run_operation))
            .route("/api/report", get(report))
            .route("/api/open", post(open))
            .route("/api/ports", post(scan_ports))
            .route("/api/at/identify", post(identify))
            .route("/api/at/preview", post(preview))
            .route("/api/at/confirm", post(confirm))
            .route("/api/at/dismiss", post(dismiss))
            .route("/api/at/info", post(read_info))
            .route("/api/at/stop", post(stop))
            .route("/api/at/preset", get(load_preset).post(save_preset))
            .route("/api/at/log", post(save_at_log))
            .route("/api/bye", post(bye))
            .route("/api/quit", post(quit))
            .layer(middleware::from_fn_with_state(self.clone(), guard))
            .with_state(self.clone())
    }
    /// Exits after the page has gone away and no operation is running.
    pub async fn watchdog(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let seen = *self.last_seen.lock().unwrap();
            let inner = self.lock();
            if inner.busy.is_some() {
                continue;
            }
            let closed = inner
                .closing
                .is_some_and(|at| at >= seen && at.elapsed() > CLOSE_GRACE);
            if closed || seen.elapsed() > IDLE_EXIT {
                drop(inner);
                self.shutdown.notify_one();
                return;
            }
        }
    }
}

fn label(operation: &str) -> &'static str {
    match operation {
        "install" => "安装 / 升级",
        "diagnose" => "故障诊断",
        "web" => "打开管理页面",
        _ => "串口操作",
    }
}
fn failure(code: StatusCode, message: &str) -> Response {
    (code, Json(json!({"ok": false, "error": message}))).into_response()
}
fn accepted() -> Response {
    (StatusCode::ACCEPTED, Json(json!({"ok": true}))).into_response()
}
fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}
fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|c| c.trim().strip_prefix(&format!("{COOKIE}=")))
}
fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

async fn guard(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let expected_host = format!("127.0.0.1:{}", app.port);
    // Rejecting any other Host blocks DNS rebinding; the browser always connects by IP.
    let single = |name: header::HeaderName| {
        let mut values = headers.get_all(name).iter();
        match (values.next(), values.next()) {
            (Some(value), None) => value.to_str().ok(),
            _ => None,
        }
    };
    if single(header::HOST) != Some(expected_host.as_str()) {
        return failure(StatusCode::MISDIRECTED_REQUEST, "invalid host");
    }
    if headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|site| !matches!(site, "same-origin" | "none"))
    {
        return failure(StatusCode::FORBIDDEN, "cross-site request forbidden");
    }
    let launch =
        request.uri().path() == "/" && request.uri().query().is_some_and(|q| q.starts_with("k="));
    let authorized = cookie_token(headers).is_some_and(|t| constant_eq(t, &app.token));
    if !authorized && !launch {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<!doctype html><meta charset=utf-8><title>SimpleAdmin</title><p style=\"font:16px system-ui;margin:3em\">请双击 SimpleAdmin-Setup.exe 打开设备助手。此页面只接受由设备助手直接打开的浏览器窗口。</p>",
        )
            .into_response();
    }
    if request.method() != axum::http::Method::GET
        && single(header::ORIGIN) != Some(format!("http://{expected_host}").as_str())
    {
        return failure(StatusCode::FORBIDDEN, "origin forbidden");
    }
    if authorized {
        *app.last_seen.lock().unwrap() = Instant::now();
        if request.uri().path() != "/api/bye" {
            *app.page_seen.lock().unwrap() = Some(Instant::now());
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
    );
    response
}

#[derive(Deserialize)]
struct Launch {
    k: Option<String>,
}
async fn index(State(app): State<Arc<App>>, Query(launch): Query<Launch>) -> Response {
    if let Some(token) = launch.k {
        if !constant_eq(&token, &app.token) {
            return failure(StatusCode::FORBIDDEN, "invalid launch token");
        }
        // Move the token from the address bar into an HttpOnly cookie.
        let mut response = Redirect::to("/").into_response();
        response.headers_mut().insert(
            header::SET_COOKIE,
            HeaderValue::from_str(&format!(
                "{COOKIE}={}; Path=/; HttpOnly; SameSite=Strict",
                app.token
            ))
            .unwrap(),
        );
        return response;
    }
    asset(
        "text/html; charset=utf-8",
        include_str!("../web/index.html"),
    )
}

#[derive(Deserialize)]
struct Cursors {
    #[serde(default)]
    log: u64,
    #[serde(default)]
    at: u64,
}
async fn state(State(app): State<Arc<App>>, Query(cursors): Query<Cursors>) -> Response {
    let inner = app.lock();
    let identity = inner.identity.as_ref().map(|(port, identity)| {
        json!({
            "port": port,
            "manufacturer": identity.manufacturer,
            "model": identity.model,
            "firmware": identity.firmware,
            "imei": identity.masked_imei(),
            "supported": identity.supported() && identity.imei.len() == 15,
        })
    });
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "busy": inner.busy,
        "devices": inner.devices,
        "devicesError": inner.devices_error,
        "devicesScanned": inner.devices_scanned,
        "operation": inner.operation,
        "stage": inner.stage,
        "outcome": inner.outcome,
        "log": inner.log.since(cursors.log),
        "ports": inner.ports,
        "portsError": inner.ports_error,
        "identity": identity,
        "prepare": inner.prepare,
        "pending": inner.pending,
        "info": inner.info,
        "atLog": inner.at_log.since(cursors.at),
        "reports": app.reports(),
    }))
    .into_response()
}

async fn refresh_devices(State(app): State<Arc<App>>) -> Response {
    use std::sync::atomic::Ordering;
    // Listing devices is read-only; skip it while an operation drives adb or a scan is running.
    if app.lock().busy.is_some() || app.scanning.swap(true, Ordering::SeqCst) {
        return accepted();
    }
    let adb = app.adb.clone();
    let result = tokio::task::spawn_blocking(move || adb.devices())
        .await
        .unwrap_or_else(|e| Err(e.into()));
    {
        let mut inner = app.lock();
        inner.devices_scanned = true;
        match result {
            Ok(devices) => {
                inner.devices = devices;
                inner.devices_error = None;
            }
            Err(e) => inner.devices_error = Some(e.to_string()),
        }
    }
    app.scanning.store(false, Ordering::SeqCst);
    accepted()
}

#[derive(Deserialize)]
struct RunRequest {
    operation: String,
    serial: String,
    #[serde(default)]
    http_port: Option<String>,
    #[serde(default)]
    web_username: Option<String>,
    #[serde(default)]
    web_password: Option<String>,
    #[serde(default)]
    root_password: Option<String>,
}
struct EngineSink {
    app: Arc<App>,
}
impl engine::Sink for EngineSink {
    fn line(&mut self, text: &str) {
        self.app.lock().log.push(text)
    }
    fn stage(&mut self, stage: Stage) {
        self.app.lock().stage = Some(stage)
    }
}
async fn run_operation(State(app): State<Arc<App>>, Json(request): Json<RunRequest>) -> Response {
    let operation = match request.operation.as_str() {
        "install" => {
            let http_port = match request.http_port.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(value) => match engine::parse_port(value) {
                    Some(port) => Some(port),
                    None => {
                        return failure(
                            StatusCode::BAD_REQUEST,
                            "请输入 1–65535 的整数，例如 80 或 8080。尚未修改设备。",
                        );
                    }
                },
            };
            let web = request
                .web_username
                .as_deref()
                .zip(request.web_password.as_deref());
            if web.is_none() && (request.web_username.is_some() || request.web_password.is_some()) {
                return failure(
                    StatusCode::BAD_REQUEST,
                    "Web 账号和密码需要同时填写。尚未修改设备。",
                );
            }
            let credentials = match credentials::build(web, request.root_password.as_deref()) {
                Ok(json) => json,
                Err(e) => return failure(StatusCode::BAD_REQUEST, &format!("{e} 尚未修改设备。")),
            };
            Operation::Install {
                http_port,
                credentials,
            }
        }
        "diagnose" => Operation::Diagnose,
        "web" => Operation::OpenWeb,
        _ => return failure(StatusCode::BAD_REQUEST, "unknown operation"),
    };
    let ready = app
        .lock()
        .devices
        .iter()
        .any(|d| d.serial == request.serial && d.ready());
    if !ready {
        return failure(StatusCode::BAD_REQUEST, "请先选择已连接并授权的设备。");
    }
    let name = operation.name();
    if let Err(response) = app.begin(name) {
        return *response;
    }
    {
        let mut inner = app.lock();
        inner.operation = Some(name);
        inner.stage = Some(Stage::Device);
        inner.outcome = None;
        inner.log = Log {
            next: inner.log.next,
            ..Log::default()
        };
    }
    let worker = app.clone();
    tokio::task::spawn_blocking(move || {
        let mut sink = EngineSink {
            app: worker.clone(),
        };
        let outcome = engine::run(
            &operation,
            &worker.adb,
            &worker.development,
            &request.serial,
            &worker.reports(),
            &mut sink,
        );
        if outcome.ok
            && matches!(operation, Operation::OpenWeb)
            && let Some(url) = &outcome.url
            && let Err(e) = crate::open_url(url)
        {
            worker.lock().log.push(&format!(
                "浏览器未能自动打开，请复制此地址访问：{url}（{e}）"
            ));
        }
        worker.lock().outcome = Some(outcome);
        worker.finish();
    });
    accepted()
}

async fn report(State(app): State<Arc<App>>) -> Response {
    let path = app.lock().outcome.as_ref().and_then(|o| o.report.clone());
    let Some(path) = path else {
        return failure(
            StatusCode::NOT_FOUND,
            "报告文件尚未生成，请先运行安装或诊断。",
        );
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => Json(json!({"path": path, "text": text})).into_response(),
        Err(_) => failure(
            StatusCode::NOT_FOUND,
            "报告文件已被移动或删除，请重新运行诊断。",
        ),
    }
}

#[derive(Deserialize)]
struct OpenRequest {
    target: String,
}
async fn open(State(app): State<Arc<App>>, Json(request): Json<OpenRequest>) -> Response {
    let result = match request.target.as_str() {
        "report" => match app.lock().outcome.as_ref().and_then(|o| o.report.clone()) {
            Some(path) if path.is_file() => crate::open_text(&path),
            _ => Err(anyhow::anyhow!(
                "报告文件尚未生成或已被移动，请重新运行诊断。"
            )),
        },
        "reports" => std::fs::create_dir_all(app.reports())
            .map_err(Into::into)
            .and_then(|_| crate::open_folder(&app.reports())),
        "web" => match app.lock().outcome.as_ref().and_then(|o| o.url.clone()) {
            Some(url) => crate::open_url(&url),
            None => Err(anyhow::anyhow!(
                "没有可用的管理页面地址，请点击“打开管理页面”。"
            )),
        },
        _ => Err(anyhow::anyhow!("unknown target")),
    };
    match result {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => failure(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

async fn scan_ports(State(app): State<Arc<App>>) -> Response {
    let result = tokio::task::spawn_blocking(qualcomm::list_ports)
        .await
        .unwrap_or_else(|e| Err(e.into()));
    {
        let mut inner = app.lock();
        match result {
            Ok(ports) => {
                if ports.is_empty() {
                    inner.prepare = Some(status(
                        "未发现串口",
                        "检查 USB 数据线与模块串口驱动。若已有 ADB，可直接前往安装。",
                        "warning",
                    ));
                }
                inner.ports = ports;
                inner.ports_error = None;
            }
            Err(e) => inner.ports_error = Some(e.to_string()),
        }
    }
    accepted()
}

#[derive(Deserialize)]
struct PortRequest {
    port: String,
}

/// Runs serial work in the background. Unless `verify` is false, the module on the port must
/// still be the one identified earlier before anything is sent.
fn serial_task(
    app: &Arc<App>,
    port: String,
    title: &'static str,
    verify: bool,
    work: impl FnOnce(&Arc<App>, &mut dyn AtLink, &Cancel) -> Result<()> + Send + 'static,
) -> Response {
    let expected = {
        let inner = app.lock();
        inner.identity.clone()
    };
    let expected = match (verify, expected) {
        (false, _) => None,
        (true, Some((identified, identity)))
            if identified == port && identity.supported() && identity.imei.len() == 15 =>
        {
            Some(identity)
        }
        (true, _) => {
            app.lock().prepare = Some(status(
                "请先识别模块",
                "仅对已确认的移远高通型号开放配置操作。",
                "warning",
            ));
            return failure(StatusCode::CONFLICT, "请先识别模块。");
        }
    };
    if let Err(response) = app.begin("serial") {
        return *response;
    }
    let cancel = app.cancel.lock().unwrap().clone();
    {
        let mut inner = app.lock();
        inner.prepare = Some(status(
            title,
            "请保持连接，正在与模块通信。确认窗口出现前只读取信息。",
            "info",
        ));
        inner.at_log.push(&format!("{title} · {port}"));
    }
    let worker = app.clone();
    tokio::task::spawn_blocking(move || {
        let result = qualcomm::SerialAtLink::open(&port).and_then(|mut link| {
            if let Some(expected) = &expected {
                qualcomm::verify_identity(&mut link, expected, &cancel)?;
            }
            work(&worker, &mut link, &cancel)
        });
        if let Err(error) = result {
            let mut inner = worker.lock();
            inner.identity = None;
            inner.pending = None;
            if error.is::<qualcomm::Cancelled>() {
                inner.prepare = Some(status(
                    "已停止后续操作",
                    "已执行的指令不会自动撤销，请重新识别模块检查状态。",
                    "warning",
                ));
                inner.at_log.push("用户停止操作。");
            } else {
                inner.prepare = Some(status("操作未完成", &error.to_string(), "error"));
                inner.at_log.push(&error.to_string());
            }
        }
        worker.finish();
    });
    accepted()
}

async fn identify(State(app): State<Arc<App>>, Json(request): Json<PortRequest>) -> Response {
    let port = request.port.clone();
    serial_task(
        &app,
        request.port,
        "识别模块",
        false,
        move |app, link, cancel| {
            let found = qualcomm::identify(link, cancel)?;
            let mut inner = app.lock();
            inner
                .at_log
                .push(&format!("识别结果：{} / {}", found.model, found.firmware));
            inner.prepare = Some(if found.supported() {
                status(
                    "已识别移远高通模块",
                    "可以开启 ADB，或在“联网方式”中配置网口。SimpleAdmin 安装仍需通过设备系统兼容性检查。",
                    "success",
                )
            } else {
                status(
                    "型号暂未适配",
                    "保留识别信息，不开放解锁或配置修改。紫光展锐/MTK 型号不能套用此流程。",
                    "warning",
                )
            });
            inner.identity = Some((port, found));
            inner.info.clear();
            Ok(())
        },
    )
}

#[derive(Deserialize)]
struct PreviewRequest {
    port: String,
    action: String,
    #[serde(default)]
    enable: bool,
    #[serde(default)]
    profile: Option<NetworkProfile>,
    #[serde(default)]
    driver: Option<String>,
    #[serde(default)]
    commands: Option<String>,
}
fn pending(
    app: &Arc<App>,
    port: &str,
    title: &str,
    description: &str,
    commands: Vec<String>,
    destructive: bool,
    kind: PendingKind,
) {
    let mut inner = app.lock();
    inner.pending = Some(Pending {
        id: hex::encode(rand::random::<[u8; 8]>()),
        title: title.into(),
        description: description.into(),
        commands,
        destructive,
        port: port.into(),
        kind,
    });
    inner.prepare = Some(status(title, "请在确认窗口中核对指令。", "info"));
}
async fn preview(State(app): State<Arc<App>>, Json(request): Json<PreviewRequest>) -> Response {
    let port = request.port.clone();
    let commands_only = |title: &str,
                         description: &str,
                         commands: Vec<String>,
                         destructive: bool,
                         disconnect: bool| {
        // Writes are confirmed first; the module is re-verified when the plan runs.
        let identified = app
            .lock()
            .identity
            .as_ref()
            .is_some_and(|(p, i)| *p == port && i.supported() && i.imei.len() == 15);
        if !identified {
            return failure(StatusCode::CONFLICT, "请先识别模块。");
        }
        pending(
            &app,
            &port,
            title,
            description,
            commands,
            destructive,
            PendingKind::Commands {
                disconnect,
                recheck: None,
            },
        );
        accepted()
    };
    match request.action.as_str() {
        "restart" => commands_only(
            "重启模块",
            "当前网络会中断，USB 和 ADB 可能暂时消失。模块恢复后前往安装页刷新连接。",
            vec!["AT+CFUN=1,1".into()],
            false,
            true,
        ),
        "factory" => commands_only(
            "恢复模块出厂设置",
            "将恢复模块配置，可能清除 APN、频段、USB 等自定义设置并重启。此操作不能撤销，也不等同于卸载 SimpleAdmin。",
            vec!["AT+QCFG=\"ResetFactory\"".into()],
            true,
            true,
        ),
        "custom" => match qualcomm::parse_custom(request.commands.as_deref().unwrap_or("")) {
            Ok(commands) => commands_only(
                "执行自定义 AT",
                "请核对每一条指令。配置类命令可能中断网络或重启模块；中途停止不会撤销已经执行的指令。",
                commands,
                false,
                false,
            ),
            Err(e) => {
                app.lock().prepare = Some(status("请检查指令", &e.to_string(), "warning"));
                failure(StatusCode::BAD_REQUEST, &e.to_string())
            }
        },
        "adb" => serial_task(
            &app,
            request.port,
            "检查 ADB 配置",
            true,
            move |app, link, cancel| {
                let response = qualcomm::require(link, "AT+QCFG=\"usbcfg\"", cancel)?;
                app.lock()
                    .at_log
                    .push(&format!("USB 配置原始返回：{response}"));
                let profile = UsbProfile::parse(&response)?;
                if profile.adb_enabled() {
                    let mut inner = app.lock();
                    inner.prepare = Some(status(
                        "ADB 接口已开启",
                        "未修改配置。请前往安装页刷新设备；若仍未出现，请检查驱动或手动重启模块。",
                        "success",
                    ));
                    inner.at_log.push(&format!(
                        "USB 配置倒数第二项为 {}，ADB 已开启，跳过解锁。",
                        profile.adb()
                    ));
                    return Ok(());
                }
                let command = profile.enable_adb_command();
                pending(
                    app,
                    &port,
                    "启用 ADB",
                    "将 USB 配置倒数第二项从 0 改为 2，开启免授权 ADB；保留原 VID、PID 和其他接口。不计算或发送 ADB 密钥。",
                    vec![command],
                    false,
                    PendingKind::Adb(profile),
                );
                Ok(())
            },
        ),
        "ethernet" => {
            let Some(profile) = request.profile else {
                return failure(StatusCode::BAD_REQUEST, "请选择连接方案。");
            };
            let driver = request.driver.clone();
            let enable = request.enable;
            let title = format!(
                "{}{}",
                if enable { "配置 " } else { "停用 " },
                profile.label()
            );
            serial_task(
                &app,
                request.port,
                "检查联网配置",
                true,
                move |app, link, cancel| {
                    let response = qualcomm::require(link, "AT+QMAP=\"MPDN_RULE\"", cancel)?;
                    app.lock().at_log.push(&format!(
                        "现有 MPDN 规则：{}",
                        if response.is_empty() {
                            "无"
                        } else {
                            &response
                        }
                    ));
                    let has_rule = qualcomm::has_mpdn_rule_zero(&response)?;
                    let commands =
                        qualcomm::ethernet_plan(driver.as_deref(), profile, enable, has_rule)?;
                    let description = format!(
                        "{}{}{}配置可能需重启生效，执行后可单独选择重启。",
                        if has_rule {
                            "检测到 MPDN 规则 0，将先关闭该规则。"
                        } else {
                            "未配置 MPDN 规则 0，无需关闭。"
                        },
                        if enable {
                            "使用 QMAPWAC 自动拨号，不创建 MPDN 规则。"
                        } else {
                            "关闭 QMAPWAC 自动拨号，网络将中断。"
                        },
                        if profile == NetworkProfile::Pcie {
                            "PCIe 方案按转接板选择网卡型号。"
                        } else {
                            "USB 方案切换数据接口与网卡模式，不修改 PCIe 网卡或 SIM 检测。"
                        },
                    );
                    pending(
                        app,
                        &port,
                        &title,
                        &description,
                        commands,
                        false,
                        PendingKind::Commands {
                            disconnect: false,
                            recheck: Some((driver.clone(), profile, enable)),
                        },
                    );
                    Ok(())
                },
            )
        }
        _ => failure(StatusCode::BAD_REQUEST, "unknown action"),
    }
}

#[derive(Deserialize)]
struct ConfirmRequest {
    id: String,
}
async fn confirm(State(app): State<Arc<App>>, Json(request): Json<ConfirmRequest>) -> Response {
    let plan = {
        let mut inner = app.lock();
        match inner.pending.take() {
            Some(plan) if plan.id == request.id => plan,
            other => {
                inner.pending = other;
                return failure(StatusCode::CONFLICT, "确认已过期，请重新预览。");
            }
        }
    };
    let title: &'static str = match plan.kind {
        PendingKind::Adb(_) => "启用 ADB",
        _ => "执行已确认的指令",
    };
    serial_task(
        &app,
        plan.port.clone(),
        title,
        true,
        move |app, link, cancel| {
            let log = |line: String| app.lock().at_log.push(&line);
            match &plan.kind {
                PendingKind::Adb(profile) => {
                    let changed = qualcomm::apply_adb(link, profile, cancel)?;
                    let mut inner = app.lock();
                    if changed {
                        inner.prepare = Some(status(
                            "ADB 配置已验证",
                            "USB 配置中的 ADB 已开启。请重启模块后前往安装页刷新；ADB 连接可用后才能安装。",
                            "success",
                        ));
                        inner
                            .at_log
                            .push("ADB USB 配置写入并复查通过，未自动重启。");
                    } else {
                        inner.prepare = Some(status(
                            "ADB 接口已开启",
                            "复查时倒数第二项已为 1 或 2，已跳过 USB 配置写入。",
                            "success",
                        ));
                        inner.at_log.push("ADB 已开启，跳过解锁。");
                    }
                }
                PendingKind::Commands {
                    disconnect,
                    recheck,
                } => {
                    if let Some((driver, profile, enable)) = recheck {
                        // The plan depends on the MPDN rule seen at preview time.
                        let response = qualcomm::require(link, "AT+QMAP=\"MPDN_RULE\"", cancel)?;
                        let has_rule = qualcomm::has_mpdn_rule_zero(&response)?;
                        if qualcomm::ethernet_plan(driver.as_deref(), *profile, *enable, has_rule)?
                            != plan.commands
                        {
                            bail!("模块配置已变化，请重新预览；未执行任何指令。")
                        }
                    }
                    let result =
                        qualcomm::execute_plan(link, &plan.commands, cancel, &mut |line| log(line));
                    match result {
                        Err(e) if *disconnect && !e.is::<qualcomm::Cancelled>() => {
                            let mut inner = app.lock();
                            inner.identity = None;
                            inner.prepare = Some(status(
                                "指令结果未确认",
                                &format!(
                                    "模块可能正在断开或重启。请等待恢复并重新识别，不会自动重复发送。{e}"
                                ),
                                "warning",
                            ));
                            return Ok(());
                        }
                        other => other?,
                    }
                    let mut inner = app.lock();
                    if *disconnect {
                        inner.identity = None;
                    }
                    inner.prepare = Some(status(
                        "指令已被模块接受",
                        if *disconnect {
                            "请等待模块恢复连接，再前往安装页刷新设备。"
                        } else {
                            "配置类指令可能需重启生效。可重新读取设备信息核对，或前往安装后台。"
                        },
                        "success",
                    ));
                }
            }
            Ok(())
        },
    )
}

async fn dismiss(State(app): State<Arc<App>>) -> Response {
    let mut inner = app.lock();
    if inner.pending.take().is_some() {
        inner.prepare = Some(status("已取消", "未修改模块配置。", "info"));
    }
    Json(json!({"ok": true})).into_response()
}

async fn read_info(State(app): State<Arc<App>>, Json(request): Json<PortRequest>) -> Response {
    app.lock().info.clear();
    serial_task(
        &app,
        request.port,
        "读取设备信息",
        true,
        |app, link, cancel| {
            for (command, label) in qualcomm::INFO_COMMANDS {
                let reply = link.send(command, cancel)?;
                let value = if reply.ok {
                    qualcomm::body(&reply)
                } else {
                    "固件未提供".into()
                };
                let value = if value.is_empty() {
                    "无返回数据".into()
                } else {
                    value
                };
                let mut inner = app.lock();
                inner.at_log.push(&format!("{label}：{value}"));
                inner.info.push(InfoItem { label, value });
            }
            app.lock().prepare = Some(status(
                "设备信息读取完成",
                "未持续轮询；未读取短信，不支持的项目已标记。",
                "success",
            ));
            Ok(())
        },
    )
}

async fn stop(State(app): State<Arc<App>>) -> Response {
    app.cancel.lock().unwrap().cancel();
    Json(json!({"ok": true})).into_response()
}

fn preset_path(app: &App) -> PathBuf {
    app.data_dir.join("quectel-qualcomm-at.txt")
}
#[derive(Deserialize)]
struct PresetRequest {
    text: String,
}
async fn save_preset(State(app): State<Arc<App>>, Json(request): Json<PresetRequest>) -> Response {
    let result = qualcomm::parse_custom(&request.text).and_then(|_| {
        std::fs::create_dir_all(&app.data_dir)?;
        std::fs::write(preset_path(&app), &request.text)?;
        Ok(())
    });
    let mut inner = app.lock();
    inner.prepare = Some(match &result {
        Ok(()) => status(
            "已保存指令",
            "仅保存在此电脑，再次打开不会自动执行。",
            "success",
        ),
        Err(e) => status("保存失败", &e.to_string(), "warning"),
    });
    match result {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => failure(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}
async fn load_preset(State(app): State<Arc<App>>) -> Response {
    let result = (|| -> Result<String> {
        let path = preset_path(&app);
        if std::fs::metadata(&path).context("尚未保存指令。")?.len() > 20000 {
            bail!("指令文件过大。")
        }
        let text = std::fs::read_to_string(path)?;
        qualcomm::parse_custom(&text)?;
        Ok(text)
    })();
    let mut inner = app.lock();
    match result {
        Ok(text) => {
            inner.prepare = Some(status("指令已载入", "仅填入编辑框，尚未执行。", "info"));
            Json(json!({"ok": true, "text": text})).into_response()
        }
        Err(e) => {
            inner.prepare = Some(status("读取失败", &e.to_string(), "warning"));
            failure(StatusCode::NOT_FOUND, &e.to_string())
        }
    }
}
async fn save_at_log(State(app): State<Arc<App>>) -> Response {
    let text = app.lock().at_log.text();
    let path = app.reports().join(format!(
        "serial-{}.txt",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    let result = std::fs::create_dir_all(app.reports()).and_then(|_| std::fs::write(&path, text));
    let mut inner = app.lock();
    match result {
        Ok(()) => {
            inner.prepare = Some(status(
                "操作记录已保存",
                &format!(
                    "{}。记录可能包含网络地址和设备状态，分享前请检查。",
                    path.display()
                ),
                "success",
            ));
            Json(json!({"ok": true, "path": path})).into_response()
        }
        Err(e) => {
            inner.prepare = Some(status("保存失败", &e.to_string(), "error"));
            failure(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        }
    }
}

async fn bye(State(app): State<Arc<App>>) -> Response {
    app.lock().closing = Some(Instant::now());
    Json(json!({"ok": true})).into_response()
}
async fn quit(State(app): State<Arc<App>>) -> Response {
    if let Some(current) = app.lock().busy {
        return failure(
            StatusCode::CONFLICT,
            &format!(
                "当前操作（{}）尚未结束。安装过程中退出可能中断文件传输。",
                label(current)
            ),
        );
    }
    app.shutdown.notify_one();
    Json(json!({"ok": true})).into_response()
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
