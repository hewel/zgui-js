//! Direct niri event-stream subscriptions. No compositor actions or CLI processes are used.
//!
//! A worker retains current state, not an event queue. `take` creates sorted vectors of shared
//! immutable rows only when the owner consumes a delivery. Intermediate states may coalesce.
//! Protocol facts follow niri's `niri-ipc` at 8ed0da4 (26.04); unrelated event tags and extra row
//! fields are intentionally ignored for forward compatibility.

use crate::ServiceError;
use serde::{Deserialize, Deserializer, de::{self, MapAccess, Visitor}};
use serde_json::value::RawValue;
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fmt,
    io::{self, BufRead, BufReader, Write},
    mem,
    net::Shutdown,
    os::{fd::{AsRawFd, FromRawFd, OwnedFd}, unix::{ffi::OsStrExt, net::UnixStream}},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    thread,
};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_ROWS: usize = 16_384;
const MAX_STATE_TEXT_BYTES: usize = 8 * 1024 * 1024;
const POLL_MILLIS: i32 = 100;

/// An owned normalized window. IDs retain all 64 bits.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct NiriWindow {
    pub id: u64,
    #[serde(deserialize_with = "required_option")]
    pub title: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub app_id: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub pid: Option<u32>,
    #[serde(deserialize_with = "required_option")]
    pub workspace_id: Option<u64>,
    pub is_focused: bool,
    pub is_floating: bool,
    pub is_urgent: bool,
}

/// An owned normalized workspace. An active workspace need not be focused.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct NiriWorkspace {
    pub id: u64,
    pub idx: u8,
    #[serde(deserialize_with = "required_option")]
    pub name: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub output: Option<String>,
    pub is_urgent: bool,
    pub is_active: bool,
    pub is_focused: bool,
    #[serde(deserialize_with = "required_option")]
    pub active_window_id: Option<u64>,
}

/// A consistent owner-side observation, sorted by ascending row ID.
/// Cross-references may briefly refer to absent rows, as permitted by niri's event protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NiriSnapshot {
    pub windows: Vec<Arc<NiriWindow>>,
    pub workspaces: Vec<Arc<NiriWorkspace>>,
}

/// One background connection, with at most one pending snapshot or terminal error.
///
/// `start` only spawns work; connect, handshake and reads happen on that worker. The wake
/// function must be a nonblocking, thread-safe owner notification, not a UI/JS callback.
/// Connection, EOF and protocol failures are terminal (no reconnect). `cancel` and Drop
/// discard pending delivery and shut down an installed socket without joining the worker.
/// Cancellation also covers a connect in progress; the worker polls at most every 100 ms.
/// Frames are limited to 1 MiB, state to 16,384 rows of each kind and 8 MiB of row text.
/// Error codes are `Io`, `NiriProtocol`, and `TooLarge`.
pub struct NiriSubscription {
    shared: Arc<Shared>,
}

