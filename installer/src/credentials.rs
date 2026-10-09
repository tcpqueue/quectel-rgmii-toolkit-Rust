//! Account options chosen in the installer. They travel to the module over adb stdin as JSON
//! and are checked again on the module by `simpleadmin-httpd install-credentials --check`.

use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Default, Serialize)]
pub struct Credentials {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_password: Option<String>,
}

fn password(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 128 || value.contains(['\r', '\n', '\0']) {
        bail!("密码需为 1–128 字节，不能包含换行或空字符；中文字符通常占 3 字节。")
    }
    Ok(())
}

/// Returns the JSON document for the module, or `None` when both options keep existing values.
pub fn build(web: Option<(&str, &str)>, root: Option<&str>) -> Result<Option<String>> {
    let mut value = Credentials::default();
    if let Some((username, web_password)) = web {
        if username.is_empty()
            || username.len() > 64
            || !username
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            bail!("Web 账号需为 1–64 位字母、数字、点、下划线或短横线。")
        }
        password(web_password)?;
        if web_password.trim() != web_password {
            bail!("Web 密码的开头和结尾不能使用空白字符。")
        }
        value.web_username = Some(username.into());
        value.web_password = Some(web_password.into());
    }
    if let Some(root_password) = root {
        password(root_password)?;
        value.root_password = Some(root_password.into());
    }
    if web.is_none() && root.is_none() {
        return Ok(None);
    }
    Ok(Some(serde_json::to_string(&value)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selections_preserve_existing_settings() {
        assert!(build(None, None).unwrap().is_none());
        assert!(
            build(Some(("owner", "secret")), None)
                .unwrap()
                .unwrap()
                .contains("web_username")
        );
        assert!(
            !build(None, Some("secret"))
                .unwrap()
                .unwrap()
                .contains("web_username")
        );
    }
    #[test]
    fn validation_prevents_truncation_and_format_ambiguity() {
        for user in ["", "bad:name", "#comment", &"a".repeat(65)] {
            assert!(build(Some((user, "secret")), None).is_err(), "{user}");
        }
        for password in ["", "abc\nxyz", "secret ", &"好".repeat(43)] {
            assert!(build(Some(("user", password)), None).is_err(), "{password}");
        }
        let json = build(Some(("user", "a:\"$你好")), Some(" 'secret' "))
            .unwrap()
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["web_password"], "a:\"$你好");
        assert_eq!(value["root_password"], " 'secret' ");
    }
}
