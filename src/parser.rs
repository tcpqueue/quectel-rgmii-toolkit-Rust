use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;

pub fn lines(raw: &str) -> impl Iterator<Item = &str> {
    raw.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "OK")
}
pub fn fields(raw: &str) -> Vec<String> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(raw.trim().as_bytes())
        .records()
        .next()
        .and_then(Result::ok)
        .map(|r| {
            r.iter()
                .map(|v| v.trim().trim_matches('"').to_owned())
                .collect()
        })
        .unwrap_or_default()
}
pub fn number(s: &str) -> i64 {
    s.trim().parse().unwrap_or(0)
}
pub fn text<'a>(data: &'a Value, key: &str) -> &'a str {
    data[key].as_str().unwrap_or("")
}
pub fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}
fn put(data: &mut Value, key: &str, value: impl Into<Value>) {
    data[key] = value.into();
}
pub fn plain(s: &str) -> bool {
    let up = s.to_ascii_uppercase();
    !s.is_empty() && up != "OK" && up != "ERROR" && !up.starts_with("AT+") && !s.starts_with('+')
}
pub fn ok(s: &str) -> bool {
    let up = s.to_ascii_uppercase();
    up.contains("OK") && !up.contains("ERROR")
}
pub fn decode_ucs2(s: &str) -> String {
    let clean = s.trim().replace(' ', "");
    if clean.is_empty()
        || !clean.len().is_multiple_of(4)
        || !clean.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return s.into();
    }
    clean
        .as_bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| {
            let n = u32::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap();
            char::from_u32(n).unwrap_or(char::REPLACEMENT_CHARACTER)
        })
        .collect()
}
pub fn model(raw: &str) -> String {
    for s in lines(raw) {
        let s = if s.to_ascii_uppercase().starts_with("+CGMM:") {
            s.split_once(':').unwrap().1.trim()
        } else if plain(s) {
            s
        } else {
            continue;
        };
        let s = s.trim_matches('"').trim();
        if !s.is_empty()
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._ -".contains(&c))
        {
            return s.into();
        }
    }
    "-".into()
}
pub fn human_bytes(mut bytes: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut i = 0;
    while bytes >= 1024.0 && i < 4 {
        bytes /= 1024.0;
        i += 1
    }
    if i > 0 && bytes < 10.0 {
        format!("{bytes:.1} {}", units[i])
    } else {
        format!("{bytes:.0} {}", units[i])
    }
}
fn hex_decimal(s: &str) -> String {
    if s.is_empty() || s == "-" {
        return "-".into();
    }
    format!("{} ({})", s, i64::from_str_radix(s, 16).unwrap_or(0))
}
fn cell_id(data: &mut Value, s: &str) {
    if s.is_empty() || s == "-" {
        return;
    }
    if s.len() > 2 && s.is_ascii() {
        let (prefix, last) = s.split_at(s.len() - 2);
        data["eNBID"] = json!(prefix);
        data["cellID"] = json!(format!(
            "{}({}), {}({})",
            last,
            i64::from_str_radix(last, 16).unwrap_or(0),
            s,
            i64::from_str_radix(s, 16).unwrap_or(0)
        ));
    } else {
        data["cellID"] = json!(s)
    }
}
fn append(data: &mut Value, key: &str, s: &str) {
    if s.is_empty() || s == "-" {
        return;
    }
    let old = text(data, key);
    if old.is_empty() || old == "-" {
        data[key] = json!(s)
    } else if !old.split(" / ").any(|v| v == s) {
        data[key] = json!(format!("{old} / {s}"))
    }
}
fn signal(data: &mut Value, p: &[String], radio: &str, rsrp: usize, rsrq: usize, sinr: usize) {
    for (name, index, min, scale) in [
        ("rsrp", rsrp, -120, 40),
        ("rsrq", rsrq, -20, 12),
        ("sinr", sinr, 0, 20),
    ] {
        if let Some(value) = p.get(index) {
            put(data, &format!("{name}{radio}"), value.clone());
            put(
                data,
                &format!("{name}{radio}Percentage"),
                ((number(value) - min) * 100 / scale).clamp(0, 100),
            );
        }
    }
}
fn qeng(data: &mut Value, p: &[String]) {
    let Some(kind) = p.first() else { return };
    if kind.eq_ignore_ascii_case("servingcell") {
        if p.len() < 3 {
            return;
        }
        let radio = &p[2];
        if radio != "NR5G-SA" && radio != "LTE" {
            return;
        }
        data["network_mode"] = json!(format!(
            "{}{}",
            radio,
            p.get(3)
                .filter(|v| !v.is_empty())
                .map(|v| format!(" {v}"))
                .unwrap_or_default()
        ));
        if let Some(cid) = p.get(6) {
            cell_id(data, cid)
        }
        if let Some(pci) = p.get(7).filter(|v| !v.is_empty()) {
            data["pcc_pci"] = json!(pci)
        }
        if let Some(freq) = p.get(9) {
            append(data, "earfcns", freq)
        }
        if radio == "NR5G-SA" {
            if let Some(tac) = p.get(8) {
                data["tac"] = json!(hex_decimal(tac))
            }
            signal(data, p, "NR", 12, 13, 14);
        } else {
            if let Some(tac) = p.get(12) {
                data["tac"] = json!(hex_decimal(tac))
            }
            signal(data, p, "LTE", 13, 14, 16);
            if let Some(rssi) = p.get(15) {
                data["rssi"] = json!(rssi)
            }
        }
    } else if kind == "LTE" {
        data["network_mode"] = json!(kind);
        if let Some(cid) = p.get(4) {
            cell_id(data, cid)
        }
        if let Some(pci) = p.get(5) {
            data["pcc_pci"] = json!(pci)
        }
        if let Some(freq) = p.get(6) {
            append(data, "earfcns", freq)
        }
        if let Some(tac) = p.get(10) {
            data["tac"] = json!(hex_decimal(tac))
        }
        if let Some(rssi) = p.get(13) {
            data["rssi"] = json!(rssi)
        }
        signal(data, p, "LTE", 11, 12, 14);
    } else if kind == "NR5G-NSA" || kind == "NR5G-SA" {
        data["network_mode"] = json!(kind);
        if let Some(pci) = p.get(3) {
            data["pcc_pci"] = json!(pci)
        }
        if let Some(freq) = p.get(7) {
            append(data, "earfcns", freq)
        }
        signal(data, p, "NR", 4, 6, 5);
    }
}
fn bandwidth(code: &str, nr: bool) -> String {
    let n = number(code);
    if nr {
        return match n {
            0..=5 => format!("{}MHz", (n + 1) * 5),
            6..=12 => format!("{}MHz", (n - 2) * 10),
            _ => "-".into(),
        };
    }
    match n {
        0 | 6 => "1.4MHz",
        1 | 15 => "3MHz",
        2 | 25 => "5MHz",
        3 | 50 => "10MHz",
        4 | 75 => "15MHz",
        5 | 100 => "20MHz",
        _ => "-",
    }
    .into()
}
fn qca(data: &mut Value, records: &[Vec<String>]) {
    let (mut bands, mut widths, mut freqs, mut scc) = (vec![], vec![], vec![], vec![]);
    static BAND: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(NR5G|NR|LTE)\s+BAND\s+(\d+)").unwrap());
    for p in records.iter().filter(|p| p.len() >= 4) {
        let nr = p[3].to_ascii_uppercase().contains("NR");
        if !p[1].is_empty() {
            freqs.push(p[1].clone())
        }
        let band = if let Some(c) = BAND.captures(&p[3].to_ascii_uppercase()) {
            format!("{}{}", if nr { "N" } else { "B" }, &c[2])
        } else {
            p[3].clone()
        };
        if !band.is_empty() {
            bands.push(band)
        }
        let bw = bandwidth(&p[2], nr);
        if bw != "-" {
            widths.push(bw)
        }
        let idx = if p.len() > 5 && (!nr || p[0].eq_ignore_ascii_case("SCC")) {
            5
        } else {
            4
        };
        if let Some(pci) = p.get(idx).filter(|v| !v.is_empty() && *v != "-") {
            if p[0].eq_ignore_ascii_case("PCC") {
                data["pcc_pci"] = json!(pci)
            }
            if p[0].eq_ignore_ascii_case("SCC") {
                scc.push(pci.clone())
            }
        }
    }
    for (key, v, sep) in [
        ("bands", bands, " / "),
        ("bandwidth", widths, " / "),
        ("earfcns", freqs, " / "),
        ("scc_pci", scc, " + "),
    ] {
        if !v.is_empty() {
            data[key] = json!(v.join(sep))
        }
    }
}
pub fn dashboard(raw: &str) -> Value {
    let mut d = json!({});
    for key in [
        "active_sim",
        "network_provider",
        "mccmnc",
        "apn",
        "network_mode",
        "ipv4",
        "ipv6",
        "bands",
        "bandwidth",
        "csq",
        "rssi",
        "cellID",
        "eNBID",
        "tac",
        "rsrqLTE",
        "rsrqNR",
        "rsrpLTE",
        "rsrpNR",
        "sinrLTE",
        "sinrNR",
        "earfcns",
        "pcc_pci",
        "scc_pci",
        "signalAssessment",
        "nr_rx_human",
        "nr_tx_human",
        "nr_dl_speed",
        "nr_ul_speed",
    ] {
        d[key] = json!("-")
    }
    for key in [
        "nr_rx_bytes",
        "nr_tx_bytes",
        "signalPercentage",
        "rsrqLTEPercentage",
        "rsrqNRPercentage",
        "rsrpLTEPercentage",
        "rsrpNRPercentage",
        "sinrLTEPercentage",
        "sinrNRPercentage",
    ] {
        d[key] = json!(0)
    }
    for key in ["prxqrsrp", "drxqrsrp", "rx2qrsrp", "rx3qrsrp"] {
        d[key] = json!("None")
    }
    d["sim"] = json!("未激活");
    d["temperature"] = json!("N/A");
    let (mut sum, mut count, mut sim) = (0, 0, None);
    let mut carriers = vec![];
    let (mut lte, mut nr) = (None, None);
    for line in lines(raw) {
        let up = line.to_ascii_uppercase();
        if up.contains("SIM NOT INSERTED")
            || up.contains("+CME ERROR: 10")
            || up.contains("+CPIN: NOT INSERTED")
        {
            sim = Some(false)
        }
        let Some((key, tail)) = line.split_once(':') else {
            continue;
        };
        let p = fields(tail);
        match key {
            "+QTEMP" if p.len() > 1 => {
                if let Ok(v) = p[1].parse::<i64>()
                    && v != 0
                    && v != -273
                {
                    sum += v;
                    count += 1
                }
            }
            "+QSIMSTAT" if p.len() > 1 => {
                sim = Some(p[1] == "1");
                d["sim"] = json!(if p[1] == "1" {
                    "已激活"
                } else {
                    "未激活"
                })
            }
            "+CSQ" if !p.is_empty() => d["csq"] = json!(p[0]),
            "+QUIMSLOT" => d["active_sim"] = json!(tail.trim()),
            "+QSPN" if p.len() >= 5 => {
                let name = if !p[2].is_empty() {
                    &p[2]
                } else if !p[0].is_empty() {
                    &p[0]
                } else {
                    "-"
                };
                d["network_provider"] = json!(if p[3] != "0" {
                    decode_ucs2(name)
                } else {
                    name.into()
                });
                d["mccmnc"] = json!(p[4]);
            }
            "+CGCONTRDP" => {
                if p.get(2).is_some_and(|v| !v.is_empty()) {
                    d["apn"] = json!(p[2])
                }
                for (i, key) in [(3, "ipv4"), (4, "ipv6")] {
                    if let Some(v) = p.get(i).filter(|v| {
                        !v.is_empty()
                            && *v != "0.0.0.0"
                            && *v != "0:0:0:0:0:0:0:0"
                            && *v != "0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0"
                    }) && text(&d, key) == "-"
                    {
                        d[key] = json!(v);
                        d["sim"] = json!("已激活")
                    }
                }
            }
            "+QMAP" if p.len() > 4 && p[0] == "WWAN" && !p[4].is_empty() => {
                if p[3] == "IPV4" {
                    d["ipv4"] = json!(p[4]);
                    d["sim"] = json!("已激活")
                }
                if p[3] == "IPV6" {
                    d["ipv6"] = json!(p[4]);
                    d["sim"] = json!("已激活")
                }
            }
            "+QENG" => qeng(&mut d, &p),
            "+QCAINFO" => carriers.push(p),
            "+QRSRP" if p.len() >= 4 => {
                if line.contains("LTE") {
                    lte = Some(p)
                } else if line.contains("NR5G") {
                    nr = Some(p)
                }
            }
            "+QGDNRCNT" if p.len() >= 2 => {
                for (i, key, human) in [
                    (0, "nr_rx_bytes", "nr_rx_human"),
                    (1, "nr_tx_bytes", "nr_tx_human"),
                ] {
                    d[key] = json!(number(&p[i]));
                    d[human] = json!(human_bytes(number(&p[i]) as f64));
                }
            }
            _ => {}
        }
    }
    if count > 0 {
        d["temperature"] = json!(((sum + count / 2) / count).to_string())
    }
    if sim == Some(false) {
        for key in [
            "sim",
            "active_sim",
            "network_provider",
            "mccmnc",
            "apn",
            "ipv4",
            "ipv6",
            "bands",
            "bandwidth",
            "csq",
            "rssi",
            "cellID",
            "eNBID",
            "tac",
            "rsrqLTE",
            "rsrqNR",
            "rsrpLTE",
            "rsrpNR",
            "sinrLTE",
            "sinrNR",
            "prxqrsrp",
            "drxqrsrp",
            "rx2qrsrp",
            "rx3qrsrp",
            "earfcns",
            "pcc_pci",
            "scc_pci",
            "nr_rx_human",
            "nr_tx_human",
        ] {
            d[key] = json!("-")
        }
        for key in [
            "nr_rx_bytes",
            "nr_tx_bytes",
            "signalPercentage",
            "rsrqLTEPercentage",
            "rsrqNRPercentage",
            "rsrpLTEPercentage",
            "rsrpNRPercentage",
            "sinrLTEPercentage",
            "sinrNRPercentage",
        ] {
            d[key] = json!(0)
        }
        d["sim"] = json!("未激活");
        d["network_mode"] = json!("未插卡");
        d["signalAssessment"] = json!("未知");
        return d;
    }
    qca(&mut d, &carriers);
    for (i, key) in ["prxqrsrp", "drxqrsrp", "rx2qrsrp", "rx3qrsrp"]
        .iter()
        .enumerate()
    {
        if let Some(v) = match (&lte, &nr) {
            (Some(l), Some(n)) => Some(format!("{}/{}", l[i], n[i])),
            (Some(v), None) | (None, Some(v)) => Some(v[i].clone()),
            _ => None,
        } {
            d[key] = json!(v)
        }
    }
    let best = |name: &str| {
        d[format!("{name}LTEPercentage")]
            .as_i64()
            .unwrap_or(0)
            .max(d[format!("{name}NRPercentage")].as_i64().unwrap_or(0))
    };
    let (rsrp, rsrq, sinr) = (best("rsrp"), best("rsrq"), best("sinr"));
    if rsrp > 0 || rsrq > 0 || sinr > 0 {
        let pct = ((rsrp * 50 + rsrq * 25 + sinr * 25 + 50) / 100).clamp(0, 100);
        d["signalPercentage"] = json!(pct);
        d["signalAssessment"] = json!(match pct {
            80.. => "优秀",
            60.. => "良好",
            40.. => "一般",
            _ => "差",
        });
    }
    d
}
pub fn device(raw: &str) -> Value {
    let mut d = json!({"manufacturer":"-","modelName":"-","firmwareVersion":"-","simStatus":"未知","simInserted":false,"imsi":"-","iccid":"-","imei":"-","lanIp":"-","wwanIpv4":"-","wwanIpv6":"-","phoneNumber":"-"});
    let (mut sim, mut absent) = (None, false);
    let mut values = vec![];
    static IMEI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{14,17}").unwrap());
    for line in lines(raw) {
        let up = line.to_ascii_uppercase();
        if up.contains("SIM NOT INSERTED")
            || up.contains("+CME ERROR: 10")
            || up.contains("+CPIN: NOT INSERTED")
        {
            sim = Some(false);
            absent = true
        }
        let (key, tail) = line.split_once(':').unwrap_or(("", line));
        let p = fields(tail);
        match key {
            "+QSIMSTAT" if p.len() > 1 => {
                sim = Some(p[1] == "1");
                absent |= p[1] != "1"
            }
            "+CPIN" => {
                sim = Some(up.contains("READY"));
                absent |= !up.contains("READY")
            }
            "+CGSN" => {
                if let Some(v) = IMEI.find(line) {
                    d["imei"] = json!(v.as_str())
                }
            }
            "+ICCID" if !tail.trim().is_empty() => {
                d["iccid"] = json!(tail.trim());
                sim = Some(true)
            }
            "+QMAP" if p.len() > 3 && p[0] == "LANIP" => d["lanIp"] = json!(p[3]),
            "+QMAP" if p.len() > 4 && p[0] == "WWAN" => {
                if p[3] == "IPV4" {
                    d["wwanIpv4"] = json!(p[4])
                }
                if p[3] == "IPV6" {
                    d["wwanIpv6"] = json!(p[4])
                }
            }
            "+CNUM" => {
                if let Some(v) = p
                    .iter()
                    .find(|v| v.starts_with('+') || digits(v).len() >= 5)
                {
                    d["phoneNumber"] = json!(v)
                }
            }
            _ => {
                if plain(line) {
                    values.push(line)
                }
            }
        }
    }
    let numeric =
        |s: &&str| s.len() >= 10 && s.len() <= 20 && s.bytes().all(|c| c.is_ascii_digit());
    let numbers: Vec<_> = values.iter().copied().filter(numeric).collect();
    let names: Vec<_> = values.iter().copied().filter(|s| !numeric(s)).collect();
    if let Some(v) = names.first() {
        d["manufacturer"] = json!(v)
    }
    if names.len() > 2 {
        d["modelName"] = json!(names[1]);
        d["firmwareVersion"] = json!(names[2])
    } else if names.len() > 1 {
        d["firmwareVersion"] = json!(names[1])
    }
    if text(&d, "imei") == "-"
        && let Some(v) = numbers.iter().find(|s| (14..=17).contains(&s.len()))
    {
        d["imei"] = json!(v)
    }
    if let Some(v) = numbers
        .iter()
        .find(|s| **s != text(&d, "imei") && (14..=16).contains(&s.len()))
    {
        d["imsi"] = json!(v);
        sim = Some(true)
    }
    if absent || sim == Some(false) {
        for key in ["imsi", "iccid", "wwanIpv4", "wwanIpv6"] {
            d[key] = json!("-")
        }
        d["simStatus"] = json!("未插卡");
        d["phoneNumber"] = json!("未插卡");
    } else if sim == Some(true) {
        d["simStatus"] = json!("已插卡");
        d["simInserted"] = json!(true);
        if text(&d, "phoneNumber") == "-" {
            d["phoneNumber"] = json!("无本机号码")
        }
    }
    d
}
pub fn bands(raw: &str) -> Value {
    let mut out = json!({"locked_lte_bands":"","locked_nsa_bands":"","locked_sa_bands":""});
    static BAND: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#""([^"]+)"\s*,\s*([0-9:]+)"#).unwrap());
    for p in BAND.captures_iter(raw) {
        let key = match &p[1] {
            "lte_band" => "locked_lte_bands",
            "nsa_nr5g_band" => "locked_nsa_bands",
            "nr5g_band" => "locked_sa_bands",
            _ => continue,
        };
        out[key] = json!(&p[2]);
    }
    out
}
pub fn network(raw: &str) -> Value {
    let mut out = json!({"sim":"-","apn":"-","cellLockStatus":"未锁定","prefNetwork":"-","nrModeControl":"未禁用","nrModeControlNum":"0","bands":"-","pdpType":"-"});
    let (mut lte, mut nr) = (false, false);
    let mut bands = vec![];
    for line in lines(raw) {
        let Some((key, tail)) = line.split_once(':') else {
            continue;
        };
        let p = fields(tail);
        match key {
            "+QUIMSLOT" => out["sim"] = json!(tail.trim()),
            "+CGCONTRDP" if p.len() > 2 && !p[2].is_empty() => out["apn"] = json!(p[2]),
            "+CGDCONT" if line.contains("1,") && p.len() > 1 => {
                out["pdpType"] = json!(p[1]);
                if p.len() > 2 && text(&out, "apn") == "-" {
                    out["apn"] = json!(p[2])
                }
            }
            "+QNWLOCK" if p.len() > 1 => match p[0].as_str() {
                "common/4g" => lte = p[1] != "0",
                "common/5g" => nr = p[1] != "0",
                _ => {}
            },
            "+QNWPREFCFG" if p.len() > 1 => match p[0].as_str() {
                "mode_pref" => out["prefNetwork"] = json!(p[1]),
                "nr5g_disable_mode" => out["nrModeControlNum"] = json!(p[1]),
                _ => {}
            },
            "+QCAINFO" if p.len() > 3 => bands.push((
                if p[0].eq_ignore_ascii_case("PCC") {
                    0
                } else if p[0].eq_ignore_ascii_case("SCC") {
                    1
                } else {
                    2
                },
                p[3].clone(),
            )),
            _ => {}
        }
    }
    out["cellLockStatus"] = json!(match (lte, nr) {
        (true, true) => "已锁定4G和5G",
        (true, false) => "已锁定4G",
        (false, true) => "已锁定5G",
        _ => "未锁定",
    });
    out["nrModeControl"] = json!(match text(&out, "nrModeControlNum") {
        "1" => "禁用SA",
        "2" => "禁用NSA",
        _ => "未禁用",
    });
    bands.sort_by_key(|p| p.0);
    let names: Vec<_> = bands
        .into_iter()
        .map(|p| p.1)
        .filter(|s| !s.is_empty())
        .collect();
    if !names.is_empty() {
        out["bands"] = json!(names.join(" / "))
    }
    out
}
pub fn settings(raw: &str) -> Value {
    let mut out = json!({"ipPassStatus":false,"DNSV6ProxyStatus":false,"DNSV4ProxyStatus":false,"currentUsbNetMode":"未知","dmzMode":"0","dmzIP":"","lanIpStart":"","lanIpEnd":"","lanGwIp":"","imei":"-"});
    static IMEI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{14,17}").unwrap());
    for line in lines(raw) {
        let (key, tail) = line.split_once(':').unwrap_or(("", line));
        let p = fields(tail);
        if key == "+QMAP" && !p.is_empty() {
            match p[0].to_ascii_uppercase().as_str() {
                "MPDN_RULE" if p.len() > 2 && p[2] == "1" => out["ipPassStatus"] = json!(true),
                "DHCPV6DNS" => {
                    out["DNSV6ProxyStatus"] =
                        json!(line.to_ascii_lowercase().contains("\"enable\""))
                }
                "DHCPV4DNS" => {
                    out["DNSV4ProxyStatus"] =
                        json!(line.to_ascii_lowercase().contains("\"enable\""))
                }
                "DMZ" if p.len() > 1 => {
                    out["dmzMode"] = json!(p[1]);
                    if p.len() > 3 && p[1] == "1" {
                        out["dmzIP"] = json!(p[3])
                    }
                }
                "LANIP" if p.len() > 3 => {
                    for (i, k) in [(1, "lanIpStart"), (2, "lanIpEnd")] {
                        let a: Vec<_> = p[i].split('.').collect();
                        out[k] = json!(if a.len() == 4 { a[3] } else { "" })
                    }
                    out["lanGwIp"] = json!(p[3])
                }
                _ => {}
            }
        } else if p.len() > 1 && p[0] == "usbnet" {
            out["currentUsbNetMode"] = json!(match p[1].as_str() {
                "0" => "RMNET",
                "1" => "ECM",
                "2" => "MBIM",
                "3" => "RNDIS",
                _ => "",
            });
        } else if (key.eq_ignore_ascii_case("+CGSN") || (plain(line) && text(&out, "imei") == "-"))
            && let Some(v) = IMEI.find(line)
        {
            out["imei"] = json!(v.as_str())
        }
    }
    out
}
pub fn scan(raw: &str) -> Value {
    let (mut lte, mut nr) = (vec![], vec![]);
    for line in lines(raw).filter(|s| s.contains("+QSCAN:")) {
        let p = fields(line.trim_start_matches("+QSCAN:"));
        if p.len() < 13 {
            continue;
        }
        let is_nr = line.to_ascii_uppercase().contains("NR5G");
        let carrier = match format!("{}{}", p[1], p[2]).as_str() {
            "46000" => "中国移动",
            "46001" | "46009" => "中国联通",
            "46003" | "46011" => "中国电信",
            "46015" => "中国广电",
            "46020" => "中国铁通",
            _ => "",
        };
        let cell = json!({"type":if is_nr{"NR5G"}else{"LTE"},"provider":if carrier.is_empty(){format!("{} {}",p[1],p[2])}else{carrier.into()},"band":p[12],"freq":p[3],"pci":p[4],"rsrp":p[5]});
        if is_nr { nr.push(cell) } else { lte.push(cell) }
    }
    json!({"nr5g_cells_parsed":nr,"lte_cells_parsed":lte})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn go_parser_contracts() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../tests/fixtures/go-parsers.json")).unwrap();
        for (i, c) in cases.iter().enumerate() {
            let raw = c["raw"].as_str().unwrap();
            let got = match c["kind"].as_str().unwrap() {
                "dashboard" => dashboard(raw),
                "device" => device(raw),
                "network" => network(raw),
                "bands" => bands(raw),
                "settings" => settings(raw),
                "scan" => scan(raw),
                _ => unreachable!(),
            };
            assert_eq!(got, c["expected"], "case {i} {} input: {raw}", c["kind"]);
        }
    }
}
