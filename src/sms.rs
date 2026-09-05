use crate::parser::{decode_ucs2, digits, fields, text};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::LazyLock};

pub fn normalize_number(number: &str, imsi: &str) -> Result<String> {
    let clean: String = number
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    let value = if let Some(rest) = clean.strip_prefix("00") {
        format!("+{}", digits(rest))
    } else if clean.starts_with('+') {
        format!("+{}", digits(&clean))
    } else {
        let clean = digits(&clean);
        if clean.is_empty() {
            bail!("missing number")
        }
        let imsi = imsi
            .lines()
            .map(digits)
            .find(|v| (10..=20).contains(&v.len()))
            .context("AT+CIMI did not return a valid IMSI; enter an international number")?;
        static CODES: LazyLock<HashMap<String, String>> =
            LazyLock::new(|| serde_json::from_str(include_str!("calling-codes.json")).unwrap());
        let code = CODES
            .get(&imsi[..3])
            .context("unknown MCC; enter an international number")?;
        format!("+{code}{clean}")
    };
    if !(2..=21).contains(&value.len()) {
        bail!("invalid phone number")
    }
    Ok(value)
}
pub fn submit(number: &str, message: &str, reference: u8) -> Result<Vec<(String, usize)>> {
    let number_digits = digits(number);
    if number_digits.is_empty() || number_digits.len() > 20 {
        bail!("invalid number")
    }
    let mut segments = Vec::<Vec<u16>>::new();
    let mut segment = Vec::new();
    for c in message.chars() {
        let mut buf = [0; 2];
        let units = c.encode_utf16(&mut buf);
        if segment.len() + units.len() > 67 {
            segments.push(std::mem::take(&mut segment))
        }
        segment.extend_from_slice(units)
    }
    if !segment.is_empty() {
        segments.push(segment)
    }
    if segments.is_empty() || segments.len() > 255 {
        bail!("message is empty or too long")
    }
    let mut padded = number_digits.clone();
    if padded.len() % 2 == 1 {
        padded.push('F')
    }
    let address: Vec<u8> = padded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| {
            ((p[1] as char).to_digit(16).unwrap() * 16 + (p[0] as char).to_digit(16).unwrap()) as u8
        })
        .collect();
    let total = segments.len();
    let mut out = Vec::new();
    for (i, units) in segments.into_iter().enumerate() {
        let mut user = Vec::new();
        if total > 1 {
            user.extend([5, 0, 3, reference.max(1), total as u8, (i + 1) as u8])
        }
        for n in units {
            user.extend(n.to_be_bytes())
        }
        let mut data = vec![
            0,
            if total > 1 { 0x51 } else { 0x11 },
            0,
            number_digits.len() as u8,
            if number.starts_with('+') { 0x91 } else { 0x81 },
        ];
        data.extend(&address);
        data.extend([0, 8, 0xAA, user.len() as u8]);
        data.extend(user);
        let len = data.len() - 1;
        out.push((hex::encode_upper(data), len));
    }
    Ok(out)
}
fn gsm7(data: &[u8], count: usize, skip: usize) -> String {
    const TABLE: &str = "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞ ÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà";
    let table: Vec<char> = TABLE.chars().collect();
    let mut result = String::new();
    let mut escape = false;
    for i in 0..count {
        let bit = (skip + i) * 7;
        if bit + 7 > data.len() * 8 {
            break;
        }
        let mut n = 0;
        for j in 0..7 {
            n |= ((data[(bit + j) / 8] >> ((bit + j) % 8)) & 1) << j
        }
        if escape {
            result.push(match n {
                10 => '\x0c',
                20 => '^',
                40 => '{',
                41 => '}',
                47 => '\\',
                60 => '[',
                61 => '~',
                62 => ']',
                64 => '|',
                101 => '€',
                _ => ' ',
            });
            escape = false
        } else if n == 27 {
            escape = true
        } else {
            result.push(table[n as usize])
        }
    }
    result
}
fn address(raw: &[u8], count: usize, toa: u8) -> String {
    if toa & 0x70 == 0x50 {
        return gsm7(raw, count * 4 / 7, 0);
    }
    let mut value = String::new();
    for b in raw {
        for n in [b & 15, b >> 4] {
            if n <= 9 {
                value.push((b'0' + n) as char)
            }
        }
    }
    value.truncate(value.len().min(count));
    if toa & 0x90 == 0x90 && !value.is_empty() {
        value.insert(0, '+')
    }
    value
}
fn timestamp(raw: &[u8]) -> String {
    let d = |b: u8| (b & 15) * 10 + (b >> 4);
    format!(
        "{:02}/{:02}/{:02},{:02}:{:02}:{:02}{}{:02}",
        d(raw[0]),
        d(raw[1]),
        d(raw[2]),
        d(raw[3]),
        d(raw[4]),
        d(raw[5]),
        if raw[6] & 8 != 0 { "-" } else { "+" },
        d(raw[6] & !8)
    )
}
fn deliver(raw: &str, index: i64) -> Option<(Value, String)> {
    let bytes = hex::decode(raw.replace(' ', "")).ok()?;
    let mut pos = 0;
    fn take<'a>(b: &'a [u8], p: &mut usize, n: usize) -> Option<&'a [u8]> {
        let value = b.get(*p..p.checked_add(n)?)?;
        *p += n;
        Some(value)
    }
    let len = take(&bytes, &mut pos, 1)?[0] as usize;
    let smsc = take(&bytes, &mut pos, len)?;
    let center = if len > 0 {
        address(&smsc[1..], (len - 1) * 2, smsc[0])
    } else {
        String::new()
    };
    let first = take(&bytes, &mut pos, 1)?[0];
    if first & 3 != 0 {
        return None;
    }
    let header = take(&bytes, &mut pos, 2)?;
    let count = header[0] as usize;
    let toa = header[1];
    let raw_address = take(&bytes, &mut pos, count.div_ceil(2))?;
    let sender = address(raw_address, count, toa);
    let dcs = take(&bytes, &mut pos, 2)?[1];
    let date = timestamp(take(&bytes, &mut pos, 7)?);
    let udl = take(&bytes, &mut pos, 1)?[0] as usize;
    let octets = if dcs & 12 == 0 {
        (udl * 7).div_ceil(8)
    } else {
        udl
    };
    let user = take(&bytes, &mut pos, octets)?;
    let mut start = 0;
    let (mut reference, mut total, mut sequence) = (String::new(), 0, 0);
    if first & 0x40 != 0 {
        start = *user.first()? as usize + 1;
        let udh = user.get(1..start)?;
        let mut p = 0;
        while p + 2 <= udh.len() {
            let kind = udh[p];
            let n = udh[p + 1] as usize;
            p += 2;
            let data = udh.get(p..p + n)?;
            p += n;
            if kind == 0 && n == 3 {
                reference = format!("8:{:02X}", data[0]);
                total = data[1];
                sequence = data[2]
            }
            if kind == 8 && n == 4 {
                reference = format!("16:{:02X}{:02X}", data[0], data[1]);
                total = data[2];
                sequence = data[3]
            }
        }
    }
    let content = &user[start..];
    let body = match dcs & 12 {
        8 => {
            let units: Vec<_> = content
                .as_chunks::<2>()
                .0
                .iter()
                .map(|v| u16::from_be_bytes([v[0], v[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        4 => hex::encode_upper(content),
        _ => {
            let skip = (start * 8).div_ceil(7);
            gsm7(user, udl.saturating_sub(skip), skip)
        }
    };
    let mut entry = json!({"sender":sender,"date":date,"text":body,"indices":[index]});
    if !reference.is_empty() {
        entry["concatRef"] = json!(format!("{sender}:{reference}"));
        entry["concatTotal"] = json!(total);
        entry["concatSeq"] = json!(sequence)
    }
    Some((entry, center))
}
fn newlines(raw: &str) -> String {
    let mut value = raw.replace("\r\n", "\n");
    for c in ['\r', '\x0b', '\x0c', '\u{85}', '\u{2028}', '\u{2029}'] {
        value = value.replace(c, "\n")
    }
    value
        .replace("\\r\\n", "\n")
        .replace("\\n", "\n")
        .replace("\\r", "\n")
}
fn merge(mut group: Vec<Value>) -> Value {
    if group.len() == 1 {
        return group.remove(0);
    }
    let mut message = group[0].clone();
    message.as_object_mut().unwrap().remove("concatSeq");
    group.sort_by_key(|v| {
        (
            v["concatSeq"].as_i64().unwrap_or(0),
            v["indices"][0].as_i64().unwrap_or(0),
        )
    });
    let mut indices = Vec::new();
    let mut body = String::new();
    for v in group {
        body.push_str(text(&v, "text"));
        indices.extend(
            v["indices"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_i64),
        )
    }
    indices.sort();
    message["text"] = json!(body);
    message["indices"] = json!(indices);
    message
}
fn adjacent_fragment(a: &Value, b: &Value) -> bool {
    if text(a, "sender").is_empty() || text(a, "sender") != text(b, "sender") {
        return false;
    }
    let date_a = text(a, "date");
    let date_b = text(b, "date");
    let close = if !date_a.is_empty() && date_a == date_b {
        true
    } else {
        let parse =
            |s: &str| chrono::NaiveDateTime::parse_from_str(s.get(..17)?, "%y/%m/%d,%H:%M:%S").ok();
        match (parse(date_a), parse(date_b)) {
            (Some(a), Some(b)) => (a - b).num_seconds().abs() <= 5,
            _ => false,
        }
    };
    close
        && a["indices"]
            .as_array()
            .and_then(|v| v.last())
            .and_then(Value::as_i64)
            .is_some_and(|last| b["indices"][0].as_i64() == Some(last + 1))
}
pub fn list(raw: &str) -> Value {
    let lines: Vec<_> = raw.lines().map(str::trim).collect();
    let mut entries = Vec::new();
    let mut centers = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if let Some(tail) = line.strip_prefix("+CSCA:")
            && let Some(v) = fields(tail).first()
        {
            centers.push(decode_ucs2(v))
        }
        let Some(tail) = line.strip_prefix("+CMGL:") else {
            continue;
        };
        let p = fields(tail);
        let Some(index) = p.first() else { continue };
        let index = index.parse::<i64>().unwrap_or(0);
        let mut body = Vec::new();
        while i < lines.len() && !lines[i].starts_with("+CMGL:") {
            let l = lines[i];
            i += 1;
            if !l.is_empty()
                && l != "OK"
                && !l.starts_with('+')
                && !l.to_ascii_uppercase().starts_with("AT+")
            {
                body.push(l)
            }
        }
        if let Some((v, center)) = body.first().and_then(|v| deliver(v, index)) {
            if !center.is_empty() {
                centers.push(center)
            }
            entries.push(v);
            continue;
        }
        let compact = body.join("");
        let decoded = decode_ucs2(&compact);
        let body = if decoded != compact {
            decoded
        } else {
            body.join("\n")
        };
        entries.push(json!({"sender":p.get(2).map(|v|decode_ucs2(v)).unwrap_or_default(),"date":p.get(4).or(p.get(3)).cloned().unwrap_or_default(),"text":body,"indices":[index]}));
    }
    let mut messages = Vec::new();
    let mut groups: HashMap<String, Vec<Value>> = HashMap::new();
    for v in &entries {
        let key = text(v, "concatRef");
        if !key.is_empty() {
            groups.entry(key.into()).or_default().push(v.clone())
        }
    }
    let mut fragments = Vec::new();
    for v in entries {
        let key = text(&v, "concatRef");
        if !key.is_empty() {
            if let Some(group) = groups.remove(key) {
                if !fragments.is_empty() {
                    messages.push(merge(std::mem::take(&mut fragments)));
                }
                messages.push(merge(group))
            }
        } else {
            if fragments
                .last()
                .is_some_and(|previous| !adjacent_fragment(previous, &v))
            {
                messages.push(merge(std::mem::take(&mut fragments)));
            }
            fragments.push(v)
        }
    }
    if !fragments.is_empty() {
        messages.push(merge(fragments));
    }
    for v in &mut messages {
        let normalized = newlines(text(v, "text"));
        v["textLines"] = json!(normalized.split('\n').collect::<Vec<_>>());
        v["text"] = json!(normalized)
    }
    json!({"messages":messages,"serviceCenters":centers})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf16_segments_fit_sms() {
        let result = submit("+8613800138000", &"😀".repeat(100), 1).unwrap();
        assert_eq!(result.len(), 4);
        for (pdu, _) in result {
            assert!(hex::decode(pdu).unwrap().len() <= 155)
        }
    }
    #[test]
    fn go_sms_contracts() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../tests/fixtures/go-sms.json")).unwrap();
        for (i, c) in cases.iter().enumerate() {
            let mut expected = c["expected"].clone();
            // Go's fmt.Sprint on a missing map key emits "<nil>"; it is not a real UDH reference.
            for message in expected["messages"].as_array_mut().unwrap() {
                if message["concatRef"] == "<nil>" {
                    message.as_object_mut().unwrap().remove("concatRef");
                }
            }
            assert_eq!(list(c["raw"].as_str().unwrap()), expected, "SMS case {i}")
        }
    }
    #[test]
    fn unrelated_legacy_messages_are_not_combined() {
        let raw = "+CMGL: 0,\"REC READ\",\"10001\",,\"26/05/20,20:17:07+32\"\nHello\n+CMGL: 1,\"REC READ\",\"10002\",,\"26/05/20,20:17:07+32\"\nWorld\nOK";
        assert_eq!(list(raw)["messages"].as_array().unwrap().len(), 2);
        let raw = raw
            .replace("10002", "10001")
            .replace("1,\"REC READ\"", "3,\"REC READ\"");
        assert_eq!(list(&raw)["messages"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn truncated_pdus_do_not_panic() {
        for n in 0..256 {
            let raw = "00".repeat(n);
            let _ = deliver(&raw, 1);
        }
    }
}
