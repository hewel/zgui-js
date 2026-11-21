//! Outbound Copy-only drag source lifecycle on a private Wayland socket.

#![cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use zgui_geom::{CssPx, Size};
use zgui_platform::{
    AppHandler, DragEvent, OutboundDragPayload, PlatformCx, SurfaceAttributes, SurfaceEvent,
    SurfaceId, WakeReason,
};
use zgui_platform_wayland::WaylandApp;
use zgui_vocab::PointerAction;

const CLIENT: &str = "ZGUI_DND_SOURCE_PROTOCOL_CLIENT";
const TEST: &str = "outbound_copy_only_drag_source_lifecycle_on_the_wire";
const TIMEOUT: Duration = Duration::from_secs(5);
const PRESS_1_SERIAL: u32 = 303;
const RELEASE_1_SERIAL: u32 = 304;
const PRESS_2_SERIAL: u32 = 305;
const CONFIGURE_SERIAL: u32 = 40;

#[test]
fn outbound_copy_only_drag_source_lifecycle_on_the_wire() {
    if std::env::var_os(CLIENT).is_some() {
        let mut app = WaylandApp::new(DragClient::default())
            .expect("connect to the private compositor");
        app.run().expect("Wayland client loop");
        let h = app.handler();
        assert!(h.refused_before_press, "must refuse outbound drag before press");
        assert!(h.finished_accepted, "must observe accepted OutboundFinished");
        assert!(h.refused_after_release, "must refuse outbound drag after release");
        assert!(h.cancelled_second, "must cancel second outbound drag cleanly");
        return;
    }

    let socket = SocketFile(
        std::env::temp_dir().join(format!("zgui-dnd-src-{}.sock", std::process::id())),
    );
    let listener = UnixListener::bind(&socket.0).expect("bind private Wayland socket");
    listener.set_nonblocking(true).expect("nonblocking listener");
    let mut child = Command::new(std::env::current_exe().expect("locate test binary"))
        .args(["--exact", TEST])
        .env(CLIENT, "1")
        .env("WAYLAND_DISPLAY", &socket.0)
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .expect("launch Wayland client");
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
            Err(error) => panic!("accept client: {error}"),
        }
    };
    stream.set_read_timeout(Some(TIMEOUT)).expect("read timeout");
    stream.set_write_timeout(Some(TIMEOUT)).expect("write timeout");

    let mut server = Compositor::new(stream);
    server.run();
    let status = child.wait().expect("wait for client");
    assert!(status.success(), "Wayland client exited with {status}");

    assert_eq!(server.started_drags.len(), 2, "expected exactly 2 started outbound drags");
    for (idx, drag) in server.started_drags.iter().enumerate() {
        assert_eq!(
            drag.actions, 1,
            "drag #{idx} must advertise DndAction::Copy (1) only, never Move (2)"
        );
        assert_eq!(drag.offered_mimes, vec!["text/plain".to_owned()]);
        assert_eq!(drag.icon, 0);
        assert!(drag.destroyed, "drag #{idx} wl_data_source must be destroyed");
    }
    assert_eq!(server.started_drags[0].serial, PRESS_1_SERIAL);
    assert_eq!(server.started_drags[1].serial, PRESS_2_SERIAL);
}

struct SocketFile(std::path::PathBuf);
impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Default)]
struct DragClient {
    surface: Option<SurfaceId>,
    press_count: u32,
    refused_before_press: bool,
    finished_accepted: bool,
    refused_after_release: bool,
    cancelled_second: bool,
}

impl AppHandler for DragClient {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        let mut attrs = SurfaceAttributes::new("dnd-source-test");
        attrs.size = Some(Size::new(CssPx(200.0), CssPx(120.0)));
        let surface = cx.create_surface(&attrs).expect("create window");
        let id = surface.id();
        // Attempting an outbound drag before any live press must fail!
        let payload = OutboundDragPayload::new(vec![("text/plain".into(), b"early".to_vec())]);
        assert!(surface.begin_outbound_drag(payload).is_err());
        self.refused_before_press = true;
        self.surface = Some(id);
    }

    fn surfaces_lost(&mut self, _cx: &dyn PlatformCx) {}

    fn surface_event(&mut self, cx: &dyn PlatformCx, id: SurfaceId, event: SurfaceEvent) {
        let Some(surface) = cx.surface(id) else { return };
        match event {
            SurfaceEvent::Pointer { action: PointerAction::Pressed, .. } => {
                self.press_count += 1;
                if self.press_count == 1 {
                    let payload = OutboundDragPayload::new(vec![(
                        "text/plain".into(),
                        b"copied-payload".to_vec(),
                    )]);
                    surface
                        .begin_outbound_drag(payload)
                        .expect("start Copy-only drag during live press #1");
                } else if self.press_count == 2 {
                    let payload = OutboundDragPayload::new(vec![(
                        "text/plain".into(),
                        b"second-payload".to_vec(),
                    )]);
                    surface
                        .begin_outbound_drag(payload)
                        .expect("start Copy-only drag during live press #2");
                    // Immediately cancel (simulating retirement/explicit cancel).
                    surface.cancel_outbound_drag();
                    self.cancelled_second = true;
                    cx.request_exit();
                }
            }
            SurfaceEvent::Drag(DragEvent::OutboundFinished { accepted }) => {
                assert!(accepted, "first drag must report accepted=true on dnd_finished");
                self.finished_accepted = true;
            }
            SurfaceEvent::Pointer { action: PointerAction::Released, .. } => {
                let payload = OutboundDragPayload::new(vec![(
                    "text/plain".into(),
                    b"stale-after-release".to_vec(),
                )]);
                assert!(
                    surface.begin_outbound_drag(payload).is_err(),
                    "must reject outbound drag after pointer release"
                );
                self.refused_after_release = true;
                // Signal the mock compositor that Released was verified so it can send press #2.
                surface.set_title("ready-for-press-2");
            }
            _ => {}
        }
    }

    fn wake(&mut self, _cx: &dyn PlatformCx, _reason: WakeReason) {}
}

