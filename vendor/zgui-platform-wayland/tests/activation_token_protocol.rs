//! External `xdg_activation_v1` token minting, same-turn surface destruction wire ordering,
//! cancellation/timeout single-destroy discipline, touch-cancel grant revocation, and
//! `SessionLock` Caps Lock focus replay on a private Wayland socket.

#![cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]

use std::collections::{HashMap, VecDeque};
use std::io::{IoSlice, IoSliceMut, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
};
use zgui_geom::{Css, CssPx, Point, Rect, Size};
use zgui_platform::{
    ActivationTokenSerial, AppHandler, InputGrant, KeyboardInteractivity, LayerPlacement,
    MonitorTarget, PendingActivationToken, PlatformCx, PopupPlacement, SessionLockState,
    SurfaceAttributes, SurfaceEvent, SurfaceId, SurfaceRole, WakeReason,
};
use zgui_platform_wayland::WaylandApp;
use zgui_vocab::{KeyState, NamedKey, PointerAction};

const CLIENT_ENV: &str = "ZGUI_ACTIVATION_TOKEN_PROTOCOL_CLIENT";
const TIMEOUT: Duration = Duration::from_secs(5);

const LAYER_CONFIGURE: u32 = 10;
const POPUP_CONFIGURE: u32 = 20;
const LAYER_PTR_SERIAL: u32 = 101;
const TOUCH_CANCEL_SERIAL: u32 = 201;
const TOUCH_MINT_SERIAL: u32 = 202;
const POPUP_KEY_SERIAL: u32 = 301;
const CANCEL_RACE_SERIAL: u32 = 401;
const LOCK_PTR_SERIAL: u32 = 501;

const CAPS_LOCK_MASK: u32 = 1 << 1;
const XKB_V1: u32 = 1;
const KEY_ENTER_EVDEV: u32 = 28;

const KEYMAP: &str = "xkb_keymap {
    xkb_keycodes { include \"evdev+aliases(qwerty)\" };
    xkb_types { include \"complete\" };
    xkb_compat { include \"complete\" };
    xkb_symbols { include \"pc+us+inet(evdev)\" };
};\0";

struct SocketFile(std::path::PathBuf);
impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn spawn_client(test_name: &str, mode: &str) -> (SocketFile, std::process::Child, UnixStream) {
    let socket = SocketFile(std::env::temp_dir().join(format!(
        "zgui-act-tok-{mode}-{}.sock",
        std::process::id()
    )));
    let listener = UnixListener::bind(&socket.0).expect("bind private Wayland socket");
    listener.set_nonblocking(true).expect("nonblocking accept");
    let mut child = Command::new(std::env::current_exe().expect("locate test binary"))
        .args(["--exact", test_name])
        .env(CLIENT_ENV, mode)
        .env("WAYLAND_DISPLAY", &socket.0)
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .expect("launch Wayland client");
    let start = Instant::now();
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    child.try_wait().expect("poll client").is_none(),
                    "client exited before connecting"
                );
                assert!(start.elapsed() < TIMEOUT, "client did not connect");
                thread::sleep(Duration::from_millis(1));
            }
            Err(err) => panic!("accept client: {err}"),
        }
    };
    stream.set_read_timeout(Some(TIMEOUT)).expect("read timeout");
    stream.set_write_timeout(Some(TIMEOUT)).expect("write timeout");
    (socket, child, stream)
}

