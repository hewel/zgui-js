//! Layer and nested popups on a private socket, with the real Wayland client backend.

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

use zgui_geom::{Css, CssPx, DevicePx, Edges, Point, Rect, Size};
use zgui_platform::{
    Anchor, AppHandler, CloseReason, Constrain, InputSerial, LayerPlacement, PlatformCx,
    PlatformError, PopupPlacement, SurfaceAttributes, SurfaceEvent, SurfaceId, SurfaceRole,
    WakeReason,
};
use zgui_platform_wayland::WaylandApp;
use zgui_vocab::PointerAction;

const CLIENT: &str = "ZGUI_POPUP_PROTOCOL_CLIENT";
const TEST: &str = "layer_and_nested_popup_lifecycle_on_the_wire";
const TIMEOUT: Duration = Duration::from_secs(5);
const LAYER_PRESS: u32 = 101;
const POPUP_PRESS: u32 = 202;
const LAYER_CONFIGURE: u32 = 10;
const FIRST_CONFIGURE: u32 = 20;
const MOVED_CONFIGURE: u32 = 21;
const NESTED_CONFIGURE: u32 = 30;

#[test]
fn layer_and_nested_popup_lifecycle_on_the_wire() {
    if std::env::var_os(CLIENT).is_some() {
        let mut app = WaylandApp::new(Menus::default()).expect("connect to the private compositor");
        app.run().expect("Wayland client loop");
        assert!(app.handler().dismissed, "popup_done must reach the app");
        return;
    }

    let socket = SocketFile(std::env::temp_dir().join(format!("zgui-popup-{}.sock", std::process::id())));
    let listener = UnixListener::bind(&socket.0).expect("bind the private Wayland socket");
    listener.set_nonblocking(true).expect("bound the accept wait");
    let mut child = Command::new(std::env::current_exe().expect("locate this test binary"))
        .args(["--exact", TEST])
        .env(CLIENT, "1")
        .env("WAYLAND_DISPLAY", &socket.0)
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .expect("launch the actual Wayland backend");
    let start = Instant::now();
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(child.try_wait().expect("poll test client").is_none(), "client exited before connecting");
                assert!(start.elapsed() < TIMEOUT, "client did not connect");
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("accepting test client: {error}"),
        }
    };
    stream.set_read_timeout(Some(TIMEOUT)).expect("bound each read");
    stream.set_write_timeout(Some(TIMEOUT)).expect("bound each write");
    let mut server = Compositor::new(stream);
    server.run();
    let status = child.wait().expect("wait for test client");
    assert!(status.success(), "the Wayland client exited with {status}");
    let [first, nested] = server.popups.as_slice() else {
        panic!(
            "expected the original popup and its child, got {} popup roles",
            server.popups.len()
        );
    };
    assert!(server.repositioned, "the configured popup must reposition on its existing role");
    assert_eq!(
        first.window_geometries,
        [[0, 0, 59, 29]],
        "first popup must emit xdg_surface.set_window_geometry paired with its frame presentation",
    );
    let layer = server.layer.expect("layer exists");
    assert_eq!(
        server.destroyed,
        [nested.role, nested.xdg, nested.wl, first.role, first.xdg, first.wl, layer.role, layer.wl],
        "each popup must be destroyed before its parent, and each role before its wl_surface",
    );
}

struct SocketFile(std::path::PathBuf);

impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Default)]
struct Menus {
    layer: Option<SurfaceId>,
    first: Option<SurfaceId>,
    nested: Option<SurfaceId>,
    moved: bool,
    dismissed: bool,
}

fn size(width: f32, height: f32) -> Size<CssPx, Css> {
    Size::new(CssPx(width), CssPx(height))
}

fn placement(parent: SurfaceId, grab: Option<InputSerial>) -> PopupPlacement {
    let rect = Rect::new(Point::new(CssPx(4.0), CssPx(6.0)), size(14.0, 9.0));
    let mut placement = PopupPlacement::below(parent, rect);
    placement.anchor = Anchor::TopRight;
    placement.gravity = Anchor::BottomLeft;
    placement.offset = Point::new(CssPx(-2.5), CssPx(6.5));
    placement.constrain = Constrain { slide: true, flip: false, resize: true };
    placement.grab = grab;
    placement.reactive = true;
    placement
}

fn open_popup(cx: &dyn PlatformCx, placement: PopupPlacement, extent: Size<CssPx, Css>) -> SurfaceId {
    let mut attrs = SurfaceAttributes::popup(placement);
    attrs.size = Some(extent);
    cx.create_surface(&attrs).expect("open popup against a live parent").id()
}

