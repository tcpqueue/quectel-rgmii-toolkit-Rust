use std::time::Duration;
pub const PENDING: &str = "AT command is running in background; cached data is not ready yet.";
pub fn action(command: &str) -> bool {
    let upper = command.to_ascii_uppercase();
    if upper == "AT&F" || upper.contains(";AT&F") {
        return true;
    }
    [
        "+CFUN=",
        "+EGMR=",
        "+CMGD",
        "+CMGS",
        "+QSCAN=",
        "+CGDCONT=",
        "+QMAPWAC=",
        "+QCFG=\"USBNET\",",
        "+QMAP=\"MPDN_RULE\",0",
        "+QMAP=\"DHCPV6DNS\",",
        "+QMAP=\"DHCPV4DNS\",",
        "+QMAP=\"DMZ\",",
        "+QMAP=\"LANIP\",",
        "+QNWPREFCFG=\"LTE_BAND\",",
        "+QNWPREFCFG=\"NSA_NR5G_BAND\",",
        "+QNWPREFCFG=\"NR5G_BAND\",",
        "+QNWPREFCFG=\"MODE_PREF\",",
        "+QNWPREFCFG=\"NR5G_DISABLE_MODE\",",
        "+QNWLOCK=\"COMMON/4G\",",
        "+QNWLOCK=\"COMMON/5G\", ",
    ]
    .iter()
    .any(|p| upper.contains(p.trim_end()))
}
pub fn timeout(command: &str) -> Duration {
    let parts = crate::at::split(command);
    Duration::from_millis(
        parts
            .iter()
            .map(|p| {
                let up = p.trim().to_ascii_uppercase();
                // The manual allows 5 s for any MPDN_rule write, enabling IP passthrough included.
                if up.contains("+QMAP=\"MPDN_RULE\",") {
                    10000
                } else if up.contains("QSCAN") {
                    120000
                } else if up.contains("+QNWLOCK=") && up.contains("\",") {
                    // Setting or clearing a lock re-selects the cell; an RM520N-EU answers after ~1.3 s.
                    10000
                } else {
                    1000
                }
            })
            .sum::<u64>()
            .max(1000),
    )
}
pub fn max_age(command: &str) -> Duration {
    let up = command.trim().to_ascii_uppercase();
    let contains = |items: &[&str]| items.iter().all(|s| up.contains(s));
    let secs = if action(command) {
        0
    } else if up == "AT+CGMM" || up == "AT+CGMI;+CGSN;+QGMR;+CIMI;+ICCID;+CNUM" {
        600
    } else if up == "AT+QMAP=\"LANIP\""
        || contains(&[
            "+QMAP=\"MPDN_RULE\"",
            "+QMAP=\"DHCPV6DNS\"",
            "+QCFG=\"USBNET\"",
            "+QMAP=\"DMZ\"",
            "+QMAP=\"DHCPV4DNS\"",
        ])
        || contains(&[
            "+QNWPREFCFG=\"LTE_BAND\"",
            "+QNWPREFCFG= \"NSA_NR5G_BAND\"",
            "+QNWPREFCFG= \"NR5G_BAND\"",
        ])
    {
        60
    } else if contains(&[
        "+QNWPREFCFG=\"MODE_PREF\"",
        "+QNWPREFCFG=\"NR5G_DISABLE_MODE\"",
        "+CGDCONT?",
        "+CGCONTRDP=1",
        "+QNWLOCK=\"COMMON/4G\"",
        "+QNWLOCK=\"COMMON/5G\"",
    ]) {
        30
    } else if sms(command) {
        5
    } else if [
        "+QSIMSTAT",
        "+CPIN",
        "+QMAP=\"WWAN\"",
        "+QCAINFO",
        "+QENG",
        "+QRSRP",
        "+CSQ",
        "+QTEMP",
    ]
    .iter()
    .any(|s| up.contains(s))
    {
        3
    } else if ["+CIMI", "+ICCID", "+CNUM", "+CGMI", "+CGSN", "+QGMR"]
        .iter()
        .any(|s| up.contains(s))
    {
        600
    } else {
        3
    };
    Duration::from_secs(secs)
}
pub fn sms(command: &str) -> bool {
    let up = command.to_ascii_uppercase();
    up.contains("+CMGL=4") || up.contains("+CMGL=\"ALL\"")
}
pub fn immediate(command: &str) -> bool {
    command.trim().eq_ignore_ascii_case("AT+CGMM")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn go_policy_contracts() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/fixtures/go-at-policy.json")).unwrap();
        for c in cases {
            let cmd = c["command"].as_str().unwrap();
            assert_eq!(action(cmd), c["action"].as_bool().unwrap(), "action {cmd}");
            // Go waited 1 s for lock and MPDN_rule writes, shorter than a real modem needs.
            let expected = if (cmd.contains("+QNWLOCK=") && cmd.contains("\","))
                || cmd.to_ascii_uppercase().contains("+QMAP=\"MPDN_RULE\",")
            {
                10000
            } else {
                c["timeout"].as_u64().unwrap() as u128
            };
            assert_eq!(timeout(cmd).as_millis(), expected, "timeout {cmd}");
            assert_eq!(
                max_age(cmd).as_millis(),
                c["maxAge"].as_u64().unwrap() as u128,
                "maxAge {cmd}"
            );
        }
    }
    #[test]
    fn cell_lock_writes_wait_for_reselection() {
        for cmd in [
            "AT+QNWLOCK=\"common/5g\",108,504990,30,41",
            "AT+QNWLOCK=\"common/5g\",0",
            "AT+QNWLOCK=\"common/4g\",1,1300,262",
        ] {
            assert_eq!(timeout(cmd), Duration::from_secs(10), "{cmd}");
        }
        assert_eq!(timeout("AT+QNWLOCK=\"common/5g\""), Duration::from_secs(1));
    }
    #[test]
    fn mpdn_rule_writes_get_the_documented_five_seconds() {
        for cmd in [
            "AT+QMAP=\"MPDN_RULE\",0,1,0,1,1,\"FF:FF:FF:FF:FF:FF\"",
            "AT+QMAP=\"MPDN_RULE\",0",
        ] {
            assert!(timeout(cmd) >= Duration::from_secs(5), "{cmd}");
        }
    }
}
