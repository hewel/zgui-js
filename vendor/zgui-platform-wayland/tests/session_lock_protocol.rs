//! Real session-lock requests and shared-memory pixels on a private Wayland socket.

#![cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{IoSliceMut, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, recvmsg};
use zgui_platform::{
    AppHandler, InputRegion, MonitorTarget, PlatformCx, PlatformError, SessionLockState, Surface,
    SurfaceAttributes, SurfaceEvent, SurfaceId, SurfaceRole, WakeReason,
};
use zgui_platform_wayland::WaylandApp;

const CLIENT: &str = "ZGUI_SESSION_LOCK_PROTOCOL_CLIENT";
const TEST: &str = "lock_surface_lifecycle_on_the_wire";
const TIMEOUT: Duration = Duration::from_secs(5);
const FIRST: u32 = 6;
const SECOND: u32 = 7;
const VIEWPORTER: u32 = 8;
const INITIAL: [u32; 2] = [32, 20];
const RESIZED: [u32; 2] = [48, 25];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Unlock,
    UnlockAndExit,
    Disconnect,
    Refuse,
    Finish,
    NoSubcompositor,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Self::Unlock => "unlock",
            Self::UnlockAndExit => "unlock-and-exit",
            Self::Disconnect => "disconnect",
            Self::Refuse => "refuse",
            Self::Finish => "finish",
            Self::NoSubcompositor => "no-subcompositor",
        }
    }

    fn explicitly_unlocks(self) -> bool {
        matches!(self, Self::Unlock | Self::UnlockAndExit)
    }
}

#[test]
fn lock_surface_lifecycle_on_the_wire() {
    let scenarios = [
        Scenario::Unlock,
        Scenario::UnlockAndExit,
        Scenario::Disconnect,
        Scenario::Refuse,
        Scenario::Finish,
        Scenario::NoSubcompositor,
    ];
    if let Ok(client) = std::env::var(CLIENT) {
        let scenario = scenarios
            .into_iter()
            .find(|scenario| scenario.name() == client)
            .expect("known fixture scenario");
        let mut app = WaylandApp::new(Locker::new(scenario)).expect("connect to the fixture");
        app.run().expect("run the actual backend");
        assert!(app.handler().complete, "{scenario:?} did not complete");
        return;
    }
    for scenario in scenarios {
        run_fixture(scenario);
    }
}

fn run_fixture(scenario: Scenario) {
    let path = std::env::temp_dir().join(format!(
        "zgui-lock-{}-{}.sock",
        std::process::id(),
        scenario.name()
    ));
    let listener = UnixListener::bind(&path).expect("bind a private Wayland socket");
    let _socket = SocketFile(path.clone());
    listener
        .set_nonblocking(true)
        .expect("bound the accept wait");
    let mut child = Command::new(std::env::current_exe().expect("locate this test binary"))
        .args(["--exact", TEST])
        .env(CLIENT, scenario.name())
        .env("WAYLAND_DISPLAY", path)
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .expect("launch the Wayland backend");
    let start = Instant::now();
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    child.try_wait().expect("poll client").is_none(),
                    "client exited before connecting"
                );
                assert!(start.elapsed() < TIMEOUT, "client did not connect");
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("accepting test client: {error}"),
        }
    };
    stream
        .set_read_timeout(Some(TIMEOUT))
        .expect("bound each read");
    stream
        .set_write_timeout(Some(TIMEOUT))
        .expect("bound each write");
    let mut server = Compositor::new(scenario, stream);
    server.run();
    let status = child.wait().expect("wait for test client");
    assert!(status.success(), "{scenario:?} client exited with {status}");
    assert_eq!(
        server.lock_requests,
        u32::from(scenario != Scenario::NoSubcompositor),
        "repeated requests must be idempotent"
    );
    match scenario {
        Scenario::Unlock | Scenario::UnlockAndExit => {
            assert_eq!(
                server.coverage,
                [(FIRST, INITIAL), (FIRST, RESIZED), (SECOND, INITIAL)],
                "fallback covers the initial output, its resize, and the hotplugged output"
            );
            assert_eq!(
                server.claims, 3,
                "the predecessor and both candidate attempts use the same lock role"
            );
            assert_eq!(
                server.role_destroys, [SECOND, FIRST],
                "only hotplug removal and explicit unlock retire the two parent roles"
            );
            assert!(
                server.unlock_sent && server.sync_answered,
                "explicit unlock waits for the compositor sync"
            );
        }
        Scenario::Disconnect => {
            assert_eq!(
                server.coverage,
                [(FIRST, INITIAL), (FIRST, RESIZED)],
                "the initial output stays covered when resized before disconnect"
            );
            assert!(
                server.locked,
                "a disappeared client must leave the session locked"
            );
            assert!(
                server.role_destroys.is_empty(),
                "disconnect leaves active roles to the compositor"
            );
            assert!(!server.unlock_sent, "disconnect must not send an unlock request");
        }
        Scenario::Refuse => {
            assert!(server.coverage.is_empty(), "refused acquisition has no committed coverage");
            assert!(
                server.refusal_destroyed,
                "a refused lock uses the ordinary destructor"
            );
            assert!(!server.unlock_sent, "refusal cannot authorize an unlock request");
        }
        Scenario::Finish => {
            assert_eq!(
                server.coverage,
                [(FIRST, INITIAL)],
                "the acquired output is covered before the compositor finishes the lock"
            );
            assert!(
                server.finished,
                "the fixture terminated the acquired lock object"
            );
            assert!(!server.unlock_sent, "compositor finish is not authorization to unlock");
        }
        Scenario::NoSubcompositor => {
            assert!(server.outputs.is_empty(), "missing prerequisites prevent role creation");
            assert!(!server.unlock_sent, "unsupported locking cannot trigger an unlock");
        }
    }
}

