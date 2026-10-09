//! Serial AT preparation for Quectel Qualcomm modules: identification, ADB unlock through the
//! USB configuration, network profiles and read-only diagnostics. Every write is previewed and
//! confirmed in the page before it is sent, and nothing here computes or sends ADB keys.

use anyhow::{Context, Result, bail};
use regex::Regex;
use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// Shared stop flag checked between commands and while waiting for a reply.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);
impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst)
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn check(&self) -> Result<()> {
        if self.cancelled() {
            bail!(Cancelled)
        }
        Ok(())
    }
}
/// Error marker for a user-requested stop.
#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("已停止后续操作")
    }
}
impl std::error::Error for Cancelled {}

pub struct AtReply {
    pub text: String,
    pub ok: bool,
}
pub fn is_terminal(line: &str) -> bool {
    line == "OK"
        || line == "ERROR"
        || line.starts_with("+CME ERROR:")
        || line.starts_with("+CMS ERROR:")
}

pub trait AtLink {
    fn send(&mut self, command: &str, cancel: &Cancel) -> Result<AtReply>;
}

/// One AT serial port session at 115200 8N1 with DTR/RTS asserted.
pub struct SerialAtLink {
    port: Box<dyn serialport::SerialPort>,
    failed: bool,
}
impl SerialAtLink {
    pub fn open(name: &str) -> Result<Self> {
        if !valid_port_name(name) {
            bail!("串口名称无效。")
        }
        let mut port = serialport::new(name, 115200)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .timeout(Duration::from_millis(20))
            .open()
            .with_context(|| format!("无法打开 {name}，请关闭占用串口的其他软件后重试"))?;
        // Pseudo-terminals used in tests have no modem control lines.
        let lines = port
            .write_data_terminal_ready(true)
            .and_then(|_| port.write_request_to_send(true));
        if cfg!(windows) {
            lines?;
        }
        // USB AT drivers may release queued replies when DTR/RTS are asserted. Wait for quiet
        // before the first transaction so an old OK is never paired with a new query.
        let started = Instant::now();
        let mut last_data = Duration::ZERO;
        let mut buffer = [0u8; 1024];
        while started.elapsed() < Duration::from_secs(2) {
            match port.read(&mut buffer) {
                Ok(n) if n > 0 => last_data = started.elapsed(),
                _ => {}
            }
            if started.elapsed() - last_data >= Duration::from_millis(200) {
                break;
            }
        }
        if started.elapsed() - last_data < Duration::from_millis(200) {
            bail!("串口持续有积压响应，请关闭其他串口工具后重试。")
        }
        port.clear(serialport::ClearBuffer::Input)?;
        Ok(Self {
            port,
            failed: false,
        })
    }
    fn exchange(&mut self, command: &str, cancel: &Cancel) -> Result<AtReply> {
        self.port.clear(serialport::ClearBuffer::Input)?;
        self.port.write_all(format!("{command}\r").as_bytes())?;
        let started = Instant::now();
        let mut output = String::new();
        let mut line = Vec::new();
        let mut buffer = [0u8; 512];
        while started.elapsed() < Duration::from_secs(8) {
            cancel.check()?;
            let n = match self.port.read(&mut buffer) {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => 0,
                Err(e) => return Err(e.into()),
            };
            for &byte in &buffer[..n] {
                if byte == b'\r' || byte == b'\n' {
                    let value = String::from_utf8_lossy(&line).trim().to_owned();
                    line.clear();
                    if value.is_empty() || value.eq_ignore_ascii_case(command) {
                        continue;
                    }
                    output.push_str(&value);
                    output.push('\n');
                    if is_terminal(&value) {
                        return Ok(AtReply {
                            ok: value == "OK",
                            text: output,
                        });
                    }
                } else {
                    line.push(byte);
                }
                if output.len() + line.len() > 32768 {
                    bail!("串口响应过长，已停止接收。")
                }
            }
        }
        bail!("模块未在 8 秒内返回完整结果。请确认选择的是 AT 串口，并关闭其他占用串口的软件。")
    }
}
impl AtLink for SerialAtLink {
    fn send(&mut self, command: &str, cancel: &Cancel) -> Result<AtReply> {
        validate_command(command)?;
        if self.failed {
            bail!("上一条指令未完成，请重新识别模块后再试。")
        }
        cancel.check()?;
        let result = self.exchange(command, cancel);
        // A half-read reply would desynchronize every later command on this session.
        if result.is_err() {
            self.failed = true;
        }
        result
    }
}
fn valid_port_name(name: &str) -> bool {
    if cfg!(windows) {
        static COM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^COM[1-9][0-9]*$").unwrap());
        COM.is_match(name)
    } else {
        (name.starts_with("/dev/tty") || name.starts_with("/dev/pts/")) && !name.contains("..")
    }
}

