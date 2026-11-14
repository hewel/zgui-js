use super::{Error, output::OutputState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io::{Read, Write}, process::{Command, Stdio}, sync::atomic::{AtomicBool, Ordering}, thread, time::{Duration, Instant}};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Report {
    pub result: Value,
    pub output_state: Option<OutputState>,
}

fn drain(mut reader: impl Read, maximum: usize) -> std::io::Result<(Vec<u8>, u64)> {
    let mut retained = Vec::new();
    let mut dropped = 0;
    let mut chunk = [0; 4096];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 { break; }
        let keep = count.min(maximum.saturating_sub(retained.len()));
        retained.extend_from_slice(&chunk[..keep]);
        dropped += (count - keep) as u64;
    }
    Ok((retained, dropped))
}

pub(crate) fn run(command: &mut Command, timeout: Duration) -> Result<Report, Error> {
    controlled(command, timeout, None)
}

pub(super) fn controlled(command: &mut Command, timeout: Duration, canceled: Option<&AtomicBool>) -> Result<Report, Error> {
    // A terminal interrupt belongs to the controller. A dead controller must not
    // leave its worker alive, including the race between fork and exec.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let parent = std::process::id() as libc::pid_t;
        command.process_group(0);
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    return Err(std::io::Error::other("preview controller exited before worker startup"));
                }
                Ok(())
            });
        }
    }
    let started = Instant::now();
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|error| Error::new("E_WORKER_CRASH", 9, format!("cannot start preview worker: {error}")))?;
    let stdout = child.stdout.take().expect("worker stdout pipe");
    let stderr = child.stderr.take().expect("worker stderr pipe");
    let stdout = thread::spawn(move || drain(stdout, 64 * 1024));
    let stderr = thread::spawn(move || drain(stderr, 256 * 1024));
    let mut termination = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if canceled.is_some_and(|flag| flag.load(Ordering::Acquire)) => {
                termination = Some(Error::new("E_RUN_CANCELED", 5, "run canceled by controller; worker terminated, clean teardown unverified"));
                let _ = child.kill();
                break child.wait();
            }
            Ok(None) if started.elapsed() >= timeout => {
                termination = Some(Error::new("E_WATCHDOG", 5, format!("worker exceeded {} ms; terminated, clean teardown unverified", timeout.as_millis())));
                let _ = child.kill();
                break child.wait();
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error);
            }
        }
    };
    let stdout = stdout.join().map_err(|_| Error::new("E_WORKER_CRASH", 9, "worker result reader panicked"))?
        .map_err(|error| Error::new("E_WORKER_CRASH", 9, error.to_string()))?;
    let stderr = stderr.join().map_err(|_| Error::new("E_WORKER_CRASH", 9, "worker log reader panicked"))?
        .map_err(|error| Error::new("E_WORKER_CRASH", 9, error.to_string()))?;
    let mut log=std::io::stderr().lock();
    log.write_all(&stderr.0).map_err(|error|Error::new("E_OUTPUT_IO",8,format!("cannot publish worker log: {error}")))?;
    if stderr.1 > 0 {
        writeln!(log,"preview: dropped {} worker log bytes",stderr.1)
            .map_err(|error|Error::new("E_OUTPUT_IO",8,format!("cannot publish worker log: {error}")))?;
    }
    if let Some(error) = termination { return Err(error); }
    let status = status.map_err(|error| Error::new("E_WORKER_CRASH", 9, error.to_string()))?;
    if !status.success() {
        return Err(Error::new("E_WORKER_CRASH", 9, format!("worker terminated with {status}; clean teardown unverified")));
    }
    if stdout.1 != 0 {
        return Err(Error::new("E_WORKER_CRASH", 9, "worker exceeded its 64 KiB control-message limit"));
    }
    serde_json::from_slice(&stdout.0)
        .map_err(|error| Error::new("E_WORKER_CRASH", 9, format!("invalid worker control message: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminates_a_blocked_worker_without_claiming_teardown() {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.args(["--exact", "preview::supervisor::tests::blocked_worker", "--ignored", "--nocapture"])
            .env("ZGUI_TEST_BLOCKED_PREVIEW", "1");
        let failure = run(&mut child, Duration::from_millis(500)).err().expect("blocked worker must fail");
        assert_eq!(failure.code, "E_WATCHDOG");
        assert_eq!(failure.exit_code, 5);
        assert!(failure.message.contains("clean teardown unverified"));
    }
    #[test]
    #[ignore = "owned subprocess fixture for the watchdog test"]
    fn blocked_worker() {
        assert_eq!(std::env::var("ZGUI_TEST_BLOCKED_PREVIEW").as_deref(), Ok("1"));
        loop { thread::park(); }
    }
}
