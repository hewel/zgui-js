use crate::runtime_paths::{effective_uid, open_canonical_dir, open_directory, require_private};
use host_services::control;
use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const RUNTIME_SUBDIR: &str = "zgui-js";
const STAGE_SOCKET: &str = "socket";
static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub(crate) struct Control {
    pub(crate) override_path: Option<PathBuf>,
    pub(crate) recover_default_stale: bool,
    active: Option<Active>,
}

struct Active {
    app_id: String,
    generations: Vec<u64>,
    listener: control::Listener,
    socket: BoundSocket,
}

impl Active {
    fn routes(&self, app_id: &str, generations: impl Iterator<Item = u64>) -> bool {
        if self.app_id != app_id {
            return false;
        }
        let mut count = 0;
        for generation in generations {
            if !self.generations.contains(&generation) {
                return false;
            }
            count += 1;
        }
        count == self.generations.len()
    }
}

impl Control {
    /// Returns `false` if no explicit path or default runtime directory is configured.
    ///
    /// A change in routed generations creates a fresh listener even if the app ID
    /// stays the same. Handler subscription changes do not affect the route.
    pub(crate) fn reconcile<W: Fn() + Send + Sync + 'static, I: Iterator<Item = u64>>(
        &mut self,
        app_id: &str,
        generations: impl Fn() -> I,
        wake: impl FnOnce() -> W,
    ) -> io::Result<bool> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.routes(app_id, generations()))
        {
            return Ok(true);
        }
        // Unpublish and discard the old listener, not just its owner inbox.
        // In-flight connections and the kernel backlog are tied to its inode.
        self.stop()?;
        let default_path;
        let path = if let Some(path) = &self.override_path {
            path.as_path()
        } else {
            let Some(runtime) = runtime_dir() else {
                return Ok(false);
            };
            default_path = default_socket_path(&runtime, app_id)?;
            open_default_parent(&runtime, ParentMode::Create)?;
            &default_path
        };
        let (listener, socket) = if self.recover_default_stale && self.override_path.is_none() {
            match BoundSocket::elect(path)? {
                SocketElection::Bound(listener, socket) => (listener, socket),
                SocketElection::Live(_) => return Err(io::ErrorKind::AddrInUse.into()),
            }
        } else {
            BoundSocket::bind(path)?
        };
        let listener = control::Listener::start(listener, decode, encode, wake())?;
        self.active = Some(Active {
            app_id: app_id.to_owned(),
            listener,
            generations: generations().collect(),
            socket,
        });
        Ok(true)
    }

    pub(crate) fn try_recv(&self) -> Option<(&str, &[u64], control::Inbound)> {
        let active = self.active.as_ref()?;
        Some((
            &active.app_id,
            &active.generations,
            active.listener.try_recv()?,
        ))
    }

    pub(crate) fn stop(&mut self) -> io::Result<()> {
        let Some(Active {
            listener,
            mut socket,
            ..
        }) = self.active.take()
        else {
            return Ok(());
        };
        drop(listener);
        socket.unlink()
    }

    /// Unpublish the endpoint and wait for the listener to exit at final shutdown.
    pub(crate) fn close(&mut self) -> io::Result<()> {
        let Some(Active {
            listener,
            mut socket,
            ..
        }) = self.active.take()
        else {
            return Ok(());
        };
        let unlinked = socket.unlink();
        unlinked.and(listener.close())
    }
}

pub(crate) fn command(args: impl Iterator<Item = OsString>) -> Result<(), Box<dyn Error>> {
    let Message { endpoint, args } = Message::parse(args)?;
    let mut request = RequestFrame(Vec::new());
    serde_json::to_writer(&mut request, &args)?;
    let path = match endpoint {
        Endpoint::Path(path) => path,
        Endpoint::App(app_id) => {
            let runtime = runtime_dir().ok_or("msg --app-id requires XDG_RUNTIME_DIR")?;
            let path = default_socket_path(&runtime, &app_id)?;
            open_default_parent(&runtime, ParentMode::Existing)?;
            path
        }
    };
    let socket = connect(&path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("connect control socket {}: {error}", path.display()),
        )
    })?;
    let reply = control::exchange(socket, &request.0)?;
    match serde_json::from_slice::<Reply<String>>(&reply)? {
        Reply::Ok { result } if result.len() <= control::MAX_REPLY_BYTES => {
            writeln!(io::stdout().lock(), "{result}")?;
            Ok(())
        }
        Reply::Error { message } if message.len() <= control::MAX_REPLY_BYTES => {
            Err(format!("control message rejected: {message}").into())
        }
        _ => Err("control reply exceeds byte limit".into()),
    }
}

