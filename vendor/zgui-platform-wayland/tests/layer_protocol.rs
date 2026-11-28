//! A compositor fixture speaking the real Wayland wire protocol on a private Unix socket.

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
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use zgui_geom::{Css, CssPx, Edges, Size};
use zgui_platform::{
    AppHandler, ExclusiveZone, InputRect, InputRegion, KeyboardInteractivity, Layer,
    LayerPlacement, MonitorTarget, PlatformCx, Surface, SurfaceAttributes, SurfaceEvent,
    SurfaceId, SurfaceRole, WakeReason,
};
use zgui_platform_wayland::WaylandApp;

const CLIENT: &str = "ZGUI_LAYER_PROTOCOL_CLIENT";
const TIMEOUT: Duration = Duration::from_secs(5);
const FIRST: u32 = 5;
const SECOND: u32 = 6;
const RECTANGLES: [[i32; 4]; 2] = [[-2, 3, 80, 12], [104, 8, 13, 20]];

#[test]
fn live_layer_changes_input_regions_and_output_hotplug() {
    if std::env::var_os(CLIENT).is_some() {
        let mut app = WaylandApp::new(Bar::default()).expect("connect to the protocol fixture");
        app.run().expect("Wayland client loop");
        let bar = app.handler();
        assert!(
            bar.updated,
            "the first configure must trigger live layer and input updates"
        );
        assert_eq!(bar.hotplug, 3, "add, geometry change, and removal");
        return;
    }

    let path = std::env::temp_dir().join(format!("zgui-layer-{}.sock", std::process::id()));
    let listener = UnixListener::bind(&path).expect("bind a private Wayland socket");
    let _socket = SocketFile(path.clone());
    listener.set_nonblocking(true).expect("bound the accept wait");
    let mut child = Command::new(std::env::current_exe().expect("locate this test binary"))
        .args(["--exact", "live_layer_changes_input_regions_and_output_hotplug"])
        .env(CLIENT, "1")
        .env("WAYLAND_DISPLAY", path)
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .expect("launch the actual Wayland backend as a client");
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
    assert!(
        status.success(),
        "the Wayland protocol client exited with {status}"
    );
    assert_eq!(server.layers.len(), 2, "edge changes must not recreate the primary role");
    assert_eq!(
        server.layer_commits,
        [
            (13, 0, 32, 2, 32, [0, 0, 0, 0], 0),
            (11, 44, 0, 3, 17, [7, -2, 9, 1], 2),
            (13, 0, 32, 2, 32, [0, 0, 0, 0], 0),
        ],
        "the first bar commits top, right, then top placement on the same layer role"
    );
    assert_eq!(
        server.input_commits,
        [Some(RECTANGLES.to_vec()), Some(Vec::new()), None],
        "each input change commits immediately: rectangle union, empty, then default"
    );
    let committed = server.input_commits[0]
        .as_deref()
        .expect("committed rectangle union");
    let hits = |x, y| {
        committed.iter().any(|&[left, top, width, height]| {
            (left..left + width).contains(&x) && (top..top + height).contains(&y)
        })
    };
    assert!(hits(10, 10), "(10, 10) in the first rectangle must hit the bar");
    assert!(
        hits(110, 10),
        "(110, 10) in the second rectangle must hit the bar"
    );
    assert!(
        !hits(90, 10),
        "(90, 10) in the gap between rectangles must pass through"
    );
    assert!(
        !hits(10, 16),
        "(10, 16) below the first rectangle must pass through"
    );
    assert_eq!(server.output_binds, [FIRST, SECOND], "one layer on each named output");
    assert_eq!(
        server.layer_namespaces,
        ["dev.example.Shell", "z-shell-bar"],
        "unnamed layer falls back to application_id while explicit layer namespace overrides it"
    );
}

struct SocketFile(std::path::PathBuf);

impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Default)]
struct Bar {
    primary: Option<Arc<dyn Surface>>,
    second: Option<SurfaceId>,
    updated: bool,
    hotplug: u8,
}

fn horizontal(output: &str) -> LayerPlacement {
    LayerPlacement::default()
        .with_anchors(Edges { top: true, right: true, bottom: false, left: true })
        .with_exclusive_zone(ExclusiveZone::Reserve(CssPx(32.0)))
        .with_monitor(MonitorTarget::Name(output.to_owned()))
}

fn size(width: f32, height: f32) -> Size<CssPx, Css> {
    Size::new(CssPx(width), CssPx(height))
}

