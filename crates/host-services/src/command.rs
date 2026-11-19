//! Allowlisted process execution with bounded, owned output and cancellable workers.

use crate::{
    ServiceError,
    worker::{Publisher, UNWATCHED, Worker, pollfd},
};
use std::{
    env,
    ffi::{CString, OsStr},
    fmt, fs,
    io::{self, BufRead, BufReader},
    mem,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{ffi::OsStrExt, process::CommandExt},
    },
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    ptr,
    sync::Arc,
};

/// Maximum bytes of combined stdout and stderr in buffered mode.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
/// Maximum bytes in one streaming line, excluding its LF delimiter.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// Determines how output is delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Returns a single record with both output streams and the exit status.
    Buffered,
    /// Delivers one event per line followed by the exit status.
    Stream,
}

/// A buffered command's complete UTF-8 output and exit code.
#[derive(Debug, PartialEq, Eq)]
pub struct Output {
    /// The numeric exit code, or `None` if terminated by a signal.
    pub status: Option<i32>,
    /// Raw UTF-8 stdout, including original delimiters.
    pub stdout: String,
    /// Raw UTF-8 stderr, including original delimiters.
    pub stderr: String,
}

/// A line of streaming output or the command's terminal event.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A stdout line, without LF; a final unterminated line is also delivered.
    Stdout(String),
    /// A stderr line, without LF; a final unterminated line is also delivered.
    Stderr(String),
    /// Buffered completion, including nonzero exit codes.
    Output(Output),
    /// Streaming completion, with `None` for termination by a signal.
    Exit(Option<i32>),
    /// Terminal launch, I/O, output-limit or UTF-8 failure.
    Error(ServiceError),
}

/// A process in its own process group with bounded output delivery.
///
/// Dropping this handle signals cancellation without waiting. The worker kills
/// a cancelled group and reaps its leader; normal exit leaves descendants alone.
/// This is not a sandbox: an approved program can choose to leave its group.
pub struct Process {
    worker: Worker<Event>,
}

impl fmt::Debug for Process {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("active", &self.is_active())
            .finish_non_exhaustive()
    }
}

/// Checks whether a program is a bare executable name or an absolute path.
///
/// This validates syntax only; the caller must authorize the exact input.
///
/// # Errors
///
/// Rejects empty names, NUL, and relative paths (including `.` and `..`).
pub fn validate_program(program: &str) -> io::Result<()> {
    if program.is_empty()
        || program.as_bytes().contains(&0)
        || matches!(program, "." | "..")
        || (program.contains('/') && !program.starts_with('/'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a bare executable name or an absolute path without NUL",
        ));
    }
    Ok(())
}

impl Process {
    /// Schedules literal execution of a program that the caller has authorized.
    ///
    /// Bare names are looked up in the host's `PATH`, skipping non-absolute
    /// directories. There is no implicit shell. The child inherits the host
    /// environment, has closed stdin, and writes to separate output pipes.
    /// `wake` runs on the worker after an event has been published and must not block.
    ///
    /// # Errors
    ///
    /// Invalid input or worker setup failure returns an I/O error. Launch and
    /// output failures arrive as [`Event::Error`].
    pub fn spawn(
        program: String,
        args: Vec<String>,
        mode: Mode,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        validate_program(&program)?;
        if args.iter().any(|arg| arg.as_bytes().contains(&0)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command arguments cannot contain NUL",
            ));
        }
        let worker =
            Worker::start(
                "zgui-command",
                MAX_OUTPUT_BYTES,
                wake,
                move |publisher| match run(program, args, mode, publisher) {
                    Ok(event) => event,
                    Err(error) => Some(Event::Error(error)),
                },
            )?;
        Ok(Self { worker })
    }

    /// Returns the next event if one is ready, without waiting.
    pub fn try_recv(&self) -> Option<Event> {
        self.worker.take()
    }

    /// Returns whether the program is launching or its process group needs cleanup.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.worker.active()
    }

    /// Cancels the command and waits for its worker to finish cleanup.
    ///
    /// # Errors
    ///
    /// Returns an error if the worker panicked.
    pub fn close(self) -> io::Result<()> {
        self.worker.close()
    }
}