#[derive(Clone, serde::Serialize)]
pub struct AtPort {
    pub name: String,
    pub description: String,
    pub quectel: bool,
}
/// Serial ports with Quectel (VID 2C7C) interfaces labelled and listed first.
pub fn list_ports() -> Result<Vec<AtPort>> {
    let mut ports: Vec<AtPort> = serialport::available_ports()
        .context("串口列表读取失败")?
        .into_iter()
        .filter(|p| valid_port_name(&p.port_name))
        .map(|p| {
            let (description, quectel) = match p.port_type {
                serialport::SerialPortType::UsbPort(usb) if usb.vid == 0x2C7C => (
                    usb.product.unwrap_or_else(|| "Quectel USB 串口".into()),
                    true,
                ),
                serialport::SerialPortType::UsbPort(usb) => {
                    (usb.product.unwrap_or_default(), false)
                }
                _ => (String::new(), false),
            };
            AtPort {
                name: p.port_name,
                description,
                quectel,
            }
        })
        .collect();
    let number = |name: &str| {
        name.trim_start_matches(|c: char| !c.is_ascii_digit())
            .parse::<u32>()
            .unwrap_or(u32::MAX)
    };
    #[cfg(not(windows))]
    if let Ok(extra) = std::env::var("SIMPLEADMIN_TEST_PORTS") {
        ports.extend(
            extra
                .split(',')
                .filter(|p| valid_port_name(p))
                .map(|p| AtPort {
                    name: p.into(),
                    description: "Quectel USB AT Port（模拟设备）".into(),
                    quectel: true,
                }),
        );
    }
    ports.sort_by_key(|p| (!p.quectel, number(&p.name), p.name.clone()));
    ports.dedup_by(|a, b| a.name == b.name);
    Ok(ports)
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ModuleIdentity {
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    pub imei: String,
}
static SUPPORTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(RM500Q|RM502Q|RM520N|RM521F|RG500Q|RG502Q|RG520N|RG520F|RG521F)([-\s]|$)")
        .unwrap()
});
impl ModuleIdentity {
    pub fn supported(&self) -> bool {
        self.manufacturer.to_ascii_uppercase().contains("QUECTEL")
            && SUPPORTED.is_match(&self.model)
    }
    pub fn same_device(&self, other: &ModuleIdentity) -> bool {
        self.manufacturer == other.manufacturer
            && self.model == other.model
            && self.imei == other.imei
            && !self.imei.is_empty()
    }
    pub fn masked_imei(&self) -> String {
        if self.imei.len() == 15 {
            format!("•••••••••••{}", &self.imei[11..])
        } else {
            "未读到，请重新识别".into()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsbProfile {
    pub fields: Vec<String>,
}
fn usb_id(value: &str) -> Option<u32> {
    let (digits, radix, max_len) = match value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Some(hex) => (hex, 16, 4),
        None => (value, 10, 5),
    };
    if digits.is_empty() || digits.len() > max_len || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    u32::from_str_radix(digits, radix)
        .ok()
        .filter(|id| *id <= 65535)
}
static USBCFG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?im)(?:\A|[\r\n])\s*\+QCFG:\s*"usbcfg"\s*,([^\r\n]+)"#).unwrap()
});
impl UsbProfile {
    pub fn adb(&self) -> u8 {
        self.fields[self.fields.len() - 2].parse().unwrap_or(0)
    }
    pub fn adb_enabled(&self) -> bool {
        matches!(self.adb(), 1 | 2)
    }
    /// The same configuration with only the ADB interface (second to last field) set to 2.
    pub fn enable_adb_command(&self) -> String {
        let mut fields = self.fields.clone();
        let index = fields.len() - 2;
        fields[index] = "2".into();
        format!("AT+QCFG=\"usbcfg\",{}", fields.join(","))
    }
    pub fn parse(response: &str) -> Result<Self> {
        let matches: Vec<_> = USBCFG.captures_iter(response).collect();
        if matches.len() != 1 {
            bail!(
                "未收到唯一完整的 USB 配置响应，请在操作记录中查看原始返回值后重新检查。未发送解锁指令。"
            )
        }
        let fields: Vec<String> = matches[0][1]
            .split(',')
            .map(|s| s.trim().to_owned())
            .collect();
        if fields.len() != 9 {
            bail!(
                "USB 配置返回 {} 个字段，当前需要 VID、PID 和 7 个接口参数；未修改配置。",
                fields.len()
            )
        }
        if usb_id(&fields[0]) != Some(0x2C7C) {
            bail!("USB VID 不是移远 2C7C，未修改配置。")
        }
        if usb_id(&fields[1]).is_none() {
            bail!("USB PID 不是有效的 16 位编号，未修改配置。")
        }
        if fields[2..]
            .iter()
            .any(|s| !matches!(s.as_str(), "0" | "1" | "2"))
        {
            bail!("USB 接口参数不在 0/1/2 范围内，未修改配置。")
        }
        Ok(Self { fields })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkProfile {
    Pcie,
    Ecm,
    Rndis,
}
impl NetworkProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pcie => "PCIe 转网口",
            Self::Ecm => "ECM",
            Self::Rndis => "RNDIS",
        }
    }
}