struct SocketFile(std::path::PathBuf);

impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct Locker {
    scenario: Scenario,
    first: Option<Arc<dyn Surface>>,
    replacement: Option<Arc<dyn Surface>>,
    hotplug_seen: bool,
    hotplug_removed: bool,
    locked_seen: bool,
    complete: bool,
}

impl Locker {
    fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            first: None,
            replacement: None,
            hotplug_seen: false,
            hotplug_removed: false,
            locked_seen: false,
            complete: false,
        }
    }
}

fn lock_attributes(replacing_lock: Option<SurfaceId>) -> SurfaceAttributes {
    let mut attrs = SurfaceAttributes::default();
    attrs.role = SurfaceRole::Lock(MonitorTarget::Name("TEST-1".to_owned()));
    attrs.replacing_lock = replacing_lock;
    attrs
}

impl AppHandler for Locker {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        if self.scenario == Scenario::NoSubcompositor {
            assert!(
                !cx.capabilities().session_lock,
                "a lock without subsurfaces cannot display authentication content"
            );
            assert!(
                matches!(cx.request_session_lock(), Err(PlatformError::Unsupported(_))),
                "a missing subcompositor makes the lock capability unavailable"
            );
            assert_eq!(
                cx.session_lock_state(),
                SessionLockState::Unlocked,
                "an unsupported request leaves the session unlocked"
            );
            self.complete = true;
            cx.request_exit();
            return;
        }
        assert!(cx.capabilities().session_lock, "all lock prerequisites are present");
        assert_eq!(
            cx.session_lock_state(),
            SessionLockState::Unlocked,
            "the initial lock state is unlocked"
        );
        assert!(cx.try_unlock_session().is_err(), "no lock exists to unlock");
        cx.request_session_lock().expect("request session lock");
        cx.request_session_lock()
            .expect("pending requests are idempotent");
        assert_eq!(
            cx.session_lock_state(),
            SessionLockState::Pending,
            "a request is not acquisition until the compositor confirms it"
        );
        assert!(
            cx.try_unlock_session().is_err(),
            "cannot unlock an unconfirmed lock"
        );
        assert!(
            cx.surfaces().is_empty(),
            "dark fallback is owned by the backend"
        );
        if self.scenario.explicitly_unlocks() {
            let attrs = lock_attributes(None);
            let first = cx.create_surface(&attrs).expect("claim the platform-owned output");
            assert!(cx.create_surface(&attrs).is_err(), "implicit replacement is forbidden");
            let replacement_attrs = lock_attributes(Some(first.id()));
            let rejected = cx
                .create_surface(&replacement_attrs)
                .expect("stage a candidate without releasing the predecessor");
            assert_ne!(first.id(), rejected.id(), "overlapping claims need independent handles");
            assert!(
                cx.create_surface(&replacement_attrs).is_err(),
                "only one replacement can be staged at a time"
            );
            assert!(
                cx.create_surface(&lock_attributes(Some(rejected.id()))).is_err(),
                "a staged candidate does not own the primary claim"
            );
            cx.destroy_surface(rejected.id());
            assert!(
                cx.surface(rejected.id()).is_none(),
                "rejecting a candidate removes its own live surface immediately"
            );
            assert!(
                cx.surface(first.id()).is_some(),
                "rejecting a candidate must leave its predecessor live"
            );
            let replacement = cx
                .create_surface(&replacement_attrs)
                .expect("stage another candidate after rejection");
            assert_ne!(replacement.id(), rejected.id(), "rejected handles cannot be reused");
            first
                .set_shell_input_region(&InputRegion::Empty)
                .expect("claim before configure");
            first.set_visible(false);
            first.set_visible(true);
            self.first = Some(first);
            self.replacement = Some(replacement);
        }
    }

    fn surface_event(&mut self, _cx: &dyn PlatformCx, _id: SurfaceId, _event: SurfaceEvent) {}

    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        if matches!(reason, WakeReason::MonitorsChanged) {
            let second = cx
                .monitors()
                .iter()
                .any(|m| m.name.as_deref() == Some("TEST-2"));
            if second && !self.hotplug_seen {
                self.hotplug_seen = true;
                if self.scenario == Scenario::Disconnect {
                    assert_eq!(
                        cx.session_lock_state(),
                        SessionLockState::Locked,
                        "hotplug before disconnect must leave the lock acquired"
                    );
                    self.complete = true;
                    cx.request_exit();
                } else if self.scenario.explicitly_unlocks() {
                    let first = self.first.as_ref().expect("predecessor is still claimed");
                    let replacement = self.replacement.as_ref().expect("candidate remains staged");
                    assert_eq!(first.size().width.0, 48.0, "configure resized the predecessor");
                    assert_eq!(replacement.size(), first.size(), "configure also resized the candidate");
                    if self.scenario == Scenario::Unlock {
                        cx.destroy_surface(first.id());
                        assert!(cx.surface(first.id()).is_none(), "promotion retires the old claim");
                        assert!(
                            cx.surface(replacement.id()).is_some(),
                            "promotion keeps the candidate live without reacquiring a lock role"
                        );
                        assert!(
                            cx.create_surface(&lock_attributes(Some(first.id()))).is_err(),
                            "a retired predecessor cannot authorize another replacement"
                        );
                        self.first = self.replacement.take();
                    }
                }
            } else if !second && self.hotplug_seen && !self.hotplug_removed {
                self.hotplug_removed = true;
                assert_eq!(
                    cx.session_lock_state(),
                    SessionLockState::Locked,
                    "removing an output does not release the remaining lock"
                );
                cx.try_unlock_session().expect("explicit unlock");
                assert_eq!(
                    cx.session_lock_state(),
                    SessionLockState::Unlocking,
                    "explicit unlock is pending until the compositor sync"
                );
                assert!(cx.try_unlock_session().is_err(), "do not send a second unlock");
                if self.scenario == Scenario::UnlockAndExit {
                    cx.request_exit();
                }
            }
            return;
        }
        if !matches!(
            reason,
            WakeReason::AppWork | WakeReason::SessionLocked | WakeReason::SessionLockFinished
        ) {
            return;
        }
        match cx.session_lock_state() {
            SessionLockState::Locked if !self.locked_seen => {
                self.locked_seen = true;
                cx.request_session_lock()
                    .expect("locked requests are idempotent");
            }
            SessionLockState::Refused if self.scenario == Scenario::Refuse => {
                assert!(cx.try_unlock_session().is_err(), "refusal did not acquire a lock to release");
                self.complete = true;
                cx.request_exit();
            }
            SessionLockState::Finished if self.scenario == Scenario::Finish => {
                assert!(self.locked_seen, "finish follows a confirmed acquisition");
                assert!(
                    cx.try_unlock_session().is_err(),
                    "finished is not authorization to unlock"
                );
                assert!(
                    cx.request_session_lock().is_err(),
                    "no safe replacement for this terminal lock object"
                );
                self.complete = true;
                cx.request_exit();
            }
            SessionLockState::Unlocked if self.hotplug_removed => {
                let id = self.first.take().expect("replacement claim").id();
                assert!(
                    cx.surface(id).is_none(),
                    "sync retires the UI and platform role"
                );
                self.complete = true;
                cx.request_exit();
            }
            _ => {}
        }
    }

    fn shutting_down(&mut self, cx: &dyn PlatformCx) {
        if self.scenario == Scenario::UnlockAndExit {
            assert_eq!(
                cx.session_lock_state(),
                SessionLockState::Unlocked,
                "orderly exit waits for sync"
            );
            let predecessor = self.first.take().expect("predecessor during unlock").id();
            let replacement = self.replacement.take().expect("staged candidate during unlock").id();
            assert!(
                cx.surface(predecessor).is_none(),
                "unlock sync retires the predecessor's claim"
            );
            assert!(
                cx.surface(replacement).is_none(),
                "unlock sync also retires the staged candidate"
            );
            self.complete = true;
        }
    }

    fn deadline_reached(&mut self, _: &dyn PlatformCx) {}
}

