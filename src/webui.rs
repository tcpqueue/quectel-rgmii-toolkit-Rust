use crate::{
    auth,
    server::{App, json_response},
};
use axum::response::Response;
use serde_json::json;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Notify};

pub struct ListenerState {
    address: Mutex<Option<SocketAddr>>,
    next: Mutex<Option<TcpListener>>,
    changed: Notify,
}
impl Default for ListenerState {
    fn default() -> Self {
        Self {
            address: Mutex::new(None),
            next: Mutex::new(None),
            changed: Notify::new(),
        }
    }
}
impl ListenerState {
    pub fn port(&self, fallback: &str) -> u16 {
        self.address
            .lock()
            .unwrap()
            .map(|a| a.port())
            .unwrap_or_else(|| {
                fallback
                    .rsplit(':')
                    .next()
                    .unwrap_or("80")
                    .parse()
                    .unwrap_or(80)
            })
    }
}
pub async fn serve(app: Arc<App>, mut listener: TcpListener) -> anyhow::Result<()> {
    *app.webui.address.lock().unwrap() = Some(listener.local_addr()?);
    let permits = Arc::new(tokio::sync::Semaphore::new(32));
    loop {
        tokio::select! {
            result = crate::http::serve_with_permits(listener, app.router(), permits.clone()) => { result?; return Ok(()); },
            _ = app.webui.changed.notified() => {
                listener = app.webui.next.lock().unwrap().take().ok_or_else(|| anyhow::anyhow!("missing replacement listener"))?;
            },
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}
pub fn snapshot(app: &App) -> Response {
    match auth::read(&app.auth.path) {
        Ok((username, _)) => json_response(
            200,
            json!({"username":username,"http_port":app.webui.port(&app.config.http),"http_enabled":app.config.no_tls}),
        ),
        Err(_) => json_response(500, json!({"ok":false,"error":"auth config error"})),
    }
}
fn failure(code: u16, error: &str) -> Response {
    json_response(code, json!({"ok":false,"error":error}))
}
pub async fn change_port(app: &Arc<App>, p: &crate::actions::Params) -> Response {
    let _guard = app.auth.mutation.lock().await;
    if !app.config.no_tls {
        return failure(400, "HTTP port changes are unavailable in HTTPS mode");
    }
    let (_, password) = match auth::read(&app.auth.path) {
        Ok(c) => c,
        Err(_) => return failure(500, "auth config error"),
    };
    if !auth::equal(p.get("current_password"), &password) {
        return failure(403, "current password incorrect");
    }
    let raw = p.get("http_port");
    let port = match raw.parse::<u16>() {
        Ok(port) if port > 0 && port.to_string() == raw => port,
        _ => return failure(400, "HTTP port must be 1-65535"),
    };
    if app.webui.next.lock().unwrap().is_some() {
        return failure(409, "HTTP port change is still being applied");
    }
    let address = *app.webui.address.lock().unwrap();
    let Some(mut address) = address else {
        return failure(409, "HTTP listener is not ready");
    };
    if address.port() == port {
        return json_response(200, json!({"ok":true,"changed":false,"http_port":port}));
    }
    address.set_port(port);
    let listener = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(_) => return failure(409, "HTTP port is already in use or unavailable"),
    };
    if !app.config.mock && firewall(port).await.is_err() {
        return failure(
            500,
            "Unable to configure HTTP firewall; current port was kept",
        );
    }
    let store = app.store.clone();
    let path = app.auth.path.with_file_name("http_port");
    let result = tokio::task::spawn_blocking(move || {
        store.write(&path, format!("{port}\n").as_bytes(), 0o644)
    })
    .await;
    let (saved, warning) = match result {
        Ok(Ok(())) => (true, None),
        Ok(Err(e)) if e.committed => (true, Some(e.to_string())),
        Ok(Err(_)) | Err(_) => (false, None),
    };
    if !saved {
        return failure(500, "HTTP port was not saved; current port was kept");
    }
    *app.webui.address.lock().unwrap() = Some(address);
    *app.webui.next.lock().unwrap() = Some(listener);
    app.webui.changed.notify_one();
    json_response(
        200,
        json!({"ok":true,"changed":true,"http_port":port,"warning":warning}),
    )
}
async fn iptables(args: &[&str]) -> anyhow::Result<bool> {
    let mut command = tokio::process::Command::new("iptables");
    command.args(args).kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(5), command.output()).await??;
    Ok(result.status.success())
}
async fn firewall(port: u16) -> anyhow::Result<()> {
    let port = port.to_string();
    // Queue the new listener only after the firmware's cellular block is restored above ACCEPT.
    if !iptables(&["-C", "INPUT", "-p", "tcp", "--dport", &port, "-j", "ACCEPT"]).await?
        && !iptables(&[
            "-I", "INPUT", "1", "-p", "tcp", "--dport", &port, "-j", "ACCEPT",
        ])
        .await?
    {
        anyhow::bail!("cannot add HTTP accept rule");
    }
    for entry in std::fs::read_dir("/sys/class/net")? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if !name.starts_with("rmnet") {
            continue;
        }
        if iptables(&[
            "-C", "INPUT", "-i", &name, "-p", "tcp", "--dport", "80", "-j", "DROP",
        ])
        .await?
        {
            if iptables(&[
                "-C", "INPUT", "-i", &name, "-p", "tcp", "--dport", &port, "-j", "DROP",
            ])
            .await?
                && !iptables(&[
                    "-D", "INPUT", "-i", &name, "-p", "tcp", "--dport", &port, "-j", "DROP",
                ])
                .await?
            {
                anyhow::bail!("cannot update cellular block");
            }
            if !iptables(&[
                "-I", "INPUT", "1", "-i", &name, "-p", "tcp", "--dport", &port, "-j", "DROP",
            ])
            .await?
            {
                anyhow::bail!("cannot preserve cellular block");
            }
        }
    }
    Ok(())
}