enum Endpoint {
    Path(PathBuf),
    App(String),
}

struct Message {
    endpoint: Endpoint,
    args: Vec<String>,
}

impl Message {
    fn parse(args: impl Iterator<Item = OsString>) -> Result<Self, &'static str> {
        let mut args = args.peekable();
        let mut endpoint = None;
        while let Some(flag) =
            args.next_if(|arg| matches!(arg.to_str(), Some("--" | "--app-id" | "--control-socket")))
        {
            if flag == "--" {
                break;
            }
            if endpoint.is_some() {
                return Err("msg requires exactly one of --app-id and --control-socket");
            }
            endpoint = Some(if flag == "--app-id" {
                let id = args.next().ok_or("msg --app-id requires a package ID")?;
                Endpoint::App(
                    id.into_string()
                        .map_err(|_| "msg package ID is not UTF-8")?,
                )
            } else {
                Endpoint::Path(
                    args.next()
                        .ok_or("msg --control-socket requires a path")?
                        .into(),
                )
            });
        }
        let endpoint =
            endpoint.ok_or("msg requires --app-id or --control-socket before its arguments")?;
        let mut argv = Vec::new();
        let mut bytes = 0;
        for arg in args {
            if argv.len() == control::MAX_ARGS {
                return Err("control request has too many arguments");
            }
            let arg = arg
                .into_string()
                .map_err(|_| "control arguments must be UTF-8")?;
            bytes += arg.len();
            if bytes > control::MAX_REQUEST_BYTES {
                return Err("control request exceeds byte limit");
            }
            argv.push(arg);
        }
        Ok(Self {
            endpoint,
            args: argv,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase", deny_unknown_fields)]
enum Reply<T> {
    Ok { result: T },
    Error { message: T },
}

fn encode(reply: Result<&str, &str>) -> io::Result<Vec<u8>> {
    let wire = match reply {
        Ok(result) => Reply::Ok { result },
        Err(message) => Reply::Error { message },
    };
    serde_json::to_vec(&wire).map_err(io::Error::other)
}

fn decode(frame: &str) -> Result<Vec<String>, String> {
    struct Argv;

    impl<'de> Visitor<'de> for Argv {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a JSON array of control argument strings")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut args = Vec::new();
            for _ in 0..control::MAX_ARGS {
                let Some(arg) = seq.next_element()? else {
                    return Ok(args);
                };
                args.push(arg);
            }
            if seq.next_element::<IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom(
                    "control request has too many arguments",
                ));
            }
            Ok(args)
        }
    }

    use serde::Deserializer as _;
    let mut decoder = serde_json::Deserializer::from_str(frame);
    let args = decoder
        .deserialize_seq(Argv)
        .map_err(|error| error.to_string())?;
    decoder.end().map_err(|error| error.to_string())?;
    Ok(args)
}

struct RequestFrame(Vec<u8>);

impl Write for RequestFrame {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > control::MAX_REQUEST_BYTES - self.0.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "control request exceeds byte limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn runtime_dir() -> Option<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn instance_socket_path(app_id: &str) -> io::Result<PathBuf> {
    let runtime = runtime_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "singleInstance requires XDG_RUNTIME_DIR"))?;
    let path = socket_path(&runtime, app_id, ".instance.sock")?;
    open_default_parent(&runtime, ParentMode::Create)?;
    Ok(path)
}

fn default_socket_path(runtime: &Path, app_id: &str) -> io::Result<PathBuf> {
    socket_path(runtime, app_id, ".sock")
}

fn socket_path(runtime: &Path, app_id: &str, suffix: &str) -> io::Result<PathBuf> {
    validate_absolute(runtime)?;
    if app_id.trim().is_empty()
        || app_id.contains('/')
        || app_id.contains('\\')
        || matches!(app_id, "." | "..")
        || app_id.chars().any(char::is_control)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "package ID cannot name a control socket",
        ));
    }
    let path = runtime.join(RUNTIME_SUBDIR).join(format!("{app_id}{suffix}"));
    validate_socket_path(&path)?;
    Ok(path)
}