#[derive(Clone, Copy, Debug)]
enum Object {
    Registry,
    Compositor,
    Subcompositor,
    Subsurface,
    Shm,
    Pool,
    Buffer {
        offset: u64,
        extent: [u32; 2],
        stride: u64,
    },
    XdgShell,
    Manager,
    Lock,
    Output(u32),
    Surface,
    LockSurface,
    Region,
    Viewporter,
    Viewport(u32),
}

struct OutputLock {
    output: u32,
    surface: u32,
    serial: u32,
    extent: [u32; 2],
    acknowledged: Option<(u32, [u32; 2])>,
    buffer: Option<u32>,
    destination: Option<[u32; 2]>,
    checked_extent: bool,
}

struct Compositor {
    scenario: Scenario,
    stream: UnixStream,
    descriptors: VecDeque<OwnedFd>,
    objects: HashMap<u32, Object>,
    outputs: HashMap<u32, OutputLock>,
    used_surfaces: HashSet<u32>,
    pool: Option<File>,
    registry: u32,
    lock_requests: u32,
    lock_id: u32,
    locked: bool,
    finished: bool,
    refusal_destroyed: bool,
    unlock_sent: bool,
    sync_answered: bool,
    coverage: Vec<(u32, [u32; 2])>,
    role_destroys: Vec<u32>,
    claims: u32,
}

