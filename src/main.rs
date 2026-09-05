mod actions;
mod at;
mod at_policy;
mod auth;
mod cleanup;
mod console;
mod forwarding;
mod mock;
mod parser;
mod persistence;
mod server;
mod sms;
mod system;
mod telemetry;
mod tls;

use anyhow::{Result, bail};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Clone, Debug)]
#[command(version, about = "Quectel RGMII administration server")]
pub struct Config {
    #[arg(long = "static", default_value = "/usrdata/simpleadmin/www")]
    pub static_dir: PathBuf,
    #[arg(long, default_value = "/usrdata/simpleadmin/simpleadmin.auth")]
    pub auth_file: PathBuf,
    #[arg(long, default_value = ":80")]
    pub http: String,
    #[arg(long, default_value = ":443")]
    pub https: String,
    #[arg(long,default_value_t=true,action=clap::ArgAction::Set,num_args=0..=1,default_missing_value="true",require_equals=true)]
    pub no_tls: bool,
    #[arg(long,default_value_t=false,action=clap::ArgAction::Set,num_args=0..=1,default_missing_value="true",require_equals=true)]
    pub mock: bool,
    #[arg(long, default_value = "/usrdata/simpleadmin/ttlvalue")]
    pub ttl_file: PathBuf,
    #[arg(long, default_value = "")]
    pub at_devices: String,
    #[arg(long, default_value = "/usrdata/simpleadmin/at_devices.conf")]
    pub at_devices_file: PathBuf,
    #[arg(long, default_value = "/usrdata/simpleadmin/server.crt")]
    pub cert: PathBuf,
    #[arg(long, default_value = "/usrdata/simpleadmin/server.key")]
    pub key: PathBuf,
    #[arg(long, default_value = "/usrdata/simpleadmin/zbims-ca.crt")]
    pub ca_cert: PathBuf,
    #[arg(long, default_value = "/usrdata/simpleadmin/zbims-ca.key")]
    pub ca_key: PathBuf,
    #[arg(long, default_value_t = false)]
    pub at_debug: bool,
}
impl Config {
    fn devices(&self) -> Vec<String> {
        collect_devices(&self.at_devices, &self.at_devices_file)
    }
}
fn collect_devices(explicit: &str, file: &std::path::Path) -> Vec<String> {
    let env = std::env::var("SIMPLEADMIN_AT_DEVICES").unwrap_or_default();
    let source = if !env.trim().is_empty() {
        env
    } else if explicit.is_empty() {
        std::fs::read_to_string(file).unwrap_or_default()
    } else {
        explicit.to_owned()
    };
    let mut devices = Vec::new();
    for line in source.lines() {
        let line = line.split('#').next().unwrap_or("");
        for path in line
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if path == "/dev/smd11" && !devices.contains(&path.to_string()) {
                devices.push(path.into())
            }
        }
    }
    if devices.is_empty() {
        devices.push("/dev/smd11".into())
    }
    devices
}
#[derive(Parser)]
struct AtCommand {
    #[arg(long, default_value = "")]
    devices: String,
    #[arg(long, default_value = "/usrdata/simpleadmin/at_devices.conf")]
    devices_file: PathBuf,
    #[arg(long, default_value_t = 1000)]
    timeout_ms: u64,
    #[arg(long)]
    debug: bool,
    #[arg(required = true, trailing_var_arg = true)]
    command: Vec<String>,
}
pub fn listen_address(raw: &str) -> String {
    if raw.starts_with(':') {
        format!("0.0.0.0{raw}")
    } else {
        raw.into()
    }
}
fn main() {
    if let Err(e) = entry() {
        eprintln!("{e:#}");
        let saved = e
            .downcast_ref::<persistence::SavedError>()
            .is_some_and(|error| error.committed);
        std::process::exit(if saved { 2 } else { 1 })
    }
}
fn entry() -> Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    for arg in args.iter_mut().skip(1) {
        if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2 {
            arg.insert(0, '-')
        }
    }
    let sub = args.get(1).map(String::as_str).unwrap_or("");
    if sub == "root-password-init" {
        let store = persistence::Store::new(false);
        let marker = PathBuf::from("/usrdata/simpleadmin/root-password.initialized");
        if !marker.exists() {
            auth::change_root(&store, "", "admin", true)?;
            store.write(&marker, b"1\n", 0o600)?
        }
        return Ok(());
    }
    if sub == "passwd" {
        use std::io::Read;
        let mut password = String::new();
        std::io::stdin().take(130).read_to_string(&mut password)?;
        let password = password.trim_end_matches(['\r', '\n']);
        auth::validate(password)?;
        let path = args
            .windows(2)
            .find(|p| p[0] == "--auth-file")
            .map(|p| PathBuf::from(&p[1]))
            .unwrap_or_else(|| PathBuf::from("/usrdata/simpleadmin/simpleadmin.auth"));
        let (user, _) = auth::read(&path).unwrap_or(("admin".into(), String::new()));
        persistence::Store::new(false).write(
            &path,
            format!("{user}:{password}\n").as_bytes(),
            0o600,
        )?;
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(4)
        .thread_stack_size(256 * 1024)
        .build()?;
    if sub == "at" {
        let cli = AtCommand::parse_from(
            std::iter::once(args[0].clone()).chain(args[2..].iter().cloned()),
        );
        let command = cli.command.join(" ");
        return runtime.block_on(async {
            let at = at::At::start(false, collect_devices(&cli.devices, &cli.devices_file))?;
            println!(
                "{}",
                at.transaction_timeout(
                    &command,
                    None,
                    Some(std::time::Duration::from_millis(
                        cli.timeout_ms.clamp(300, 120000)
                    ))
                )
                .await?
            );
            Ok(())
        });
    }
    if sub == "ttl" {
        let action = args.get(2).map(String::as_str).unwrap_or("status");
        return runtime.block_on(async {
            let path = PathBuf::from("/usrdata/simpleadmin/ttlvalue");
            let value = system::ttl(&path);
            match action {
                "status" => println!("enabled={} ttl={value}", value > 0),
                "apply" | "start" | "restart" => {
                    for line in system::apply_ttl(value, false).await? {
                        println!("{line}")
                    }
                }
                "off" | "stop" => {
                    system::set_ttl(
                        0,
                        false,
                        &path,
                        std::sync::Arc::new(persistence::Store::new(false)),
                    )
                    .await?;
                }
                _ => bail!("invalid ttl action"),
            }
            Ok(())
        });
    }
    if sub == "serve" {
        args.remove(1);
    }
    let config = Config::parse_from(args);
    runtime.block_on(async move {
        let app = server::App::new(config)?;
        if !app.config.mock {
            let value = system::ttl(&app.config.ttl_file);
            if value > 0 {
                system::apply_ttl(value, false).await?;
            }
        }
        app.start();
        if app.config.no_tls {
            let listener = tokio::net::TcpListener::bind(listen_address(&app.config.http)).await?;
            println!("HTTP listening on {}", listener.local_addr()?);
            axum::serve(listener, app.router())
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
        } else {
            tls::serve(app).await?
        }
        Ok(())
    })
}