enum ParentMode {
    Existing,
    Create,
}

fn open_default_parent(runtime: &Path, mode: ParentMode) -> io::Result<()> {
    let runtime = open_parent(runtime)?;
    require_private(&runtime, 0o777)?;
    let directory = fd_path(&runtime).join(RUNTIME_SUBDIR);
    if let ParentMode::Create = mode {
        match DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    require_private(&open_directory(&directory)?, 0o777)
}

fn validate_absolute(path: &Path) -> io::Result<()> {
    let bytes = path.as_os_str().as_bytes();
    let valid = bytes == b"/"
        || bytes.strip_prefix(b"/").is_some_and(|tail| {
            !tail
                .iter()
                .any(|byte| byte.is_ascii_control() || *byte == b'\\')
                && tail
                    .split(|&byte| byte == b'/')
                    .all(|part| !matches!(part, b"" | b"." | b".."))
        });
    if !valid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control path must be canonical and absolute",
        ));
    }
    Ok(())
}

fn validate_socket_path(path: &Path) -> io::Result<(&Path, &std::ffi::OsStr)> {
    validate_absolute(path)?;
    let (Some(directory), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control socket requires a file name",
        ));
    };
    SocketAddr::from_pathname(path)?;
    Ok((directory, name))
}

fn open_parent(path: &Path) -> io::Result<File> {
    let directory = open_canonical_dir(path)?;
    let metadata = directory.metadata()?;
    if metadata.uid() != effective_uid() || metadata.mode() & 0o022 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "control socket parent must be owned by the effective user and not writable by others",
        ));
    }
    Ok(directory)
}

fn open_inode(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
        .open(path)
}

fn fd_path(file: &File) -> PathBuf {
    // Linux procfs path lookup pins subsequent operations to the open inode,
    // even if an ancestor of the original pathname has been renamed.
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

fn connect(path: &Path) -> io::Result<UnixStream> {
    let (parent, name) = validate_socket_path(path)?;
    let parent = open_parent(parent)?;
    let inode = open_inode(&fd_path(&parent).join(name))?;
    let metadata = inode.metadata()?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "control endpoint must be an owner-only socket",
        ));
    }
    control::connect(&fd_path(&inode))
}

#[derive(Debug)]
pub(crate) struct BoundSocket {
    path: PathBuf,
    inode: Option<File>,
    _parent: File,
}

pub(crate) enum SocketElection {
    Live(UnixStream),
    Bound(UnixListener, BoundSocket),
}

fn same_socket(expected: &fs::Metadata, current: &fs::Metadata) -> bool {
    current.file_type().is_socket()
        && current.uid() == effective_uid()
        && (expected.dev(), expected.ino()) == (current.dev(), current.ino())
}

fn lock_election(directory: &File) -> io::Result<()> {
    use std::{fs::TryLockError, thread, time::{Duration, Instant}};
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        match directory.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => {
                let remaining = end.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "socket election deadline exceeded"));
                }
                thread::sleep(remaining.min(Duration::from_millis(5)));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
}

impl BoundSocket {
    fn bind(path: &Path) -> io::Result<(UnixListener, Self)> {
        let (parent, name) = validate_socket_path(path)?;
        Self::bind_in(open_parent(parent)?, name)
    }