struct Shared {
    inner: Mutex<Inner>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct Inner {
    cancelled: bool,
    pending: bool,
    terminal: Option<ServiceError>,
    socket: Option<UnixStream>,
    state: State,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn cancelled(&self) -> bool {
        self.lock().cancelled
    }

    fn finish(&self, result: Result<(), ServiceError>) {
        let notify = {
            let mut inner = self.lock();
            inner.socket.take();
            if inner.cancelled {
                return;
            }
            // A worker may only exit normally after cancellation.
            let error = result.err().unwrap_or_else(|| ServiceError::new("Io", "niri worker stopped"));
            inner.terminal = Some(error);
            let notify = !inner.pending;
            inner.pending = true;
            notify
        };
        if notify {
            (self.wake)();
        }
    }

    fn apply(&self, event: Event) -> Result<(), ServiceError> {
        let notify = {
            let mut inner = self.lock();
            if inner.cancelled {
                return Ok(());
            }
            let changed = inner.state.apply(event)?;
            let notify = changed && inner.state.ready() && !inner.pending;
            if notify {
                inner.pending = true;
            }
            notify
        };
        if notify {
            (self.wake)();
        }
        Ok(())
    }
}

impl NiriSubscription {
    pub fn start(path: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self, ServiceError> {
        let shared = Arc::new(Shared { inner: Mutex::new(Inner::default()), wake });
        let worker = Arc::clone(&shared);
        thread::Builder::new().name("niri-events".into()).spawn(move || {
            let result = run(&path, &worker);
            worker.finish(result);
        }).map_err(ServiceError::from)?;
        Ok(Self { shared })
    }

    /// Consume the latest state or one terminal error. No deliveries follow cancellation.
    pub fn take(&self) -> Option<Result<NiriSnapshot, ServiceError>> {
        let mut inner = self.shared.lock();
        if inner.cancelled || !inner.pending {
            return None;
        }
        inner.pending = false;
        if let Some(error) = inner.terminal.take() {
            Some(Err(error))
        } else {
            Some(Ok(inner.state.snapshot()))
        }
    }

    pub fn has_pending(&self) -> bool {
        let inner = self.shared.lock();
        !inner.cancelled && inner.pending
    }

    /// Idempotently suppress delivery and interrupt transport. Does not wait for the worker.
    pub fn cancel(&self) {
        let mut inner = self.shared.lock();
        inner.cancelled = true;
        inner.pending = false;
        inner.terminal = None;
        inner.state = State::default();
        if let Some(socket) = inner.socket.take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for NiriSubscription {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn protocol(message: impl Into<String>) -> ServiceError {
    ServiceError::new("NiriProtocol", message)
}

// AF_UNIX connect can block on a full listen backlog. Build it nonblocking from the outset,
// register the shutdown handle before connecting, and poll with cancellation checks.
fn connect(path: &Path, shared: &Shared) -> Result<UnixStream, ServiceError> {
    let bytes = OsStr::new(path).as_bytes();
    // SAFETY: sockaddr_un is a plain C record; its zero representation is valid.
    let mut address: libc::sockaddr_un = unsafe { mem::zeroed() };
    if bytes.is_empty() || bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(protocol("invalid niri Unix socket path"));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    // SAFETY: arguments are valid constants; the returned descriptor is owned exactly once.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: a successful socket call returned a new owned descriptor.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let socket = UnixStream::from(fd);
    {
        let mut inner = shared.lock();
        if inner.cancelled {
            return Ok(socket);
        }
        inner.socket = Some(socket.try_clone()?);
    }
    let len = mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    // SAFETY: address and length describe initialized sockaddr_un storage for this call.
    let result = unsafe { libc::connect(socket.as_raw_fd(), (&address as *const libc::sockaddr_un).cast(), len as libc::socklen_t) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(error.into());
        }
        wait_ready(&socket, libc::POLLOUT, shared)?;
        if let Some(error) = socket.take_error()? {
            return Err(error.into());
        }
    }
    Ok(socket)
}

fn wait_ready(socket: &UnixStream, events: libc::c_short, shared: &Shared) -> Result<(), ServiceError> {
    loop {
        if shared.cancelled() {
            return Err(ServiceError::new("Cancelled", "niri subscription was cancelled"));
        }
        let mut fd = libc::pollfd { fd: socket.as_raw_fd(), events, revents: 0 };
        // SAFETY: poll borrows one initialized descriptor record for the duration of the call.
        let result = unsafe { libc::poll(&mut fd, 1, POLL_MILLIS) };
        if result > 0 {
            return Ok(());
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        }
    }
}

fn run(path: &Path, shared: &Shared) -> Result<(), ServiceError> {
    let mut socket = connect(path, shared)?;
    let mut request = &b"\"EventStream\"\n"[..];
    while !request.is_empty() {
        if shared.cancelled() {
            return Ok(());
        }
        match socket.write(request) {
            Ok(0) => return Err(protocol("niri socket closed during handshake")),
            Ok(written) => request = &request[written..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => wait_ready(&socket, libc::POLLOUT, shared)?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            Err(error) => return Err(error.into()),
        }
    }
    let mut reader = BufReader::new(socket);
    let mut line = Vec::new();
    read_line(&mut reader, &mut line, shared)?;
    let reply: Result<String, String> = serde_json::from_slice(&line).map_err(|error| protocol(format!("invalid niri handshake: {error}")))?;
    match reply {
        Ok(response) if response == "Handled" => {},
        Ok(_) => return Err(protocol("niri did not acknowledge EventStream")),
        Err(message) => return Err(protocol(format!("niri rejected EventStream: {message}"))),
    }
    loop {
        read_line(&mut reader, &mut line, shared)?;
        let Some(event) = parse_event(&line)? else { continue };
        shared.apply(event)?;
    }
}

fn read_line(reader: &mut BufReader<UnixStream>, line: &mut Vec<u8>, shared: &Shared) -> Result<(), ServiceError> {
    line.clear();
    loop {
        if shared.cancelled() {
            return Err(ServiceError::new("Cancelled", "niri subscription was cancelled"));
        }
        let available = match reader.fill_buf() {
            Ok([]) => return Err(protocol("niri event stream disconnected")),
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                wait_ready(reader.get_ref(), libc::POLLIN, shared)?;
                continue;
            },
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        if line.len() + count > MAX_LINE_BYTES {
            return Err(ServiceError::new("TooLarge", "niri IPC frame exceeds 1 MiB"));
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            return Ok(());
        }
    }
}

fn required_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

#[derive(Deserialize)]
enum Event {
    WorkspacesChanged { workspaces: Vec<NiriWorkspace> },
    WorkspaceUrgencyChanged { id: u64, urgent: bool },
    WorkspaceActivated { id: u64, focused: bool },
    WorkspaceActiveWindowChanged {
        workspace_id: u64,
        #[serde(deserialize_with = "required_option")]
        active_window_id: Option<u64>,
    },
    WindowsChanged { windows: Vec<NiriWindow> },
    WindowOpenedOrChanged { window: NiriWindow },
    WindowClosed { id: u64 },
    WindowFocusChanged {
        #[serde(deserialize_with = "required_option")]
        id: Option<u64>,
    },
    WindowUrgencyChanged { id: u64, urgent: bool },
}

// Inspect just the tag, borrowing the raw payload instead of constructing a second JSON tree.
// Unknown event payloads are still required to be valid, bounded JSON.
#[derive(Deserialize)]
struct EventTag<'a>(#[serde(borrow)] std::borrow::Cow<'a, str>);

struct Envelope<'a>(EventTag<'a>);
impl<'de> Deserialize<'de> for Envelope<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EnvelopeVisitor;
        impl<'de> Visitor<'de> for EnvelopeVisitor {
            type Value = Envelope<'de>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a single externally tagged event")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let (tag, _): (EventTag<'de>, &'de RawValue) = map.next_entry()?.ok_or_else(|| de::Error::custom("empty event"))?;
                if map.next_entry::<de::IgnoredAny, de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("multiple event tags"));
                }
                Ok(Envelope(tag))
            }
        }
        deserializer.deserialize_map(EnvelopeVisitor)
    }
}

