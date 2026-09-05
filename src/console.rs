use crate::server::App;
use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;
use std::{sync::Arc, time::Duration};

pub async fn run(app: Arc<App>, mut socket: WebSocket, token: String) {
    let Some(mut revoked) = app.auth.watch(&token) else {
        return;
    };
    tokio::select! {
        _=revoked.changed()=>{},
        _=session(app,&mut socket,&token)=>{}
    }
    let _ = socket.close().await;
}
async fn send(socket: &mut WebSocket, data: &[u8]) -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(10),
        socket.send(Message::Binary(data.to_vec().into())),
    )
    .await??;
    Ok(())
}
async fn line(socket: &mut WebSocket, prompt: &str, hidden: bool) -> Result<String> {
    send(socket, prompt.as_bytes()).await?;
    let mut data = Vec::new();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(120), socket.recv())
            .await?
            .context("closed")??;
        let bytes = match message {
            Message::Text(v) => v.as_bytes().to_vec(),
            Message::Binary(v) => v.to_vec(),
            Message::Close(_) => anyhow::bail!("closed"),
            _ => continue,
        };
        for b in bytes {
            match b {
                b'\r' | b'\n' => {
                    send(socket, b"\r\n").await?;
                    return Ok(String::from_utf8_lossy(&data).into_owned());
                }
                8 | 127 => {
                    if !data.is_empty() {
                        data.pop();
                        if !hidden {
                            send(socket, b"\x08 \x08").await?
                        }
                    }
                }
                0..=31 => {}
                _ => {
                    if data.len() >= 128 {
                        anyhow::bail!("input too long")
                    }
                    data.push(b);
                    if !hidden {
                        send(socket, &[b]).await?
                    }
                }
            }
        }
    }
}
async fn session(app: Arc<App>, socket: &mut WebSocket, token: &str) -> Result<()> {
    send(socket, b"Terminal login required\r\n").await?;
    let mut valid = false;
    for _ in 0..3 {
        let user = line(socket, "login: ", false).await?;
        let password = line(socket, "Password: ", true).await?;
        let copy = app.clone();
        valid = tokio::task::spawn_blocking(move || {
            crate::auth::equal(user.trim(), "root")
                && copy.auth.root_matches(&password, copy.config.mock)
        })
        .await?;
        if valid {
            break;
        }
        send(socket, b"\r\nLogin incorrect\r\n").await?;
    }
    if !valid {
        send(socket, b"Too many failed login attempts\r\n").await?;
        return Ok(());
    }
    if !app.auth.valid(token, true) {
        return Ok(());
    }
    send(socket, b"\r\n").await?;
    native_session(app, socket, token).await
}
#[cfg(unix)]
async fn native_session(app: Arc<App>, socket: &mut WebSocket, token: &str) -> Result<()> {
    use std::{
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::OpenOptionsExt,
        },
    };
    let master = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open("/dev/ptmx")?;
    let fd = master.as_raw_fd();
    if unsafe { libc::grantpt(fd) } != 0 || unsafe { libc::unlockpt(fd) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut name = [0 as libc::c_char; 128];
    if unsafe { libc::ptsname_r(fd, name.as_mut_ptr(), name.len()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let slave_fd = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if slave_fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let slave = unsafe { std::fs::File::from_raw_fd(slave_fd) };
    let size = libc::winsize {
        ws_row: 32,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(fd, libc::TIOCSWINSZ, &size);
    }
    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("HISTFILE", "/dev/null")
        .stdin(slave.try_clone()?)
        .stdout(slave.try_clone()?)
        .stderr(slave)
        .kill_on_drop(true);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGHUP);
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    struct ProcessGroup(u32);
    impl Drop for ProcessGroup {
        fn drop(&mut self) {
            unsafe {
                libc::kill(-(self.0 as i32), libc::SIGKILL);
            }
        }
    }
    let _group = ProcessGroup(child.id().context("missing shell PID")?);
    let master = tokio::io::unix::AsyncFd::new(master)?;
    let mut buf = [0u8; 4096];
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _=child.wait()=>break,
            _=heartbeat.tick()=>{if !app.auth.valid(token,false){break}socket.send(Message::Ping(Vec::new().into())).await?;},
            ready=master.readable()=>{let mut ready=ready?;match ready.try_io(|inner|inner.get_ref().read(&mut buf)){Ok(Ok(0))=>break,Ok(Ok(n))=>send(socket,&buf[..n]).await?,Ok(Err(_))=>break,Err(_)=>{}}},
            message=socket.recv()=>{
                let bytes=match message {Some(Ok(Message::Text(v)))=>v.as_bytes().to_vec(),Some(Ok(Message::Binary(v)))=>v.to_vec(),Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,_=>continue};
                if !app.auth.valid(token,true){break}let mut offset=0;
                while offset<bytes.len(){let mut ready=master.writable().await?;match ready.try_io(|inner|inner.get_ref().write(&bytes[offset..])){Ok(Ok(0))=>anyhow::bail!("PTY closed"),Ok(Ok(n))=>offset+=n,Ok(Err(e))=>return Err(e.into()),Err(_)=>{}}}
            }
        }
    }
    Ok(())
}
#[cfg(not(unix))]
async fn native_session(_: Arc<App>, socket: &mut WebSocket, _: &str) -> Result<()> {
    send(socket, b"Native terminal requires Linux.\r\n").await
}