impl Compositor {
    fn new(scenario: Scenario, stream: UnixStream) -> Self {
        Self {
            scenario,
            stream,
            descriptors: VecDeque::new(),
            objects: HashMap::new(),
            used_surfaces: HashSet::new(),
            outputs: HashMap::new(),
            pool: None,
            registry: 0,
            lock_requests: 0,
            lock_id: 0,
            locked: false,
            finished: false,
            refusal_destroyed: false,
            unlock_sent: false,
            sync_answered: false,
            coverage: Vec::new(),
            role_destroys: Vec::new(),
            claims: 0,
        }
    }

    fn read(&mut self, mut bytes: &mut [u8]) -> bool {
        while !bytes.is_empty() {
            let mut storage = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
            let mut ancillary = RecvAncillaryBuffer::new(&mut storage);
            let message = recvmsg(
                &self.stream,
                &mut [IoSliceMut::new(bytes)],
                &mut ancillary,
                RecvFlags::empty(),
            )
            .expect("read Wayland request");
            for control in ancillary.drain() {
                if let RecvAncillaryMessage::ScmRights(fds) = control {
                    self.descriptors.extend(fds);
                }
            }
            if message.bytes == 0 {
                return false;
            }
            bytes = &mut bytes[message.bytes..];
        }
        true
    }

    fn run(&mut self) {
        let mut header = [0; 8];
        while self.read(&mut header) {
            let [id, descriptor] = [word(&header, 0), word(&header, 1)];
            let mut body = vec![0; (descriptor >> 16) as usize - 8];
            assert!(self.read(&mut body), "complete request body");
            self.request(id, descriptor & 0xffff, &body);
        }
    }