pub const INFO_COMMANDS: [(&str, &str); 31] = [
    ("AT+CPIN?", "SIM 状态"),
    ("AT+CFUN?", "模块功能"),
    ("AT+QTEMP", "温度传感器"),
    ("AT+QUIMSLOT?", "SIM 卡槽"),
    ("AT+QSIMDET?", "SIM 检测电平"),
    ("AT+QSIMSTAT?", "SIM 插入检测"),
    ("AT+QCFG=\"usbcfg\"", "USB 接口"),
    ("AT+QCFG=\"pcie/mode\"", "PCIe 模式"),
    ("AT+QCFG=\"data_interface\"", "数据接口"),
    ("AT+QCFG=\"usbnet\"", "USB 网卡模式"),
    ("AT+QETH=\"eth_driver\"", "以太网驱动"),
    ("AT+QMAP=\"WWAN\"", "移动网络 IP"),
    ("AT+QMAP=\"LANIP\"", "局域网 IP"),
    ("AT+QMAP=\"MPDN_rule\"", "MPDN 规则"),
    ("AT+CGDCONT?", "APN 配置"),
    ("AT+QSPN", "运营商"),
    ("AT+QNWINFO", "当前网络"),
    ("AT+QENG=\"servingcell\"", "服务小区"),
    ("AT+QCAINFO", "载波聚合"),
    ("AT+QRSRP", "RSRP"),
    ("AT+QRSRQ", "RSRQ"),
    ("AT+QSINR", "SINR"),
    ("AT+CSQ", "CSQ"),
    ("AT+QNWPREFCFG=\"mode_pref\"", "网络偏好"),
    ("AT+QNWPREFCFG=\"nr5g_disable_mode\"", "5G 模式限制"),
    ("AT+QNWPREFCFG=\"lte_band\"", "LTE 频段"),
    ("AT+QNWPREFCFG=\"nsa_nr5g_band\"", "NSA 频段"),
    ("AT+QNWPREFCFG=\"nr5g_band\"", "SA 频段"),
    ("AT+QNWLOCK=\"common/4g\"", "4G 小区锁定"),
    ("AT+QNWLOCK=\"common/5g\"", "5G 小区锁定"),
    ("AT+QMAPWAC?", "自动拨号"),
];