impl AppHandler for Menus {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        let mut attrs = SurfaceAttributes::default();
        attrs.role = SurfaceRole::Layer(LayerPlacement::default());
        attrs.size = Some(size(200.0, 40.0));
        self.layer = Some(cx.create_surface(&attrs).expect("open layer").id());
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, id: SurfaceId, event: SurfaceEvent) {
        match event {
            SurfaceEvent::Pointer { action: PointerAction::Pressed, .. } if Some(id) == self.layer => {
                assert!(self.first.is_none(), "only one press on the layer");
                let serial = cx.surface(id).expect("configured layer parent").input_serial();
                let mut invalid = placement(id, serial);
                invalid.offset.x = CssPx(2_147_483_648.0);
                let attrs = SurfaceAttributes::popup(invalid);
                assert!(
                    matches!(cx.create_surface(&attrs), Err(PlatformError::SurfaceCreation(_))),
                    "creating a popup with an out-of-range offset must fail"
                );
                self.first = Some(open_popup(cx, placement(id, serial), size(60.0, 30.0)));
                cx.surface(id).expect("configured layer parent").pre_present_notify();
            }
            SurfaceEvent::Resized(_) if Some(id) == self.first && !self.moved => {
                let popup = cx.surface(id).expect("first popup remains live");
                let parent = self.layer.expect("layer parent");
                let serial = cx.surface(parent).expect("layer parent").input_serial();
                let mut moved = placement(parent, serial);
                moved.offset = Point::new(CssPx(10.5), CssPx(f32::NAN));
                assert!(
                    matches!(
                        popup.reconfigure_popup(&moved, size(72.0, 38.0)),
                        Err(PlatformError::SurfaceCreation(_))
                    ),
                    "repositioning a popup with a nonfinite offset must fail"
                );
                moved.offset.y = CssPx(-1.5);
                popup.reconfigure_popup(&moved, size(72.0, 38.0)).expect("reposition configured popup");
                // Submit the first presentation through the renderer hook after staging the move.
                popup.pre_present_notify();
                self.moved = true;
            }
            SurfaceEvent::Pointer { action: PointerAction::Pressed, .. } if Some(id) == self.first => {
                assert!(self.nested.is_none(), "only one press on the first popup");
                let serial = cx.surface(id).expect("first popup").input_serial().expect("popup press serial");
                let rect = Rect::new(Point::new(CssPx(1.0), CssPx(2.0)), size(6.0, 8.0));
                self.nested = Some(open_popup(cx, PopupPlacement::below(id, rect).with_grab(serial), size(28.0, 16.0)));
            }
            SurfaceEvent::Closed(CloseReason::Dismissed) | SurfaceEvent::Destroyed if Some(id) == self.nested => {
                assert!(!self.dismissed, "one dismissal for the nested popup");
                self.dismissed = true;
                cx.destroy_surface(self.layer.expect("destroy the root of the popup chain"));
                for id in [self.layer, self.first, self.nested].into_iter().flatten() {
                    assert!(cx.surface(id).is_none(), "destroying the layer removes descendant {id:?}");
                }
                cx.request_exit();
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
    LayerShell,
    Seat,
    Pointer,
    Surface,
    Layer,
    Positioner,
    XdgSurface(u32),
    Popup(usize),
    Region,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Positioner {
    size: [i32; 2],
    rect: [i32; 4],
    anchor: u32,
    gravity: u32,
    constraints: u32,
    offset: [i32; 2],
    reactive: bool,
}

#[derive(Clone, Copy)]
struct Layer {
    role: u32,
    wl: u32,
}

struct Popup {
    role: u32,
    xdg: u32,
    wl: u32,
    associated: bool,
    grabbed: bool,
    committed: bool,
    presenting: bool,
    window_geometries: Vec<[i32; 4]>,
}

struct Compositor {
    stream: UnixStream,
    objects: HashMap<u32, Object>,
    layer: Option<Layer>,
    layer_committed: bool,
    layer_ack: bool,
    seat: Option<u32>,
    pointer: Option<u32>,
    layer_pressed: bool,
    positioners: HashMap<u32, Positioner>,
    popups: Vec<Popup>,
    repositioned: bool,
    destroyed: Vec<u32>,
}

impl Compositor {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            objects: HashMap::new(),
            layer: None,
            layer_committed: false,
            layer_ack: false,
            seat: None,
            pointer: None,
            layer_pressed: false,
            positioners: HashMap::new(),
            popups: Vec::new(),
            repositioned: false,
            destroyed: Vec::new(),
        }
    }

    fn run(&mut self) {
        let mut header = [0; 8];
        loop {
            match self.stream.read_exact(&mut header) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return,
                Err(error) => panic!("reading a Wayland request: {error}"),
            }
            let [id, descriptor] = [word(&header, 0), word(&header, 1)];
            let mut body = vec![0; (descriptor >> 16) as usize - 8];
            self.stream.read_exact(&mut body).expect("Wayland request body");
            self.request(id, descriptor & 0xffff, &body);
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
                        (4, "zwlr_layer_shell_v1", 4),
                        (5, "wl_seat", 7),
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
        match (self.objects[&id], opcode) {
            (Object::Registry, 0) => {
                let bound = arg(3 + (arg(1) as usize).div_ceil(4));
                let object = match arg(0) {
                    1 => Object::Compositor,
                    2 => Object::Shm,
                    3 => Object::XdgShell,
                    4 => Object::LayerShell,
                    5 => Object::Seat,
                    name => panic!("unexpected global {name}"),
                };
                self.objects.insert(bound, object);
                if matches!(object, Object::Seat) {
                    self.seat = Some(bound);
                    self.event_bytes(bound, 1, &string("test-seat"));
                    self.event(bound, 0, &[1]); // pointer capability
                }
            }
            (Object::Seat, 0) => {
                self.objects.insert(arg(0), Object::Pointer);
                self.pointer = Some(arg(0));
                self.press_layer_when_ready();
            }
            (Object::Compositor, 0) => { self.objects.insert(arg(0), Object::Surface); }
            (Object::Compositor, 1) => { self.objects.insert(arg(0), Object::Region); }
            (Object::LayerShell, 0) => {
                assert!(self.layer.is_none(), "only one layer role");
                self.layer = Some(Layer { role: arg(0), wl: arg(1) });
                self.objects.insert(arg(0), Object::Layer);
            }
            (Object::Layer, 5) => {
                let first = &mut self.popups[0];
                assert_eq!(arg(0), first.role, "associate only the outer popup with the layer");
                assert!(!first.committed && !first.associated, "associate once, before the first commit");
                first.associated = true;
            }
            (Object::Layer, 6) => {
                assert_eq!(arg(0), LAYER_CONFIGURE, "layer acknowledges configure");
                self.layer_ack = true;
                self.press_layer_when_ready();
            }
            (Object::XdgShell, 1) => {
                self.objects.insert(arg(0), Object::Positioner);
                self.positioners.insert(arg(0), Positioner::default());
            }
            (Object::XdgShell, 2) => { self.objects.insert(arg(0), Object::XdgSurface(arg(1))); }
            (Object::Positioner, 0) => { self.positioners.remove(&id); }
            (Object::Positioner, 1..=9) => self.position(id, opcode, &arg),
            (Object::XdgSurface(wl), 2) => self.popup(arg(0), id, wl, arg(1), arg(2)),
            (Object::Popup(index), 1) => {
                let popup = &mut self.popups[index];
                assert!(!popup.grabbed && !popup.committed, "grab once, before the first commit");
                assert_eq!(Some(arg(0)), self.seat, "grab names the pressing seat");
                assert_eq!(arg(1), [LAYER_PRESS, POPUP_PRESS][index], "grab quotes the press, not the enter serial");
                popup.grabbed = true;
            }
            (Object::Popup(0), 2) => self.reposition(id, arg(0), arg(1)),
            (Object::XdgSurface(_), 3) => {
                let geom = [0, 1, 2, 3].map(|i| arg(i) as i32);
                if let Some(popup) = self.popups.iter_mut().find(|popup| popup.xdg == id) {
                    popup.window_geometries.push(geom);
                }
            }
            (Object::XdgSurface(wl), 4) => self.ack_popup(wl, arg(0)),
            (Object::Surface, 1) => {
                if let Some(popup) = self.popups.iter().find(|popup| popup.wl == id) {
                    assert!(popup.committed, "first popup commit must be bufferless");
                }
            }
            (Object::Surface, 3) => {
                if let Some(popup) = self.popups.iter_mut().find(|popup| popup.wl == id) {
                    popup.presenting = true;
                }
            }
            (Object::Surface, 6) => self.commit(id),
            (Object::Layer, 7) | (Object::Surface | Object::Popup(_) | Object::XdgSurface(_), 0) => {
                self.destroyed.push(id);
            }
            (Object::Layer, 0..=4 | 8)
            | (Object::LayerShell, 1)
            | (Object::XdgShell, 0 | 3)
            | (Object::Seat, 3)
            | (Object::Pointer, 0 | 1)
            | (Object::Region, 0..=2)
            | (Object::Surface, 2 | 4 | 5 | 7..=10) => {}
            (object, _) => panic!("unexpected {object:?} opcode {opcode}"),
        }
    }

    fn position(&mut self, id: u32, opcode: u32, arg: &impl Fn(usize) -> u32) {
        let place = self.positioners.get_mut(&id).expect("live positioner");
        match opcode {
            1 => place.size = [0, 1].map(|i| arg(i) as i32),
            2 => place.rect = [0, 1, 2, 3].map(|i| arg(i) as i32),
            3 => place.anchor = arg(0),
            4 => place.gravity = arg(0),
            5 => place.constraints = arg(0),
            6 => place.offset = [0, 1].map(|i| arg(i) as i32),
            7 => place.reactive = true,
            8 | 9 => {} // Optional parent size / configure serial.
            _ => panic!("unexpected xdg_positioner {id} opcode {opcode}"),
        }
    }

    fn popup(&mut self, role: u32, xdg: u32, wl: u32, parent: u32, positioner: u32) {
        let index = self.popups.len();
        let expected = match index {
            0 => {
                assert_eq!(parent, 0, "layer popups must use a null XDG parent");
                Positioner { size: [60, 30], rect: [4, 6, 14, 9], anchor: 7, gravity: 6, constraints: 51, offset: [-3, 7], reactive: true }
            }
            1 => {
                assert_eq!(parent, self.popups[0].xdg, "nested popup names the outer xdg_surface");
                Positioner { size: [28, 16], rect: [1, 2, 6, 8], anchor: 6, gravity: 8, constraints: 3, offset: [0, 0], reactive: false }
            }
            _ => panic!("unexpected popup role #{index}"),
        };
        assert_eq!(self.positioners[&positioner], expected, "positioner for popup #{index}");
        self.objects.insert(role, Object::Popup(index));
        self.popups.push(Popup {
            role,
            xdg,
            wl,
            associated: false,
            grabbed: false,
            committed: false,
            presenting: false,
            window_geometries: Vec::new(),
        });
    }

    fn commit(&mut self, wl: u32) {
        let layer = self.layer.expect("layer must be created before any commit");
        if wl == layer.wl {
            if !self.layer_committed {
                self.layer_committed = true;
                self.event(layer.role, 0, &[LAYER_CONFIGURE, 200, 40]);
            }
            return;
        }
        let (index, popup) = self.popups.iter_mut().enumerate().find(|(_, popup)| popup.wl == wl).expect("surface has a popup role");
        if popup.committed {
            return;
        }
        assert_eq!(popup.associated, index == 0, "only the outer popup associates to the layer before its first commit");
        assert!(popup.grabbed, "popup #{index} grabbed before its first commit");
        popup.committed = true;
        let (role, xdg) = (popup.role, popup.xdg);
        let (serial, width, height) = [(FIRST_CONFIGURE, 59, 29), (NESTED_CONFIGURE, 28, 16)][index];
        self.event(role, 0, &[0, 0, width, height]);
        self.event(xdg, 0, &[serial]);
    }

    fn reposition(&mut self, id: u32, positioner: u32, token: u32) {
        assert!(!self.repositioned, "one live reposition request");
        assert!(self.popups[0].committed, "reposition follows the initial configure");
        assert!(self.popups[0].presenting, "reposition waits for the first mapped presentation");
        assert_eq!(self.positioners[&positioner], Positioner {
            size: [72, 38], rect: [4, 6, 14, 9], anchor: 7, gravity: 6,
            constraints: 51, offset: [11, -2], reactive: true,
        }, "live reposition carries the changed offset and size");
        self.repositioned = true;
        self.event(id, 2, &[token]);
        self.event(id, 0, &[0, 0, 72, 38]);
        self.event(self.popups[0].xdg, 0, &[MOVED_CONFIGURE]);
    }

    fn ack_popup(&mut self, wl: u32, serial: u32) {
        match serial {
            FIRST_CONFIGURE => assert_eq!(wl, self.popups[0].wl, "first configure ack"),
            MOVED_CONFIGURE => {
                assert_eq!(wl, self.popups[0].wl, "reposition configure ack");
                let layer = self.layer.expect("layer exists");
                let pointer = self.pointer.expect("the pointer is bound");
                self.event(pointer, 3, &[POPUP_PRESS - 4, 100, 0x110, 0]); // release on layer
                self.event(pointer, 1, &[POPUP_PRESS - 3, layer.wl]); // leave layer
                self.press(wl, POPUP_PRESS);
            }
            NESTED_CONFIGURE => {
                assert_eq!(wl, self.popups[1].wl, "nested configure ack");
                self.event(self.popups[1].role, 1, &[]); // xdg_popup.popup_done
            }
            _ => panic!("unexpected xdg_surface ack_configure serial {serial}"),
        }
    }

    fn press_layer_when_ready(&mut self) {
        if self.layer_ack && self.pointer.is_some() && !self.layer_pressed {
            self.layer_pressed = true;
            self.press(self.layer.expect("layer exists").wl, LAYER_PRESS);
        }
    }

    fn press(&mut self, wl: u32, serial: u32) {
        let pointer = self.pointer.expect("the pointer is bound");
        self.event(pointer, 0, &[serial - 1, wl, 5 << 8, 6 << 8]);
        self.event(pointer, 3, &[serial, serial, 0x110, 1]);
        self.event(pointer, 5, &[]); // wl_pointer.frame
    }

    fn done(&mut self, callback: u32) {
        self.event(callback, 0, &[0]);
        self.event(1, 1, &[callback]);
    }

    fn event(&mut self, id: u32, opcode: u32, body: &[u32]) {
        self.event_bytes(id, opcode, &words(body));
    }

    fn event_bytes(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let length = u32::try_from(8 + body.len()).expect("small Wayland event");
        let mut event = words(&[id, length << 16 | opcode]);
        event.extend(body);
        self.stream.write_all(&event).expect("send Wayland event");
    }
}

fn word(bytes: &[u8], index: usize) -> u32 {
    u32::from_ne_bytes(bytes[index * 4..][..4].try_into().expect("Wayland argument word"))
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|word| word.to_ne_bytes()).collect()
}