struct Argv {
    strings: Vec<CString>,
    pointers: Vec<*const libc::c_char>,
}

impl Argv {
    fn new(program: String, args: Vec<String>) -> io::Result<Self> {
        let strings = std::iter::once(program)
            .chain(args)
            .map(CString::new)
            .collect::<Result<Vec<_>, _>>()?;
        let mut pointers = Vec::with_capacity(strings.len() + 1);
        pointers.extend(strings.iter().map(|arg| arg.as_ptr()));
        pointers.push(ptr::null());
        Ok(Self { strings, pointers })
    }

    fn exec(&self, path: &CString) -> io::Result<()> {
        // SAFETY: the pathname and null-terminated argv point to live C strings.
        // This callback only invokes async-signal-safe execv and reads errno.
        unsafe { libc::execv(path.as_ptr(), self.pointers.as_ptr()) };
        Err(io::Error::last_os_error())
    }
}

// SAFETY: the pointers refer only to immutable heap allocations owned by this
// struct. Moving or sharing it does not change their addresses or allow mutation.
unsafe impl Send for Argv {}
// SAFETY: the same immutable allocations remain valid for all shared readers.
unsafe impl Sync for Argv {}

fn spawn_exact(path: &Path, args: &Arc<Argv>) -> io::Result<Child> {
    let executable = CString::new(path.as_os_str().as_bytes())?;
    let args = Arc::clone(args);
    let mut command = Command::new(path);
    command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: no allocation, locks or global modification in the post-fork
    // callback. execv, unlike execvp, never interprets ENOEXEC via a shell.
    unsafe { command.pre_exec(move || args.exec(&executable)) };
    command.spawn()
}

fn launch(
    program: String,
    args: Vec<String>,
    publisher: &Publisher<Event>,
) -> io::Result<Option<Child>> {
    if publisher.stopped() {
        return Ok(None);
    }
    let argv = Arc::new(Argv::new(program, args)?);
    let program = Path::new(OsStr::from_bytes(argv.strings[0].as_bytes()));
    if program.is_absolute() {
        return spawn_exact(program, &argv).map(Some);
    }
    let path = env::var_os("PATH").unwrap_or_default();
    let mut denied = None;
    for directory in env::split_paths(&path).filter(|path| path.is_absolute()) {
        if publisher.stopped() {
            return Ok(None);
        }
        let path = directory.join(program);
        let result = match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => spawn_exact(&path, &argv),
            Ok(_) => continue,
            Err(error) => Err(error),
        };
        match result {
            Ok(child) => return Ok(Some(child)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => denied = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(denied.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "executable not found in host PATH")
    }))
}

struct Group {
    child: Option<Child>,
    pid: libc::pid_t,
    pidfd: Option<OwnedFd>,
}