fn open(cx: &dyn PlatformCx, output: &str, namespace: Option<&str>) -> Arc<dyn Surface> {
    let mut placement = horizontal(output);
    if let Some(ns) = namespace {
        placement = placement.with_namespace(ns);
    }
    let mut attrs = SurfaceAttributes::default().with_application_id("dev.example.Shell");
    attrs.role = SurfaceRole::Layer(placement);
    attrs.size = Some(size(0.0, 32.0));
    cx.create_surface(&attrs).expect("open a bar on a named output")
}

impl AppHandler for Bar {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        assert_eq!(cx.monitors().len(), 1, "initial registry is complete before startup");
        assert_eq!(
            cx.monitors()[0].name.as_deref(),
            Some("TEST-1"),
            "the initial output name must be available before opening the bar"
        );
        self.primary = Some(open(cx, "TEST-1", None));
    }

    fn surface_event(&mut self, _cx: &dyn PlatformCx, id: SurfaceId, event: SurfaceEvent) {
        let primary = self.primary.as_ref().expect("first bar exists");
        if self.updated || id != primary.id() || !matches!(event, SurfaceEvent::Resized(_)) {
            return;
        }
        let vertical = LayerPlacement::new(Layer::Overlay)
            .with_anchors(Edges { top: true, right: true, bottom: true, left: false })
            .with_margin(Edges { top: CssPx(7.0), right: CssPx(-2.0), bottom: CssPx(9.0), left: CssPx(1.0) })
            .with_exclusive_zone(ExclusiveZone::Reserve(CssPx(17.0)))
            .with_keyboard(KeyboardInteractivity::OnDemand)
            .with_monitor(MonitorTarget::Name("TEST-1".to_owned()));
        let signed_overflow = i32::MAX as f32;
        for (width, height) in [(signed_overflow, 32.0), (32.0, signed_overflow)] {
            assert!(
                primary.request_size(size(width, height)).is_none(),
                "request_size must reject ({width}, {height}) outside the signed extent range"
            );
        }
        let unsigned_overflow = u32::MAX as f32;
        for (width, height) in [(unsigned_overflow, 32.0), (32.0, unsigned_overflow)] {
            assert!(
                primary.reconfigure_layer(&vertical, size(width, height)).is_err(),
                "layer reconfiguration must reject ({width}, {height}) outside the unsigned extent range"
            );
        }
        primary.reconfigure_layer(&vertical, size(44.0, 0.0)).expect("flip to the right edge");
        primary.reconfigure_layer(&horizontal("TEST-1"), size(0.0, 32.0)).expect("flip back to the top edge");
        assert!(primary.reconfigure_layer(&horizontal("TEST-2"), size(0.0, 32.0)).is_err(), "an existing surface cannot move to another output");
        assert!(
            primary
                .reconfigure_layer(&horizontal("TEST-1").with_namespace("changed"), size(0.0, 32.0))
                .is_err(),
            "an existing surface cannot change its layer namespace"
        );
        primary.set_shell_input_region(&InputRegion::Rects(RECTANGLES.map(|[x, y, width, height]| InputRect { x, y, width, height }).to_vec())).expect("commit two hit rectangles");
        primary.set_shell_input_region(&InputRegion::Empty).expect("commit pointer passthrough");
        primary.set_shell_input_region(&InputRegion::Full).expect("restore compositor-default input");
        self.updated = true;
    }

    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        if !matches!(reason, WakeReason::MonitorsChanged) {
            return;
        }
        let monitors = cx.monitors();
        let second = monitors.iter().find(|monitor| monitor.name.as_deref() == Some("TEST-2"));
        match (self.hotplug, second) {
            (0, Some(monitor)) => {
                assert_eq!(monitor.position.x.0, 1920.0, "TEST-2 starts beside TEST-1");
                assert_eq!(
                    monitor.size.width.0, 1280.0,
                    "TEST-2 publishes its initial mode width"
                );
                self.second = Some(open(cx, "TEST-2", Some("z-shell-bar")).id());
            }
            (1, Some(monitor)) => {
                assert_eq!(
                    monitor.position.x.0, 2000.0,
                    "TEST-2 publishes its new position"
                );
                let removed = self.second.take().expect("the auxiliary bar exists");
                cx.destroy_surface(removed);
                assert!(
                    cx.surface(removed).is_none(),
                    "destroying the auxiliary bar removes its ID"
                );
            }
            (2, None) => {
                assert_eq!(
                    monitors.len(), 1,
                    "removing TEST-2 leaves only the initial output"
                );
                cx.request_exit();
            }
            _ => panic!("unexpected hotplug state {}: {monitors:?}", self.hotplug),
        }
        self.hotplug += 1;
    }
}