fn string(value: &str) -> Vec<u8> {
    let length = value.len() + 1;
    let mut bytes = words(&[u32::try_from(length).expect("short interface name")]);
    bytes.extend_from_slice(value.as_bytes());
    bytes.resize(4 + length.next_multiple_of(4), 0);
    bytes
}

const INSETS_TEST: &str = "popup_geometry_insets_protocol_contract_on_the_wire";
const INSETS_V2_TEST: &str = "popup_geometry_insets_on_xdg_v2_without_reposition";

fn spawn_client(test_name: &str, suffix: &str) -> (SocketFile, std::process::Child, UnixStream) {
    let socket = SocketFile(
        std::env::temp_dir().join(format!("zgui-popup-{suffix}-{}.sock", std::process::id())),
    );
    let listener = UnixListener::bind(&socket.0).expect("bind the private Wayland socket");
    listener.set_nonblocking(true).expect("bound the accept wait");
    let mut child = Command::new(std::env::current_exe().expect("locate this test binary"))
        .args(["--exact", test_name])
        .env(CLIENT, "1")
        .env("WAYLAND_DISPLAY", &socket.0)
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .expect("launch the actual Wayland backend");
    let start = Instant::now();
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    child.try_wait().expect("poll test client").is_none(),
                    "client exited before connecting"
                );
                assert!(start.elapsed() < TIMEOUT, "client did not connect");
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("accepting test client: {error}"),
        }
    };
    stream.set_read_timeout(Some(TIMEOUT)).expect("bound each read");
    stream.set_write_timeout(Some(TIMEOUT)).expect("bound each write");
    (socket, child, stream)
}