    fn request(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let arg = |offset| word(body, offset);
        if id == 1 {
            match opcode {
                0 => {
                    if self.unlock_sent {
                        // The client must wait for this callback, not only queue a sync followed by destruction.
                        self.stream
                            .set_read_timeout(Some(Duration::from_millis(50)))
                            .expect("short sync probe");
                        let mut bytes = [0];
                        let read = recvmsg(
                            &self.stream,
                            &mut [IoSliceMut::new(&mut bytes)],
                            &mut RecvAncillaryBuffer::default(),
                            RecvFlags::PEEK,
                        );
                        assert!(
                            read.is_err_and(|err| err == rustix::io::Errno::AGAIN
                                || err == rustix::io::Errno::TIMEDOUT),
                            "client must wait for sync acknowledgement before further requests"
                        );
                        self.stream
                            .set_read_timeout(Some(TIMEOUT))
                            .expect("restore request timeout");
                        self.sync_answered = true;
                    }
                    self.done(arg(0));
                }
                1 => {
                    self.registry = arg(0);
                    self.objects.insert(self.registry, Object::Registry);
                    self.global(1, "wl_compositor", 6);
                    self.global(2, "wl_shm", 1);
                    self.global(3, "xdg_wm_base", 6);
                    self.global(4, "ext_session_lock_manager_v1", 1);
                    if self.scenario != Scenario::NoSubcompositor {
                        self.global(5, "wl_subcompositor", 1);
                    }
                    if self.scenario == Scenario::Disconnect {
                        self.global(VIEWPORTER, "wp_viewporter", 1);
                    }
                    self.global(FIRST, "wl_output", 4);
                }
                _ => panic!("unexpected display opcode {opcode}"),
            }
            return;
        }
        match (self.objects[&id], opcode) {
            (Object::Registry, 0) => {
                let global = arg(0);
                let bound = arg(3 + (arg(1) as usize).div_ceil(4));
                let object = match global {
                    1 => Object::Compositor,
                    2 => Object::Shm,
                    3 => Object::XdgShell,
                    4 => Object::Manager,
                    5 => Object::Subcompositor,
                    FIRST | SECOND => Object::Output(global),
                    VIEWPORTER => Object::Viewporter,
                    _ => panic!("unexpected global {global}"),
                };
                self.objects.insert(bound, object);
                if let Object::Output(output) = object {
                    self.geometry(bound, 0);
                    self.event(bound, 1, &[3, INITIAL[0], INITIAL[1], 60_000]);
                    self.event_bytes(
                        bound,
                        4,
                        &string(if output == FIRST { "TEST-1" } else { "TEST-2" }),
                    );
                    self.event(bound, 2, &[]);
                    if output == SECOND && self.scenario == Scenario::Finish {
                        self.finished = true;
                        self.event(self.lock_id, 1, &[]);
                    }
                }
            }
            (Object::Compositor, 0) => {
                self.objects.insert(arg(0), Object::Surface);
            }
            (Object::Compositor, 1) => {
                self.objects.insert(arg(0), Object::Region);
            }
            (Object::Manager, 1) => {
                self.lock_requests += 1;
                self.lock_id = arg(0);
                self.objects.insert(arg(0), Object::Lock);
                if self.scenario == Scenario::Refuse {
                    self.event(arg(0), 1, &[]);
                }
            }
            (Object::Lock, 1) => {
                let Object::Output(output) = self.objects[&arg(2)] else {
                    panic!("lock surface must name a wl_output");
                };
                assert!(
                    !self.outputs.values().any(|role| role.output == output),
                    "duplicate lock role for output {output}"
                );
                assert!(
                    !self.used_surfaces.contains(&arg(1)),
                    "lock role requires a fresh wl_surface"
                );
                let role = OutputLock {
                    output,
                    surface: arg(1),
                    serial: 10,
                    extent: INITIAL,
                    acknowledged: None,
                    buffer: None,
                    destination: None,
                    checked_extent: false,
                };
                self.objects.insert(arg(0), Object::LockSurface);
                self.outputs.insert(arg(0), role);
                if self.scenario != Scenario::Refuse
                    && (output == FIRST || self.scenario.explicitly_unlocks())
                {
                    self.event(arg(0), 0, &[10, INITIAL[0], INITIAL[1]]);
                }
            }
            (Object::Lock, 0) => {
                assert_eq!(
                    self.scenario,
                    Scenario::Refuse,
                    "destroy is illegal after locked"
                );
                self.refusal_destroyed = true;
            }
            (Object::Lock, 2) => {
                assert!(self.scenario.explicitly_unlocks(), "no implicit unlock");
                assert!(
                    self.locked && !self.finished,
                    "unlock is legal only after acquisition"
                );
                self.unlock_sent = true;
                self.locked = false;
            }
            (Object::LockSurface, 0) => {
                let role = self.outputs.remove(&id).expect("live lock surface");
                if role.output == FIRST && self.scenario.explicitly_unlocks() {
                    assert!(
                        self.sync_answered,
                        "platform role destroyed before unlock sync"
                    );
                }
                self.role_destroys.push(role.output);
            }
            (Object::LockSurface, 1) => {
                let role = self.outputs.get_mut(&id).expect("live lock surface");
                assert_eq!(arg(0), role.serial, "ack identifies the configure being handled");
                assert_ne!(
                    role.acknowledged.map(|(serial, _)| serial),
                    Some(arg(0)),
                    "one ack per configure"
                );
                role.acknowledged = Some((arg(0), role.extent));
                role.checked_extent = false;
            }
            (Object::Shm, 0) => {
                let fd = self
                    .descriptors
                    .pop_front()
                    .expect("wl_shm.create_pool passes a backing fd");
                self.pool = Some(fd.into());
                self.objects.insert(arg(0), Object::Pool);
            }
            (Object::Pool, 0) => {
                assert_eq!(arg(5), 1, "fallback is opaque XRGB8888");
                self.objects.insert(
                    arg(0),
                    Object::Buffer {
                        offset: arg(1).into(),
                        extent: [arg(2), arg(3)],
                        stride: arg(4).into(),
                    },
                );
            }
            (Object::Subcompositor, 1) => {
                assert!(
                    self.outputs.values().any(|role| role.surface == arg(2)),
                    "UI is a child of the dark fallback"
                );
                self.objects.insert(arg(0), Object::Subsurface);
                self.claims += 1;
            }
            (Object::Surface, 0) => {
                assert!(
                    !self.outputs.values().any(|role| role.surface == id),
                    "a lock wl_surface must outlive its role"
                );
            }
            (Object::Surface, 1) => {
                self.used_surfaces.insert(id);
                if let Some(role) = self.outputs.values_mut().find(|role| role.surface == id) {
                    assert!(
                        role.acknowledged.is_some(),
                        "no lock buffer before ack_configure"
                    );
                    assert_ne!(arg(0), 0, "no lock role may ever attach a null buffer");
                    role.buffer = Some(arg(0));
                }
            }
            (Object::Surface, 3) => {
                self.done(arg(0));
            }
            (Object::Surface, 6) => self.commit(id),
            (Object::Viewporter, 1) => {
                self.objects.insert(arg(0), Object::Viewport(arg(1)));
            }
            (Object::Viewport(surface), 2) => {
                let role = self
                    .outputs
                    .values_mut()
                    .find(|role| role.surface == surface)
                    .expect("viewport belongs to the lock surface");
                role.destination = Some([arg(0), arg(1)]);
            }
            (Object::Surface, 2 | 4 | 5 | 7 | 8 | 9)
            | (Object::Region, 0 | 1)
            | (Object::Pool, 1 | 2)
            | (Object::Buffer { .. }, 0)
            | (Object::Output(_), 0)
            | (Object::Manager | Object::Subcompositor | Object::XdgShell, 0)
            | (Object::Viewporter | Object::Viewport(_), 0)
            | (Object::Subsurface, 0 | 5) => {}
            (object, _) => panic!("unexpected {object:?} opcode {opcode}"),
        }
    }