impl Group {
    fn new(child: Child) -> io::Result<Self> {
        let mut child = child;
        let Ok(pid) = libc::pid_t::try_from(child.id()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("child pid is outside the platform range"));
        };
        let pidfd = pidfd(pid);
        Ok(Self {
            child: Some(child),
            pid,
            pidfd,
        })
    }

    fn pipes(&mut self) -> io::Result<[Pipe; 2]> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("command was already reaped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("stdout pipe missing"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("stderr pipe missing"))?;
        Ok([Pipe::new(stdout.into())?, Pipe::new(stderr.into())?])
    }

    fn leader_exited(&self) -> io::Result<bool> {
        // SAFETY: siginfo_t is valid zero-initialized when used as waitid output.
        let mut info: libc::siginfo_t = unsafe { mem::zeroed() };
        loop {
            // SAFETY: info is writable; the PID is an unreaped child owned by this guard.
            let wait = unsafe {
                libc::waitid(
                    libc::P_PID,
                    libc::id_t::try_from(self.pid).map_err(io::Error::other)?,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if wait == 0 {
                // SAFETY: the child-related fields of siginfo are initialized by
                // waitid, or stay zero when it finds no matching event.
                return Ok(unsafe { info.si_pid() } != 0);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    fn terminate(&mut self) -> io::Result<()> {
        let Some(child) = &mut self.child else {
            return Ok(());
        };
        // Signal before reaping. The leader's still-allocated PID prevents its
        // group id from being recycled to an unrelated command.
        // SAFETY: the positive child PID is the identifier for our new process group.
        if unsafe { libc::kill(-self.pid, libc::SIGKILL) } == -1 {
            let error = io::Error::last_os_error();
            // Even if the group is gone, the owned leader might have left it.
            let _ = child.kill();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("command was already reaped"))?
            .wait()?;
        self.child = None;
        Ok(status)
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

fn pidfd(pid: libc::pid_t) -> Option<OwnedFd> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: pidfd_open takes a valid child PID and reserved zero flags.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if let Ok(fd) = libc::c_int::try_from(fd)
            && fd >= 0
        {
            // SAFETY: a successful pidfd_open yields a new, owned descriptor.
            return Some(unsafe { OwnedFd::from_raw_fd(fd) });
        }
    }
    None
}

struct Pipe {
    reader: BufReader<fs::File>,
    data: Vec<u8>,
    closed: bool,
    after_exit_bytes: Option<usize>,
}

enum Step {
    Wait,
    Read,
    Line(String),
}

impl Pipe {
    fn new(fd: OwnedFd) -> io::Result<Self> {
        // SAFETY: fd remains live for both fcntl calls; the read end alone is
        // made nonblocking, without changing the child's write end.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: F_SETFL accepts the retrieved status flags plus O_NONBLOCK.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            reader: BufReader::new(fd.into()),
            data: Vec::new(),
            closed: false,
            after_exit_bytes: None,
        })
    }

    fn limit_to_queued_bytes(&mut self) -> io::Result<()> {
        let mut queued: libc::c_int = 0;
        // SAFETY: the reader owns the pipe and FIONREAD writes a c_int to `queued`.
        let result = unsafe {
            libc::ioctl(
                self.reader.get_ref().as_raw_fd(),
                libc::FIONREAD,
                &mut queued,
            )
        };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        let queued = usize::try_from(queued).map_err(io::Error::other)?;
        self.after_exit_bytes = Some(self.reader.buffer().len() + queued);
        Ok(())
    }

    fn read(&mut self, mode: Mode, buffered: &mut usize) -> Result<Step, ServiceError> {
        let limit = self.after_exit_bytes.unwrap_or(usize::MAX);
        let input = if limit == 0 {
            &[]
        } else {
            match self.reader.fill_buf() {
                Ok(input) => &input[..input.len().min(limit)],
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(Step::Wait),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(Step::Read),
                Err(error) => return Err(error.into()),
            }
        };
        if input.is_empty() {
            self.closed = true;
            return if mode == Mode::Stream && !self.data.is_empty() {
                Ok(Step::Line(as_text(mem::take(&mut self.data))?))
            } else {
                Ok(Step::Read)
            };
        }
        if mode == Mode::Buffered {
            if input.len() > MAX_OUTPUT_BYTES - *buffered {
                return Err(ServiceError::new(
                    "TooLarge",
                    "combined command output exceeds the byte limit",
                ));
            }
            let bytes = input.len();
            *buffered += bytes;
            self.data.extend_from_slice(input);
            self.consume(bytes);
            return Ok(Step::Read);
        }
        let newline = input.iter().position(|byte| *byte == b'\n');
        let bytes = newline.unwrap_or(input.len());
        if bytes > MAX_LINE_BYTES - self.data.len() {
            return Err(ServiceError::new(
                "TooLarge",
                "command line exceeds the byte limit",
            ));
        }
        self.data.extend_from_slice(&input[..bytes]);
        self.consume(bytes + usize::from(newline.is_some()));
        if newline.is_some() {
            Ok(Step::Line(as_text(mem::take(&mut self.data))?))
        } else {
            Ok(Step::Read)
        }
    }

    fn consume(&mut self, bytes: usize) {
        self.reader.consume(bytes);
        if let Some(remaining) = &mut self.after_exit_bytes {
            *remaining -= bytes;
        }
    }
}

fn as_text(bytes: Vec<u8>) -> Result<String, ServiceError> {
    String::from_utf8(bytes)
        .map_err(|_| ServiceError::new("InvalidUtf8", "command output is not valid UTF-8"))
}

fn run(
    program: String,
    args: Vec<String>,
    mode: Mode,
    publisher: &Publisher<Event>,
) -> Result<Option<Event>, ServiceError> {
    let Some(child) = launch(program, args, publisher)? else {
        return Ok(None);
    };
    let mut group = Group::new(child)?;
    let result = collect_output(&mut group, mode, publisher);
    if group.child.is_some() {
        let signal = group.terminate();
        let reaped = group.wait();
        return result.and_then(|event| {
            signal?;
            reaped?;
            Ok(event)
        });
    }
    result
}

fn collect_output(
    group: &mut Group,
    mode: Mode,
    publisher: &Publisher<Event>,
) -> Result<Option<Event>, ServiceError> {
    let mut pipes = group.pipes()?;
    let mut pending = None;
    let mut next = 0;
    let mut buffered = 0;
    let mut leader_exited = false;
    loop {
        let mut more_data = true;
        // Bound the burst so a continuously writable descendant cannot starve
        // monitoring of the group leader or prevent cancellation.
        for _ in 0..32 {
            if publisher.stopped() {
                return Ok(None);
            }
            if let Some((event, bytes)) = pending.take()
                && let Err(blocked) = publisher.offer(event, bytes)
            {
                pending = Some(blocked);
                more_data = false;
                break;
            }
            let mut progressed = false;
            for index in [next, 1 - next] {
                let pipe: &mut Pipe = &mut pipes[index];
                if pipe.closed {
                    continue;
                }
                match pipe.read(mode, &mut buffered)? {
                    Step::Wait => {}
                    Step::Read => progressed = true,
                    Step::Line(line) => {
                        let bytes = line.len();
                        let event = if index == 0 {
                            Event::Stdout(line)
                        } else {
                            Event::Stderr(line)
                        };
                        pending = publisher.offer(event, bytes).err();
                        progressed = true;
                        next = 1 - index;
                        if pending.is_some() {
                            break;
                        }
                    }
                }
            }
            if !progressed {
                more_data = false;
                break;
            }
        }
        if leader_exited && pending.is_none() && pipes.iter().all(|pipe| pipe.closed) {
            if publisher.stopped() {
                return Ok(None);
            }
            let status = group.wait()?;
            let [stdout, stderr] = pipes;
            return Ok(Some(match mode {
                Mode::Buffered => Event::Output(Output {
                    status: status.code(),
                    stdout: as_text(stdout.data)?,
                    stderr: as_text(stderr.data)?,
                }),
                Mode::Stream => Event::Exit(status.code()),
            }));
        }
        let mut fds = [UNWATCHED; 4];
        if pending.is_none() {
            for (slot, pipe) in fds[1..3].iter_mut().zip(&pipes) {
                if !pipe.closed {
                    *slot = pollfd(pipe.reader.get_ref(), libc::POLLIN);
                }
            }
        }
        if !leader_exited && let Some(pidfd) = &group.pidfd {
            fds[3] = pollfd(pidfd, libc::POLLIN);
        }
        let fallback = !leader_exited && group.pidfd.is_none();
        let timeout = if more_data {
            0
        } else if fallback {
            50
        } else {
            -1
        };
        publisher.poll(&mut fds, timeout)?;
        if fds[3].revents != 0 || (fallback && group.leader_exited()?) {
            leader_exited = true;
            // Drain what the leader left without waiting for descendants to
            // close their copies or letting new writes postpone command exit.
            for pipe in pipes.iter_mut().filter(|pipe| !pipe.closed) {
                pipe.limit_to_queued_bytes()?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{
        MAX_EVENTS,
        tests::{DEADLINE, TempDir, observe, waiter},
    };
    use std::{
        os::unix::fs::PermissionsExt,
        thread,
        time::{Duration, Instant},
    };

    fn shell(
        script: &str,
        arguments: &[&str],
        mode: Mode,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Process {
        let args = ["-c", script, "test"]
            .into_iter()
            .chain(arguments.iter().copied())
            .map(str::to_owned)
            .collect();
        Process::spawn("/bin/sh".into(), args, mode, wake).expect("start the explicit test shell")
    }

    #[test]
    fn command_passes_literal_arguments_without_a_shell() {
        let (wake, inbox) = waiter();
        let literal = "$(printf expanded); *\nquoted ' text";
        let process = Process::spawn(
            "printf".into(),
            vec!["%s".into(), literal.into()],
            Mode::Buffered,
            wake,
        )
        .expect("resolve printf through the trusted PATH and launch it literally");
        assert_eq!(
            observe(&inbox, || process.try_recv()),
            Event::Output(Output {
                status: Some(0),
                stdout: literal.into(),
                stderr: String::new()
            }),
            "shell syntax remains literal argv data"
        );
        assert!(
            !process.is_active(),
            "completed buffered result requires reaping the command leader"
        );
    }

    #[test]
    fn command_rejects_relative_paths_and_does_not_interpret_unrecognized_executables() {
        let (wake, inbox) = waiter();
        let relative = Process::spawn("./hello".into(), vec![], Mode::Buffered, Arc::clone(&wake));
        assert_eq!(
            relative
                .expect_err("relative executable path is forbidden")
                .kind(),
            io::ErrorKind::InvalidInput,
            "relative executable paths must not resolve against cwd"
        );
        let dir = TempDir::new();
        let path = dir.join("no-shebang");
        fs::write(&path, "exit 0\n")
            .expect("write text that would succeed if silently run by a shell");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("make the shebang-less text file executable");
        let path = path.to_str().expect("test path is UTF-8");
        let process = Process::spawn(path.into(), vec![], Mode::Buffered, wake)
            .expect("start worker for shebang-less file");
        let Event::Error(error) = observe(&inbox, || process.try_recv()) else {
            panic!("spawn must not implicitly run the shell for an unrecognized executable");
        };
        assert_eq!(
            error,
            io::Error::from_raw_os_error(libc::ENOEXEC).into(),
            "the kernel must reject the executable format"
        );
    }

    #[test]
    fn buffered_command_drains_both_pipes_and_returns_nonzero_status() {
        let (wake, inbox) = waiter();
        let process = shell(
            "(head -c 131072 /dev/zero)& head -c 262144 /dev/zero >&2; wait; exit 7",
            &[],
            Mode::Buffered,
            wake,
        );
        let Event::Output(output) = observe(&inbox, || process.try_recv()) else {
            panic!("expected both buffered pipes, not a deadlock or service error");
        };
        assert_eq!(
            output.status,
            Some(7),
            "nonzero exit is a successful output record"
        );
        assert_eq!(
            output.stdout,
            "\0".repeat(131072),
            "stdout must drain more than a typical pipe capacity"
        );
        assert_eq!(
            output.stderr,
            "\0".repeat(262144),
            "stderr must drain concurrently with stdout"
        );
    }

    #[test]
    fn streaming_preserves_each_pipe_order_and_flushes_final_partial_lines() {
        let (wake, inbox) = waiter();
        let process = shell(
            "printf 'one\\n\\nlast'; printf 'bad\\nend' >&2; exit 4",
            &[],
            Mode::Stream,
            wake,
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        loop {
            match observe(&inbox, || process.try_recv()) {
                Event::Stdout(line) => stdout.push(line),
                Event::Stderr(line) => stderr.push(line),
                Event::Exit(status) => {
                    assert_eq!(
                        status,
                        Some(4),
                        "stream exit follows all lines and preserves the program's status"
                    );
                    break;
                }
                event => panic!("unexpected streaming command event: {event:?}"),
            }
        }
        assert_eq!(
            stdout,
            ["one", "", "last"],
            "stdout preserves empty and final unterminated lines"
        );
        assert_eq!(
            stderr,
            ["bad", "end"],
            "stderr is ordered independently of stdout"
        );
    }

    #[test]
    fn command_bounds_total_buffered_output_and_each_streaming_line() {
        for (mode, maximum) in [
            (Mode::Buffered, MAX_OUTPUT_BYTES),
            (Mode::Stream, MAX_LINE_BYTES),
        ] {
            let (wake, inbox) = waiter();
            let limit = (maximum + 1).to_string();
            let process = shell("head -c \"$1\" /dev/zero", &[&limit], mode, wake);
            let Event::Error(error) = observe(&inbox, || process.try_recv()) else {
                panic!("expected exceeding {mode:?} output bounds to terminate with an error");
            };
            assert_eq!(
                error.code, "TooLarge",
                "limits must reject oversize {mode:?} output"
            );
            assert!(
                !process.is_active(),
                "output limit errors must clean up the process group before publication"
            );
        }
    }

    #[test]
    fn cancellation_kills_the_process_group_while_stream_publication_is_backpressured() {
        let dir = TempDir::new();
        let pidfile = dir.join("pids");
        let pids_arg = pidfile.to_str().expect("test pid path is UTF-8");
        let (wake, inbox) = waiter();
        let process = shell(
            "sleep 30 & printf '%s %s\\n' \"$$\" \"$!\" > \"$1\"; yes line",
            &[pids_arg],
            Mode::Stream,
            wake,
        );
        for _ in 0..MAX_EVENTS {
            inbox
                .recv_timeout(DEADLINE)
                .expect("fill the event queue without consuming any output");
        }
        let pids = fs::read_to_string(pidfile).expect("read leader and descendant process ids");
        let mut pids = pids.split_whitespace();
        let leader = pids.next().expect("the shell reports its leader pid");
        let descendant = pids
            .next()
            .expect("the shell reports its background child pid");
        process.close().expect("cancel and join the process worker");
        assert!(
            !Path::new(&format!("/proc/{leader}")).exists(),
            "explicit close must reap the group leader before returning"
        );
        let deadline = Instant::now() + DEADLINE;
        let stat = format!("/proc/{descendant}/stat");
        while let Ok(contents) = fs::read_to_string(&stat) {
            if contents
                .rsplit_once(") ")
                .is_some_and(|(_, status)| status.starts_with('Z'))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "descendant {descendant} must not survive group cancellation"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn leader_exit_leaves_descendants_alive_even_if_they_inherited_its_pipes() {
        let dir = TempDir::new();
        let release = dir.join("release");
        let proof = dir.join("proof");
        let (wake, inbox) = waiter();
        let process = shell(
            r#"(remaining=100; while test "$remaining" -gt 0; do
                    if test -f "$1"; then printf survived > "$2"; exit; fi
                    remaining=$((remaining - 1))
                    sleep 0.05
                done) & printf done; exit 9"#,
            &[
                release.to_str().expect("UTF-8 release path"),
                proof.to_str().expect("UTF-8 proof path"),
            ],
            Mode::Buffered,
            wake,
        );
        assert_eq!(
            observe(&inbox, || process.try_recv()),
            Event::Output(Output {
                status: Some(9),
                stdout: "done".into(),
                stderr: String::new()
            }),
            "inherited stdout must not postpone the leader's result"
        );
        process.close().expect("join the finished process worker");
        fs::write(release, []).expect("release the surviving descendant");
        let deadline = Instant::now() + DEADLINE;
        while fs::read(&proof).as_deref().ok() != Some(b"survived") {
            assert!(
                Instant::now() < deadline,
                "the descendant must remain able to run after the leader's result"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}