#[test]
fn popup_geometry_insets_protocol_contract_on_the_wire() {
    if std::env::var_os(CLIENT).is_some() {
        let mut app = WaylandApp::new(InsetsClient::default()).expect("connect to compositor");
        app.run().expect("Wayland client loop");
        assert_eq!(app.handler().stage, 5, "all inset protocol stages must complete");
        return;
    }

    let (_socket, mut child, stream) = spawn_client(INSETS_TEST, "insets");
    let mut server = InsetsCompositor::new(stream, 6);
    server.run();
    let status = child.wait().expect("wait for test client");
    assert!(status.success(), "the Wayland client exited with {status}");
    assert_eq!(server.popups.len(), 2, "outer and nested popup roles");
    assert_eq!(
        server.popups[0].initial_positioner.size,
        [100, 60],
        "xdg_positioner.set_size must always receive visual geometry G, never drawable S"
    );
    assert_eq!(
        server.popups[0].reposition_count, 0,
        "restatements, equal-sum inset swaps, and zero-reset must not issue xdg_popup.reposition"
    );
    assert_eq!(
        server.popups[0].window_geometries,
        vec![
            [12, 16, 96, 56],
            [12, 16, 96, 56],
            [20, 16, 96, 56],
            [0, 0, 96, 56],
        ],
        "outer popup must emit set_window_geometry on first frame, scale change (still logical), equal-sum swap, and explicit zero reset"
    );
    assert_eq!(
        server.popups[1].initial_positioner.size,
        [40, 20],
        "nested popup positioner size must be visual"
    );
    assert_eq!(
        server.popups[1].initial_positioner.rect,
        [28, 30, 30, 18],
        "nested popup anchor_rect must subtract parent (L=12, T=16)"
    );
    assert_eq!(
        server.popups[1].repositioned_rects,
        vec![[20, 30, 30, 18], [40, 46, 30, 10]],
        "parent inset changes must re-translate and clamp nested child anchor_rect"
    );
    assert_eq!(
        server.popups[1].window_geometries,
        vec![[8, 8, 40, 20]],
        "nested popup must emit its own set_window_geometry"
    );
}