    pub(crate) fn elect(path: &Path) -> io::Result<SocketElection> {
        let (parent, name) = validate_socket_path(path)?;
        let parent = open_parent(parent)?;
        require_private(&parent, 0o777)?;
        let anchor = fd_path(&parent);
        let path = anchor.join(name);
        // Serialize stale checks, unlink and publication, not the application
        // handshake. Lock the pinned directory without a second persistent path.
        let election = File::open(anchor)?;
        lock_election(&election)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "endpoint must be a Unix socket owned by the effective user (no symlinks)"));
                }
                let inode = open_inode(&path)?;
                if !same_socket(&metadata, &inode.metadata()?) {
                    return Err(io::Error::other("endpoint changed during inspection"));
                }
                match control::connect(&fd_path(&inode)) {
                    Ok(stream) => {
                        if metadata.mode() & 0o777 != 0o600 {
                            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "live endpoint is not mode 0600"));
                        }
                        return Ok(SocketElection::Live(stream));
                    }
                    Err(error) if matches!(error.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound) => {
                        match fs::symlink_metadata(&path) {
                            Ok(current) if same_socket(&metadata, &current) => fs::remove_file(&path)?,
                            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                            Err(error) => return Err(error),
                            Ok(_) => return Err(io::Error::other("endpoint changed during stale recovery")),
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        let (listener, bound) = Self::bind_in(parent, name)?;
        Ok(SocketElection::Bound(listener, bound))
    }

    fn bind_in(parent: File, name: &std::ffi::OsStr) -> io::Result<(UnixListener, Self)> {
        let anchored = fd_path(&parent);
        let path = anchored.join(name);
        let stage = Stage::new(&anchored)?;
        let staged_socket = stage.0.join(STAGE_SOCKET);
        // Bind behind a private directory, then publish the secured inode without
        // replacing an existing path. The Unix connection is resolved by inode.
        let listener = UnixListener::bind(&staged_socket)?;
        fs::set_permissions(&staged_socket, fs::Permissions::from_mode(0o600))?;
        let inode = open_inode(&staged_socket)?;
        fs::hard_link(&staged_socket, &path)?;
        Ok((
            listener,
            Self {
                path,
                inode: Some(inode),
                _parent: parent,
            },
        ))
    }

    pub(crate) fn unlink(&mut self) -> io::Result<()> {
        let Some(inode) = self.inode.take() else {
            return Ok(());
        };
        let unlink = || -> io::Result<()> {
            let published = fs::symlink_metadata(&self.path)?;
            let bound = inode.metadata()?;
            if same_socket(&bound, &published) {
                fs::remove_file(&self.path)?;
            }
            Ok(())
        };
        match unlink() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!("unlink control socket {}: {error}", self.path.display()),
            )),
            Ok(()) => Ok(()),
        }
    }
}

impl Drop for BoundSocket {
    fn drop(&mut self) {
        if let Err(error) = self.unlink() {
            let _ = writeln!(io::stderr().lock(), "{error}");
        }
    }
}

struct Stage(PathBuf);

