use crate::parser::{decode_ucs2, digits, fields, text};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::LazyLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Storage {
    #[default]
    ME,
    SM,
}
impl Storage {
    pub fn name(self) -> &'static str {
        match self {
            Self::ME => "ME",
            Self::SM => "SM",
        }
    }
    pub fn is_me(&self) -> bool {
        *self == Self::ME
    }
    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "" | "ME" => Ok(Self::ME),
            "SM" => Ok(Self::SM),
            _ => bail!("invalid SMS storage"),
        }
    }
    pub fn of(entry: &Value) -> Self {
        if entry["storage"] == "SM" {
            Self::SM
        } else {
            Self::ME
        }
    }
}

fn storage_blocks(raw: &str) -> Option<Vec<(Storage, &str)>> {
    if !raw.starts_with("+SASTORE:") {
        return None;
    }
    Some(
        raw.split("+SASTORE:")
            .skip(1)
            .filter_map(|block| {
                let (bank, data) = block.split_once('\n')?;
                Some((Storage::parse(bank.trim()).ok()?, data))
            })
            .collect(),
    )
}

pub fn normalize_number(number: &str, imsi: &str) -> Result<String> {
    let clean: String = number
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    let value =
        if !clean.is_empty() && clean.len() <= 6 && clean.bytes().all(|b| b.is_ascii_digit()) {
            clean
        } else if let Some(rest) = clean.strip_prefix("00") {
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
    if !(1..=21).contains(&value.len()) {
        bail!("invalid phone number")
    }
    Ok(value)
}
pub fn sent(raw: &str) -> bool {
    crate::parser::ok(raw)
        && raw.lines().any(|line| {
            line.trim()
                .strip_prefix("+CMGS:")
                .and_then(|v| v.trim().split(',').next())
                .is_some_and(|v| v.trim().parse::<u32>().is_ok())
        })
}
pub fn submit(number: &str, message: &str, reference: u8) -> Result<Vec<(String, usize)>> {
    let number_digits = digits(number);
    if number_digits.is_empty() || number_digits.len() > 20 {
        bail!("invalid number")
    }
    let gsm_units = message.chars().try_fold(0usize, |n, c| {
        gsm7_character(c).map(|(_, second)| n + 1 + usize::from(second.is_some()))
    });
    let gsm = gsm_units.is_some();
    let units = gsm_units.unwrap_or_else(|| message.encode_utf16().count());
    let limit = match (gsm, units) {
        (true, 0..=160) => 160,
        (true, _) => 153,
        (false, 0..=70) => 70,
        (false, _) => 67,
    };
    let mut segments = Vec::<Vec<u16>>::new();
    let mut segment = Vec::new();
    for c in message.chars() {
        let mut buf = [0; 2];
        let units = if gsm {
            let (first, second) = gsm7_character(c).unwrap();
            buf = [u16::from(first), u16::from(second.unwrap_or(0))];
            &buf[..1 + usize::from(second.is_some())]
        } else {
            c.encode_utf16(&mut buf)
        };
        if segment.len() + units.len() > limit {
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
        let udl = if gsm {
            let skip = (user.len() * 8).div_ceil(7);
            let count = skip + units.len();
            user.resize((count * 7).div_ceil(8), 0);
            for (offset, n) in units.into_iter().enumerate() {
                for bit in 0..7 {
                    let position = (skip + offset) * 7 + bit;
                    user[position / 8] |= (((n >> bit) & 1) as u8) << (position % 8);
                }
            }
            count
        } else {
            for n in units {
                user.extend(n.to_be_bytes())
            }
            user.len()
        };
        let mut data = vec![
            0,
            if total > 1 { 0x41 } else { 0x01 },
            (i + 1) as u8,
            number_digits.len() as u8,
            if number.starts_with('+') { 0x91 } else { 0x81 },
        ];
        data.extend(&address);
        data.extend([0, if gsm { 0 } else { 8 }, udl as u8]);
        data.extend(user);
        let len = data.len() - 1;
        out.push((hex::encode_upper(data), len));
    }
    Ok(out)
}
const GSM7_TABLE: &str = "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞ ÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà";
fn gsm7_character(c: char) -> Option<(u8, Option<u8>)> {
    if let Some(index) = GSM7_TABLE.chars().position(|v| v == c) {
        return Some((index as u8, None));
    }
    let extension = match c {
        '\x0c' => 10,
        '^' => 20,
        '{' => 40,
        '}' => 41,
        '\\' => 47,
        '[' => 60,
        '~' => 61,
        ']' => 62,
        '|' => 64,
        '€' => 101,
        _ => return None,
    };
    Some((27, Some(extension)))
}
fn gsm7(data: &[u8], count: usize, skip: usize) -> String {
    let table: Vec<char> = GSM7_TABLE.chars().collect();
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
// Forward only received messages; keep UDH fragments separate until all parts arrive.
pub fn received(raw: &str) -> Vec<Value> {
    if let Some(blocks) = storage_blocks(raw) {
        return blocks
            .into_iter()
            .flat_map(|(storage, data)| {
                received(data).into_iter().map(move |mut entry| {
                    entry["storage"] = json!(storage);
                    entry
                })
            })
            .collect();
    }
    let mut messages = Vec::new();
    for block in raw.split("+CMGL:").skip(1) {
        let mut lines = block.lines();
        let header = fields(lines.next().unwrap_or_default());
        if !matches!(
            header.get(1).map(String::as_str),
            Some("0" | "1" | "REC READ" | "REC UNREAD")
        ) {
            continue;
        }
        let Some(index) = header.first().and_then(|n| n.parse::<i64>().ok()) else {
            continue;
        };
        let body: Vec<_> = lines
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "OK")
            .collect();
        let entry = if let Some((entry, _)) = body.first().and_then(|pdu| deliver(pdu, index)) {
            Some(entry)
        } else if header.get(1).is_some_and(|s| s.starts_with("REC ")) {
            list(&format!("+CMGL:{block}"))["messages"]
                .as_array()
                .and_then(|v| v.first())
                .cloned()
        } else {
            None
        };
        if let Some(mut entry) = entry {
            entry["text"] = json!(newlines(text(&entry, "text")));
            messages.push(entry);
        }
    }
    messages
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
    if a["_pdu"] == true || b["_pdu"] == true {
        return false;
    }
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
    if let Some(blocks) = storage_blocks(raw) {
        let mut messages = Vec::new();
        let mut centers = Vec::new();
        for (storage, data) in blocks {
            let mut bank = list(data);
            for mut entry in bank["messages"].as_array_mut().unwrap().drain(..) {
                entry["storage"] = json!(storage);
                messages.push(entry);
            }
            centers.append(bank["serviceCenters"].as_array_mut().unwrap());
        }
        return json!({"messages":messages,"serviceCenters":centers});
    }
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
        if let Some((mut v, center)) = body.first().and_then(|v| deliver(v, index)) {
            if !center.is_empty() {
                centers.push(center)
            }
            v["_pdu"] = json!(true);
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
                for parts in crate::forwarding::split_fragments(group) {
                    messages.push(merge(parts));
                }
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
        v.as_object_mut().unwrap().remove("_pdu");
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
    fn vohive_submit_corpus_matches_byte_for_byte() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../tests/fixtures/vohive-sms-submit.json")).unwrap();
        for case in cases {
            let parts = submit(text(&case, "number"), text(&case, "message"), 1).unwrap();
            let expected = case["tpdus"].as_array().unwrap();
            assert_eq!(parts.len(), expected.len());
            for (i, (pdu, len)) in parts.iter().enumerate() {
                assert_eq!(
                    &pdu[2..].to_lowercase(),
                    expected[i].as_str().unwrap(),
                    "case {} part {i}",
                    text(&case, "message")
                );
                assert_eq!(*len as u64, case["lengths"][i].as_u64().unwrap());
            }
        }
    }
    #[test]
    fn short_codes_do_not_require_or_gain_an_imsi_country_code() {
        for number in ["10086", "888", "1"] {
            assert_eq!(normalize_number(number, "ERROR").unwrap(), number);
        }
        assert_eq!(
            normalize_number("13800138000", "460001234567890").unwrap(),
            "+8613800138000"
        );
    }
    #[test]
    fn modem_acknowledgement_is_required() {
        assert!(sent("+CMGS: 12\r\nOK\r\n"));
        for raw in [
            "OK",
            ">",
            "+CMGS: 1\r\n+CMS ERROR: 302",
            "+CMGS: nope\r\nOK",
            "+CMGS: 1",
        ] {
            assert!(!sent(raw));
        }
    }
    #[test]
    fn storage_indices_are_kept_separate() {
        let pdu = "00000D91683108108300F0000862908021436500044F60597D";
        let bank = format!("+CMGL: 1,0,,23\n{pdu}\nOK\n");
        let raw = format!("+SASTORE: ME\n{bank}+SASTORE: SM\n{bank}");
        let items = received(&raw);
        assert_eq!(items.len(), 2);
        assert_eq!(Storage::of(&items[0]), Storage::ME);
        assert_eq!(Storage::of(&items[1]), Storage::SM);
        assert_ne!(
            crate::forwarding::identity(&items[0]),
            crate::forwarding::identity(&items[1])
        );
        assert_eq!(list(&raw)["messages"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn separate_pdus_from_same_sender_are_not_merged() {
        let pdu = "00000D91683108108300F0000862908021436500044F60597D";
        let raw = format!("+CMGL: 1,0,,23\n{pdu}\n+CMGL: 2,0,,23\n{pdu}\nOK\n");
        let data = list(&raw);
        assert_eq!(data["messages"].as_array().unwrap().len(), 2);
        assert!(data["messages"][0].get("_pdu").is_none());
    }
    #[test]
    fn submit_uses_default_smsc_and_tpdu_octet_length() {
        let parts = submit("+8613800138000", "你好", 7).unwrap();
        assert_eq!(
            parts,
            [("0001010D91683108108300F00008044F60597D".into(), 18)]
        );
    }
    #[test]
    fn vohive_gsm7_receive_fixture_handles_spare_bits_and_storage_padding() {
        // VoHive fork pkg/smscodec/pdu_trim_test.go, pinned in docs/sms-compatibility.md.
        let pdu = "0004038101F100006250724190410A3754747A0E4ABBCD6F793B4C4FBFDDA0F41CE47ED341617B38CD0E8BD96590F92D07E5DF7539283C1EBFEB6E3A889E87971B";
        for suffix in [String::new(), "00".repeat(128)] {
            let raw = format!("+CMGL: 7,1,,69\r\n{pdu}{suffix}\r\nOK\r\n");
            let items = received(&raw);
            assert_eq!(items.len(), 1);
            assert_eq!(items[0]["sender"], "101");
            assert_eq!(
                items[0]["text"],
                "This information is not available for your account type"
            );
            assert_eq!(items[0]["indices"], json!([7]));
            assert!(items[0].get("concatRef").is_none());
        }
    }
    #[test]
    fn ucs2_receive_preserves_number_text_and_timestamp() {
        let raw = "+CMGL: 4,0,,23\r\n00000D91683108108300F0000862908021436500044F60597D\r\nOK\r\n";
        let items = received(raw);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["sender"], "+8613800138000");
        assert_eq!(items[0]["text"], "你好");
        assert_eq!(items[0]["date"], "26/09/08,12:34:56+00");
    }
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