#[derive(Clone, Copy, Debug)]
enum Object {
    Registry,
    Compositor,
    Shm,
    XdgShell,
    LayerShell,
    Output(u32),
    Surface,
    Layer(usize),
    Region,
}

// Layer state committed by a wl_surface, not merely requested on its layer-role object.
type LayerState = (u32, u32, u32, u32, i32, [i32; 4], u32);

struct LayerObject {
    id: u32,
    surface: u32,
    global: u32,
    state: LayerState,
    changed: bool,
    configured: bool,
    transitional: bool,
}

struct Compositor {
    stream: UnixStream,
    objects: HashMap<u32, Object>,
    registry: u32,
    regions: HashMap<u32, Vec<[i32; 4]>>,
    pending_input: HashMap<u32, Option<Vec<[i32; 4]>>>,
    layers: Vec<LayerObject>,
    layer_commits: Vec<LayerState>,
    input_commits: Vec<Option<Vec<[i32; 4]>>>,
    output_binds: Vec<u32>,
    layer_namespaces: Vec<String>,
}

impl Compositor {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            objects: HashMap::new(),
            registry: 0,
            regions: HashMap::new(),
            pending_input: HashMap::new(),
            layers: Vec::new(),
            layer_commits: Vec::new(),
            input_commits: Vec::new(),
            output_binds: Vec::new(),
            layer_namespaces: Vec::new(),
        }
    }

    fn run(&mut self) {
        loop {
            let mut header = [0; 8];
            match self.stream.read_exact(&mut header) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return,
                Err(error) => panic!("reading a Wayland request: {error}"),
            }
            let id = word(&header, 0);
            let descriptor = word(&header, 1);
            let opcode = descriptor & 0xffff;
            let mut body = vec![0; (descriptor >> 16) as usize - 8];
            self.stream.read_exact(&mut body).expect("Wayland request body");
            self.request(id, opcode, &body);
        }
    }

    fn request(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let arg = |offset| word(body, offset);
        if id == 1 {
            match opcode {
                0 => {
                    self.event(arg(0), 0, &words(&[0])); // wl_callback.done
                    self.event(1, 1, &words(&[arg(0)])); // wl_display.delete_id
                }
                1 => {
                    self.registry = arg(0);
                    self.objects.insert(self.registry, Object::Registry);
                    self.global(1, "wl_compositor", 6);
                    self.global(2, "wl_shm", 1);
                    self.global(3, "xdg_wm_base", 6);
                    self.global(4, "zwlr_layer_shell_v1", 4);
                    self.global(FIRST, "wl_output", 4);
                }
                _ => panic!("unexpected display opcode {opcode}"),
            }
            return;
        }
        match (self.objects[&id], opcode) {
            (Object::Registry, 0) => {
                let global = arg(0);
                let version_index = 2 + (arg(1) as usize).div_ceil(4);
                let bound = arg(version_index + 1);
                let object = match global {
                    1 => Object::Compositor,
                    2 => Object::Shm,
                    3 => Object::XdgShell,
                    4 => Object::LayerShell,
                    FIRST | SECOND => Object::Output(global),
                    _ => panic!("unexpected global {global}"),
                };
                self.objects.insert(bound, object);
                if let Object::Output(global) = object {
                    let (name, x, width) = if global == FIRST { ("TEST-1", 0, 1920) } else { ("TEST-2", 1920, 1280) };
                    self.geometry(bound, x);
                    self.event(bound, 1, &words(&[3, width, 1080, 60_000]));
                    self.event(bound, 3, &words(&[1]));
                    self.event(bound, 4, &string(name));
                    self.event(bound, 2, &[]); // wl_output.done
                }
            }
            (Object::Compositor, 0) => { self.objects.insert(arg(0), Object::Surface); }
            (Object::Compositor, 1) => {
                self.objects.insert(arg(0), Object::Region);
                self.regions.insert(arg(0), Vec::new());
            }
            (Object::LayerShell, 0) => {
                let Object::Output(global) = self.objects[&arg(2)] else { panic!("layer must be bound to the requested output"); };
                let ns_len = arg(4) as usize;
                assert!(ns_len >= 1, "Wayland string length includes NUL terminator");
                let ns = std::str::from_utf8(&body[20..20 + ns_len - 1])
                    .expect("valid UTF-8 layer namespace")
                    .to_owned();
                self.layer_namespaces.push(ns);
                self.output_binds.push(global);
                self.objects.insert(arg(0), Object::Layer(self.layers.len()));
                self.layers.push(LayerObject {
                    id: arg(0), surface: arg(1), global,
                    state: (0, 0, 0, arg(3), 0, [0; 4], 0),
                    changed: true, configured: false, transitional: false,
                });
            }
            (Object::Layer(index), 0..=4 | 8) => {
                let layer = &mut self.layers[index];
                let (anchor, width, height, level, zone, margin, keyboard) = &mut layer.state;
                match opcode {
                    0 => {
                        (*width, *height) = (arg(0), arg(1));
                        layer.transitional = *width > 0 && *height > 0;
                    }
                    1 => {
                        let next = arg(0);
                        let loses_stretch = (*anchor & 12 == 12 && next & 12 != 12 && *width == 0)
                            || (*anchor & 3 == 3 && next & 3 != 3 && *height == 0);
                        assert!(
                            !loses_stretch,
                            "layer {id}: set a positive transitional size before changing anchors \
                             from {anchor:#06b} to {next:#06b} at {width} by {height}"
                        );
                        *anchor = next;
                    }
                    2 => *zone = arg(0) as i32,
                    3 => *margin = [0, 1, 2, 3].map(|offset| arg(offset) as i32),
                    4 => *keyboard = arg(0),
                    8 => *level = arg(0),
                    _ => unreachable!(),
                }
                layer.changed = true;
            }
            (Object::Layer(_), 6) => {} // ack_configure
            (Object::Layer(index), 7) => {
                if self.layers[index].global == SECOND {
                    self.event(self.registry, 1, &words(&[SECOND])); // wl_registry.global_remove
                }
            }
            (Object::Region, 0) => { self.regions.remove(&id); }
            (Object::Region, 1) => {
                self.regions.get_mut(&id).expect("region exists").push([0, 1, 2, 3].map(|offset| arg(offset) as i32));
            }
            (Object::Surface, 5) => {
                let region = (arg(0) != 0).then(|| self.regions[&arg(0)].clone());
                self.pending_input.insert(id, region);
            }
            (Object::Surface, 6) => self.commit(id),
            (Object::Surface, 0 | 3 | 8) | (Object::Output(_), 0) | (Object::LayerShell, 1) | (Object::XdgShell, 0) => {},
            (object, _) => panic!("unexpected {object:?} opcode {opcode}"),
        }
    }

    fn commit(&mut self, id: u32) {
        if let Some(input) = self.pending_input.remove(&id) {
            self.input_commits.push(input);
            if self.input_commits.len() == 3 {
                self.global(SECOND, "wl_output", 4);
            }
        }
        let Some(layer) = self.layers.iter_mut().find(|layer| layer.surface == id) else { return; };
        let (anchor, width, height, ..) = layer.state;
        assert!(
            width > 0 || anchor & 12 == 12,
            "surface {id} committed zero width without opposite horizontal anchors: {anchor:#06b}"
        );
        assert!(
            height > 0 || anchor & 3 == 3,
            "surface {id} committed zero height without opposite vertical anchors: {anchor:#06b}"
        );
        if layer.global == FIRST && layer.changed && !layer.transitional {
            self.layer_commits.push(layer.state);
        }
        layer.changed = false;
        if !layer.configured {
            layer.configured = true;
            let (role, is_second) = (layer.id, layer.global == SECOND);
            self.event(role, 0, &words(&[10, 640, 32])); // zwlr_layer_surface_v1.configure
            if is_second {
                let output = self.objects.iter().find_map(|(&id, object)| matches!(object, Object::Output(SECOND)).then_some(id)).expect("second output is bound");
                self.geometry(output, 2000);
                self.event(output, 2, &[]);
            }
        }
    }

    fn geometry(&mut self, output: u32, x: u32) {
        let mut args = words(&[x, 0, 300, 200, 0]);
        args.extend(string("Test"));
        args.extend(string("Monitor"));
        args.extend(words(&[0]));
        self.event(output, 0, &args); // wl_output.geometry
    }

    fn global(&mut self, name: u32, interface: &str, version: u32) {
        let mut args = words(&[name]);
        args.extend(string(interface));
        args.extend(words(&[version]));
        self.event(self.registry, 0, &args); // wl_registry.global
    }

    fn event(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let length = u32::try_from(8 + body.len()).expect("small Wayland message");
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
    let mut bytes = words(&[u32::try_from(length).expect("short protocol string")]);
    bytes.extend(value.as_bytes());
    bytes.resize(4 + length.next_multiple_of(4), 0);
    bytes
}