#[test]
fn activation_token_minting_wire_order_and_cancellation_on_the_wire() {
    const TEST_NAME: &str = "activation_token_minting_wire_order_and_cancellation_on_the_wire";
    if std::env::var(CLIENT_ENV).as_deref() == Ok("mint") {
        let mut app =
            WaylandApp::new(MintClient::default()).expect("connect to private Wayland compositor");
        app.run().expect("Wayland client loop");
        let h = app.handler();
        assert!(h.foreign_surface_rejected, "popup must reject layer surface's grant");
        assert!(h.saw_touch_cancel_revocation, "wl_touch.cancel must emit CancelPressGrants");
        assert_eq!(
            h.resolved_tokens,
            vec![
                "tok-layer-pointer".to_owned(),
                "tok-layer-touch".to_owned(),
                "tok-popup-after-destroy".to_owned(),
            ],
            "must resolve exact three tokens and ignore late done after cancel"
        );
        return;
    }

    let (_socket, mut child, stream) = spawn_client(TEST_NAME, "mint");
    let mut server = MintCompositor::new(stream);
    server.run();
    let status = child.wait().expect("wait for client");
    assert!(status.success(), "Wayland client exited with {status}");

    assert_eq!(
        server.activate_calls, 0,
        "external token export must NEVER call xdg_activation_v1.activate"
    );
    assert_eq!(
        server.tokens.len(),
        4,
        "expected 4 xdg_activation_token_v1 objects (pointer, touch, popup-key, cancelled)"
    );

    let layer_wl = server.layer.expect("layer created").wl;
    let popup_wl = server.popup.as_ref().expect("popup created").wl;
    let seat_id = server.seat.expect("seat bound");

    // Token #0: Layer pointer press with app_id="" -> set_app_id omitted!
    let t0 = &server.tokens[0];
    assert_eq!(t0.surface, Some(layer_wl));
    assert_eq!(t0.serial_and_seat, Some((LAYER_PTR_SERIAL, seat_id)));
    assert_eq!(t0.app_id, None, "empty app_id must omit set_app_id without falling back to host app_id");
    assert!(t0.committed);
    assert_eq!(t0.destroy_count, 1, "token #0 must be destroyed exactly once on done");

    // Token #1: Layer touch down with explicit app_id.
    let t1 = &server.tokens[1];
    assert_eq!(t1.surface, Some(layer_wl));
    assert_eq!(t1.serial_and_seat, Some((TOUCH_MINT_SERIAL, seat_id)));
    assert_eq!(t1.app_id.as_deref(), Some("org.freedesktop.Nautilus"));
    assert!(t1.committed);
    assert_eq!(t1.destroy_count, 1, "token #1 must be destroyed exactly once on done");

    // Token #2: Popup keyboard Enter press followed IMMEDIATELY by destroy_surface(popup) in same turn.
    let t2 = &server.tokens[2];
    assert_eq!(
        t2.surface,
        Some(popup_wl),
        "popup gesture must quote popup wl_surface, never parent layer wl_surface"
    );
    assert_eq!(t2.serial_and_seat, Some((POPUP_KEY_SERIAL, seat_id)));
    assert_eq!(t2.app_id.as_deref(), Some("org.gnome.Ptyxis"));
    assert!(t2.committed);
    let commit_seq = t2.commit_seq.expect("token #2 commit sequence recorded");
    let popup_destroy_seq = server
        .popup
        .as_ref()
        .and_then(|p| p.first_destroy_seq)
        .expect("popup destroy sequence recorded");
    assert!(
        commit_seq < popup_destroy_seq,
        "xdg_activation_token_v1.commit (seq {commit_seq}) must arrive strictly BEFORE popup/wl_surface destroy (seq {popup_destroy_seq})"
    );
    assert_eq!(t2.destroy_count, 1, "token #2 must be destroyed once after post-surface-destroy done");

    // Token #3: Cancelled before done -> destroyed once, late done causes no second destroy.
    let t3 = &server.tokens[3];
    assert_eq!(t3.surface, Some(layer_wl));
    assert_eq!(t3.serial_and_seat, Some((CANCEL_RACE_SERIAL, seat_id)));
    assert_eq!(t3.app_id, None);
    assert!(t3.committed);
    assert_eq!(t3.destroy_count, 1, "cancelled token #3 must be destroyed exactly once");
}

#[derive(Default)]
struct MintClient {
    layer: Option<SurfaceId>,
    popup: Option<SurfaceId>,
    last_grant: Option<InputGrant>,
    stage: u8,
    foreign_surface_rejected: bool,
    saw_touch_cancel_revocation: bool,
    expected_serials: Vec<ActivationTokenSerial>,
    cancelled_serial: Option<ActivationTokenSerial>,
    resolved_tokens: Vec<String>,
}