fn parse_event(line: &[u8]) -> Result<Option<Event>, ServiceError> {
    let Envelope(EventTag(tag)) = serde_json::from_slice::<Envelope<'_>>(line).map_err(|error| protocol(format!("invalid niri event: {error}")))?;
    match tag.as_ref() {
        "WorkspacesChanged" | "WorkspaceUrgencyChanged" | "WorkspaceActivated" |
        "WorkspaceActiveWindowChanged" | "WindowsChanged" | "WindowOpenedOrChanged" |
        "WindowClosed" | "WindowFocusChanged" | "WindowUrgencyChanged" => {
            serde_json::from_slice(line).map(Some).map_err(|error| protocol(format!("malformed {tag}: {error}")))
        },
        _ => Ok(None),
    }
}

#[derive(Default)]
struct State {
    windows: BTreeMap<u64, Arc<NiriWindow>>,
    workspaces: BTreeMap<u64, Arc<NiriWorkspace>>,
    have_windows: bool,
    have_workspaces: bool,
    text_bytes: usize,
}

impl NiriWindow {
    fn text_bytes(&self) -> usize {
        self.title.as_ref().map_or(0, String::len) + self.app_id.as_ref().map_or(0, String::len)
    }
}

impl NiriWorkspace {
    fn text_bytes(&self) -> usize {
        self.name.as_ref().map_or(0, String::len) + self.output.as_ref().map_or(0, String::len)
    }
}

impl State {
    fn ready(&self) -> bool {
        self.have_windows && self.have_workspaces
    }

    fn snapshot(&self) -> NiriSnapshot {
        NiriSnapshot { windows: self.windows.values().cloned().collect(), workspaces: self.workspaces.values().cloned().collect() }
    }