    fn commit(&mut self, surface: u32) {
        self.used_surfaces.insert(surface);
        let Some((&id, role)) = self
            .outputs
            .iter_mut()
            .find(|(_, role)| role.surface == surface)
        else {
            return;
        };
        let (_, expected_extent) = role
            .acknowledged
            .expect("cannot commit before first ack_configure");
        let buffer = role.buffer.expect("cannot commit an empty lock surface");
        let Object::Buffer {
            offset,
            extent,
            stride,
        } = self.objects[&buffer]
        else {
            panic!("expected a SHM buffer");
        };
        if self.scenario == Scenario::Disconnect {
            assert_eq!(extent, [1, 1], "viewport scales one immutable black pixel");
            assert_eq!(
                role.destination,
                Some(expected_extent),
                "the viewport stretches the black pixel to the exact configured extent"
            );
        } else {
            assert_eq!(
                extent, expected_extent,
                "fallback matches exact configured extent"
            );
        }
        if role.checked_extent {
            return;
        }
        let mut pixel = [0xff; 4];
        let file = self.pool.as_ref().expect("fallback pool exists");
        for row in 0..extent[1] {
            for col in 0..extent[0] {
                file.read_exact_at(
                    &mut pixel,
                    offset + u64::from(row) * stride + u64::from(col) * 4,
                )
                .expect("read a fallback pixel");
                assert_eq!(
                    u32::from_ne_bytes(pixel) & 0x00ff_ffff,
                    0,
                    "fallback must cover the full output in black"
                );
            }
        }
        role.checked_extent = true;
        self.coverage.push((role.output, expected_extent));
        match (self.scenario, role.output, expected_extent) {
            (_, FIRST, INITIAL) => {
                self.locked = true;
                self.event(self.lock_id, 0, &[]);
                if self.scenario == Scenario::Finish {
                    // Request a registry update so there is one client turn to observe Locked.
                    self.global(SECOND, "wl_output", 4);
                } else {
                    let role = self.outputs.get_mut(&id).expect("first output lock");
                    role.serial += 1;
                    role.extent = RESIZED;
                    self.event(id, 0, &[11, RESIZED[0], RESIZED[1]]);
                }
            }
            (Scenario::Unlock | Scenario::UnlockAndExit, FIRST, RESIZED) => {
                self.global(SECOND, "wl_output", 4)
            }
            (Scenario::Unlock | Scenario::UnlockAndExit, SECOND, INITIAL) => {
                let output = self
                    .objects
                    .iter()
                    .find_map(|(&id, object)| {
                        matches!(object, Object::Output(SECOND)).then_some(id)
                    })
                    .expect("second output is bound");
                self.geometry(output, 25);
                self.event(output, 2, &[]);
                self.event(self.registry, 1, &[SECOND]);
            }
            (Scenario::Disconnect, FIRST, RESIZED) => self.global(SECOND, "wl_output", 4),
            _ => {}
        }
    }