#[derive(Default)]
struct InsetsClient {
    layer: Option<SurfaceId>,
    outer: Option<SurfaceId>,
    child: Option<SurfaceId>,
    outer_placement: Option<PopupPlacement>,
    stage: u8,
}

impl AppHandler for InsetsClient {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        let mut attrs = SurfaceAttributes::default();
        attrs.role = SurfaceRole::Layer(LayerPlacement::default());
        attrs.size = Some(size(300.0, 40.0));
        self.layer = Some(cx.create_surface(&attrs).expect("open layer").id());
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, id: SurfaceId, event: SurfaceEvent) {
        match event {
            SurfaceEvent::Resized(_) if Some(id) == self.layer && self.outer.is_none() => {
                let rect = Rect::new(Point::new(CssPx(10.0), CssPx(10.0)), size(40.0, 20.0));
                let p = PopupPlacement::below(id, rect)
                    .with_geometry_insets(Edges::new(CssPx(16.0), CssPx(28.0), CssPx(32.0), CssPx(12.0)));
                self.outer_placement = Some(p.clone());
                self.outer = Some(open_popup(cx, p, size(100.0, 60.0)));
                cx.surface(id).expect("layer").pre_present_notify();
            }
            SurfaceEvent::Resized(extent) if Some(id) == self.outer && self.stage == 0 => {
                // Compositor clamped visual G=(96, 56) + insets (12+28=40, 16+32=48) -> S=(136, 104).
                assert_eq!(extent, Size::new(DevicePx(136.0), DevicePx(104.0)));
                let outer = cx.surface(id).expect("outer popup");
                outer.pre_present_notify();
                // Dedup check: calling reconfigure_popup with identical visual size & insets must not reposition.
                let same = self.outer_placement.clone().expect("placement");
                outer.reconfigure_popup(&same, size(100.0, 60.0)).expect("dedup reconfigure");
                self.stage = 1;
            }
            SurfaceEvent::ScaleFactorChanged { scale_factor, size: extent }
                if Some(id) == self.outer && self.stage == 1 =>
            {
                assert!((scale_factor - 2.0).abs() < 1e-6);
                // ScaleFactorChanged must scale S=(136, 104) -> (272, 208) without double-adding insets!
                assert_eq!(extent, Size::new(DevicePx(272.0), DevicePx(208.0)));
                let outer = cx.surface(id).expect("outer popup");
                outer.pre_present_notify();
                // Open nested child anchored at surface-local (40, 46, 30, 18) -> translated (28, 30, 30, 18).
                let anchor = Rect::new(Point::new(CssPx(40.0), CssPx(46.0)), size(30.0, 18.0));
                let child_place = PopupPlacement::below(id, anchor)
                    .with_geometry_insets(Edges::new(CssPx(8.0), CssPx(8.0), CssPx(8.0), CssPx(8.0)));
                self.child = Some(open_popup(cx, child_place, size(40.0, 20.0)));
                self.stage = 2;
            }
            SurfaceEvent::Resized(extent) if Some(id) == self.child && self.stage == 2 => {
                // Child visual (40, 20) + insets 8 -> (56, 36) at scale 1 (no preferred_buffer_scale on child).
                assert_eq!(extent, Size::new(DevicePx(56.0), DevicePx(36.0)));
                cx.surface(id).expect("child popup").pre_present_notify();
                // Stage 3: equal-sum L/R swap on outer (12/28 -> 20/20, L+R still 40).
                let outer = cx.surface(self.outer.expect("outer")).expect("outer surface");
                let mut swapped = self.outer_placement.clone().expect("placement");
                swapped.geometry_insets = Edges::new(CssPx(16.0), CssPx(20.0), CssPx(32.0), CssPx(20.0));
                self.outer_placement = Some(swapped.clone());
                outer.reconfigure_popup(&swapped, size(100.0, 60.0)).expect("equal-sum swap");
                // Drawable pixel size is unchanged at 272x208; pre_present must still emit set_window_geometry(20, 16, 96, 56).
                assert_eq!(outer.size(), Size::new(DevicePx(272.0), DevicePx(208.0)));
                outer.pre_present_notify();
                self.stage = 3;
            }
            SurfaceEvent::RedrawRequested if Some(id) == self.outer && self.stage == 3 => {
                // Stage 4: reset outer insets back to zero (must emit explicit set_window_geometry(0, 0, 96, 56)).
                let outer = cx.surface(id).expect("outer surface");
                let mut zeroed = self.outer_placement.clone().expect("placement");
                zeroed.geometry_insets = Edges::new(CssPx(0.0), CssPx(0.0), CssPx(0.0), CssPx(0.0));
                self.outer_placement = Some(zeroed.clone());
                outer.reconfigure_popup(&zeroed, size(100.0, 60.0)).expect("zero reset");
                self.stage = 4;
            }
            SurfaceEvent::Resized(extent) if Some(id) == self.outer && self.stage == 4 => {
                // Reset to zero insets: visual (96, 56) * scale 2 = (192, 112).
                assert_eq!(extent, Size::new(DevicePx(192.0), DevicePx(112.0)));
                let outer = cx.surface(id).expect("outer surface");
                outer.pre_present_notify();
                outer.request_redraw();
                self.stage = 5;
            }
            SurfaceEvent::RedrawRequested if Some(id) == self.outer && self.stage == 5 => {
                cx.destroy_surface(self.layer.expect("layer"));
                cx.request_exit();
            }
            _ => {}
        }
    }

    fn wake(&mut self, _cx: &dyn PlatformCx, _reason: WakeReason) {}
}