pub fn validate_command(command: &str) -> Result<()> {
    if command.trim().is_empty()
        || command.len() > 512
        || !command
            .get(..2)
            .is_some_and(|p| p.eq_ignore_ascii_case("AT"))
        || command.chars().any(|c| !(' '..='~').contains(&c))
        || command.contains(';')
    {
        bail!("每行只填一条 AT 指令（最多 512 个字符），不使用分号拼接或控制字符。")
    }
    Ok(())
}
/// Reply lines without the echo and final result code.
pub fn body(reply: &AtReply) -> String {
    reply
        .text
        .split('\n')
        .map(str::trim)
        .filter(|s| {
            !s.is_empty()
                && !is_terminal(s)
                && !s.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("AT"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn require(link: &mut dyn AtLink, command: &str, cancel: &Cancel) -> Result<String> {
    let reply = link.send(command, cancel)?;
    if !reply.ok {
        bail!("{command} 返回错误：{}", reply.text.trim())
    }
    Ok(body(&reply))
}
static IMEI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\D)(\d{15})(?:\D|$)").unwrap());
pub fn identify(link: &mut dyn AtLink, cancel: &Cancel) -> Result<ModuleIdentity> {
    require(link, "AT", cancel)?;
    let manufacturer = require(link, "AT+CGMI", cancel)?;
    let model = require(link, "AT+GMM", cancel)?
        .replace("+GMM:", "")
        .trim()
        .trim_matches('"')
        .to_owned();
    let firmware = require(link, "AT+GMR", cancel)?;
    let imei = IMEI
        .captures(&require(link, "AT+CGSN", cancel)?)
        .map(|c| c[1].to_owned())
        .unwrap_or_default();
    Ok(ModuleIdentity {
        manufacturer,
        model,
        firmware,
        imei,
    })
}
pub fn verify_identity(
    link: &mut dyn AtLink,
    expected: &ModuleIdentity,
    cancel: &Cancel,
) -> Result<()> {
    let current = identify(link, cancel)?;
    if !current.supported() || !current.same_device(expected) {
        bail!("串口上的模块与刚才识别的不一致或型号未适配，请重新识别；未执行配置修改。")
    }
    Ok(())
}
/// Sets the ADB interface to 2 when it is still 0. Returns false when ADB is already on.
pub fn apply_adb(link: &mut dyn AtLink, previewed: &UsbProfile, cancel: &Cancel) -> Result<bool> {
    cancel.check()?;
    let latest = UsbProfile::parse(&require(link, "AT+QCFG=\"usbcfg\"", cancel)?)?;
    if latest.adb_enabled() {
        return Ok(false);
    }
    if latest != *previewed {
        bail!("USB 配置已变化，请重新检查；未修改配置。")
    }
    let command = previewed.enable_adb_command();
    require(link, &command, cancel)?;
    let verified = UsbProfile::parse(&require(link, "AT+QCFG=\"usbcfg\"", cancel)?)?;
    if verified.adb() != 2 || verified.enable_adb_command() != command {
        bail!("USB 配置复查不一致，请重新识别模块；不要重复发送指令。")
    }
    Ok(true)
}
static BLOCKED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^AT\+(?:CMGS|CMSS|CMGW|CMGD|CMGC|QCMGS|QCMGD|QADBKEY)\b").unwrap()
});
pub fn parse_custom(text: &str) -> Result<Vec<String>> {
    let commands: Vec<String> = text
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if commands.is_empty() || commands.len() > 32 {
        bail!("请输入 1–32 条 AT 指令，每行一条。")
    }
    for command in &commands {
        validate_command(command)?;
        if BLOCKED.is_match(command) {
            bail!(
                "此工具不提供短信发送/删除或 ADB 密钥操作，开启 ADB 请使用“连接模块”中的“开启 ADB”。"
            )
        }
    }
    Ok(commands)
}
static MPDN_ZERO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?im)(?:\A|[\r\n])\s*\+QMAP:\s*"MPDN_RULE"\s*,\s*0\s*,([^\r\n]+)"#).unwrap()
});
pub fn has_mpdn_rule_zero(response: &str) -> Result<bool> {
    let matches: Vec<_> = MPDN_ZERO.captures_iter(response).collect();
    if matches.is_empty() {
        if response.trim().is_empty() || response.to_ascii_uppercase().contains("+QMAP:") {
            return Ok(false);
        }
        bail!("未能识别 MPDN 规则查询结果，未修改配置。")
    }
    if matches.len() != 1 {
        bail!("MPDN 规则 0 返回多次，未修改配置。")
    }
    let fields: Vec<&str> = matches[0][1].split(',').map(str::trim).collect();
    if fields.len() < 4 || fields[0].is_empty() || !fields[0].bytes().all(|b| b.is_ascii_digit()) {
        bail!("MPDN 规则 0 格式无效，未修改配置。")
    }
    Ok(fields[0] != "0")
}
pub fn ethernet_plan(
    driver: Option<&str>,
    profile: NetworkProfile,
    enable: bool,
    has_rule_zero: bool,
) -> Result<Vec<String>> {
    let mut commands = Vec::new();
    if has_rule_zero {
        commands.push("AT+QMAP=\"MPDN_RULE\",0".into());
    }
    match profile {
        NetworkProfile::Pcie => {
            let driver = driver
                .filter(|d| matches!(*d, "r8125" | "r8168"))
                .context("请选择网卡型号。")?;
            if enable {
                commands.push("AT+QCFG=\"data_interface\",1,0".into());
                commands.push("AT+QCFG=\"pcie/mode\",1".into());
            }
            commands.push(format!(
                "AT+QETH=\"eth_driver\",\"{driver}\",{}",
                u8::from(enable)
            ));
        }
        NetworkProfile::Ecm | NetworkProfile::Rndis if enable => {
            commands.push("AT+QCFG=\"data_interface\",0,0".into());
            commands.push(format!(
                "AT+QCFG=\"usbnet\",{}",
                if profile == NetworkProfile::Ecm { 1 } else { 3 }
            ));
        }
        _ => {}
    }
    commands.push(format!("AT+QMAPWAC={}", u8::from(enable)));
    Ok(commands)
}
pub fn execute_plan(
    link: &mut dyn AtLink,
    commands: &[String],
    cancel: &Cancel,
    progress: &mut dyn FnMut(String),
) -> Result<()> {
    for (done, command) in commands.iter().enumerate() {
        cancel.check()?;
        progress(format!("发送：{command}"));
        match require(link, command, cancel) {
            Ok(result) => {
                if !result.is_empty() {
                    progress(result)
                }
            }
            Err(e) if e.is::<Cancelled>() => return Err(e),
            Err(e) => bail!("已完成 {done} 条，后续已停止。已执行的配置不会自动回滚。{e}"),
        }
        progress(format!("已确认：{command}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const QUERY: &str = "AT+QCFG=\"usbcfg\"";
    const CLOSED: &str = "+QCFG: \"usbcfg\",0x2C7C,0x0801,1,0,1,1,0,0,1";
    fn profile(adb: u8) -> String {
        format!("{}{adb},1", &CLOSED[..CLOSED.len() - 3])
    }
    #[derive(Default)]
    struct Link(VecDeque<(String, String, bool)>);
    impl Link {
        fn add(mut self, command: &str, response: &str) -> Self {
            self.0.push_back((command.into(), response.into(), true));
            self
        }
        fn fail(mut self, command: &str) -> Self {
            self.0.push_back((command.into(), String::new(), false));
            self
        }
        fn done(&self) {
            assert!(self.0.is_empty(), "unsent: {:?}", self.0)
        }
    }
    impl AtLink for Link {
        fn send(&mut self, command: &str, cancel: &Cancel) -> Result<AtReply> {
            cancel.check()?;
            let (expected, response, ok) = self
                .0
                .pop_front()
                .unwrap_or_else(|| panic!("unexpected command {command}"));
            assert_eq!(command, expected);
            Ok(AtReply {
                ok,
                text: format!("{response}\r\n{}\r\n", if ok { "OK" } else { "ERROR" }),
            })
        }
    }
    fn identity(imei: &str) -> Link {
        Link::default()
            .add("AT", "")
            .add("AT+CGMI", "Quectel")
            .add("AT+GMM", "RM520N-EU")
            .add("AT+GMR", "test")
            .add("AT+CGSN", imei)
    }
    fn none() -> Cancel {
        Cancel::default()
    }

    #[test]
    fn usb_field_preservation() {
        let original = UsbProfile::parse(CLOSED).unwrap();
        assert!(!original.adb_enabled());
        assert_eq!(
            original.enable_adb_command(),
            "AT+QCFG=\"usbcfg\",0x2C7C,0x0801,1,0,1,1,0,2,1"
        );
        assert_eq!(
            UsbProfile::parse(&CLOSED.replace("0801", "0800"))
                .unwrap()
                .fields[1],
            "0x0800"
        );
    }
    #[test]
    fn enabled_adb_skips_every_write() {
        for adb in [1, 2] {
            let mut link = Link::default().add(QUERY, &profile(adb));
            assert!(UsbProfile::parse(&profile(adb)).unwrap().adb_enabled());
            let previewed = UsbProfile::parse(&profile(adb)).unwrap();
            assert!(!apply_adb(&mut link, &previewed, &none()).unwrap());
            link.done();
        }
        // ADB switched on while the confirmation dialog was open.
        let mut link = Link::default().add(QUERY, &profile(2));
        assert!(!apply_adb(&mut link, &UsbProfile::parse(CLOSED).unwrap(), &none()).unwrap());
        link.done();
    }
    #[test]
    fn unknown_usb_profiles_rejected() {
        for response in [
            CLOSED.replace("0801", "100000"),
            CLOSED.replace("2C7C", "1234"),
            format!("{CLOSED},1"),
            profile(3),
            "ERROR".into(),
        ] {
            assert!(UsbProfile::parse(&response).is_err(), "{response}");
        }
    }
    #[test]
    fn direct_adb_write_and_failures() {
        let previewed = UsbProfile::parse(CLOSED).unwrap();
        let setter = previewed.enable_adb_command();
        let mut link = Link::default()
            .add(QUERY, CLOSED)
            .add(&setter, "")
            .add(QUERY, &profile(2));
        assert!(apply_adb(&mut link, &previewed, &none()).unwrap());
        link.done();
        // A changed USB profile never sends the setter.
        let mut link = Link::default().add(QUERY, &CLOSED.replace("0801", "0800"));
        assert!(apply_adb(&mut link, &previewed, &none()).is_err());
        link.done();
        // A rejected setter stops before reading back.
        let mut link = Link::default().add(QUERY, CLOSED).fail(&setter);
        assert!(apply_adb(&mut link, &previewed, &none()).is_err());
        link.done();
        // Readback mismatch is a failure.
        let mut link = Link::default()
            .add(QUERY, CLOSED)
            .add(&setter, "")
            .add(QUERY, CLOSED);
        assert!(apply_adb(&mut link, &previewed, &none()).is_err());
        link.done();
        // A cancelled unlock sends nothing.
        let cancel = Cancel::default();
        cancel.cancel();
        let error = apply_adb(&mut Link::default(), &previewed, &cancel).unwrap_err();
        assert!(error.is::<Cancelled>());
    }
    #[test]
    fn device_identity_verified() {
        let mut link = identity("123456789012345");
        let found = identify(&mut link, &none()).unwrap();
        assert!(found.supported());
        assert_eq!(found.masked_imei(), "•••••••••••2345");
        link.done();
        let mut other = identity("999999999999999");
        assert!(verify_identity(&mut other, &found, &none()).is_err());
        other.done();
    }
    #[test]
    fn unsupported_chipsets_remain_locked() {
        for model in ["RM500U-CN", "RG650V", "FM350", "RM520NXYZ", "RM500QTEST"] {
            let identity = ModuleIdentity {
                manufacturer: "Quectel".into(),
                model: model.into(),
                firmware: String::new(),
                imei: String::new(),
            };
            assert!(!identity.supported(), "{model}");
        }
    }
    #[test]
    fn batch_stops_at_first_rejection() {
        let mut link = Link::default().add("AT", "").fail("AT+CFUN?");
        let commands = ["AT", "AT+CFUN?", "AT+QNWINFO"].map(String::from);
        let error = execute_plan(&mut link, &commands, &none(), &mut |_| {}).unwrap_err();
        assert!(error.to_string().starts_with("已完成 1 条"));
        link.done();
    }
    #[test]
    fn custom_sms_and_key_commands_blocked() {
        for command in [
            "AT+CMGS=\"123\"",
            "at+cmgd=1",
            "AT+CMSS=1",
            "AT+QADBKEY?",
            "AT+CMGW",
            "AT+CMGC",
            "AT+QCMGS",
        ] {
            assert!(parse_custom(command).is_err(), "{command}");
        }
    }
    #[test]
    fn custom_command_boundaries() {
        assert_eq!(parse_custom("AT\r\nAT+QTEMP\n").unwrap().len(), 2);
        for command in [
            String::new(),
            "AT;AT".into(),
            "AT\t+CSQ".into(),
            "AT\u{1a}".into(),
            "A".repeat(513),
            vec!["AT"; 33].join("\n"),
        ] {
            assert!(parse_custom(&command).is_err(), "{command:?}");
        }
    }
    #[test]
    fn original_pid_and_vid_forms_preserved() {
        for pid in ["0x0900", "0x0125", "0xFFFF", "0x801", "2049"] {
            let profile = UsbProfile::parse(&CLOSED.replace("0x0801", pid)).unwrap();
            assert!(profile.enable_adb_command().contains(&format!(",{pid},")));
        }
        assert!(
            UsbProfile::parse(&profile(2).replace("0x2C7C", "11388"))
                .unwrap()
                .adb_enabled()
        );
        let live = UsbProfile::parse("+QCFG: \"usbcfg\",0x2C7C,0x0801,1,1,1,1,1,2,0").unwrap();
        assert!(live.adb() == 2 && live.adb_enabled());
    }
    #[test]
    fn network_plans() {
        assert_eq!(
            ethernet_plan(Some("r8125"), NetworkProfile::Pcie, true, true).unwrap(),
            [
                "AT+QMAP=\"MPDN_RULE\",0",
                "AT+QCFG=\"data_interface\",1,0",
                "AT+QCFG=\"pcie/mode\",1",
                "AT+QETH=\"eth_driver\",\"r8125\",1",
                "AT+QMAPWAC=1"
            ]
        );
        assert_eq!(
            ethernet_plan(Some("r8168"), NetworkProfile::Pcie, false, false).unwrap(),
            ["AT+QETH=\"eth_driver\",\"r8168\",0", "AT+QMAPWAC=0"]
        );
        assert!(ethernet_plan(Some("other"), NetworkProfile::Pcie, true, false).is_err());
        for (profile, mode) in [(NetworkProfile::Ecm, 1), (NetworkProfile::Rndis, 3)] {
            assert_eq!(
                ethernet_plan(None, profile, true, false).unwrap(),
                [
                    "AT+QCFG=\"data_interface\",0,0".to_owned(),
                    format!("AT+QCFG=\"usbnet\",{mode}"),
                    "AT+QMAPWAC=1".into()
                ]
            );
            assert_eq!(
                ethernet_plan(None, profile, false, true).unwrap(),
                ["AT+QMAP=\"MPDN_RULE\",0", "AT+QMAPWAC=0"]
            );
        }
    }
    #[test]
    fn mpdn_rule_zero_detection() {
        assert!(
            has_mpdn_rule_zero("+QMAP: \"MPDN_rule\",0,1,0,0,1\n+QMAP: \"MPDN_rule\",1,0,0,0,0")
                .unwrap()
        );
        assert!(!has_mpdn_rule_zero("+QMAP: \"MPDN_rule\",0,0,0,0,0").unwrap());
        assert!(!has_mpdn_rule_zero("").unwrap());
        assert!(has_mpdn_rule_zero("ERROR").is_err());
    }
    #[test]
    fn exact_response_terminators() {
        for line in ["OK", "ERROR", "+CME ERROR: 10", "+CMS ERROR: 500"] {
            assert!(is_terminal(line));
        }
        for line in ["BOOK", "NOT OK", "+QIND: \"OK\""] {
            assert!(!is_terminal(line));
        }
    }
}