#[derive(Clone, Copy, Debug)]
enum Object {
    Registry,
    Compositor,
    Shm,
    XdgShell,
    Seat,
    Pointer,
    DataDeviceManager,
    DataDevice,
    DataSource(usize),
    Surface,
    XdgSurface(u32),
    Toplevel,
    Region,
}

#[derive(Default)]
struct RecordedDrag {
    offered_mimes: Vec<String>,
    actions: u32,
    origin: u32,
    icon: u32,
    serial: u32,
    started: bool,
    destroyed: bool,
}

struct Compositor {
    stream: UnixStream,
    objects: HashMap<u32, Object>,
    seat: Option<u32>,
    pointer: Option<u32>,
    data_device: Option<u32>,
    surface_wl: Option<u32>,
    configured: bool,
    sent_press_1: bool,
    sent_release_1: bool,
    sent_press_2: bool,
    started_drags: Vec<RecordedDrag>,
}

impl Compositor {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            objects: HashMap::new(),
            seat: None,
            pointer: None,
            data_device: None,
            surface_wl: None,
            configured: false,
            sent_press_1: false,
            sent_release_1: false,
            sent_press_2: false,
            started_drags: Vec::new(),
        }
    }

    fn run(&mut self) {
        let mut header = [0; 8];
        loop {
            match self.stream.read_exact(&mut header) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return,
                Err(e) => panic!("read Wayland request: {e}"),
            }
            let [id, desc] = [word(&header, 0), word(&header, 1)];
            let mut body = vec![0; (desc >> 16) as usize - 8];
            self.stream.read_exact(&mut body).expect("request body");
            self.request(id, desc & 0xffff, &body);
        }
    }

    fn request(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let arg = |offset| word(body, offset);
        if id == 1 {
            match opcode {
                0 => self.done(arg(0)),
                1 => {
                    let registry = arg(0);
                    self.objects.insert(registry, Object::Registry);
                    for (name, interface, version) in [
                        (1, "wl_compositor", 6),
                        (2, "wl_shm", 1),
                        (3, "xdg_wm_base", 6),
                        (4, "wl_seat", 7),
                        (5, "wl_data_device_manager", 3),
                    ] {
                        let mut args = words(&[name]);
                        args.extend(string(interface));
                        args.extend(words(&[version]));
                        self.event_bytes(registry, 0, &args);
                    }
                }
                _ => panic!("unexpected wl_display opcode {opcode}"),
            }
            return;
        }
        let Some(&obj) = self.objects.get(&id) else { return };
        match (obj, opcode) {
            (Object::Registry, 0) => {
                let bound = arg(3 + (arg(1) as usize).div_ceil(4));
                let kind = match arg(0) {
                    1 => Object::Compositor,
                    2 => Object::Shm,
                    3 => Object::XdgShell,
                    4 => {
                        self.seat = Some(bound);
                        self.event_bytes(bound, 1, &string("test-seat"));
                        // Announce pointer capability (1) on wl_seat.
                        self.event(bound, 0, &[1]);
                        Object::Seat
                    }
                    5 => Object::DataDeviceManager,
                    n => panic!("unexpected global {n}"),
                };
                self.objects.insert(bound, kind);
            }
            (Object::Compositor, 0) => {
                self.objects.insert(arg(0), Object::Surface);
            }
            (Object::Compositor, 1) => {
                self.objects.insert(arg(0), Object::Region);
            }
            (Object::Seat, 0) => {
                let ptr = arg(0);
                self.pointer = Some(ptr);
                self.objects.insert(ptr, Object::Pointer);
                self.maybe_send_press_1();
            }
            (Object::DataDeviceManager, 0) => {
                // create_data_source(id)
                let src = arg(0);
                let idx = self.started_drags.len();
                self.started_drags.push(RecordedDrag::default());
                self.objects.insert(src, Object::DataSource(idx));
            }
            (Object::DataDeviceManager, 1) => {
                // get_data_device(id, seat)
                let dev = arg(0);
                self.data_device = Some(dev);
                self.objects.insert(dev, Object::DataDevice);
                self.maybe_send_press_1();
            }
            (Object::DataSource(idx), 0) => {
                // offer(mime_type)
                let len = arg(0) as usize;
                let mime = std::str::from_utf8(&body[4..4 + len - 1])
                    .expect("utf8 mime")
                    .to_owned();
                self.started_drags[idx].offered_mimes.push(mime);
            }
            (Object::DataSource(idx), 1) => {
                // destroy
                self.started_drags[idx].destroyed = true;
                if idx == 0 && !self.sent_release_1 {
                    self.send_release_1();
                }
            }
            (Object::DataSource(idx), 2) => {
                // set_actions(dnd_actions)
                self.started_drags[idx].actions = arg(0);
            }
            (Object::DataDevice, 0) => {
                // start_drag(source, origin, icon, serial)
                let [src, origin, icon, serial] = [arg(0), arg(1), arg(2), arg(3)];
                if let Some(&Object::DataSource(idx)) = self.objects.get(&src) {
                    let drag = &mut self.started_drags[idx];
                    drag.origin = origin;
                    drag.icon = icon;
                    drag.serial = serial;
                    drag.started = true;
                    if idx == 0 {
                        // Simulate destination accepting text/plain + Copy + drop + finish.
                        self.event_bytes(src, 0, &string("text/plain")); // target
                        self.event(src, 5, &[1]); // action(Copy)
                        self.event(src, 3, &[]); // dnd_drop_performed
                        self.event(src, 4, &[]); // dnd_finished
                    }
                }
            }
            (Object::XdgShell, 2) => {
                // get_xdg_surface(id, surface)
                self.objects.insert(arg(0), Object::XdgSurface(arg(1)));
            }
            (Object::XdgSurface(wl), 1) => {
                // get_toplevel(id)
                let toplevel = arg(0);
                self.surface_wl = Some(wl);
                self.objects.insert(toplevel, Object::Toplevel);
                // configure toplevel + xdg_surface
                self.event(toplevel, 0, &[200, 120, 0]);
                self.event(id, 0, &[CONFIGURE_SERIAL]);
                self.configured = true;
                self.maybe_send_press_1();
            }
            (Object::Toplevel, 2) => {
                // xdg_toplevel.set_title -> client acknowledged release #1; send press #2.
                if self.sent_release_1 && !self.sent_press_2 {
                    self.send_press_2();
                }
            }
            _ => {}
        }
    }

    fn maybe_send_press_1(&mut self) {
        if self.sent_press_1 || !self.configured || self.data_device.is_none() {
            return;
        }
        let (Some(ptr), Some(surf)) = (self.pointer, self.surface_wl) else {
            return;
        };
        self.sent_press_1 = true;
        // wl_pointer.enter(serial, surface, x_fixed, y_fixed)
        self.event(ptr, 0, &[10, surf, 20 << 8, 20 << 8]);
        // wl_pointer.button(serial, time, button=BTN_LEFT(272), state=PRESSED(1))
        self.event(ptr, 3, &[PRESS_1_SERIAL, 100, 272, 1]);
        // wl_pointer.frame
        self.event(ptr, 5, &[]);
    }

    fn send_release_1(&mut self) {
        let Some(ptr) = self.pointer else { return };
        self.sent_release_1 = true;
        // Release first press -> client checks refusal after release.
        self.event(ptr, 3, &[RELEASE_1_SERIAL, 110, 272, 0]);
        self.event(ptr, 5, &[]);
    }

    fn send_press_2(&mut self) {
        let Some(ptr) = self.pointer else { return };
        // Second live press -> client starts second drag and cancels it.
        self.sent_press_2 = true;
        self.event(ptr, 3, &[PRESS_2_SERIAL, 120, 272, 1]);
        self.event(ptr, 5, &[]);
    }

    fn done(&mut self, callback: u32) {
        self.event(callback, 0, &[0]);
    }

    fn event(&mut self, id: u32, opcode: u32, args: &[u32]) {
        self.event_bytes(id, opcode, &words(args));
    }

    fn event_bytes(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let size = (8 + body.len()) as u32;
        self.stream
            .write_all(&words(&[id, (size << 16) | opcode]))
            .expect("write header");
        self.stream.write_all(body).expect("write body");
    }
}

fn word(bytes: &[u8], index: usize) -> u32 {
    let start = index * 4;
    u32::from_ne_bytes(bytes[start..start + 4].try_into().expect("u32"))
}

fn words(items: &[u32]) -> Vec<u8> {
    items.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

fn string(value: &str) -> Vec<u8> {
    let len = value.len() + 1;
    let mut out = words(&[len as u32]);
    out.extend_from_slice(value.as_bytes());
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}
