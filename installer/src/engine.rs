//! Install, diagnose and open-web flows over ADB. Each run selects one authorized device,
//! checks that it really is a supported module before touching it, and records every step in a
//! report file. Passwords travel over adb stdin only and never reach logs or command lines.

use crate::adb::{Adb, Output};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug)]
pub enum Operation {
    Install {
        http_port: Option<u16>,
        credentials: Option<String>,
    },
    Diagnose,
    OpenWeb,
}
impl Operation {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Install { .. } => "install",
            Self::Diagnose => "diagnose",
            Self::OpenWeb => "web",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Device,
    Upload,
    Install,
    Diagnose,
    Verify,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Outcome {
    pub ok: bool,
    pub error: Option<String>,
    /// Local ADB tunnel to the module's web page, kept open after install and open-web.
    pub url: Option<String>,
    pub http_port: Option<u16>,
    pub reboot: bool,
    pub report: Option<PathBuf>,
}

/// Receives progress while an operation runs.
pub trait Sink {
    fn line(&mut self, text: &str);
    fn stage(&mut self, stage: Stage);
}

const PREFLIGHT: &str = r#"echo SIMPLEADMIN_PREFLIGHT=1
echo SA_SYSTEM=$(uname -s)
echo SA_ARCH=$(uname -m)
echo SA_UID=$(id -u)
if [ -d /usrdata ] && { [ -c /dev/smd11 ] || [ -f /usrdata/etc/data/mobileap_cfg.xml ] || [ -f /etc/data/mobileap_cfg.xml ]; }; then echo SA_MODULE=1; else echo SA_MODULE=0; fi
if command -v bash >/dev/null 2>&1; then echo SA_BASH=1; else echo SA_BASH=0; fi
if [ -d /tmp ] && [ -w /tmp ]; then echo SA_TMP=1; else echo SA_TMP=0; fi
echo SIMPLEADMIN_PREFLIGHT_DONE=1
"#;
const CREDENTIALS: &str = "/tmp/development/install-credentials.json";

struct Run<'a> {
    adb: Adb,
    development: PathBuf,
    report: Option<std::fs::File>,
    sink: &'a mut dyn Sink,
}
impl Run<'_> {
    fn log(&mut self, text: &str) {
        self.sink.line(text);
        if let Some(file) = &mut self.report {
            let _ = writeln!(file, "{text}");
        }
    }
    /// Runs adb with output shown in the log (unless quiet) and saved to the report.
    fn adb(&mut self, args: &[&str], timeout: Duration, quiet: bool) -> Result<Output> {
        let adb = self.adb.clone();
        let mut lines = Vec::new();
        let output = adb.run(args, None, timeout, &mut |line| {
            if !quiet {
                lines.push(line.to_owned())
            }
        });
        for line in lines {
            self.log(&line)
        }
        output
    }
    fn require(&mut self, args: &[&str], timeout: Duration) -> Result<Output> {
        let output = self.adb(args, timeout, false)?;
        if !output.ok() {
            bail!("ADB 命令失败：{}（退出码 {}）", args[0], output.code)
        }
        Ok(output)
    }
    fn shell(&mut self, script: &str, timeout: Duration, quiet: bool) -> Result<Output> {
        self.adb(&["shell", script], timeout, quiet)
    }

    fn check_device(&mut self, operation: &Operation) -> Result<()> {
        self.log("正在检查所选设备的系统、模块特征、权限及临时目录（只读检查）。");
        let output = self.shell(PREFLIGHT, Duration::from_secs(30), false)?;
        let lines: Vec<&str> = output.lines.iter().map(|l| l.trim()).collect();
        let has = |value: &str| lines.contains(&value);
        if !output.ok() || !has("SIMPLEADMIN_PREFLIGHT_DONE=1") {
            bail!("无法读取所选设备的系统信息，请检查 ADB 连接并重新选择模块。")
        }
        if !has("SA_SYSTEM=Linux") || !has("SA_ARCH=armv7l") || !has("SA_MODULE=1") {
            bail!(
                "所选 ADB 设备未通过模块兼容性检查。请连接 Quectel 模块，排除手机、模拟器或其他 ADB 设备；若通过端口转发连接，请确认转发目标是模块本身。"
            )
        }
        if !matches!(operation, Operation::OpenWeb) {
            if !has("SA_UID=0") {
                bail!(
                    "当前 ADB 没有 root 权限，无法安装或诊断模块。请使用模块提供的 root ADB 连接。"
                )
            }
            if !has("SA_BASH=1") {
                bail!("模块固件缺少 Bash，当前安装器无法在此固件上运行。")
            }
            if !has("SA_TMP=1") {
                bail!(
                    "模块的 /tmp 不存在或不可写，尚未上传任何文件。请检查固件的临时目录挂载状态；根目录只读本身是正常的，不要直接解除根目录只读来绕过此检查。"
                )
            }
        }
        Ok(())
    }

    fn write_credentials(&mut self, json: &str) -> Result<()> {
        let adb = self.adb.clone();
        let output = adb
            .run(
                &[
                    "exec-in",
                    "sh",
                    "-c",
                    &format!("umask 077; cat > {CREDENTIALS}"),
                ],
                Some(json.as_bytes()),
                Duration::from_secs(15),
                &mut |_| {},
            )
            .map_err(|_| anyhow::anyhow!("安装凭据传输超时，尚未安装。"))?;
        if !output.ok() {
            bail!("安装凭据传输失败，尚未安装。")
        }
        let expected = hex::encode(Sha256::digest(json.as_bytes()));
        for _ in 0..10 {
            let digest = self.shell(
                &format!("sha256sum {CREDENTIALS}"),
                Duration::from_secs(15),
                true,
            )?;
            if digest.ok() && digest.text().trim().starts_with(&format!("{expected} ")) {
                self.log("账号密码已传入模块临时内存并校验；不写入命令行和日志。");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        bail!("安装凭据完整性校验失败，尚未安装。")
    }

    fn read_http_port(&mut self) -> Result<u16> {
        let output = self.shell(
            "if [ -e /usrdata/simpleadmin/http_port ]; then cat /usrdata/simpleadmin/http_port; else echo 80; fi",
            Duration::from_secs(15),
            true,
        )?;
        let value = output.text().trim().to_owned();
        let port = parse_port(&value)
            .filter(|_| output.ok())
            .context("无法读取模块的 HTTP 端口配置，请在安装器中勾选修改端口后重新安装。")?;
        self.log(&format!("模块 HTTP 端口：{port}"));
        Ok(port)
    }

    /// Checks the login page, app shell and language script through a fresh ADB tunnel.
    fn probe_web(&mut self, keep: bool, outcome: &mut Outcome) -> Result<()> {
        let device_port = self.read_http_port()?;
        outcome.http_port = Some(device_port);
        let target = format!("tcp:{device_port}");
        let forward = self.adb(
            &["forward", "tcp:0", &target],
            Duration::from_secs(15),
            true,
        )?;
        let local = forward
            .lines
            .iter()
            .rev()
            .find_map(|l| l.trim().parse::<u16>().ok())
            .filter(|_| forward.ok())
            .context("无法建立 ADB 网页通道。")?;
        let checked = (|| -> Result<()> {
            for (path, marker) in [
                ("/", "SimpleAdminSpaMode"),
                ("/login.html", "loginLanguage"),
                ("/js/locales.js", "root.Lang"),
            ] {
                let page = http_get(local, path)?;
                if path == "/"
                    && page.status == 303
                    && page.location.as_deref() == Some("/login.html")
                {
                    continue;
                }
                if page.status != 200 || !page.body.contains(marker) {
                    bail!(
                        "网页 {path} 返回内容不符合 SimpleAdmin（HTTP {}）。",
                        page.status
                    )
                }
            }
            Ok(())
        })();
        if checked.is_err() || !keep {
            let _ = self.adb(
                &["forward", "--remove", &format!("tcp:{local}")],
                Duration::from_secs(15),
                true,
            );
        }
        checked?;
        self.log("PC_HTTP_CHECK=OK（ADB 通道；局域网访问需另行检查）");
        if keep {
            let url = format!("http://127.0.0.1:{local}/");
            self.log(&format!("本机访问地址：{url}（保持 ADB 连接期间可用）"));
            outcome.url = Some(url);
        }
        Ok(())
    }

    fn diagnose(&mut self) -> Result<()> {
        self.log("正在收集诊断信息：不登录、不读取短信、不执行 AT 指令、不修改配置。");
        let source = self.development.join("diagnose_simpleadmin.sh");
        let remote = format!("/tmp/simpleadmin-diagnose-{}.sh", std::process::id());
        let source = source.to_string_lossy().into_owned();
        let result = self
            .require(&["push", &source, &remote], Duration::from_secs(60))
            .and_then(|_| {
                self.require(
                    &["shell", &format!("bash {remote}")],
                    Duration::from_secs(180),
                )
            });
        let _ = self.shell(&format!("rm -f {remote}"), Duration::from_secs(15), true);
        result.map(|_| ())
    }

    fn install(
        &mut self,
        http_port: Option<u16>,
        credentials: Option<&str>,
        outcome: &mut Outcome,
    ) -> Result<()> {
        if !self.development.join("SHA256SUMS").is_file() {
            bail!("安装文件校验清单缺失，请重新下载设备助手。")
        }
        self.sink.stage(Stage::Upload);
        self.require(
            &[
                "shell",
                "rm -rf /tmp/development; rm -f /tmp/simpleadmin-install-result.env",
            ],
            Duration::from_secs(30),
        )?;
        let source = self.development.to_string_lossy().into_owned();
        self.require(
            &["push", &source, "/tmp/development"],
            Duration::from_secs(300),
        )?;
        if let Some(json) = credentials {
            self.write_credentials(json)?;
        }
        self.sink.stage(Stage::Install);
        let mut command = "bash /tmp/development/install_simpleadmin_rust.sh".to_owned();
        if let Some(port) = http_port {
            command = format!("SIMPLEADMIN_HTTP_PORT={port} {command}");
        }
        let install = self.shell(&command, Duration::from_secs(600), false)?;
        let result = self.shell(
            "cat /tmp/simpleadmin-install-result.env",
            Duration::from_secs(30),
            false,
        )?;
        let lines: Vec<&str> = result.lines.iter().map(|l| l.trim()).collect();
        if !install.ok() || !result.ok() || !lines.contains(&"INSTALL_STATUS=OK") {
            bail!("安装未通过检查，请查看日志中的错误和报告中的诊断信息。")
        }
        self.sink.stage(Stage::Verify);
        self.probe_web(true, outcome)?;
        let _ = self.shell("ip -4 addr show", Duration::from_secs(15), false);
        self.log("安装和网页检查通过。请使用模块可达的 IPv4 地址和上方显示的 HTTP 端口访问。");
        if lines.contains(&"REBOOT_REQUIRED=1") {
            self.log("网络配置已变化，请手动重启模块后检查当前 IP。");
            outcome.reboot = true;
        }
        Ok(())
    }
}

pub fn parse_port(value: &str) -> Option<u16> {
    let port: u16 = value.parse().ok()?;
    (port > 0 && port.to_string() == value).then_some(port)
}

pub struct Page {
    pub status: u16,
    pub location: Option<String>,
    pub body: String,
}
/// Minimal HTTP/1.1 GET over the local ADB tunnel (no proxies, no redirects).
pub fn http_get(port: u16, path: &str) -> Result<Page> {
    let mut stream = TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(5),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nUser-Agent: SimpleAdmin-Setup\r\n\r\n"
    )?;
    let mut raw = Vec::new();
    stream.take(4 << 20).read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").context("HTTP 响应不完整。")?;
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .context("HTTP 状态行无效。")?;
    let location = lines.find_map(|l| {
        let (name, value) = l.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("location")
            .then(|| value.trim().to_owned())
    });
    Ok(Page {
        status,
        location,
        body: body.to_owned(),
    })
}

