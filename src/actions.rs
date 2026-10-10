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
/// FR1 SA cells use 15 or 30 kHz and FR2 cells 60 or 120 kHz. Quectel warns that
/// locking a cell with an SCS its band does not support crashes the module.
pub fn nr_scs_valid(band: u32, scs: u32) -> bool {
    if band < 257 {
        matches!(scs, 15 | 30)
    } else {
        matches!(scs, 60 | 120)
    }
}
fn nr_lock(p: &Params) -> Result<String> {
    // Never guess the SCS: a wrong value can crash the module.
    if p.get("scs").is_empty() {
        bail!("missing NR subcarrier spacing")
    }
    let scs = p.integer("scs", 15, 120)?;
    let band = p.integer("band", 1, 1024)?;
    if !nr_scs_valid(band, scs) {
        bail!("subcarrier spacing {scs} kHz is not valid for n{band}")
    }
    Ok(format!(
        "AT+QNWLOCK=\"common/5g\",{},{},{scs},{band}",
        p.integer("pci", 0, 1007)?,
        p.integer("earfcn", 0, 3279165)?,
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
/// Checks a PDP type and APN before they are put inside AT+CGDCONT quotes.
fn pdp_context(pdp: &str, apn: &str) -> Result<()> {
    if !["IP", "IPV6", "IPV4V6"].contains(&pdp) {
        bail!("invalid pdp type")
    }
    if apn.len() > 100
        || !apn
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
    {
        bail!("invalid APN")
    }
    Ok(())
}
/// AT+QSIMDET keeps the insert level the board was configured with; a wrong level makes the
/// module report a missing SIM.
pub fn sim_detect(enabled: bool, level: u32) -> Result<String> {
    if level > 1 {
        bail!("invalid SIM detection level")
    }
    Ok(format!("AT+QSIMDET={},{level}", u32::from(enabled)))
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
        "lock_nr_manual" => nr_lock(p)?,
        "lock_scanned_cells" => match p.get("mode") {
            "NR5G Only" => nr_lock(p)?,
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
        "pdp_save" => {
            let pdp = p.get("pdpType").to_ascii_uppercase();
            let apn = p.get("apn").trim();
            pdp_context(&pdp, apn)?;
            format!(
                "AT+CGDCONT={},\"{pdp}\",\"{apn}\"",
                p.integer("cid", 1, 42)?
            )
        }
        "pdp_delete" => format!("AT+CGDCONT={}", p.integer("cid", 1, 42)?),
        "pdp_activate" => format!("AT+CGACT=1,{}", p.integer("cid", 1, 42)?),
        "pdp_deactivate" => format!("AT+CGACT=0,{}", p.integer("cid", 1, 42)?),
        // 0 follows the MBN, 1 forces IMS on, 2 forces it off; applies after a reboot.
        "ims" => format!("AT+QCFG=\"ims\",{}", p.integer("mode", 0, 2)?),
        "roaming" => format!(
            "AT+QNWPREFCFG=\"roam_pref\",{}",
            if p.flag("enabled", true) { 255 } else { 1 }
        ),
        "sim_slot" => format!("AT+QUIMSLOT={}", p.integer("slot", 1, 2)?),
        "save_settings" => {
            let mut commands = Vec::new();
            let mut pdp = p.get("pdpType").to_ascii_uppercase();
            let apn = p.get("apn").trim();
            if !pdp.is_empty() || !apn.is_empty() {
                if pdp.is_empty() {
                    pdp = "IPV4V6".into()
                }
                pdp_context(&pdp, apn)?;
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
                // The manual requires <IP_family> with <enable>.
                "AT+QMAP=\"DMZ\",0,4".into()
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
    #[test]
    fn nr_lock_requires_an_scs_the_band_supports() {
        for (query, ok) in [
            ("mode=NR5G+Only&pci=5&earfcn=633984&band=78", false),
            ("mode=NR5G+Only&pci=5&earfcn=633984&scs=60&band=78", false),
            ("mode=NR5G+Only&pci=5&earfcn=633984&scs=240&band=78", false),
            ("mode=NR5G+Only&pci=5&earfcn=152650&scs=15&band=28", true),
            ("mode=NR5G+Only&pci=5&earfcn=2079165&scs=120&band=257", true),
            ("mode=NR5G+Only&pci=5&earfcn=2079165&scs=30&band=257", false),
        ] {
            let p = Params::parse(&format!("action=lock_scanned_cells&{query}"), "").unwrap();
            assert_eq!(network(&p).is_ok(), ok, "{query}");
        }
    }
    #[test]
    fn pdp_and_switch_commands() {
        let cmd = |q: &str| network(&Params::parse(q, "").unwrap());
        assert_eq!(
            cmd("action=pdp_save&cid=3&pdpType=ipv4v6&apn=ctwap").unwrap(),
            "AT+CGDCONT=3,\"IPV4V6\",\"ctwap\""
        );
        assert_eq!(
            cmd("action=pdp_save&cid=5&pdpType=IP&apn=").unwrap(),
            "AT+CGDCONT=5,\"IP\",\"\""
        );
        assert_eq!(cmd("action=pdp_delete&cid=3").unwrap(), "AT+CGDCONT=3");
        assert_eq!(cmd("action=pdp_activate&cid=3").unwrap(), "AT+CGACT=1,3");
        assert_eq!(cmd("action=pdp_deactivate&cid=1").unwrap(), "AT+CGACT=0,1");
        assert_eq!(cmd("action=ims&mode=2").unwrap(), "AT+QCFG=\"ims\",2");
        assert_eq!(
            cmd("action=roaming&enabled=0").unwrap(),
            "AT+QNWPREFCFG=\"roam_pref\",1"
        );
        assert_eq!(
            cmd("action=roaming&enabled=1").unwrap(),
            "AT+QNWPREFCFG=\"roam_pref\",255"
        );
        assert_eq!(cmd("action=sim_slot&slot=2").unwrap(), "AT+QUIMSLOT=2");
        for bad in [
            "action=pdp_save&cid=0&pdpType=IP&apn=a",
            "action=pdp_save&cid=43&pdpType=IP&apn=a",
            "action=pdp_save&cid=1&pdpType=PPP&apn=a",
            "action=pdp_save&cid=1&pdpType=IP&apn=a%22%3B%2BCFUN%3D0",
            "action=pdp_activate&cid=x",
            "action=ims&mode=3",
            "action=sim_slot&slot=3",
        ] {
            assert!(cmd(bad).is_err(), "{bad}");
        }
        assert_eq!(sim_detect(false, 1).unwrap(), "AT+QSIMDET=0,1");
        assert_eq!(sim_detect(true, 0).unwrap(), "AT+QSIMDET=1,0");
        assert!(sim_detect(true, 2).is_err());
    }
}