    fn apply(&mut self, event: Event) -> Result<bool, ServiceError> {
        let mut changed = false;
        match event {
            Event::WindowsChanged { windows } => {
                check_rows(windows.len())?;
                let mut next = BTreeMap::new();
                let mut bytes = 0;
                for window in windows {
                    bytes += window.text_bytes();
                    if next.insert(window.id, Arc::new(window)).is_some() {
                        return Err(protocol("duplicate window ID"));
                    }
                }
                self.text_bytes = self.replacement_text_bytes(
                    self.windows.values().map(|row| row.text_bytes()).sum(), bytes,
                )?;
                changed = !self.have_windows || self.windows != next;
                self.windows = next;
                self.have_windows = true;
            },
            Event::WorkspacesChanged { workspaces } => {
                check_rows(workspaces.len())?;
                let mut next = BTreeMap::new();
                let mut bytes = 0;
                for workspace in workspaces {
                    bytes += workspace.text_bytes();
                    if next.insert(workspace.id, Arc::new(workspace)).is_some() {
                        return Err(protocol("duplicate workspace ID"));
                    }
                }
                self.text_bytes = self.replacement_text_bytes(
                    self.workspaces.values().map(|row| row.text_bytes()).sum(), bytes,
                )?;
                changed = !self.have_workspaces || self.workspaces != next;
                self.workspaces = next;
                self.have_workspaces = true;
            },
            Event::WindowOpenedOrChanged { window } => {
                if !self.windows.contains_key(&window.id) {
                    check_rows(self.windows.len() + 1)?;
                }
                let previous = self.windows.get(&window.id);
                let row_changed = previous.is_none_or(|previous| **previous != window);
                let text_bytes = self.replacement_text_bytes(
                    previous.map_or(0, |row| row.text_bytes()), window.text_bytes(),
                )?;
                if window.is_focused {
                    for row in self.windows.values_mut() {
                        if row.id != window.id && row.is_focused {
                            Arc::make_mut(row).is_focused = false;
                            changed = true;
                        }
                    }
                }
                if row_changed {
                    self.text_bytes = text_bytes;
                    self.windows.insert(window.id, Arc::new(window));
                    changed = true;
                }
            },
            Event::WindowClosed { id } => {
                if let Some(window) = self.windows.remove(&id) {
                    self.text_bytes -= window.text_bytes();
                    changed = true;
                }
            },
            Event::WindowFocusChanged { id } => changed = self.focus_window(id),
            Event::WindowUrgencyChanged { id, urgent } => {
                if let Some(row) = self.windows.get_mut(&id) {
                    if row.is_urgent != urgent {
                        Arc::make_mut(row).is_urgent = urgent;
                        changed = true;
                    }
                }
            },
            Event::WorkspaceUrgencyChanged { id, urgent } => {
                if let Some(row) = self.workspaces.get_mut(&id) {
                    if row.is_urgent != urgent {
                        Arc::make_mut(row).is_urgent = urgent;
                        changed = true;
                    }
                }
            },
            Event::WorkspaceActiveWindowChanged { workspace_id, active_window_id } => {
                let row = self.workspaces.get_mut(&workspace_id).ok_or_else(|| protocol("active window changed on an unknown workspace"))?;
                if row.active_window_id != active_window_id {
                    Arc::make_mut(row).active_window_id = active_window_id;
                    changed = true;
                }
            },
            Event::WorkspaceActivated { id, focused } => {
                // Temporarily remove the target so output comparison borrows its string instead
                // of cloning it. References to absent windows remain valid protocol states.
                let mut target = self.workspaces.remove(&id).ok_or_else(|| protocol("unknown workspace activated"))?;
                for row in self.workspaces.values_mut() {
                    let active = row.is_active && row.output != target.output;
                    let focus = row.is_focused && !focused;
                    if row.is_active != active || row.is_focused != focus {
                        let row = Arc::make_mut(row);
                        row.is_active = active;
                        row.is_focused = focus;
                        changed = true;
                    }
                }
                if !target.is_active || (focused && !target.is_focused) {
                    let row = Arc::make_mut(&mut target);
                    row.is_active = true;
                    row.is_focused |= focused;
                    changed = true;
                }
                self.workspaces.insert(id, target);
            },
        }
        Ok(changed)
    }

    fn replacement_text_bytes(&self, removed: usize, added: usize) -> Result<usize, ServiceError> {
        let bytes = self.text_bytes - removed + added;
        if bytes > MAX_STATE_TEXT_BYTES {
            Err(ServiceError::new("TooLarge", "niri state exceeds 8 MiB of row text"))
        } else {
            Ok(bytes)
        }
    }

    fn focus_window(&mut self, id: Option<u64>) -> bool {
        let mut changed = false;
        for row in self.windows.values_mut() {
            let focused = Some(row.id) == id;
            if row.is_focused != focused {
                Arc::make_mut(row).is_focused = focused;
                changed = true;
            }
        }
        changed
    }
}

fn check_rows(count: usize) -> Result<(), ServiceError> {
    if count > MAX_ROWS {
        Err(ServiceError::new("TooLarge", "niri row count exceeds 16,384"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