/// Runs one operation against `serial` and returns the result. Never panics on device errors.
pub fn run(
    operation: &Operation,
    adb: &Adb,
    development: &Path,
    serial: &str,
    reports: &Path,
    sink: &mut dyn Sink,
) -> Outcome {
    let report_path = reports.join(format!(
        "simpleadmin-{}-{}.txt",
        chrono_stamp(),
        std::process::id()
    ));
    let report = std::fs::create_dir_all(reports)
        .and_then(|_| std::fs::File::create(&report_path))
        .ok();
    let mut outcome = Outcome {
        report: report.as_ref().map(|_| report_path.clone()),
        ..Outcome::default()
    };
    let mut run = Run {
        adb: adb.with_serial(serial),
        development: development.to_owned(),
        report,
        sink,
    };
    run.sink.stage(Stage::Device);
    run.log(&format!(
        "SimpleAdmin 设备助手 {} · {}",
        env!("CARGO_PKG_VERSION"),
        match operation {
            Operation::Install { .. } => "安装 / 升级",
            Operation::Diagnose => "故障诊断",
            Operation::OpenWeb => "打开管理页面",
        }
    ));
    let mut device_checked = false;
    let mut staged = false;
    let result = (|| -> Result<()> {
        let devices = Adb::new(&adb.path).quiet(&["devices"], Duration::from_secs(15))?;
        if !devices.ok() {
            bail!("ADB 设备列表读取失败。")
        }
        let ready = devices.lines.iter().any(|l| {
            let mut fields = l.split_whitespace();
            fields.next() == Some(serial)
                && fields.next() == Some("device")
                && fields.next().is_none()
        });
        if !ready {
            bail!("所选设备未连接或尚未授权 ADB，请刷新设备列表后重新选择。")
        }
        run.log(&format!("已选择设备：{serial}"));
        run.check_device(operation)?;
        device_checked = true;
        match operation {
            Operation::OpenWeb => {
                run.sink.stage(Stage::Verify);
                run.probe_web(true, &mut outcome)
            }
            Operation::Diagnose => {
                run.sink.stage(Stage::Diagnose);
                run.diagnose()?;
                run.sink.stage(Stage::Verify);
                run.probe_web(false, &mut outcome)
            }
            Operation::Install {
                http_port,
                credentials,
            } => {
                staged = true;
                run.install(*http_port, credentials.as_deref(), &mut outcome)
            }
        }
    })();
    match result {
        Ok(()) => outcome.ok = true,
        Err(error) => {
            let message = error.to_string();
            run.log(&format!("错误：{message}"));
            outcome.error = Some(message);
            if device_checked
                && matches!(operation, Operation::Install { .. })
                && let Err(e) = run.diagnose()
            {
                run.log(&format!("诊断未完成：{e}"));
            }
        }
    }
    if staged {
        let _ = run.shell("rm -rf /tmp/development", Duration::from_secs(30), true);
    }
    if let Some(path) = &outcome.report {
        run.log(&format!("报告已保存：{}", path.display()));
    }
    outcome
}

fn chrono_stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