#[test]
fn popup_geometry_insets_on_xdg_v2_without_reposition() {
    if std::env::var_os(CLIENT).is_some() {
        let mut app = WaylandApp::new(V2Client::default()).expect("connect to v2 compositor");
        app.run().expect("Wayland client loop");
        assert_eq!(app.handler().stage, 2, "v2 inset-only update must complete");
        return;
    }

    let (_socket, mut child, stream) = spawn_client(INSETS_V2_TEST, "insets-v2");
    let mut server = InsetsCompositor::new(stream, 2);
    server.run();
    let status = child.wait().expect("wait for v2 client");
    assert!(status.success(), "the Wayland v2 client exited with {status}");
    assert_eq!(server.popups.len(), 1);
    assert_eq!(server.popups[0].initial_positioner.size, [80, 40]);
    assert_eq!(server.popups[0].reposition_count, 0, "xdg_wm_base v2 must never call reposition");
    assert_eq!(
        server.popups[0].window_geometries,
        vec![[10, 10, 80, 40], [20, 20, 80, 40]],
        "xdg_wm_base v2 must emit initial and updated set_window_geometry locally"
    );
}

#[derive(Default)]
struct V2Client {
    layer: Option<SurfaceId>,
    popup: Option<SurfaceId>,
    placement: Option<PopupPlacement>,
    stage: u8,
}

