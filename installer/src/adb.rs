//! Runs the bundled adb client without console windows and streams its output line by line.

use anyhow::{Context, Result, bail};
use regex::Regex;
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{LazyLock, mpsc},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Device {
    pub serial: String,
    pub state: String,
    pub model: String,
}
impl Device {
    pub fn ready(&self) -> bool {
        self.state == "device"
    }
}

static DEVICE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\S+)\s+(device|unauthorized|offline)(?:\s|$)").unwrap());
static MODEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bmodel:(\S+)").unwrap());
/// Parses `adb devices -l`.
pub fn parse_devices(output: &str) -> Vec<Device> {
    output
        .lines()
        .filter_map(|line| {
            let caps = DEVICE_LINE.captures(line.trim())?;
            Some(Device {
                serial: caps[1].to_owned(),
                state: caps[2].to_owned(),
                model: MODEL
                    .captures(line)
                    .map(|m| m[1].replace('_', " "))
                    .unwrap_or_default(),
            })
        })
        .collect()
}

pub struct Output {
    pub code: i32,
    pub lines: Vec<String>,
}
impl Output {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

#[derive(Clone)]
pub struct Adb {
    pub path: PathBuf,
    pub serial: Option<String>,
}
impl Adb {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            serial: None,
        }
    }
    pub fn with_serial(&self, serial: &str) -> Self {
        Self {
            path: self.path.clone(),
            serial: Some(serial.to_owned()),
        }
    }
    /// Runs one adb command. Output lines are passed to `line` as they arrive (stdout and
    /// stderr interleaved), so long installs show progress before the process exits.
    pub fn run(
        &self,
        args: &[&str],
        stdin: Option<&[u8]>,
        timeout: Duration,
        line: &mut dyn FnMut(&str),
    ) -> Result<Output> {
        let mut command = Command::new(&self.path);
        if let Some(serial) = &self.serial {
            command.args(["-s", serial]);
        }
        command
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = self.path.parent() {
            command.current_dir(dir);
        }
        hide_window(&mut command);
        let mut child = command
            .spawn()
            .with_context(|| format!("无法运行 {}", self.path.display()))?;
        let (sender, receiver) = mpsc::channel::<String>();
        let mut readers = Vec::new();
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let sender = sender.clone();
            readers.push(std::thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                let mut buffer = Vec::new();
                while matches!(reader.read_until(b'\n', &mut buffer), Ok(n) if n > 0) {
                    let text = String::from_utf8_lossy(&buffer);
                    let _ = sender.send(text.trim_end_matches(['\r', '\n']).to_owned());
                    buffer.clear();
                }
            }));
        }
        drop(sender);
        if let Some(data) = stdin {
            let mut input = child.stdin.take().context("adb stdin unavailable")?;
            input.write_all(data)?;
            drop(input);
        }
        let deadline = Instant::now() + timeout;
        let mut lines = Vec::new();
        let mut collected = 0usize;
        let code = loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(text) => {
                    line(&text);
                    // Keep at most 1 MiB of output for result parsing.
                    if collected < 1 << 20 {
                        collected += text.len();
                        lines.push(text);
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {}
            }
            if let Some(status) = child.try_wait()? {
                // Drain what the readers still hold after the process exits.
                for reader in readers.drain(..) {
                    let _ = reader.join();
                }
                for text in receiver.try_iter() {
                    line(&text);
                    lines.push(text);
                }
                break status.code().unwrap_or(-1);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("ADB 操作超时，请检查 USB 连接后重试。")
            }
        };
        Ok(Output { code, lines })
    }
    pub fn quiet(&self, args: &[&str], timeout: Duration) -> Result<Output> {
        self.run(args, None, timeout, &mut |_| {})
    }
    pub fn devices(&self) -> Result<Vec<Device>> {
        let output = Self::new(&self.path).quiet(&["devices", "-l"], Duration::from_secs(10))?;
        if !output.ok() {
            bail!("设备检测失败或超时。请检查 USB 和 ADB 驱动，再点击刷新。")
        }
        Ok(parse_devices(&output.text()))
    }
}

#[cfg(windows)]
pub fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
pub fn hide_window(_: &mut Command) {}

/// adb.exe next to the extracted payload, or `adb` from PATH on development hosts.
pub fn locate(root: &Path) -> PathBuf {
    let bundled = root.join(if cfg!(windows) { "adb.exe" } else { "adb" });
    if bundled.is_file() {
        bundled
    } else {
        PathBuf::from("adb")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_devices_with_models_and_states() {
        let devices = parse_devices(
            "List of devices attached\nabc device usb:1-1 product:x model:RM520N_EU device:y\nbad unauthorized\noff offline\n\n* daemon started *\n",
        );
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].model, "RM520N EU");
        assert!(devices[0].ready());
        assert_eq!(devices[1].state, "unauthorized");
        assert!(!devices[2].ready());
    }
}
