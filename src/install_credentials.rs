use crate::{auth, persistence::Store};
use anyhow::{Result, bail};
use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub web_username: Option<String>,
    pub web_password: Option<String>,
    pub root_password: Option<String>,
}

impl Credentials {
    pub fn validate(&self) -> Result<()> {
        match (&self.web_username, &self.web_password) {
            (Some(user), Some(password)) => {
                if user.is_empty()
                    || user.len() > 64
                    || !user
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                {
                    bail!(
                        "Web username must use 1-64 letters, digits, underscores, dots or hyphens"
                    );
                }
                auth::validate(password)?;
                if password.trim() != password {
                    bail!("Web password cannot begin or end with whitespace");
                }
            }
            (None, None) => {}
            _ => bail!("Web username and password must be supplied together"),
        }
        if let Some(password) = &self.root_password {
            auth::validate(password)?;
        }
        Ok(())
    }
}

pub fn read(input: impl Read) -> Result<Credentials> {
    let mut bytes = Vec::new();
    input.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        bail!("Installation credentials exceed size limit");
    }
    // Parser errors may quote a password from malformed input.
    let value: Credentials = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid installation credentials"))?;
    value.validate()?;
    Ok(value)
}

pub fn run(check_only: bool) -> Result<()> {
    let value = read(std::io::stdin())?;
    if check_only {
        return Ok(());
    }
    let store = Store::new(false);
    if let Some(password) = &value.root_password {
        auth::change_root(&store, "", password, true)?;
        store.write(
            Path::new("/usrdata/simpleadmin/root-password.initialized"),
            b"1\n",
            0o600,
        )?;
    }
    if let (Some(user), Some(password)) = (&value.web_username, &value.web_password) {
        store.write(
            Path::new("/usrdata/simpleadmin/simpleadmin.auth"),
            format!("{user}:{password}\n").as_bytes(),
            0o600,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_preserve_exact_passwords() {
        let input =
            r#"{"web_username":"owner-1","web_password":"a:\"$你好","root_password":" 'secret' "}"#;
        let value = read(input.as_bytes()).unwrap();
        assert_eq!(value.web_username.as_deref(), Some("owner-1"));
        assert_eq!(value.web_password.as_deref(), Some("a:\"$你好"));
        assert_eq!(value.root_password.as_deref(), Some(" 'secret' "));
        assert!(read(b"{}".as_slice()).is_ok());
    }
    #[test]
    fn invalid_credentials_are_rejected_without_echo() {
        for input in [
            r#"{"web_username":"bad:name","web_password":"secret"}"#,
            r#"{"web_username":"user"}"#,
            r#"{"web_password":"secret"}"#,
            r##"{"web_username":"#comment","web_password":"secret"}"##,
            r#"{"web_username":"user","web_password":"secret "}"#,
            r#"{"root_password":""}"#,
            r#"{"root_password":"line\nbreak"}"#,
            r#"{"unknown":"secret"}"#,
            r#"{"root_password":secret}"#,
        ] {
            let error = read(input.as_bytes()).err().unwrap().to_string();
            assert!(!error.contains("secret"), "{error}");
        }
        assert!(read(vec![b' '; 4097].as_slice()).is_err());
        assert!(read(format!(r#"{{"root_password":"{}"}}"#, "好".repeat(43)).as_bytes()).is_err());
    }
}