    fn geometry(&mut self, output: u32, x: u32) {
        let mut args = words(&[x, 0, 300, 200, 0]);
        args.extend(string("Fixture"));
        args.extend(string("Monitor"));
        args.extend(words(&[0]));
        self.event_bytes(output, 0, &args);
    }

    fn global(&mut self, name: u32, interface: &str, version: u32) {
        let mut args = words(&[name]);
        args.extend(string(interface));
        args.extend(words(&[version]));
        self.event_bytes(self.registry, 0, &args);
    }

    fn event(&mut self, id: u32, opcode: u32, args: &[u32]) {
        self.event_bytes(id, opcode, &words(args));
    }

    fn done(&mut self, callback: u32) {
        // Send wl_callback.done and wl_display.delete_id in the same write: the client
        // may exit as soon as it has processed the callback.
        let events = words(&[callback, 12 << 16, 0, 1, 12 << 16 | 1, callback]);
        self.stream
            .write_all(&events)
            .expect("complete the callback");
    }

    fn event_bytes(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let length = u32::try_from(8 + body.len()).expect("small Wayland message");
        let mut event = words(&[id, length << 16 | opcode]);
        event.extend(body);
        self.stream.write_all(&event).expect("send Wayland event");
    }
}

fn word(bytes: &[u8], index: usize) -> u32 {
    u32::from_ne_bytes(
        bytes[index * 4..][..4]
            .try_into()
            .expect("Wayland argument word"),
    )
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|word| word.to_ne_bytes()).collect()
}

fn string(value: &str) -> Vec<u8> {
    let length = value.len() + 1;
    let mut bytes = words(&[u32::try_from(length).expect("short protocol string")]);
    bytes.extend(value.as_bytes());
    bytes.resize(4 + length.next_multiple_of(4), 0);
    bytes
}