impl AppHandler for MintClient {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        let mut attrs = SurfaceAttributes::new("dev.zgui.shell-host");
        attrs.application_id = Some("dev.zgui.shell-host".into());
        attrs.role = SurfaceRole::Layer(
            LayerPlacement::default().with_keyboard(KeyboardInteractivity::None),
        );
        attrs.size = Some(Size::new(CssPx(240.0), CssPx(40.0)));
        self.layer = Some(cx.create_surface(&attrs).expect("open layer").id());
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, id: SurfaceId, event: SurfaceEvent) {
        match event {
            SurfaceEvent::Resized(_) if Some(id) == self.layer && self.popup.is_none() => {
                let anchor = Rect::new(
                    Point::new(CssPx(10.0), CssPx(4.0)),
                    Size::<CssPx, Css>::new(CssPx(32.0), CssPx(24.0)),
                );
                let mut attrs = SurfaceAttributes::popup(PopupPlacement::below(id, anchor));
                attrs.size = Some(Size::new(CssPx(120.0), CssPx(80.0)));
                self.popup = Some(cx.create_surface(&attrs).expect("open popup").id());
                cx.surface(id).expect("layer").pre_present_notify();
            }
            SurfaceEvent::Resized(_) if Some(id) == self.popup && self.stage == 0 => {
                cx.surface(id).expect("popup").pre_present_notify();
                self.stage = 1;
            }
            SurfaceEvent::InputGrant(grant) => {
                self.last_grant = Some(grant);
            }
            SurfaceEvent::CancelPressGrants => {
                self.last_grant = None;
                self.saw_touch_cancel_revocation = true;
            }
            SurfaceEvent::Pointer {
                action: PointerAction::Pressed,
                ..
            } if Some(id) == self.layer && self.stage == 1 => {
                let grant = self.last_grant.take().expect("InputGrant paired with pointer press");
                assert_eq!(grant.serial().raw(), LAYER_PTR_SERIAL);
                // Foreign surface check: popup must refuse a grant minted for layer!
                let popup = cx.surface(self.popup.expect("popup")).expect("popup surface");
                assert!(popup.request_activation_token(grant, None).is_none());
                self.foreign_surface_rejected = true;

                let layer = cx.surface(id).expect("layer surface");
                let pending: PendingActivationToken = layer
                    .request_activation_token(grant, Some(""))
                    .expect("mint token #0 on keyboard=None layer surface");
                self.expected_serials.push(pending.serial());
                self.stage = 2;
            }
            SurfaceEvent::Pointer {
                action: PointerAction::Pressed,
                ..
            } if Some(id) == self.layer && self.stage == 2 => {
                let grant = self.last_grant.take().expect("InputGrant paired with touch press");
                if grant.serial().raw() == TOUCH_CANCEL_SERIAL {
                    // Wait for subsequent wl_touch.cancel to emit CancelPressGrants.
                } else if grant.serial().raw() == TOUCH_MINT_SERIAL {
                    assert!(
                        self.saw_touch_cancel_revocation,
                        "touch cancel must have arrived before second touch down"
                    );
                    let layer = cx.surface(id).expect("layer surface");
                    let pending = layer
                        .request_activation_token(grant, Some("org.freedesktop.Nautilus"))
                        .expect("mint token #1 from touch down");
                    self.expected_serials.push(pending.serial());
                    self.stage = 3;
                }
            }
            SurfaceEvent::Key {
                state: KeyState::Pressed,
                event: ref key_ev,
                ..
            } if Some(id) == self.popup && self.stage == 3 => {
                assert_eq!(key_ev.key, zgui_vocab::Key::Named(NamedKey::Enter));
                assert!(!key_ev.repeat);
                let grant = self.last_grant.take().expect("InputGrant paired with popup key press");
                assert_eq!(grant.serial().raw(), POPUP_KEY_SERIAL);
                let popup = cx.surface(id).expect("popup surface");
                let pending = popup
                    .request_activation_token(grant, Some("org.gnome.Ptyxis"))
                    .expect("mint token #2 on popup");
                self.expected_serials.push(pending.serial());
                // Immediately destroy the popup surface in the EXACT SAME callback turn!
                cx.destroy_surface(id);
                self.popup = None;
                self.stage = 4;
            }
            SurfaceEvent::Pointer {
                action: PointerAction::Pressed,
                ..
            } if Some(id) == self.layer && self.stage == 5 => {
                let grant = self.last_grant.take().expect("InputGrant paired with press #4");
                assert_eq!(grant.serial().raw(), CANCEL_RACE_SERIAL);
                let layer = cx.surface(id).expect("layer surface");
                let pending = layer
                    .request_activation_token(grant, None)
                    .expect("mint token #3");
                self.cancelled_serial = Some(pending.serial());
                // Cancel twice to prove idempotence before compositor sends done.
                pending.cancel();
                pending.cancel();
                // Signal compositor via layer request_redraw/set_title or destroy_surface to finish.
                cx.destroy_surface(id);
                self.stage = 6;
                cx.request_exit();
            }
            _ => {}
        }
    }

    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        if let WakeReason::ActivationToken { serial, token } = reason {
            // Verify Debug redacts raw token string.
            let debug_repr = format!(
                "{:?}",
                WakeReason::ActivationToken {
                    serial,
                    token: token.clone(),
                }
            );
            assert!(
                !debug_repr.contains(token.as_str()),
                "WakeReason Debug must redact raw token: {debug_repr}"
            );
            assert_ne!(
                Some(serial),
                self.cancelled_serial,
                "cancelled token must never deliver WakeReason::ActivationToken"
            );
            assert!(
                self.expected_serials.contains(&serial),
                "unexpected activation token serial {serial:?}"
            );
            self.resolved_tokens.push(token.into_string());
            if self.stage == 4 && self.resolved_tokens.len() == 3 {
                // Trigger final cancellation race stage via pre_present_notify on layer.
                self.stage = 5;
                cx.surface(self.layer.expect("layer")).expect("layer").pre_present_notify();
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum MintObject {
    Registry,
    Compositor,
    Shm,
    Pool,
    Buffer,
    XdgShell,
    LayerShell,
    Seat,
    Pointer,
    Keyboard,
    Touch,
    Activation,
    ActivationToken(usize),
    Surface,
    Layer,
    Positioner,
    XdgSurface(u32),
    Popup,
    Region,
}

#[derive(Clone, Copy)]
struct MintLayer {
    role: u32,
    wl: u32,
}

struct MintPopup {
    role: u32,
    xdg: u32,
    wl: u32,
    committed: bool,
    first_destroy_seq: Option<usize>,
}

#[derive(Default)]
struct RecordedToken {
    id: u32,
    surface: Option<u32>,
    serial_and_seat: Option<(u32, u32)>,
    app_id: Option<String>,
    committed: bool,
    commit_seq: Option<usize>,
    destroy_count: u32,
}

struct MintCompositor {
    stream: UnixStream,
    descriptors: VecDeque<OwnedFd>,
    objects: HashMap<u32, MintObject>,
    seq: usize,
    seat: Option<u32>,
    pointer: Option<u32>,
    keyboard: Option<u32>,
    touch: Option<u32>,
    layer: Option<MintLayer>,
    layer_committed: bool,
    popup: Option<MintPopup>,
    popup_ready: bool,
    sent_ptr_1: bool,
    tokens: Vec<RecordedToken>,
    pending_popup_token: Option<usize>,
    activate_calls: u32,
}

impl MintCompositor {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            descriptors: VecDeque::new(),
            objects: HashMap::new(),
            seq: 0,
            seat: None,
            pointer: None,
            keyboard: None,
            touch: None,
            layer: None,
            layer_committed: false,
            popup: None,
            popup_ready: false,
            sent_ptr_1: false,
            tokens: Vec::new(),
            pending_popup_token: None,
            activate_calls: 0,
        }
    }

    fn read(&mut self, mut bytes: &mut [u8]) -> bool {
        while !bytes.is_empty() {
            let mut storage = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
            let mut ancillary = RecvAncillaryBuffer::new(&mut storage);
            let message = match recvmsg(
                &self.stream,
                &mut [IoSliceMut::new(bytes)],
                &mut ancillary,
                RecvFlags::empty(),
            ) {
                Ok(msg) => msg,
                Err(rustix::io::Errno::CONNRESET) => return false,
                Err(err) => panic!("read Wayland request: {err}"),
            };
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
            let [id, desc] = [word(&header, 0), word(&header, 1)];
            let mut body = vec![0; (desc >> 16) as usize - 8];
            assert!(self.read(&mut body), "complete Wayland request body");
            self.seq += 1;
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
                    self.objects.insert(registry, MintObject::Registry);
                    for (name, interface, version) in [
                        (1, "wl_compositor", 6),
                        (2, "wl_shm", 1),
                        (3, "xdg_wm_base", 6),
                        (4, "zwlr_layer_shell_v1", 4),
                        (5, "wl_seat", 7),
                        (6, "xdg_activation_v1", 1),
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
        let Some(&obj) = self.objects.get(&id) else {
            return;
        };
        match (obj, opcode) {
            (MintObject::Registry, 0) => {
                let bound = arg(3 + (arg(1) as usize).div_ceil(4));
                let kind = match arg(0) {
                    1 => MintObject::Compositor,
                    2 => MintObject::Shm,
                    3 => MintObject::XdgShell,
                    4 => MintObject::LayerShell,
                    5 => {
                        self.seat = Some(bound);
                        self.event_bytes(bound, 1, &string("seat0"));
                        // Advertise Pointer (1) | Keyboard (2) | Touch (4) = 7.
                        self.event(bound, 0, &[7]);
                        MintObject::Seat
                    }
                    6 => MintObject::Activation,
                    n => panic!("unexpected global {n}"),
                };
                self.objects.insert(bound, kind);
            }
            (MintObject::Compositor, 0) => {
                self.objects.insert(arg(0), MintObject::Surface);
            }
            (MintObject::Compositor, 1) => {
                self.objects.insert(arg(0), MintObject::Region);
            }
            (MintObject::Shm, 0) => {
                let _ = self.descriptors.pop_front();
                self.objects.insert(arg(0), MintObject::Pool);
            }
            (MintObject::Pool, 0) => {
                self.objects.insert(arg(0), MintObject::Buffer);
            }
            (MintObject::Seat, 0) => {
                let ptr = arg(0);
                self.pointer = Some(ptr);
                self.objects.insert(ptr, MintObject::Pointer);
                self.maybe_send_pointer_1();
            }
            (MintObject::Seat, 1) => {
                let kbd = arg(0);
                self.keyboard = Some(kbd);
                self.objects.insert(kbd, MintObject::Keyboard);
                send_keymap(&mut self.stream, kbd);
                self.event(kbd, 5, &[25, 600]); // repeat_info
                self.maybe_send_pointer_1();
            }
            (MintObject::Seat, 2) => {
                let touch = arg(0);
                self.touch = Some(touch);
                self.objects.insert(touch, MintObject::Touch);
                self.maybe_send_pointer_1();
            }
            (MintObject::LayerShell, 0) => {
                self.layer = Some(MintLayer {
                    role: arg(0),
                    wl: arg(1),
                });
                self.objects.insert(arg(0), MintObject::Layer);
            }
            (MintObject::XdgShell, 1) => {
                self.objects.insert(arg(0), MintObject::Positioner);
            }
            (MintObject::XdgShell, 2) => {
                self.objects.insert(arg(0), MintObject::XdgSurface(arg(1)));
            }
            (MintObject::XdgSurface(wl), 2) => {
                let role = arg(0);
                self.objects.insert(role, MintObject::Popup);
                self.popup = Some(MintPopup {
                    role,
                    xdg: id,
                    wl,
                    committed: false,
                    first_destroy_seq: None,
                });
            }
            (MintObject::XdgSurface(wl), 4) => {
                if self.popup.as_ref().is_some_and(|p| p.wl == wl) {
                    self.popup_ready = true;
                    self.maybe_send_pointer_1();
                }
            }
            (MintObject::Surface, 3) => {
                // wl_surface.frame(callback) -> used by client pre_present_notify()
                let cb = arg(0);
                self.done(cb);
                if self.tokens.len() == 3
                    && self.tokens[2].destroy_count == 1
                    && Some(id) == self.layer.map(|l| l.wl)
                {
                    // Stage 5: send pointer press #4 for cancellation race!
                    let ptr = self.pointer.expect("pointer");
                    let layer_wl = self.layer.expect("layer").wl;
                    self.event(ptr, 0, &[CANCEL_RACE_SERIAL - 1, layer_wl, 12 << 8, 12 << 8]);
                    self.event(ptr, 3, &[CANCEL_RACE_SERIAL, 400, 0x110, 1]);
                    self.event(ptr, 5, &[]);
                }
            }
            (MintObject::Surface, 6) => {
                if let Some(layer) = self.layer
                    && id == layer.wl
                    && !self.layer_committed
                {
                    self.layer_committed = true;
                    self.event(layer.role, 0, &[LAYER_CONFIGURE, 240, 40]);
                } else if let Some(popup) = self.popup.as_mut()
                    && id == popup.wl
                    && !popup.committed
                {
                    popup.committed = true;
                    let (role, xdg) = (popup.role, popup.xdg);
                    self.event(role, 0, &[0, 0, 120, 80]);
                    self.event(xdg, 0, &[POPUP_CONFIGURE]);
                }
            }
            (MintObject::Popup | MintObject::XdgSurface(_), 0) => {
                if let Some(popup) = self.popup.as_mut()
                    && (id == popup.role || id == popup.xdg)
                {
                    popup.first_destroy_seq.get_or_insert(self.seq);
                }
            }
            (MintObject::Surface, 0) => {
                if let Some(popup) = self.popup.as_mut()
                    && id == popup.wl
                {
                    popup.first_destroy_seq.get_or_insert(self.seq);
                    // Now that popup wl_surface IS destroyed on the wire, answer pending token #2!
                    if let Some(idx) = self.pending_popup_token.take() {
                        let tok_id = self.tokens[idx].id;
                        self.event_bytes(tok_id, 0, &string("tok-popup-after-destroy"));
                    }
                } else if Some(id) == self.layer.map(|l| l.wl) {
                    // Client finished and destroyed layer -> close cleanly once EOF arrives.
                }
            }
            (MintObject::Activation, 1) => {
                let tok_id = arg(0);
                let idx = self.tokens.len();
                self.tokens.push(RecordedToken {
                    id: tok_id,
                    ..Default::default()
                });
                self.objects.insert(tok_id, MintObject::ActivationToken(idx));
            }
            (MintObject::Activation, 2) => {
                self.activate_calls += 1;
            }
            (MintObject::ActivationToken(idx), 0) => {
                self.tokens[idx].serial_and_seat = Some((arg(0), arg(1)));
            }
            (MintObject::ActivationToken(idx), 1) => {
                let len = arg(0) as usize;
                let text = std::str::from_utf8(&body[4..4 + len - 1])
                    .expect("utf8 app_id")
                    .to_owned();
                self.tokens[idx].app_id = Some(text);
            }
            (MintObject::ActivationToken(idx), 2) => {
                self.tokens[idx].surface = Some(arg(0));
            }
            (MintObject::ActivationToken(idx), 3) => {
                self.tokens[idx].committed = true;
                self.tokens[idx].commit_seq = Some(self.seq);
                let tok_id = self.tokens[idx].id;
                match idx {
                    0 => {
                        self.event_bytes(tok_id, 0, &string("tok-layer-pointer"));
                    }
                    1 => {
                        self.event_bytes(tok_id, 0, &string("tok-layer-touch"));
                    }
                    2 => {
                        // Hold done until popup's wl_surface.destroy arrives later in this same turn!
                        self.pending_popup_token = Some(idx);
                    }
                    3 => {
                        // Simulate race: send a late done before processing the client's cancel destroy!
                        self.event_bytes(tok_id, 0, &string("stale-token"));
                    }
                    _ => panic!("unexpected token #{idx}"),
                }
            }
            (MintObject::ActivationToken(idx), 4) => {
                self.tokens[idx].destroy_count += 1;
                if idx == 0 {
                    // Advance to Stage 2: send touch down + touch cancel, then touch down #2!
                    let touch = self.touch.expect("touch");
                    let layer_wl = self.layer.expect("layer").wl;
                    self.event(
                        touch,
                        0,
                        &[TOUCH_CANCEL_SERIAL, 200, layer_wl, 0, 16 << 8, 16 << 8],
                    );
                    self.event(touch, 3, &[]); // wl_touch.frame
                    self.event(touch, 4, &[]); // wl_touch.cancel -> revokes grants!
                    self.event(
                        touch,
                        0,
                        &[TOUCH_MINT_SERIAL, 210, layer_wl, 1, 20 << 8, 16 << 8],
                    );
                    self.event(touch, 3, &[]); // wl_touch.frame
                } else if idx == 1 {
                    // Advance to Stage 3: focus popup with keyboard and press Enter!
                    let kbd = self.keyboard.expect("keyboard");
                    let popup_wl = self.popup.as_ref().expect("popup").wl;
                    self.event(kbd, 1, &[POPUP_KEY_SERIAL - 1, popup_wl, 0]); // enter with empty keys array
                    self.event(kbd, 3, &[POPUP_KEY_SERIAL, 300, KEY_ENTER_EVDEV, 1]);
                }
            }
            _ => {}
        }
    }

    fn maybe_send_pointer_1(&mut self) {
        if self.sent_ptr_1
            || !self.popup_ready
            || self.pointer.is_none()
            || self.keyboard.is_none()
            || self.touch.is_none()
        {
            return;
        }
        self.sent_ptr_1 = true;
        let ptr = self.pointer.expect("pointer");
        let layer_wl = self.layer.expect("layer").wl;
        self.event(ptr, 0, &[LAYER_PTR_SERIAL - 1, layer_wl, 8 << 8, 8 << 8]);
        self.event(ptr, 3, &[LAYER_PTR_SERIAL, 100, 0x110, 1]);
        self.event(ptr, 5, &[]);
    }

    fn done(&mut self, callback: u32) {
        self.event(callback, 0, &[0]);
        self.event(1, 1, &[callback]);
    }

    fn event(&mut self, id: u32, opcode: u32, args: &[u32]) {
        self.event_bytes(id, opcode, &words(args));
    }

    fn event_bytes(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let _ = write_event(&mut self.stream, id, opcode, body);
    }
}

#[test]
fn lock_surface_denies_tokens_and_replays_latched_caps_lock_on_focus() {
    const TEST_NAME: &str = "lock_surface_denies_tokens_and_replays_latched_caps_lock_on_focus";
    if std::env::var(CLIENT_ENV).as_deref() == Ok("lock") {
        let mut app =
            WaylandApp::new(LockCapsClient::default()).expect("connect to lock compositor");
        app.run().expect("Wayland lock client loop");
        let h = app.handler();
        assert!(
            h.saw_caps_lock_on_claim,
            "claiming a SessionLock surface must immediately replay latched CAPS_LOCK"
        );
        assert!(
            h.lock_token_denied,
            "SessionLock surface must deny external activation token requests"
        );
        return;
    }

    let (_socket, mut child, stream) = spawn_client(TEST_NAME, "lock");
    let mut server = LockCapsCompositor::new(stream);
    server.run();
    let status = child.wait().expect("wait for lock client");
    assert!(status.success(), "lock client exited with {status}");
    assert_eq!(
        server.token_requests, 0,
        "SessionLock surface must emit zero xdg_activation_v1.get_activation_token requests"
    );
}

#[derive(Default)]
struct LockCapsClient {
    claim: Option<SurfaceId>,
    last_grant: Option<InputGrant>,
    saw_caps_lock_on_claim: bool,
    lock_token_denied: bool,
}

impl AppHandler for LockCapsClient {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        cx.request_session_lock().expect("request session lock");
    }

    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        if matches!(reason, WakeReason::SessionLocked)
            && cx.session_lock_state() == SessionLockState::Locked
            && self.claim.is_none()
        {
            let mut attrs = SurfaceAttributes::new("lock-ui");
            attrs.role = SurfaceRole::Lock(MonitorTarget::Name("LOCK-1".into()));
            let surface = cx.create_surface(&attrs).expect("claim lock surface");
            surface.set_visible(true);
            surface
                .set_shell_input_region(&zgui_platform::InputRegion::Full)
                .expect("commit lock claim surface");
            self.claim = Some(surface.id());
        }
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, id: SurfaceId, event: SurfaceEvent) {
        if Some(id) != self.claim {
            return;
        }
        match event {
            SurfaceEvent::ModifiersChanged(mods) => {
                if mods.caps_lock() {
                    self.saw_caps_lock_on_claim = true;
                }
            }
            SurfaceEvent::InputGrant(grant) => {
                self.last_grant = Some(grant);
            }
            SurfaceEvent::Pointer {
                action: PointerAction::Pressed,
                ..
            } => {
                let surface = cx.surface(id).expect("lock claim surface");
                // Even if an InputGrant were fabricated or captured, Lock surfaces must refuse!
                let grant = self.last_grant.take().unwrap_or_else(|| {
                    InputGrant::new(1, id, zgui_platform::InputSerial::new(LOCK_PTR_SERIAL))
                });
                assert!(surface.request_activation_token(grant, None).is_none());
                self.lock_token_denied = true;
                cx.try_unlock_session().expect("unlock session");
                cx.request_exit();
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum LockObj {
    Registry,
    Compositor,
    Subcompositor,
    Subsurface,
    Shm,
    Pool,
    Buffer,
    XdgShell,
    LockManager,
    Lock,
    LockSurface,
    Output,
    Seat,
    Pointer,
    Keyboard,
    Activation,
    Surface,
    Region,
}

struct LockCapsCompositor {
    stream: UnixStream,
    descriptors: VecDeque<OwnedFd>,
    objects: HashMap<u32, LockObj>,
    output_id: Option<u32>,
    lock_id: Option<u32>,
    fallback_wl: Option<u32>,
    claim_wl: Option<u32>,
    pointer: Option<u32>,
    keyboard: Option<u32>,
    sent_lock_focused: bool,
    sent_press: bool,
    token_requests: u32,
}

impl LockCapsCompositor {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            descriptors: VecDeque::new(),
            objects: HashMap::new(),
            output_id: None,
            lock_id: None,
            fallback_wl: None,
            claim_wl: None,
            pointer: None,
            keyboard: None,
            sent_lock_focused: false,
            sent_press: false,
            token_requests: 0,
        }
    }

    fn read(&mut self, mut bytes: &mut [u8]) -> bool {
        while !bytes.is_empty() {
            let mut storage = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
            let mut ancillary = RecvAncillaryBuffer::new(&mut storage);
            let message = match recvmsg(
                &self.stream,
                &mut [IoSliceMut::new(bytes)],
                &mut ancillary,
                RecvFlags::empty(),
            ) {
                Ok(msg) => msg,
                Err(rustix::io::Errno::CONNRESET) => return false,
                Err(err) => panic!("read lock Wayland request: {err}"),
            };
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
            let [id, desc] = [word(&header, 0), word(&header, 1)];
            let mut body = vec![0; (desc >> 16) as usize - 8];
            assert!(self.read(&mut body), "complete lock request body");
            self.request(id, desc & 0xffff, &body);
        }
    }

    fn request(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let arg = |offset| word(body, offset);
        if id == 1 {
            match opcode {
                0 => {
                    let cb = arg(0);
                    let _ = write_event(&mut self.stream, cb, 0, &words(&[0]));
                    let _ = write_event(&mut self.stream, 1, 1, &words(&[cb]));
                }
                1 => {
                    let reg = arg(0);
                    self.objects.insert(reg, LockObj::Registry);
                    for (name, interface, version) in [
                        (1, "wl_compositor", 6),
                        (2, "wl_shm", 1),
                        (3, "xdg_wm_base", 6),
                        (4, "ext_session_lock_manager_v1", 1),
                        (5, "wl_subcompositor", 1),
                        (6, "wl_output", 4),
                        (7, "wl_seat", 7),
                        (8, "xdg_activation_v1", 1),
                    ] {
                        let mut args = words(&[name]);
                        args.extend(string(interface));
                        args.extend(words(&[version]));
                        let _ = write_event(&mut self.stream, reg, 0, &args);
                    }
                }
                _ => panic!("unexpected display opcode {opcode}"),
            }
            return;
        }
        let Some(&obj) = self.objects.get(&id) else {
            return;
        };
        match (obj, opcode) {
            (LockObj::Registry, 0) => {
                let bound = arg(3 + (arg(1) as usize).div_ceil(4));
                let kind = match arg(0) {
                    1 => LockObj::Compositor,
                    2 => LockObj::Shm,
                    3 => LockObj::XdgShell,
                    4 => LockObj::LockManager,
                    5 => LockObj::Subcompositor,
                    6 => {
                        self.output_id = Some(bound);
                        // wl_output.geometry, mode, name("LOCK-1"), done
                        let mut geom = words(&[0, 0, 300, 200, 0]);
                        geom.extend(string("test"));
                        geom.extend(string("model"));
                        geom.extend(words(&[0]));
                        let _ = write_event(&mut self.stream, bound, 0, &geom);
                        let _ = write_event(&mut self.stream, bound, 1, &words(&[3, 64, 48, 60_000]));
                        let _ = write_event(&mut self.stream, bound, 4, &string("LOCK-1"));
                        let _ = write_event(&mut self.stream, bound, 2, &[]);
                        LockObj::Output
                    }
                    7 => {
                        let _ = write_event(&mut self.stream, bound, 1, &string("seat0"));
                        // Pointer (1) | Keyboard (2) = 3
                        let _ = write_event(&mut self.stream, bound, 0, &words(&[3]));
                        LockObj::Seat
                    }
                    8 => LockObj::Activation,
                    n => panic!("unexpected global {n}"),
                };
                self.objects.insert(bound, kind);
            }
            (LockObj::Compositor, 0) => {
                self.objects.insert(arg(0), LockObj::Surface);
            }
            (LockObj::Compositor, 1) => {
                self.objects.insert(arg(0), LockObj::Region);
            }
            (LockObj::Shm, 0) => {
                let _ = self.descriptors.pop_front();
                self.objects.insert(arg(0), LockObj::Pool);
            }
            (LockObj::Pool, 0) => {
                self.objects.insert(arg(0), LockObj::Buffer);
            }
            (LockObj::Seat, 0) => {
                self.pointer = Some(arg(0));
                self.objects.insert(arg(0), LockObj::Pointer);
            }
            (LockObj::Seat, 1) => {
                let kbd = arg(0);
                self.keyboard = Some(kbd);
                self.objects.insert(kbd, LockObj::Keyboard);
                send_keymap(&mut self.stream, kbd);
                // Latch CAPS_LOCK BEFORE any lock surface exists or has focus!
                let _ = write_event(
                    &mut self.stream,
                    kbd,
                    4,
                    &words(&[1, 0, 0, CAPS_LOCK_MASK, 0]),
                );
            }
            (LockObj::LockManager, 1) => {
                self.lock_id = Some(arg(0));
                self.objects.insert(arg(0), LockObj::Lock);
            }
            (LockObj::Lock, 1) => {
                let lock_surf = arg(0);
                self.fallback_wl = Some(arg(1));
                self.objects.insert(lock_surf, LockObj::LockSurface);
                // Configure fallback lock surface (serial=10, 64x48)
                let _ = write_event(&mut self.stream, lock_surf, 0, &words(&[10, 64, 48]));
            }
            (LockObj::Subcompositor, 1) => {
                // get_subsurface(id, surface=claim_wl, parent=fallback_wl)
                self.claim_wl = Some(arg(1));
                self.objects.insert(arg(0), LockObj::Subsurface);
            }
            (LockObj::Surface, 3) => {
                let cb = arg(0);
                let _ = write_event(&mut self.stream, cb, 0, &words(&[0]));
                let _ = write_event(&mut self.stream, 1, 1, &words(&[cb]));
            }
            (LockObj::Surface, 6) => {
                if Some(id) == self.fallback_wl && !self.sent_lock_focused {
                    self.sent_lock_focused = true;
                    let lock_id = self.lock_id.expect("lock object");
                    // Confirm ext_session_lock_v1.locked (event 0) and focus fallback wl_surface with keyboard!
                    let _ = write_event(&mut self.stream, lock_id, 0, &[]);
                    let kbd = self.keyboard.expect("keyboard");
                    let _ = write_event(&mut self.stream, kbd, 1, &words(&[2, id, 0]));
                } else if Some(id) == self.claim_wl && !self.sent_press {
                    // Once claim surface commits, send a pointer press on claim_wl to verify denial!
                    self.sent_press = true;
                    let ptr = self.pointer.expect("pointer");
                    let _ = write_event(
                        &mut self.stream,
                        ptr,
                        0,
                        &words(&[LOCK_PTR_SERIAL - 1, id, 10 << 8, 10 << 8]),
                    );
                    let _ = write_event(
                        &mut self.stream,
                        ptr,
                        3,
                        &words(&[LOCK_PTR_SERIAL, 500, 0x110, 1]),
                    );
                    let _ = write_event(&mut self.stream, ptr, 5, &[]);
                }
            }
            (LockObj::Activation, 1) => {
                self.token_requests += 1;
            }
            _ => {}
        }
    }
}

fn send_keymap(stream: &mut UnixStream, keyboard_id: u32) {
    let path = std::env::temp_dir().join(format!(
        "zgui-act-keymap-{}-{}",
        std::process::id(),
        keyboard_id
    ));
    {
        let mut file = std::fs::File::create(&path).expect("create keymap file");
        file.write_all(KEYMAP.as_bytes()).expect("write keymap");
    }
    let file = std::fs::File::open(&path).expect("reopen keymap ro");
    let _ = std::fs::remove_file(&path);

    let body = words(&[XKB_V1, KEYMAP.len() as u32]);
    let length = u32::try_from(8 + body.len()).expect("keymap header");
    let mut packet = words(&[keyboard_id, length << 16]);
    packet.extend(body);

    let fds = [file.as_fd()];
    let mut cmsg_space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut cmsg_space);
    assert!(ancillary.push(SendAncillaryMessage::ScmRights(&fds)));
    sendmsg(
        stream,
        &[IoSlice::new(&packet)],
        &mut ancillary,
        SendFlags::empty(),
    )
    .expect("send wl_keyboard.keymap with fd");
}

fn write_event(stream: &mut UnixStream, id: u32, opcode: u32, body: &[u8]) -> std::io::Result<()> {
    let length = u32::try_from(8 + body.len()).expect("Wayland packet length");
    let mut packet = words(&[id, (length << 16) | opcode]);
    packet.extend_from_slice(body);
    stream.write_all(&packet)
}

fn word(bytes: &[u8], index: usize) -> u32 {
    u32::from_ne_bytes(bytes[index * 4..][..4].try_into().expect("u32 word"))
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

fn string(value: &str) -> Vec<u8> {
    let len = value.len() + 1;
    let mut out = words(&[u32::try_from(len).expect("string length")]);
    out.extend_from_slice(value.as_bytes());
    out.resize(4 + len.next_multiple_of(4), 0);
    out
}
