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
            let value = if radio == "LTE" && name == "sinr" {
                // AT+QENG reports LTE SINR as X; the value in dB is X / 5 * 10 - 20.
                value
                    .trim()
                    .parse::<i64>()
                    .map(|x| (x * 2 - 20).to_string())
                    .unwrap_or_else(|_| value.clone())
            } else {
                value.clone()
            };
            put(
                data,
                &format!("{name}{radio}Percentage"),
                ((number(&value) - min) * 100 / scale).clamp(0, 100),
            );
            put(data, &format!("{name}{radio}"), value);
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
        // ARFCN follows the TAC in SA mode and the PCID in LTE mode.
        if let Some(freq) = p.get(if radio == "NR5G-SA" { 9 } else { 8 }) {
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
            13 => "200MHz".into(),
            14 => "400MHz".into(),
            15 => "35MHz".into(),
            16 => "45MHz".into(),
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
                // Quectel reports bytes_sent first, then bytes_recv.
                for (i, key, human) in [
                    (1, "nr_rx_bytes", "nr_rx_human"),
                    (0, "nr_tx_bytes", "nr_tx_human"),
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
        // -32768 marks an invalid antenna path.
        let path = |p: &Vec<String>| {
            if p[i].trim() == "-32768" {
                "-".to_owned()
            } else {
                p[i].clone()
            }
        };
        if let Some(v) = match (&lte, &nr) {
            (Some(l), Some(n)) => Some(format!("{}/{}", path(l), path(n))),
            (Some(v), None) | (None, Some(v)) => Some(path(v)),
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
    let locks = cell_locks(raw);
    let (lte, nr) = (!locks["lte"].is_null(), !locks["nr"].is_null());
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
/// A PDP address from AT+CGPADDR: IPv4, or IPv6 written as 16 dotted decimal bytes (the
/// module's default) or in colon form. Unassigned addresses are dropped.
fn pdp_address(raw: &str) -> Option<std::net::IpAddr> {
    use std::net::{IpAddr, Ipv6Addr};
    let raw = raw.trim();
    let ip = if let Ok(ip) = raw.parse::<IpAddr>() {
        ip
    } else {
        let bytes: Vec<u8> = raw
            .split('.')
            .map(|b| b.parse().ok())
            .collect::<Option<_>>()?;
        IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(bytes).ok()?))
    };
    (!ip.is_unspecified()).then_some(ip)
}
/// PDP contexts (AT+CGDCONT?, AT+CGACT?, AT+CGPADDR) and the network feature switches
/// (AT+QCFG="ims", AT+QSIMDET?, AT+QNWPREFCFG="roam_pref", AT+QUIMSLOT?).
pub fn pdp(raw: &str) -> Value {
    let mut contexts: Vec<Value> = Vec::new();
    let mut states = std::collections::HashMap::new();
    let mut addresses: std::collections::HashMap<String, Vec<std::net::IpAddr>> =
        std::collections::HashMap::new();
    let mut out = json!({"ims": null, "volte": null, "sim_detect": null, "sim_detect_level": null, "roaming": null, "sim_slot": null});
    for line in lines(raw) {
        let Some((key, tail)) = line.split_once(':') else {
            continue;
        };
        let p = fields(tail);
        match key.trim() {
            "+CGDCONT" if p.len() >= 3 && p[0].parse::<u32>().is_ok() => contexts.push(json!({
                "cid": number(&p[0]),
                "type": p[1],
                "apn": p[2],
            })),
            "+CGACT" if p.len() >= 2 => {
                states.insert(p[0].clone(), p[1] == "1");
            }
            "+CGPADDR" if !p.is_empty() => {
                addresses.insert(
                    p[0].clone(),
                    p[1..].iter().filter_map(|a| pdp_address(a)).collect(),
                );
            }
            "+QCFG" if p.len() >= 2 && p[0].eq_ignore_ascii_case("ims") => {
                // <ims_conf>: 0 follows the MBN, 1 forces IMS on, 2 forces it off.
                out["ims"] = json!(number(&p[1]));
                if let Some(volte) = p.get(2) {
                    out["volte"] = json!(volte == "1");
                }
            }
            "+QSIMDET" if p.len() >= 2 => {
                out["sim_detect"] = json!(p[0] == "1");
                out["sim_detect_level"] = json!(number(&p[1]));
            }
            "+QNWPREFCFG" if p.len() >= 2 && p[0] == "roam_pref" => {
                out["roaming"] = json!(number(&p[1]))
            }
            "+QUIMSLOT" | "+QUSIMSLOT" if !p.is_empty() => out["sim_slot"] = json!(number(&p[0])),
            _ => {}
        }
    }
    for context in &mut contexts {
        let cid = context["cid"].to_string();
        context["active"] = json!(states.get(&cid));
        let ips = addresses.get(&cid).cloned().unwrap_or_default();
        context["ipv4"] = json!(ips.iter().find(|ip| ip.is_ipv4()).map(|ip| ip.to_string()));
        context["ipv6"] = json!(ips.iter().find(|ip| ip.is_ipv6()).map(|ip| ip.to_string()));
    }
    out["contexts"] = json!(contexts);
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
        let provider = if carrier.is_empty() {
            format!("{} {}", p[1], p[2])
        } else {
            carrier.into()
        };
        // A shared cell (e.g. 460-11 and 460-01) is reported once per PLMN: keep one row.
        let list = if is_nr { &mut nr } else { &mut lte };
        if let Some(seen) = list
            .iter_mut()
            .find(|c: &&mut Value| c["freq"] == p[3].as_str() && c["pci"] == p[4].as_str())
        {
            let names = text(seen, "provider").to_owned();
            if !names.split(" / ").any(|n| n == provider) {
                seen["provider"] = json!(format!("{names} / {provider}"));
            }
            continue;
        }
        let mut cell = json!({"type":if is_nr{"NR5G"}else{"LTE"},"provider":provider,"band":p[12],"freq":p[3],"pci":p[4],"rsrp":p[5]});
        if is_nr {
            cell["scs"] = scs_khz(&p[8]);
            nr.push(cell)
        } else {
            lte.push(cell)
        }
    }
    json!({"nr5g_cells_parsed":nr,"lte_cells_parsed":lte})
}
/// QSCAN and QENG report `<scs>` as an index while QNWLOCK takes kHz; "-" means not reported.
fn scs_khz(index: &str) -> Value {
    match index {
        "0" => json!(15),
        "1" => json!(30),
        "2" => json!(60),
        "3" => json!(120),
        _ => Value::Null,
    }
}
/// Locks the module currently applies, from the `AT+QNWLOCK="common/4g"|"common/5g"` queries:
/// LTE as `[count, earfcn, pci, ...]`, NR as `[pci, arfcn, scs, band]`, null when unlocked.
pub fn cell_locks(raw: &str) -> Value {
    let mut out = json!({"lte":null,"nr":null});
    for line in lines(raw) {
        let Some(tail) = line.trim().strip_prefix("+QNWLOCK:") else {
            continue;
        };
        let p = fields(tail);
        let values: Vec<u64> = p.iter().skip(1).filter_map(|s| s.parse().ok()).collect();
        match p.first().map(String::as_str) {
            Some("common/4g") if values.first().is_some_and(|n| *n > 0) => {
                out["lte"] = json!(values)
            }
            Some("common/5g") if values.len() >= 4 => out["nr"] = json!(values),
            _ => {}
        }
    }
    out
}
/// LTE operating band for a downlink EARFCN (3GPP TS 36.101 table 5.7.3-1).
pub fn lte_band(earfcn: u32) -> Option<u32> {
    const BANDS: [(u32, u32, u32); 27] = [
        (1, 0, 599),
        (2, 600, 1199),
        (3, 1200, 1949),
        (4, 1950, 2399),
        (5, 2400, 2649),
        (7, 2750, 3449),
        (8, 3450, 3799),
        (12, 5010, 5179),
        (13, 5180, 5279),
        (14, 5280, 5379),
        (17, 5730, 5849),
        (18, 5850, 5999),
        (19, 6000, 6149),
        (20, 6150, 6449),
        (25, 8040, 8689),
        (26, 8690, 9039),
        (28, 9210, 9659),
        (32, 9920, 10359),
        (34, 36200, 36349),
        (38, 37750, 38249),
        (39, 38250, 38649),
        (40, 38650, 39649),
        (41, 39650, 41589),
        (42, 41590, 43589),
        (43, 43590, 45589),
        (66, 66436, 67335),
        (71, 68586, 68935),
    ];
    BANDS
        .iter()
        .find(|(_, low, high)| (*low..=*high).contains(&earfcn))
        .map(|b| b.0)
}
/// Serving cell plus the LTE neighbours from `AT+QENG="servingcell";+QENG="neighbourcell"`.
/// The module only reports 3G/4G neighbours, and inter-frequency entries without a PCI are
/// dropped because they cannot be locked.
pub fn neighbours(raw: &str) -> Value {
    let (mut lte, mut nr) = (vec![], vec![]);
    let at = |p: &[String], i: usize| p.get(i).cloned().unwrap_or_default();
    for line in lines(raw) {
        let Some(tail) = line.trim().strip_prefix("+QENG:") else {
            continue;
        };
        let p = fields(tail);
        let Some(head) = p.first().map(String::as_str) else {
            continue;
        };
        if let Some(role) = match head {
            "neighbourcell intra" => Some("同频邻区"),
            "neighbourcell inter" => Some("异频邻区"),
            _ => None,
        } {
            if at(&p, 1) != "LTE" || at(&p, 3).parse::<u32>().is_err() {
                continue;
            }
            let earfcn = at(&p, 2);
            let band = earfcn.parse().ok().and_then(lte_band);
            // Neighbour rows give RSRQ before RSRP.
            lte.push(json!({"type":"LTE","role":role,"band":band.map(|b| b.to_string()).unwrap_or_else(|| "-".into()),"freq":earfcn,"pci":at(&p,3),"rsrp":at(&p,5)}));
            continue;
        }
        // Serving cell rows; a cell without a PCI (still searching) cannot be locked.
        if let Some(k) = p.iter().position(|s| s == "LTE") {
            if at(&p, k + 5).parse::<u32>().is_ok() {
                lte.insert(0, json!({"type":"LTE","role":"服务小区","band":at(&p,k+7),"freq":at(&p,k+6),"pci":at(&p,k+5),"rsrp":at(&p,k+11)}));
            }
        } else if let Some(k) = p.iter().position(|s| s == "NR5G-SA")
            && at(&p, k + 5).parse::<u32>().is_ok()
        {
            nr.push(json!({"type":"NR5G","role":"服务小区","band":at(&p,k+8),"freq":at(&p,k+7),"pci":at(&p,k+5),"rsrp":at(&p,k+10),"scs":scs_khz(&at(&p,k+13))}));
        }
    }
    json!({"nr5g_cells_parsed":nr,"lte_cells_parsed":lte})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nr_scan_maps_scs_index_to_khz() {
        // Captured from an RM520N-EU (RM520NEUDAR03A08M4G); the last line is synthetic.
        let raw = "+QSCAN: \"NR5G\",460,11,633984,249,-80,-11,33,1,C9A82400C,C94B00,273,78,28,4,-\r\n\
                   +QSCAN: \"NR5G\",460,00,152890,866,-80,-14,41,0,223DB7017,58D006,160,28,22,6,-\r\n\
                   +QSCAN: \"NR5G\",460,00,524910,296,-84,-11,37,-,22382F005,58D006,162,41,28,2,-\r\nOK\r\n";
        let cells = &scan(raw)["nr5g_cells_parsed"];
        assert_eq!(cells[0]["freq"], "633984");
        assert_eq!(cells[0]["pci"], "249");
        assert_eq!(cells[0]["band"], "78");
        assert_eq!(cells[0]["scs"], 30);
        assert_eq!(cells[1]["band"], "28");
        assert_eq!(cells[1]["scs"], 15);
        assert!(cells[2]["scs"].is_null());
    }
    #[test]
    fn scan_merges_cells_shared_between_carriers() {
        // Captured from an RM520N-EU: shared cells repeat once per PLMN.
        let raw = "+QSCAN: \"NR5G\",460,11,627264,249,-91,-11,37,1,C9A824009,C94B00,273,78,24,12,-\r\n\
                   +QSCAN: \"NR5G\",460,01,627264,249,-91,-11,37,1,C9A824009,C94B00,273,78,24,12,-\r\n\
                   +QSCAN: \"NR5G\",460,00,504990,109,-88,-11,33,1,22382F002,58D006,273,41,30,6,-\r\n\
                   +QSCAN: \"NR5G\",460,15,504990,109,-88,-11,33,1,22382F002,58D006,273,41,30,6,-\r\n\
                   +QSCAN: \"LTE\",460,11,1800,499,-97,-7,16,121,B595E30,CE8A,100,3\r\n\
                   +QSCAN: \"LTE\",460,01,1800,499,-97,-7,16,121,B595E30,CE8A,100,3\r\n\
                   +QSCAN: \"LTE\",460,00,1300,261,-,-,-,0,F581B56,99D2,-,3\r\n\
                   +QSCAN: \"LTE\",460,00,1300,262,-93,-18,31,110,F581B57,99D2,100,3\r\nOK\r\n";
        let data = scan(raw);
        let nr = data["nr5g_cells_parsed"].as_array().unwrap();
        assert_eq!(nr.len(), 2);
        assert_eq!(nr[0]["provider"], "中国电信 / 中国联通");
        assert_eq!(nr[1]["provider"], "中国移动 / 中国广电");
        assert_eq!(nr[1]["scs"], 30);
        let lte = data["lte_cells_parsed"].as_array().unwrap();
        assert_eq!(
            lte.len(),
            3,
            "same EARFCN with another PCI is a different cell"
        );
        assert_eq!(lte[0]["provider"], "中国电信 / 中国联通");
    }
    #[test]
    fn neighbours_lists_serving_and_lockable_lte_neighbours() {
        let raw = "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,11,7127233,345,1600,3,5,5,CE8A,-78,-7,-50,16,0,-,40\r\n\
                   +QENG: \"neighbourcell intra\",\"LTE\",1600,346,-9,-84,-60,10,30,6,52,6,44\r\n\
                   +QENG: \"neighbourcell inter\",\"LTE\",38950,276,-3,-88,-65,0,37,7,16,6\r\n\
                   +QENG: \"neighbourcell inter\",\"LTE\",39148,-,-,-,-,-,37,0,30,7\r\nOK\r\n";
        let lte = neighbours(raw)["lte_cells_parsed"].clone();
        let cells = lte.as_array().unwrap();
        assert_eq!(cells.len(), 3, "entries without a PCI cannot be locked");
        assert_eq!(cells[0]["role"], "服务小区");
        assert_eq!(cells[0]["freq"], "1600");
        assert_eq!(cells[0]["pci"], "345");
        assert_eq!(cells[0]["band"], "3");
        assert_eq!(cells[0]["rsrp"], "-78");
        assert_eq!(cells[1]["role"], "同频邻区");
        assert_eq!(
            cells[1]["rsrp"], "-84",
            "neighbour rows list RSRQ before RSRP"
        );
        assert_eq!(cells[2]["band"], "40");
        // Captured in SA: the module reports only the serving cell.
        let sa = neighbours(
            "+QENG: \"servingcell\",\"NOCONN\",\"NR5G-SA\",\"FDD\",460,11,C9A82340B,751,C94B00,428910,1,6,-72,-10,33,0,-\r\nOK\r\nOK\r\n",
        );
        assert_eq!(sa["lte_cells_parsed"].as_array().unwrap().len(), 0);
        let nr = &sa["nr5g_cells_parsed"][0];
        assert_eq!(
            (
                &nr["freq"],
                &nr["pci"],
                &nr["band"],
                &nr["scs"],
                &nr["rsrp"]
            ),
            (
                &json!("428910"),
                &json!("751"),
                &json!("1"),
                &json!(15),
                &json!("-72")
            )
        );
    }
    #[test]
    fn lte_band_follows_earfcn_ranges() {
        for (earfcn, band) in [
            (100, 1),
            (1300, 3),
            (1800, 3),
            (2452, 5),
            (3590, 8),
            (38950, 40),
            (40936, 41),
        ] {
            assert_eq!(lte_band(earfcn), Some(band), "EARFCN {earfcn}");
        }
        assert_eq!(lte_band(70000), None);
    }
    #[test]
    fn cell_locks_reads_module_lock_values() {
        let unlocked =
            cell_locks("+QNWLOCK: \"common/4g\",0\r\nOK\r\n+QNWLOCK: \"common/5g\",0\r\nOK\r\n");
        assert!(unlocked["lte"].is_null() && unlocked["nr"].is_null());
        let locked = cell_locks(
            "+QNWLOCK: \"common/4g\",2,1300,262,1600,345\r\nOK\r\n+QNWLOCK: \"common/5g\",249,633984,30,78\r\nOK\r\n",
        );
        assert_eq!(locked["lte"], json!([2, 1300, 262, 1600, 345]));
        assert_eq!(locked["nr"], json!([249, 633984, 30, 78]));
    }
    #[test]
    fn traffic_direction_follows_quectel_sent_received_order() {
        for (sent, received) in [(1024, 8192), (0, 4096), (4096, 0), (0, 0)] {
            let data = dashboard(&format!("+QGDNRCNT: {sent},{received}\r\nOK\r\n"));
            assert_eq!(data["nr_tx_bytes"], sent, "upload is the first counter");
            assert_eq!(
                data["nr_rx_bytes"], received,
                "download is the second counter"
            );
            assert_eq!(data["nr_tx_human"], human_bytes(sent as f64));
            assert_eq!(data["nr_rx_human"], human_bytes(received as f64));
        }
    }
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
            let mut expected = c["expected"].clone();
            if c["kind"] == "dashboard" {
                // Preserve the historical Go fixtures, correcting their reversed traffic fields.
                for (rx, tx) in [
                    ("nr_rx_bytes", "nr_tx_bytes"),
                    ("nr_rx_human", "nr_tx_human"),
                ] {
                    expected[rx] = c["expected"][tx].clone();
                    expected[tx] = c["expected"][rx].clone();
                }
                // Go showed the raw LTE SINR; the manual converts it to dB as X * 2 - 20.
                if let Some(x) = expected["sinrLTE"]
                    .as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                {
                    let db = x * 2 - 20;
                    expected["sinrLTE"] = json!(db.to_string());
                    expected["sinrLTEPercentage"] = json!((db * 100 / 20).clamp(0, 100));
                }
            }
            assert_eq!(got, expected, "case {i} {} input: {raw}", c["kind"]);
        }
    }
    #[test]
    fn lte_sinr_matches_qcainfo_rssnr() {
        // Same moment from one modem: QENG reports 15, QCAINFO's RSSNR is 10 dB.
        let raw = "+QENG: \"servingcell\",\"NOCONN\"\r\n\
                   +QENG: \"LTE\",\"FDD\",420,03,29E5B01,445,1850,3,5,5,439E,-88,-7,-61,15,10,200,-\r\n\
                   +QCAINFO: \"PCC\",1850,100,\"LTE BAND 3\",1,445,-88,-8,-61,10\r\nOK\r\n";
        assert_eq!(dashboard(raw)["sinrLTE"], "10");
    }
    #[test]
    fn lte_servingcell_reads_earfcn_not_band() {
        let raw = "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,00,1A2B3C4,262,1300,3,5,5,58D0,-90,-9,-60,20,12,100,-\r\nOK\r\n";
        let d = dashboard(raw);
        assert_eq!(d["earfcns"], "1300");
        assert_eq!(d["pcc_pci"], "262");
        assert_eq!(d["sinrLTE"], "20");
    }
    #[test]
    fn nr_bandwidth_codes_follow_the_manual() {
        for (code, mhz) in [
            ("0", "5MHz"),
            ("1", "10MHz"),
            ("12", "100MHz"),
            ("13", "200MHz"),
            ("14", "400MHz"),
            ("15", "35MHz"),
            ("16", "45MHz"),
            ("17", "-"),
        ] {
            assert_eq!(bandwidth(code, true), mhz, "{code}");
        }
    }
    #[test]
    fn invalid_antenna_rsrp_is_hidden() {
        let d = dashboard("+QRSRP: -85,-32768,-90,-32768,NR5G\r\nOK\r\n");
        assert_eq!(d["prxqrsrp"], "-85");
        assert_eq!(d["drxqrsrp"], "-");
        assert_eq!(d["rx3qrsrp"], "-");
    }
    #[test]
    fn pdp_contexts_and_switches_from_an_rm520n() {
        // Captured from an RM520N-EU on China Telecom.
        let raw = "+CGDCONT: 1,\"IPV4V6\",\"ctnet\",\"0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0\",0,0,0,0,,,,,,,,,,\"\",,,,0\r\n\
                   +CGDCONT: 2,\"IPV4V6\",\"IMS\",\"0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0\",0,0,0,0,,,,,,,,,,\"\",,,,0\r\n\
                   +CGDCONT: 3,\"IPV4V6\",\"ctwap\",\"0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0\",0,0,0,0,,,,,,,,,,\"\",,,,0\r\nOK\r\n\
                   +CGACT: 1,1\r\n+CGACT: 2,1\r\n+CGACT: 3,0\r\nOK\r\n\
                   +CGPADDR: 1,\"10.80.4.9\",\"36.14.4.73.176.32.2.149.24.221.44.126.75.241.233.236\"\r\n\
                   +CGPADDR: 2,\"36.14.5.73.176.0.91.219.24.221.44.126.68.199.226.52\"\r\n\
                   +CGPADDR: 3,\"0.0.0.0\",\"0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0\"\r\nOK\r\n\
                   +QCFG: \"ims\",1,1\r\nOK\r\n\
                   +QSIMDET: 1,1\r\nOK\r\n+QNWPREFCFG: \"roam_pref\",255\r\nOK\r\n+QUIMSLOT: 1\r\nOK\r\n";
        let d = pdp(raw);
        let c = &d["contexts"];
        assert_eq!(c.as_array().unwrap().len(), 3);
        assert_eq!(c[0]["cid"], 1);
        assert_eq!(c[0]["type"], "IPV4V6");
        assert_eq!(c[0]["apn"], "ctnet");
        assert_eq!(c[0]["active"], true);
        assert_eq!(c[0]["ipv4"], "10.80.4.9");
        assert_eq!(c[0]["ipv6"], "240e:449:b020:295:18dd:2c7e:4bf1:e9ec");
        assert!(c[1]["ipv4"].is_null());
        assert_eq!(c[1]["ipv6"], "240e:549:b000:5bdb:18dd:2c7e:44c7:e234");
        assert_eq!(c[2]["active"], false);
        assert!(c[2]["ipv4"].is_null() && c[2]["ipv6"].is_null());
        assert_eq!(d["ims"], 1);
        assert_eq!(d["volte"], true);
        assert_eq!(d["sim_detect"], true);
        assert_eq!(d["sim_detect_level"], 1);
        assert_eq!(d["roaming"], 255);
        assert_eq!(d["sim_slot"], 1);
    }
    #[test]
    fn pdp_without_ims_support_keeps_the_rest() {
        let d = pdp(
            "+CGDCONT: 1,\"IP\",\"internet\",\"0.0.0.0\",0,0\r\nOK\r\nERROR\r\n+QUIMSLOT: 2\r\nOK\r\n",
        );
        assert!(d["ims"].is_null());
        assert!(d["contexts"][0]["active"].is_null());
        assert_eq!(d["sim_slot"], 2);
    }
}