impl Stage {
    fn new(parent: &Path) -> io::Result<Self> {
        for _ in 0..32 {
            let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(".ctl-{}-{sequence}", std::process::id()));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {
                    let stage = Self(path);
                    fs::set_permissions(&stage.0, fs::Permissions::from_mode(0o700))?;
                    return Ok(stage);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no free control socket staging directory",
        ))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join(STAGE_SOCKET));
        let _ = fs::remove_dir(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("zgui-control-test-{}-{id}", std::process::id()));
            DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .expect("make private control test directory");
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn published_socket_has_owner_only_permissions_and_is_removed_on_drop() {
        let dir = TestDir::new();
        let path = dir.0.join("control.sock");
        let (_listener, bound) = BoundSocket::bind(&path).expect("bind private control socket");
        let metadata = fs::symlink_metadata(&path).expect("stat published control socket");
        assert_eq!(
            metadata.mode() & 0o777,
            0o600,
            "published socket is accessible only to its owner"
        );
        assert!(
            metadata.file_type().is_socket(),
            "published path is the Unix socket inode"
        );
        let _client = connect(&path).expect("connect through owner-validated socket pathname");
        let _raw_client = UnixStream::connect(&path)
            .expect("standard socket clients can use the published pathname");
        let entries = fs::read_dir(&dir.0).expect("read parent directory").count();
        assert_eq!(entries, 1, "staging directory is gone after publication");
        drop(bound);
        assert!(!path.exists(), "only the owned socket is removed on drop");
    }

    #[test]
    fn same_app_cutover_rejects_queued_and_partial_requests() {
        use std::{
            io::{BufRead, BufReader},
            sync::mpsc,
            thread,
            time::Duration,
        };

        const DEADLINE: Duration = Duration::from_secs(5);

        let dir = TestDir::new();
        let path = dir.0.join("control.sock");
        let mut endpoint = Control {
            override_path: Some(path.clone()),
            ..Control::default()
        };
        let (wake, old_wakes) = mpsc::channel();
        endpoint
            .reconcile(
                "dev.test",
                || [1].into_iter(),
                || {
                    move || {
                        let _ = wake.send(());
                    }
                },
            )
            .expect("publish the old generation");
        let old_inode = open_inode(&path).expect("pin the old socket inode across cutover");

        let mut partial = UnixStream::connect(&path).expect("connect an old peer before cutover");
        partial
            .set_read_timeout(Some(DEADLINE))
            .expect("bound the partial peer read");
        partial
            .write_all(b"[\"partial")
            .expect("start an old request");
        let mut queued = UnixStream::connect(&path).expect("connect the queued old peer");
        queued
            .set_read_timeout(Some(DEADLINE))
            .expect("bound the queued peer read");
        queued
            .write_all(b"[\"queued\"]\n")
            .expect("submit a complete old request");
        old_wakes
            .recv_timeout(DEADLINE)
            .expect("old request is queued");

        endpoint
            .reconcile(
                "dev.test",
                || [1].into_iter(),
                || -> fn() { panic!("an unchanged route must keep its listener") },
            )
            .expect("reconcile the unchanged route");
        let (wake, new_wakes) = mpsc::channel();
        endpoint
            .reconcile(
                "dev.test",
                || [2].into_iter(),
                || {
                    move || {
                        let _ = wake.send(());
                    }
                },
            )
            .expect("publish the successor at the same path and app ID");
        let old = old_inode.metadata().expect("stat the pinned old socket");
        let new = fs::symlink_metadata(&path).expect("stat the successor socket");
        assert_ne!(
            (old.dev(), old.ino()),
            (new.dev(), new.ino()),
            "the old kernel connection backlog cannot be reused by the successor",
        );

        // Completing the old frame can race with worker retirement, but it
        // must never reach the successor.
        let _ = partial.write_all(b"\"]\n");
        let mut reply = Vec::new();
        BufReader::new(queued)
            .read_until(b'\n', &mut reply)
            .expect("read the old queue rejection");
        assert_eq!(
            serde_json::from_slice::<Reply<String>>(&reply)
                .expect("decode the old queue rejection"),
            Reply::Error {
                message: "control listener retired".to_owned()
            },
        );

        reply.clear();
        match BufReader::new(partial).read_until(b'\n', &mut reply) {
            Ok(0) => {}
            Ok(_) => assert_eq!(
                serde_json::from_slice::<Reply<String>>(&reply)
                    .expect("decode the old peer rejection"),
                Reply::Error {
                    message: "control listener retired".to_owned()
                },
            ),
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            Err(error) => panic!("the old peer should disconnect, not wait: {error}"),
        }

        let fresh = connect(&path).expect("connect after cutover");
        let client = thread::spawn(|| control::exchange(fresh, br#"["fresh"]"#));
        new_wakes
            .recv_timeout(DEADLINE)
            .expect("fresh request is queued");
        let (app_id, generations, request) =
            endpoint.try_recv().expect("receive the fresh request");
        assert_eq!(app_id, "dev.test");
        assert_eq!(generations, &[2]);
        assert_eq!(
            request.args,
            ["fresh"],
            "only post-cutover ingress reaches the successor",
        );
        request.reply(Ok("new generation".to_owned()));
        assert!(
            endpoint.try_recv().is_none(),
            "no other old-peer request may reach the successor",
        );
        let reply = client
            .join()
            .expect("join fresh peer")
            .expect("read the fresh reply");
        assert_eq!(
            serde_json::from_slice::<Reply<String>>(&reply).expect("decode the fresh reply"),
            Reply::Ok {
                result: "new generation".to_owned()
            },
        );
        endpoint.close().expect("close the successor");
        assert!(!path.exists(), "shutdown removes the current endpoint");
    }

    #[test]
    fn existing_or_replacement_files_are_not_unlinked() {
        let dir = TestDir::new();
        let path = dir.0.join("control.sock");
        fs::write(&path, "existing").expect("preexisting endpoint file");
        let error = BoundSocket::bind(&path).expect_err("refuse to replace preexisting endpoint");
        assert_eq!(
            error.kind(),
            io::ErrorKind::AlreadyExists,
            "existing path is a bind error"
        );
        assert_eq!(
            fs::read_to_string(&path).expect("read preserved file"),
            "existing",
            "preexisting contents are preserved"
        );
        fs::remove_file(&path).expect("clear preexisting file");
        let (_listener, bound) = BoundSocket::bind(&path).expect("bind control socket");
        fs::remove_file(&path).expect("unlink control socket before replacing it");
        fs::write(&path, "replacement").expect("place unrelated endpoint file");
        drop(bound);
        assert_eq!(
            fs::read_to_string(&path).expect("read preserved replacement"),
            "replacement",
            "teardown does not delete unrelated replacements"
        );
    }

    #[test]
    fn noncanonical_or_untrusted_parents_are_rejected() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new();
        let real = dir.0.join("real");
        fs::create_dir(&real).expect("create real endpoint directory");
        let alias = dir.0.join("alias");
        symlink(&real, &alias).expect("make alias for control endpoint directory");
        assert!(
            BoundSocket::bind(&alias.join("control.sock")).is_err(),
            "symlink parent is not canonical"
        );
        assert!(
            BoundSocket::bind(&real.join("../other.sock")).is_err(),
            "traversal is not canonical"
        );
        fs::set_permissions(&real, fs::Permissions::from_mode(0o777))
            .expect("make parent writable by other users");
        assert!(
            BoundSocket::bind(&real.join("control.sock")).is_err(),
            "another user must not be able to swap the endpoint"
        );
    }

    #[test]
    fn default_endpoint_creates_a_private_subdirectory_without_traversal() {
        let dir = TestDir::new();
        let path = default_socket_path(&dir.0, "dev.test")
            .expect("resolve package endpoint under runtime dir");
        assert_eq!(
            path,
            dir.0.join("zgui-js/dev.test.sock"),
            "default endpoint uses the package ID"
        );
        assert!(
            !path.parent().expect("default socket has a parent").exists(),
            "resolving a client path does not create directories"
        );
        open_default_parent(&dir.0, ParentMode::Create).expect("create private server directory");
        let metadata = fs::metadata(path.parent().expect("default socket has a parent"))
            .expect("stat server directory");
        assert_eq!(
            metadata.mode() & 0o777,
            0o700,
            "server directory is private"
        );
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o1700))
            .expect("set the sticky bit on the runtime directory");
        fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o1700))
            .expect("set the sticky bit on the control subdirectory");
        open_default_parent(&dir.0, ParentMode::Existing)
            .expect("the control CLI checks ordinary permission bits only");
        assert!(
            default_socket_path(&dir.0, "../dev.test").is_err(),
            "package ID cannot escape its directory"
        );
    }

    #[test]
    fn delimiter_preserves_literal_flags_and_json_preserves_argument_boundaries() {
        let message = Message::parse(
            [
                "--app-id",
                "test.app",
                "--",
                "--control-socket",
                "",
                "field\nvalue",
                "☀",
            ]
            .map(OsString::from)
            .into_iter(),
        )
        .expect("parse control CLI argv");
        assert_eq!(
            message.args,
            ["--control-socket", "", "field\nvalue", "☀"],
            "tokens after the delimiter are all payload"
        );
        assert_eq!(
            decode(r#"["--control-socket","","field\nvalue", "☀"]"#).expect("decode UTF-8 argv"),
            message.args,
            "request strings are not shell-joined"
        );
        assert!(
            Message::parse(
                ["--app-id", "x", "--control-socket", "/a.sock"]
                    .map(OsString::from)
                    .into_iter()
            )
            .is_err(),
            "two endpoint selectors are ambiguous"
        );
        let overflow = serde_json::to_string(&vec![""; control::MAX_ARGS + 1])
            .expect("encode too many arguments");
        assert!(
            decode(&overflow).is_err(),
            "oversized argv count is rejected"
        );
        assert!(
            decode(r#"["ok", 1]"#).is_err(),
            "only strings are accepted as arguments"
        );
    }

    #[test]
    fn reply_payload_newlines_are_escaped_in_single_frame() {
        let frame = encode(Ok("one\ntwo\n")).expect("encode multiline success");
        assert!(
            !frame.contains(&b'\n'),
            "only the transport appends a physical LF"
        );
        assert_eq!(
            serde_json::from_slice::<Reply<String>>(&frame).expect("decode reply"),
            Reply::Ok {
                result: "one\ntwo\n".to_owned()
            },
            "decoded line breaks survive an exact round trip"
        );
        let error = encode(Err("invalid\ninput")).expect("encode multiline error");
        assert_eq!(
            serde_json::from_slice::<Reply<String>>(&error).expect("decode error"),
            Reply::Error {
                message: "invalid\ninput".to_owned()
            },
            "errors are distinct from successful payloads"
        );
    }
}
