use std::{
    io,
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::{
        LazyLock,
        mpsc::{self, Sender},
    },
    thread,
    time::Duration,
};

use crate::ServiceError;

/// Admit only explicit HTTP(S) and mailto URLs, never an arbitrary command or file URI.
pub fn external_url(value: &str) -> Result<String, ServiceError> {
    let invalid = || {
        ServiceError::new(
            "InvalidPath",
            "external URL must use http://, https:// or mailto:",
        )
    };
    if value.chars().any(char::is_control) || value.trim() != value {
        return Err(invalid());
    }
    let url = url::Url::parse(value).map_err(|_| invalid())?;
    match url.scheme() {
        "http" | "https" if value.starts_with("http://") || value.starts_with("https://") => {
            if url.host().is_none() {
                return Err(invalid());
            }
        }
        "mailto" if value.starts_with("mailto:") && !url.path().is_empty() => {}
        _ => return Err(invalid()),
    }
    Ok(url.into())
}

/// Start an already authorized URI with the desktop handler without inheriting application I/O.
pub fn launch_external(uri: &str, activation_token: Option<&str>) -> Result<(), ServiceError> {
    static REAPER: LazyLock<Option<Sender<Child>>> = LazyLock::new(|| {
        let (tx, rx) = mpsc::channel::<Child>();
        thread::Builder::new()
            .name("zgui-external-reaper".into())
            .spawn(move || {
                let mut children = Vec::new();
                loop {
                    if children.is_empty() {
                        let Ok(child) = rx.recv() else {
                            return;
                        };
                        children.push(child);
                    }
                    if let Ok(child) = rx.recv_timeout(Duration::from_millis(250)) {
                        children.push(child);
                    }
                    children
                        .retain_mut(|child| child.try_wait().is_ok_and(|status| status.is_none()));
                }
            })
            .ok()
            .map(|_| tx)
    });
    let reaper = REAPER
        .as_ref()
        .ok_or_else(|| ServiceError::new("Io", "cannot initialize the external process reaper"))?;
    let mut command = Command::new("xdg-open");
    command
        .arg(uri)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.env_remove("XDG_ACTIVATION_TOKEN");
    if let Some(token) = activation_token {
        command.env("XDG_ACTIVATION_TOKEN", token);
    }
    // SAFETY: the pre-exec hook uses only the async-signal-safe setsid syscall.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let child = command.spawn()?;
    if let Err(mpsc::SendError(mut child)) = reaper.send(child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ServiceError::new("Io", "external process reaper stopped"));
    }
    Ok(())
}
