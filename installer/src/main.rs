//! SimpleAdmin device assistant: a local web UI for preparing Quectel Qualcomm modules and
//! installing SimpleAdmin over ADB. Double-clicking the exe starts a server on a random
//! 127.0.0.1 port and opens it in the default browser. On Windows a small status window stays
//! open and closing it ends the program; elsewhere the program ends after the page closes.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod adb;
mod credentials;
mod engine;
mod payload;
mod qualcomm;
mod server;
#[cfg(windows)]
mod window;

use anyhow::{Context, Result};
use std::{
    fs::{self, File, TryLockError},
    path::{Path, PathBuf},
    process::Command,
};

struct Options {
    /// Use an unpacked tree (repo root with adb and `development/`) instead of the payload.
    payload_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    port: u16,
    browser: bool,
    /// Status window on Windows; without it the program exits after the page closes.
    window: bool,
    self_test: bool,
}
fn options() -> Result<Options> {
    let mut options = Options {
        payload_dir: None,
        data_dir: None,
        port: 0,
        browser: true,
        window: cfg!(windows),
        self_test: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--payload-dir" => options.payload_dir = args.next().map(PathBuf::from),
            "--data-dir" => options.data_dir = args.next().map(PathBuf::from),
            "--port" => options.port = args.next().context("--port needs a value")?.parse()?,
            "--no-browser" => options.browser = false,
            "--no-window" => options.window = false,
            "--self-test" => options.self_test = true,
            other => anyhow::bail!("unknown option {other}"),
        }
    }
    Ok(options)
}

/// `%LOCALAPPDATA%\SimpleAdmin` on Windows, `~/.local/share/SimpleAdmin` elsewhere.
fn default_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("SimpleAdmin")
}

fn main() {
    if let Err(error) = start() {
        let data_dir = default_data_dir().join("Reports");
        let _ = fs::create_dir_all(&data_dir);
        let report = data_dir.join(format!(
            "launcher-{}.txt",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        let _ = fs::write(&report, format!("{error:?}"));
        fatal(&format!(
            "无法启动设备助手：{error:#}\n报告：{}",
            report.display()
        ));
        std::process::exit(1);
    }
}

fn start() -> Result<()> {
    let options = options()?;
    let data_dir = options.data_dir.clone().unwrap_or_else(default_data_dir);
    fs::create_dir_all(&data_dir)?;
    let root = match &options.payload_dir {
        Some(dir) => dir.clone(),
        None => payload::extract(&data_dir.join("runtime"))?,
    };
    let adb = adb::Adb::new(adb::locate(&root));
    let development = root.join("development");
    if options.self_test {
        return self_test(&adb, &development);
    }
    // One assistant at a time: a second launch just reopens the running page.
    let lock = File::create(data_dir.join("installer.lock"))?;
    let url_file = data_dir.join("installer.url");
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            let url = fs::read_to_string(&url_file).context("设备助手已在运行")?;
            #[cfg(windows)]
            window::activate_existing();
            return open_url(url.trim());
        }
        Err(TryLockError::Error(e)) => return Err(e.into()),
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(8)
        .enable_all()
        .build()?;
    let listener = runtime.block_on(tokio::net::TcpListener::bind(("127.0.0.1", options.port)))?;
    let port = listener.local_addr()?.port();
    let token = hex::encode(rand::random::<[u8; 32]>());
    let app = server::App::new(token.clone(), port, adb, development, data_dir.clone());
    let url = format!("http://127.0.0.1:{port}/?k={token}");
    write_private(&url_file, &url)?;
    println!("SimpleAdmin 设备助手：{url}");
    if options.browser {
        open_url(&url)?;
    }
    let stop = app.clone();
    let server = runtime.spawn(async move {
        let result = axum::serve(listener, stop.router())
            .with_graceful_shutdown({
                let stop = stop.clone();
                async move { stop.shutdown.notified().await }
            })
            .await;
        #[cfg(windows)]
        window::server_stopped();
        result
    });
    let result = if options.window {
        #[cfg(windows)]
        window::run(app.clone(), url)?;
        // The window is gone: stop serving, but do not wait long for unfinished adb work.
        app.shutdown.notify_one();
        let _ = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), server).await
        });
        Ok(())
    } else {
        runtime.spawn(app.clone().watchdog());
        runtime.block_on(server)?.map_err(anyhow::Error::from)
    };
    let _ = fs::remove_file(&url_file);
    drop(lock);
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    result
}

/// Writes the launch URL (it contains the session token) readable by the current user only.
fn write_private(path: &Path, contents: &str) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options.open(path)?.write_all(contents.as_bytes())?;
    Ok(())
}

fn self_test(adb: &adb::Adb, development: &Path) -> Result<()> {
    for name in [
        "SHA256SUMS",
        "install_simpleadmin_rust.sh",
        "diagnose_simpleadmin.sh",
        "simpleadmin/simpleadmin-httpd.armv7",
    ] {
        anyhow::ensure!(development.join(name).is_file(), "内置安装资源缺失：{name}");
    }
    let output = adb.quiet(&["version"], std::time::Duration::from_secs(15))?;
    anyhow::ensure!(output.ok(), "adb 无法运行");
    println!("{}", output.text());
    Ok(())
}

fn spawn(program: &str, args: &[&str]) -> Result<()> {
    // Automated UI tests record the request instead of opening windows.
    if let Some(log) = std::env::var_os("SIMPLEADMIN_NO_OPEN") {
        use std::io::Write;
        let mut file = fs::OpenOptions::new().create(true).append(true).open(log)?;
        writeln!(file, "{program} {}", args.join(" "))?;
        return Ok(());
    }
    let mut command = Command::new(program);
    command.args(args);
    command
        .spawn()
        .with_context(|| format!("无法运行 {program}"))?;
    Ok(())
}
pub fn open_url(url: &str) -> Result<()> {
    if cfg!(windows) {
        spawn("rundll32.exe", &["url.dll,FileProtocolHandler", url])
    } else {
        spawn("xdg-open", &[url])
    }
}
pub fn open_text(path: &Path) -> Result<()> {
    let path = path.to_string_lossy();
    if cfg!(windows) {
        spawn("notepad.exe", &[&path])
    } else {
        spawn("xdg-open", &[&path])
    }
}
pub fn open_folder(path: &Path) -> Result<()> {
    let path = path.to_string_lossy();
    if cfg!(windows) {
        spawn("explorer.exe", &[&path])
    } else {
        spawn("xdg-open", &[&path])
    }
}

#[cfg(windows)]
fn fatal(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, caption) = (wide(message), wide("SimpleAdmin 设备助手"));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_ICONERROR,
        );
    }
}
#[cfg(not(windows))]
fn fatal(message: &str) {
    eprintln!("{message}");
}