impl AppHandler for V2Client {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        let mut attrs = SurfaceAttributes::default();
        attrs.role = SurfaceRole::Layer(LayerPlacement::default());
        attrs.size = Some(size(300.0, 40.0));
        self.layer = Some(cx.create_surface(&attrs).expect("open layer").id());
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, id: SurfaceId, event: SurfaceEvent) {
        match event {
            SurfaceEvent::Resized(_) if Some(id) == self.layer && self.popup.is_none() => {
                let rect = Rect::new(Point::new(CssPx(8.0), CssPx(8.0)), size(24.0, 20.0));
                let p = PopupPlacement::below(id, rect)
                    .with_geometry_insets(Edges::new(CssPx(10.0), CssPx(10.0), CssPx(10.0), CssPx(10.0)));
                self.placement = Some(p.clone());
                self.popup = Some(open_popup(cx, p, size(80.0, 40.0)));
                cx.surface(id).expect("layer").pre_present_notify();
            }
            SurfaceEvent::Resized(extent) if Some(id) == self.popup && self.stage == 0 => {
                assert_eq!(extent, Size::new(DevicePx(100.0), DevicePx(60.0)));
                let popup = cx.surface(id).expect("v2 popup");
                assert!(!popup.supports_popup_reposition(), "v2 shell cannot reposition");
                popup.pre_present_notify();
                let mut updated = self.placement.clone().expect("placement");
                updated.geometry_insets = Edges::new(CssPx(20.0), CssPx(20.0), CssPx(20.0), CssPx(20.0));
                popup.reconfigure_popup(&updated, size(80.0, 40.0)).expect("local inset update on v2");
                self.stage = 1;
            }
            SurfaceEvent::Resized(extent) if Some(id) == self.popup && self.stage == 1 => {
                assert_eq!(extent, Size::new(DevicePx(120.0), DevicePx(80.0)));
                let popup = cx.surface(id).expect("v2 popup");
                popup.pre_present_notify();
                popup.request_redraw();
                self.stage = 2;
            }
            SurfaceEvent::RedrawRequested if Some(id) == self.popup && self.stage == 2 => {
                cx.destroy_surface(self.layer.expect("layer"));
                cx.request_exit();
            }
            _ => {}
        }
    }

    fn wake(&mut self, _cx: &dyn PlatformCx, _reason: WakeReason) {}
}

struct InsetsPopup {
    role: u32,
    xdg: u32,
    wl: u32,
    committed: bool,
    initial_positioner: Positioner,
    reposition_count: u32,
    repositioned_rects: Vec<[i32; 4]>,
    window_geometries: Vec<[i32; 4]>,
    sent_restatement_and_scale: bool,
}

struct InsetsCompositor {
    stream: UnixStream,
    xdg_version: u32,
    objects: HashMap<u32, Object>,
    layer: Option<Layer>,
    layer_committed: bool,
    positioners: HashMap<u32, Positioner>,
    popups: Vec<InsetsPopup>,
    next_serial: u32,
    write_closed: bool,
}

impl InsetsCompositor {
    fn new(stream: UnixStream, xdg_version: u32) -> Self {
        Self {
            stream,
            xdg_version,
            objects: HashMap::new(),
            layer: None,
            layer_committed: false,
            positioners: HashMap::new(),
            popups: Vec::new(),
            next_serial: 100,
            write_closed: false,
        }
    }

