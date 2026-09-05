use anyhow::{Context, Result, bail};
use std::net::Ipv4Addr;

#[derive(Default)]
pub struct Params(pub Vec<(String, String)>);
impl Params {
    pub fn parse(query: &str, body: &str) -> Result<Self> {
        let mut values: Vec<(String, String)> = serde_urlencoded::from_str(body)?;
        values.extend(serde_urlencoded::from_str::<Vec<(String, String)>>(query)?);
        Ok(Self(values))
    }
    pub fn get(&self, key: &str) -> &str {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }
    pub fn list(&self, key: &str, separator: char) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(k, _)| k == key || k == &format!("{key}[]"))
            .flat_map(|(_, v)| v.split(separator))
            .filter(|s| !s.is_empty())
            .collect()
    }
    pub fn flag(&self, key: &str, default: bool) -> bool {
        match self.get(key).to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        }
    }
    pub fn integer(&self, key: &str, min: u32, max: u32) -> Result<u32> {
        number(self.get(key), min, max)
    }
}
fn number(raw: &str, min: u32, max: u32) -> Result<u32> {
    let n = raw
        .trim()
        .parse::<u32>()
        .context("invalid numeric parameter")?;
    if n < min || n > max {
        bail!("numeric parameter out of range")
    }
    Ok(n)
}
fn bands(value: &str) -> Result<String> {
    if value.is_empty() {
        bail!("missing bands")
    }
    let values: Result<Vec<_>> = value
        .split(':')
        .map(|v| number(v, 1, 1024).map(|n| n.to_string()))
        .collect();
    Ok(values?.join(":"))
}
pub fn band_mode(mode: &str) -> Result<&'static str> {
    match mode.to_ascii_uppercase().as_str() {
        "LTE" => Ok("lte_band"),
        "NSA" => Ok("nsa_nr5g_band"),
        "SA" => Ok("nr5g_band"),
        _ => bail!("invalid mode"),
    }
}
pub fn scan_mode(mode: &str) -> Result<&'static str> {
    match mode {
        "Full Scan" => Ok("AT+QSCAN=3,1"),
        "LTE Only" => Ok("AT+QSCAN=1,1"),
        "NR5G Only" => Ok("AT+QSCAN=2,1"),
        _ => bail!("invalid scan mode"),
    }
}
fn nr_lock(p: &Params, scanned: bool) -> Result<String> {
    let scs = if scanned && p.get("scs").is_empty() {
        30
    } else {
        p.integer("scs", 15, 240)?
    };
    Ok(format!(
        "AT+QNWLOCK=\"common/5g\",{},{},{scs},{}",
        p.integer("pci", 0, 1007)?,
        p.integer("earfcn", 0, 3279165)?,
        p.integer("band", 1, 1024)?
    ))
}
fn lte_lock(pairs: Vec<(&str, &str)>) -> Result<String> {
    if pairs.is_empty() || pairs.len() > 10 {
        bail!("invalid LTE cell count")
    }
    let mut items = vec![pairs.len().to_string()];
    for (freq, pci) in pairs {
        items.push(number(freq, 0, 262143)?.to_string());
        items.push(number(pci, 0, 503)?.to_string())
    }
    Ok(format!("AT+QNWLOCK=\"common/4g\",{}", items.join(",")))
}
pub fn imei(p: &Params) -> Result<String> {
    let v = p.get("imei");
    if v.len() != 15 || !v.bytes().all(|c| c.is_ascii_digit()) {
        bail!("invalid imei")
    }
    Ok(format!("AT+EGMR=1,7,\"{v}\";+CFUN=1,1"))
}
pub fn network(p: &Params) -> Result<String> {
    Ok(match p.get("action") {
        "lock_bands" => format!(
            "AT+QNWPREFCFG=\"{}\",{}",
            band_mode(p.get("mode"))?,
            bands(p.get("values"))?
        ),
        "reset_bands" => format!(
            "AT+QNWPREFCFG=\"lte_band\",{};+QNWPREFCFG= \"nsa_nr5g_band\",{};+QNWPREFCFG= \"nr5g_band\",{}",
            bands(p.get("lte"))?,
            bands(p.get("nsa"))?,
            bands(p.get("sa"))?
        ),
        "unlock_lte" => "AT+QNWLOCK=\"common/4g\",0".into(),
        "unlock_nr" => "AT+QNWLOCK=\"common/5g\",0".into(),
        "lock_nr_manual" => nr_lock(p, false)?,
        "lock_scanned_cells" => match p.get("mode") {
            "NR5G Only" => nr_lock(p, true)?,
            "LTE Only" => {
                let f = p.list("earfcn", ',');
                let pci = p.list("pci", ',');
                if f.len() != pci.len() {
                    bail!("invalid LTE pairs")
                }
                lte_lock(f.into_iter().zip(pci).collect())?
            }
            _ => bail!("invalid scan mode"),
        },
        "lock_lte_manual" => {
            let values = p.list("pairs", ';');
            let pairs: Result<Vec<_>> = values
                .into_iter()
                .map(|v| v.split_once(',').context("invalid LTE pair"))
                .collect();
            let pairs = pairs?;
            if !p.get("cellNum").is_empty() && p.integer("cellNum", 1, 10)? as usize != pairs.len()
            {
                bail!("LTE cell count mismatch")
            }
            lte_lock(pairs)?
        }
        "save_settings" => {
            let mut commands = Vec::new();
            let mut pdp = p.get("pdpType").to_ascii_uppercase();
            let apn = p.get("apn").trim();
            if !pdp.is_empty() || !apn.is_empty() {
                if pdp.is_empty() {
                    pdp = "IPV4V6".into()
                }
                if !["IP", "IPV6", "IPV4V6"].contains(&pdp.as_str()) {
                    bail!("invalid pdp type")
                }
                if apn.len() > 100
                    || !apn
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
                {
                    bail!("invalid APN")
                }
                commands.push(format!("+CGDCONT=1,\"{pdp}\",\"{apn}\""));
            }
            let mode = p.get("modePref");
            if !mode.is_empty() {
                if !mode.bytes().all(|c| c.is_ascii_alphanumeric() || c == b':') {
                    bail!("invalid network mode")
                }
                commands.push(format!("+QNWPREFCFG=\"mode_pref\",{mode}"))
            }
            if !p.get("nrDisableMode").is_empty() {
                commands.push(format!(
                    "+QNWPREFCFG=\"nr5g_disable_mode\",{}",
                    p.integer("nrDisableMode", 0, 2)?
                ))
            }
            if commands.is_empty() {
                bail!("no changes")
            }
            format!("AT{}", commands.join(";"))
        }
        _ => bail!("unsupported action"),
    })
}
pub fn settings(p: &Params) -> Result<Vec<String>> {
    let action = p.get("action");
    if action == "ip_passthrough" && !p.flag("enabled", true) {
        return Ok(vec![
            "AT+QMAP=\"MPDN_RULE\",0".into(),
            "AT+QMAPWAC=1".into(),
            "AT+CFUN=1,1".into(),
        ]);
    }
    let command = match action {
        "set_imei" => imei(p)?,
        "reboot" => "AT+CFUN=1,1".into(),
        "reset_at" => "AT&F".into(),
        "manual_at" => {
            let value = p.get("command").trim();
            if value.is_empty() {
                "ATI".into()
            } else {
                value.into()
            }
        }
        "ip_passthrough" => {
            let code = match p.get("mode").to_ascii_uppercase().as_str() {
                "ETH" => 1,
                "USB" => 3,
                _ => bail!("invalid passthrough mode"),
            };
            format!("AT+QMAP=\"MPDN_RULE\",0,1,0,{code},1,\"FF:FF:FF:FF:FF:FF\"")
        }
        "dns_proxy" => {
            let family = p.get("family");
            if family != "4" && family != "6" {
                bail!("invalid dns family")
            }
            format!(
                "AT+QMAP=\"DHCPV{family}DNS\",\"{}\"",
                if p.flag("enabled", false) {
                    "enable"
                } else {
                    "disable"
                }
            )
        }
        "usbnet" => {
            let code = match p.get("mode").to_ascii_uppercase().as_str() {
                "RMNET" => 0,
                "ECM" => 1,
                "MBIM" => 2,
                "RNDIS" => 3,
                _ => bail!("invalid usbnet mode"),
            };
            format!("AT+QCFG=\"usbnet\",{code}")
        }
        "dmz" => {
            if p.flag("enabled", false) {
                format!(
                    "AT+QMAP=\"DMZ\",1,4,{}",
                    p.get("ip").parse::<Ipv4Addr>().context("invalid dmz ip")?
                )
            } else {
                "AT+QMAP=\"DMZ\",0".into()
            }
        }
        "lanip" => {
            let mut ips = Vec::new();
            for key in ["start", "end", "gateway"] {
                ips.push(
                    p.get(key)
                        .parse::<Ipv4Addr>()
                        .context("invalid lan ip")?
                        .to_string(),
                )
            }
            format!("AT+QMAP=\"LANIP\",{}", ips.join(","))
        }
        _ => bail!("unsupported action"),
    };
    Ok(vec![command])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn go_action_contracts() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/fixtures/go-actions.json")).unwrap();
        for c in cases {
            let p = Params::parse(c["query"].as_str().unwrap(), "").unwrap();
            let commands = match c["page"].as_str().unwrap() {
                "device" => vec![imei(&p).unwrap()],
                "network" => vec![network(&p).unwrap()],
                "settings" => settings(&p).unwrap(),
                _ => unreachable!(),
            };
            assert_eq!(serde_json::json!(commands), c["commands"], "{}", c["query"]);
        }
    }
    #[test]
    fn rejects_injected_parameters() {
        let p = Params(vec![
            ("action".into(), "save_settings".into()),
            ("apn".into(), "x\";+CFUN=1,1".into()),
        ]);
        assert!(network(&p).is_err());
    }
    #[test]
    fn zero_pci_is_valid() {
        let p = Params::parse(
            "action=lock_nr_manual&pci=0&earfcn=633984&scs=30&band=78",
            "",
        )
        .unwrap();
        assert_eq!(
            network(&p).unwrap(),
            "AT+QNWLOCK=\"common/5g\",0,633984,30,78"
        );
    }
}
