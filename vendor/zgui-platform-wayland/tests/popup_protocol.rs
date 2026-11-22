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

use zgui_geom::{Css, CssPx, Point, Rect, Size};
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
            | (Object::XdgSurface(_), 3)
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
        self.popups.push(Popup { role, xdg, wl, associated: false, grabbed: false, committed: false, presenting: false });
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
