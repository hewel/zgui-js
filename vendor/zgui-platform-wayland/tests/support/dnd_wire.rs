//! Private compositor for the inbound and outbound Wayland DnD wire tests.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{IoSliceMut, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, recvmsg};

const TIMEOUT: Duration = Duration::from_secs(5);
const PRESS_1_SERIAL: u32 = 303;
const RELEASE_1_SERIAL: u32 = 304;
const PRESS_2_SERIAL: u32 = 305;
const CONFIGURE_SERIAL: u32 = 40;

const URI: &str = "text/uri-list";
const PAYLOAD: &[u8] = b"file:///tmp/Family%20Photos/\r\nfile:///tmp/notes.txt\r\n";

pub(crate) fn run_client(marker: &str, args: &[&str], outbound_action: Option<u32>) {
    let socket = SocketFile(
        std::env::temp_dir().join(format!("zgui-dnd-{}.sock", std::process::id())),
    );
    let listener = UnixListener::bind(&socket.0).expect("bind private Wayland socket");
    listener.set_nonblocking(true).expect("nonblocking listener");
    let mut child = Command::new(std::env::current_exe().expect("locate test binary"))
        .args(args)
        .env(marker, outbound_action.unwrap_or(0).to_string())
        .env("WAYLAND_DISPLAY", &socket.0)
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .expect("launch Wayland client");
    let start = Instant::now();
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(child.try_wait().expect("poll client").is_none(), "client exited before connecting");
                assert!(start.elapsed() < TIMEOUT, "client did not connect");
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("accept client: {error}"),
        }
    };
    stream.set_read_timeout(Some(TIMEOUT)).expect("read timeout");
    stream.set_write_timeout(Some(TIMEOUT)).expect("write timeout");

    let mut server = Compositor::new(stream, outbound_action);
    server.run();
    let status = child.wait().expect("wait for client");
    assert!(status.success(), "Wayland client exited with {status}");
    if outbound_action.is_some() {
        assert_eq!(server.started_drags.len(), 2);
        for (idx, drag) in server.started_drags.iter().enumerate() {
            assert_eq!(drag.actions, if idx == 0 { 3 } else { 1 });
            assert_eq!(drag.offered_mimes, ["text/plain"]);
            assert_eq!(Some(drag.origin), server.surface_wl);
            assert_eq!(drag.icon, 0);
            assert!(drag.started && drag.destroyed, "drag #{idx} not started and destroyed");
        }
        assert_eq!(server.started_drags[0].serial, PRESS_1_SERIAL);
        assert_eq!(server.started_drags[1].serial, PRESS_2_SERIAL);
    } else {
        assert_eq!(server.offers.len(), 4);
        for (offer, (received, actions, finished)) in server.offers.iter().zip([
            (false, &[][..], false),
            (true, &[(1, 1), (2, 2)][..], true),
            (true, &[(0, 0), (1, 1)][..], true),
            (true, &[(2, 2)][..], false),
        ]) {
            assert_eq!(offer.received, received);
            assert_eq!(offer.actions, actions);
            assert_eq!(offer.finished, finished);
            assert!(offer.destroyed);
        }
    }
}

#[derive(Default)]
struct RecordedOffer {
    actions: Vec<(u32, u32)>,
    received: bool,
    accepted_mime: bool,
    finished: bool,
    destroyed: bool,
}

struct SocketFile(std::path::PathBuf);
impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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
    DataOffer(usize),
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
    fds: VecDeque<OwnedFd>,
    outbound_action: Option<u32>,
    seat: Option<u32>,
    pointer: Option<u32>,
    data_device: Option<u32>,
    surface_wl: Option<u32>,
    configured: bool,
    sent_press_1: bool,
    sent_release_1: bool,
    sent_press_2: bool,
    started_drags: Vec<RecordedDrag>,
    offers: Vec<RecordedOffer>,
}

impl Compositor {
    fn new(stream: UnixStream, outbound_action: Option<u32>) -> Self {
        Self {
            stream,
            objects: HashMap::new(),
            fds: VecDeque::new(),
            outbound_action,
            seat: None,
            pointer: None,
            data_device: None,
            surface_wl: None,
            configured: false,
            sent_press_1: false,
            sent_release_1: false,
            sent_press_2: false,
            started_drags: Vec::new(),
            offers: Vec::new(),
        }
    }

    fn run(&mut self) {
        let mut header = [0; 8];
        loop {
            if !self.read_exact(&mut header) { return; }
            let [id, desc] = [word(&header, 0), word(&header, 1)];
            let mut body = vec![0; (desc >> 16) as usize - 8];
            assert!(self.read_exact(&mut body), "missing Wayland request body");
            self.request(id, desc & 0xffff, &body);
        }
    }