    fn run(&mut self) {
        let mut header = [0; 8];
        // A failed callback reply does not consume requests the client already flushed.
        loop {
            match self.stream.read_exact(&mut header) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                    ) =>
                {
                    return;
                }
                Err(error) => panic!("reading a Wayland request: {error}"),
            }
            let [id, descriptor] = [word(&header, 0), word(&header, 1)];
            let mut body = vec![0; (descriptor >> 16) as usize - 8];
            match self.stream.read_exact(&mut body) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                    ) =>
                {
                    return;
                }
                Err(error) => panic!("Wayland request body: {error}"),
            }
            self.request(id, descriptor & 0xffff, &body);
        }
    }

    fn request(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let arg = |offset| word(body, offset);
        if id == 1 {
            match opcode {
                0 => {
                    let cb = arg(0);
                    self.event(cb, 0, &[0]);
                    self.event(1, 1, &[cb]);
                }
                1 => {
                    let registry = arg(0);
                    self.objects.insert(registry, Object::Registry);
                    for (name, interface, version) in [
                        (1, "wl_compositor", 6),
                        (2, "wl_shm", 1),
                        (3, "xdg_wm_base", self.xdg_version),
                        (4, "zwlr_layer_shell_v1", 4),
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
        match (self.objects[&id], opcode) {
            (Object::Registry, 0) => {
                let bound = arg(3 + (arg(1) as usize).div_ceil(4));
                let object = match arg(0) {
                    1 => Object::Compositor,
                    2 => Object::Shm,
                    3 => Object::XdgShell,
                    4 => Object::LayerShell,
                    name => panic!("unexpected global {name}"),
                };
                self.objects.insert(bound, object);
            }
            (Object::Compositor, 0) => { self.objects.insert(arg(0), Object::Surface); }
            (Object::Compositor, 1) => { self.objects.insert(arg(0), Object::Region); }
            (Object::LayerShell, 0) => {
                self.layer = Some(Layer { role: arg(0), wl: arg(1) });
                self.objects.insert(arg(0), Object::Layer);
            }
            (Object::XdgShell, 1) => {
                self.objects.insert(arg(0), Object::Positioner);
                self.positioners.insert(arg(0), Positioner::default());
            }
            (Object::XdgShell, 2) => { self.objects.insert(arg(0), Object::XdgSurface(arg(1))); }
            (Object::Positioner, 0) => { self.positioners.remove(&id); }
            (Object::Positioner, 1..=9) => {
                let place = self.positioners.get_mut(&id).expect("live positioner");
                match opcode {
                    1 => place.size = [0, 1].map(|i| arg(i) as i32),
                    2 => place.rect = [0, 1, 2, 3].map(|i| arg(i) as i32),
                    3 => place.anchor = arg(0),
                    4 => place.gravity = arg(0),
                    5 => place.constraints = arg(0),
                    6 => place.offset = [0, 1].map(|i| arg(i) as i32),
                    7 => place.reactive = true,
                    8 | 9 => {}
                    _ => unreachable!(),
                }
            }
            (Object::XdgSurface(wl), 2) => {
                let role = arg(0);
                let positioner = arg(2);
                let index = self.popups.len();
                let pos = self.positioners[&positioner];
                self.objects.insert(role, Object::Popup(index));
                self.popups.push(InsetsPopup {
                    role,
                    xdg: id,
                    wl,
                    committed: false,
                    initial_positioner: pos,
                    reposition_count: 0,
                    repositioned_rects: Vec::new(),
                    window_geometries: Vec::new(),
                    sent_restatement_and_scale: false,
                });
            }
            (Object::Popup(index), 2) => {
                let positioner = arg(0);
                let token = arg(1);
                let pos = self.positioners[&positioner];
                let popup = &mut self.popups[index];
                popup.reposition_count += 1;
                popup.repositioned_rects.push(pos.rect);
                let (role, xdg) = (popup.role, popup.xdg);
                self.next_serial += 1;
                let serial = self.next_serial;
                self.event(role, 2, &[token]);
                self.event(role, 0, &[0, 0, pos.size[0] as u32, pos.size[1] as u32]);
                self.event(xdg, 0, &[serial]);
            }
            (Object::XdgSurface(_), 3) => {
                let geom = [0, 1, 2, 3].map(|i| arg(i) as i32);
                if let Some(popup) = self.popups.iter_mut().find(|popup| popup.xdg == id) {
                    popup.window_geometries.push(geom);
                }
            }
            (Object::Surface, 3) => {
                // wl_surface.frame: fires inside pre_present_notify().
                let cb = arg(0);
                self.event(cb, 0, &[1]);
                self.event(1, 1, &[cb]);
                if self.xdg_version >= 3
                    && let Some(popup) = self.popups.first_mut()
                    && popup.wl == id
                    && !popup.sent_restatement_and_scale
                {
                    popup.sent_restatement_and_scale = true;
                    let (role, xdg, wl) = (popup.role, popup.xdg, popup.wl);
                    // Send a restated configure (96, 56) followed by wl_surface.preferred_buffer_scale(2).
                    self.next_serial += 1;
                    let serial = self.next_serial;
                    self.event(role, 0, &[0, 0, 96, 56]);
                    self.event(xdg, 0, &[serial]);
                    self.event(wl, 2, &[2]); // wl_surface.preferred_buffer_scale(2)
                }
            }
            (Object::Surface, 6) => {
                let layer = self.layer.expect("layer exists");
                if id == layer.wl {
                    if !self.layer_committed {
                        self.layer_committed = true;
                        self.event(layer.role, 0, &[LAYER_CONFIGURE, 300, 40]);
                    }
                    return;
                }
                if let Some((index, popup)) =
                    self.popups.iter_mut().enumerate().find(|(_, p)| p.wl == id)
                    && !popup.committed
                {
                    popup.committed = true;
                    let (role, xdg) = (popup.role, popup.xdg);
                    let (w, h) = if self.xdg_version < 3 {
                        (80, 40)
                    } else if index == 0 {
                        (96, 56)
                    } else {
                        (40, 20)
                    };
                    self.next_serial += 1;
                    let serial = self.next_serial;
                    self.event(role, 0, &[0, 0, w, h]);
                    self.event(xdg, 0, &[serial]);
                }
            }
            (Object::Layer, 0..=8)
            | (Object::LayerShell, 1)
            | (Object::XdgShell, 0 | 3)
            | (Object::XdgSurface(_), 0 | 4)
            | (Object::Popup(_), 0 | 1)
            | (Object::Region, 0..=2)
            | (Object::Surface, 0..=2 | 4..=5 | 7..=10) => {}
            (object, _) => panic!("unexpected {object:?} opcode {opcode}"),
        }
    }

    fn event(&mut self, id: u32, opcode: u32, body: &[u32]) {
        self.event_bytes(id, opcode, &words(body));
    }

    fn event_bytes(&mut self, id: u32, opcode: u32, body: &[u8]) {
        if self.write_closed {
            return;
        }
        let length = u32::try_from(8 + body.len()).expect("small Wayland event");
        let mut event = words(&[id, length << 16 | opcode]);
        event.extend(body);
        if let Err(error) = self.stream.write_all(&event) {
            if matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ) {
                self.write_closed = true;
            } else {
                panic!("send Wayland event: {error}");
            }
        }
    }
}