    fn read_exact(&mut self, mut bytes: &mut [u8]) -> bool {
        let total = bytes.len();
        while !bytes.is_empty() {
            let mut storage = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(8))];
            let mut ancillary = RecvAncillaryBuffer::new(&mut storage);
            let message = match recvmsg(
                &self.stream,
                &mut [IoSliceMut::new(bytes)],
                &mut ancillary,
                RecvFlags::empty(),
            ) {
                Ok(message) => message,
                // The child can close with unread compositor events after requesting exit.
                Err(rustix::io::Errno::CONNRESET) if bytes.len() == total => return false,
                Err(error) => panic!("read Wayland request: {error}"),
            };
            if message.bytes == 0 {
                assert_eq!(bytes.len(), total, "truncated Wayland request");
                return false;
            }
            for control in ancillary.drain() {
                if let RecvAncillaryMessage::ScmRights(fds) = control {
                    self.fds.extend(fds);
                }
            }
            bytes = &mut bytes[message.bytes..];
        }
        true
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
                self.maybe_start();
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
                self.maybe_start();
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
                        let action = self.outbound_action.expect("outbound wire case");
                        self.event_bytes(src, 0, &string("text/plain")); // target
                        self.event(src, 5, &[action]); // action
                        self.event(src, 3, &[]); // dnd_drop_performed
                        self.event(src, 4, &[]); // dnd_finished
                    } else {
                        self.event(src, 5, &[1]); // action(Copy), never committed
                        self.event(src, 2, &[]); // cancelled
                    }
                }
            }
            (Object::DataOffer(idx), 0) => {
                let mime_len = arg(1) as usize;
                let accepted = mime_len != 0;
                if accepted {
                    assert_eq!(&body[8..8 + mime_len - 1], URI.as_bytes());
                    assert!(self.offers[idx].received);
                }
                self.offers[idx].accepted_mime = accepted;
            }
            (Object::DataOffer(idx), 1) => {
                assert_ne!(idx, 0, "text-only offer was read");
                assert_eq!(body, string(URI));
                let fd = self.fds.pop_front().expect("receive carries a pipe fd");
                File::from(fd).write_all(PAYLOAD).expect("send URI list");
                self.offers[idx].received = true;
            }
            (Object::DataOffer(idx), 2) => {
                self.offers[idx].destroyed = true;
                if idx < 3 {
                    self.send_offer();
                }
            }
            (Object::DataOffer(idx), 3) => {
                assert!(!self.offers[idx].finished, "duplicate finish");
                self.offers[idx].finished = true;
            }
            (Object::DataOffer(idx), 4) => {
                let (mask, preferred) = (arg(0), arg(1));
                assert_eq!(self.offers[idx].accepted_mime, preferred != 0);
                let attempt = self.offers[idx].actions.len();
                self.offers[idx].actions.push((mask, preferred));
                let device = self.data_device.expect("data device created");
                match (idx, attempt) {
                    (1, 0) => {
                        assert_eq!((mask, preferred), (1, 1));
                        self.event(id, 2, &[1]); // action(Copy)
                        self.event(device, 3, &[110, 120 << 8, 20 << 8]); // motion to Move area
                    }
                    (2, 0) => {
                        assert_eq!((mask, preferred), (0, 0), "Move is not offered");
                        self.event(device, 3, &[120, 20 << 8, 20 << 8]); // motion to Copy area
                    }
                    (1, 1) | (2, 1) | (3, 0) => {
                        self.event(id, 2, &[if idx == 2 { 1 } else { 2 }]); // action
                        self.event(device, 4, &[]); // drop
                        self.event(device, 2, &[]); // leave
                    }
                    _ => panic!("unexpected negotiation for offer {idx}, attempt {attempt}"),
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
                self.maybe_start();
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

    fn maybe_start(&mut self) {
        if self.sent_press_1 || !self.configured || self.data_device.is_none() {
            return;
        }
        if self.outbound_action.is_none() {
            if self.offers.is_empty() { self.send_offer(); }
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

    fn send_offer(&mut self) {
        let idx = self.offers.len();
        let device = self.data_device.expect("data device created");
        let surface = self.surface_wl.expect("surface created");
        let offer = 0xff00_0000 + idx as u32; // compositor-allocated object
        self.offers.push(RecordedOffer::default());
        self.objects.insert(offer, Object::DataOffer(idx));
        self.event(device, 0, &[offer]); // data_offer
        self.event_bytes(offer, 0, &string(if idx == 0 { "text/plain" } else { URI }));
        self.event(offer, 1, &[match idx { 2 => 1, 3 => 2, _ => 3 }]); // source_actions
        let x = if idx < 2 { 20 } else { 120 };
        self.event(device, 1, &[50 + idx as u32, surface, x << 8, 20 << 8, offer]); // enter
        if idx == 0 {
            self.event(device, 2, &[]); // leave without offering a file
        }
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
