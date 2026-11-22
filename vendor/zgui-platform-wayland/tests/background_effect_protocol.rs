//! Wire-level `ext_background_effect_v1` protocol tests over a private Unix socket.

#![cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use zgui_geom::{Css, CssPx, Point, Rect, Size};
use zgui_platform::{
    AppHandler, BlurRegion, LayerPlacement, PlatformCx, PopupPlacement, Surface, SurfaceAttributes,
    SurfaceEvent, SurfaceId, SurfaceRole, WakeReason,
};
use zgui_platform_wayland::WaylandApp;

const CLIENT: &str = "ZGUI_BACKGROUND_EFFECT_PROTOCOL_CLIENT";
const TEST: &str = "background_effect_protocol_on_the_wire";
const TIMEOUT: Duration = Duration::from_secs(5);

const GLOBAL_COMPOSITOR: u32 = 1;
const GLOBAL_SHM: u32 = 2;
const GLOBAL_XDG: u32 = 3;
const GLOBAL_LAYER: u32 = 4;
const GLOBAL_OUTPUT: u32 = 5;
const GLOBAL_EFFECT_1: u32 = 6;
const GLOBAL_EFFECT_2: u32 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    ManagerWithBlur,
    AbsentManager,
    ZeroFlagsManager,
    DynamicCapabilitiesAndRecreation,
    EarlyPopupRetirement,
}

impl Scenario {
    const ALL: [Self; 5] = [
        Self::ManagerWithBlur,
        Self::AbsentManager,
        Self::ZeroFlagsManager,
        Self::DynamicCapabilitiesAndRecreation,
        Self::EarlyPopupRetirement,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::ManagerWithBlur => "manager-with-blur",
            Self::AbsentManager => "absent-manager",
            Self::ZeroFlagsManager => "zero-flags-manager",
            Self::DynamicCapabilitiesAndRecreation => "dynamic-capabilities-and-recreation",
            Self::EarlyPopupRetirement => "early-popup-retirement",
        }
    }
}

#[test]
fn background_effect_protocol_on_the_wire() {
    if let Ok(client) = std::env::var(CLIENT) {
        let scenario = Scenario::ALL
            .into_iter()
            .find(|s| s.name() == client)
            .expect("known scenario");
        let mut app =
            WaylandApp::new(Client::new(scenario)).expect("connect to the protocol fixture");
        app.run().expect("Wayland client loop");
        assert!(app.handler().done, "{scenario:?} client did not finish");
        return;
    }

    for scenario in Scenario::ALL {
        run_scenario(scenario);
    }
}

fn run_scenario(scenario: Scenario) {
    let path = std::env::temp_dir().join(format!(
        "zgui-blur-{}-{}.sock",
        std::process::id(),
        scenario.name()
    ));
    let listener = UnixListener::bind(&path).expect("bind private Wayland socket");
    let _socket = SocketFile(path.clone());
    listener.set_nonblocking(true).expect("nonblocking accept");
    let mut child = Command::new(std::env::current_exe().expect("locate test binary"))
        .args(["--exact", TEST])
        .env(CLIENT, scenario.name())
        .env("WAYLAND_DISPLAY", &path)
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .expect("launch Wayland backend client");

    let start = Instant::now();
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    child.try_wait().expect("poll client").is_none(),
                    "{scenario:?}: client exited before connecting"
                );
                assert!(start.elapsed() < TIMEOUT, "{scenario:?}: client timed out");
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("accepting client for {scenario:?}: {error}"),
        }
    };
    stream.set_read_timeout(Some(TIMEOUT)).expect("read timeout");
    stream.set_write_timeout(Some(TIMEOUT)).expect("write timeout");

    let mut server = Compositor::new(stream, scenario);
    server.run();

    let status = child.wait().expect("wait for client");
    assert!(status.success(), "{scenario:?}: client exited with {status}");
    server.verify();
}

struct SocketFile(std::path::PathBuf);
impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect<CssPx, Css> {
    Rect::new(Point::new(CssPx(x), CssPx(y)), Size::new(CssPx(w), CssPx(h)))
}

fn open_layer(cx: &dyn PlatformCx, width: f32, height: f32) -> Arc<dyn Surface> {
    let mut attrs = SurfaceAttributes::default();
    attrs.role = SurfaceRole::Layer(LayerPlacement::default());
    attrs.size = Some(Size::new(CssPx(width), CssPx(height)));
    cx.create_surface(&attrs).expect("create layer surface")
}

fn open_toplevel(cx: &dyn PlatformCx, width: f32, height: f32) -> Arc<dyn Surface> {
    let mut attrs = SurfaceAttributes::new("blur-toplevel");
    attrs.size = Some(Size::new(CssPx(width), CssPx(height)));
    cx.create_surface(&attrs).expect("create toplevel surface")
}

fn open_popup(cx: &dyn PlatformCx, parent: SurfaceId) -> Arc<dyn Surface> {
    let placement = PopupPlacement::below(parent, rect(4.0, 4.0, 20.0, 20.0));
    let mut attrs = SurfaceAttributes::popup(placement);
    attrs.size = Some(Size::new(CssPx(120.0), CssPx(80.0)));
    cx.create_surface(&attrs).expect("create popup surface")
}

struct Client {
    scenario: Scenario,
    primary: Option<Arc<dyn Surface>>,
    secondary: Option<Arc<dyn Surface>>,
    popup1: Option<Arc<dyn Surface>>,
    popup2: Option<Arc<dyn Surface>>,
    stage: u32,
    done: bool,
}

impl Client {
    fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            primary: None,
            secondary: None,
            popup1: None,
            popup2: None,
            stage: 0,
            done: false,
        }
    }
}

impl AppHandler for Client {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        match self.scenario {
            Scenario::ManagerWithBlur => {
                assert!(
                    cx.capabilities().supports_background_blur,
                    "blur capability must be true after manager advertises Capability::Blur"
                );
                let layer = open_layer(cx, 200.0, 40.0);
                // Stage blur BEFORE initial configure: must NOT commit prematurely!
                layer.set_blur_region(BlurRegion::Rects(&[
                    rect(-2.4, 3.2, 10.1, 8.7),
                    rect(40.0, 5.0, 20.0, 12.0),
                    rect(1.0, 1.0, 0.0, 5.0),
                    rect(f32::NAN, 0.0, 10.0, 10.0),
                ]));
                let toplevel = open_toplevel(cx, 320.0, 240.0);
                self.primary = Some(layer);
                self.secondary = Some(toplevel);
            }
            Scenario::AbsentManager | Scenario::ZeroFlagsManager => {
                assert!(
                    !cx.capabilities().supports_background_blur,
                    "blur capability must be false when manager is absent or advertises 0 flags"
                );
                let layer = open_layer(cx, 200.0, 40.0);
                layer.set_blur_region(BlurRegion::Full);
                self.primary = Some(layer);
            }
            Scenario::DynamicCapabilitiesAndRecreation => {
                assert!(
                    !cx.capabilities().supports_background_blur,
                    "blur capability starts false even when manager is bound before capabilities"
                );
                let layer = open_layer(cx, 200.0, 40.0);
                layer.set_blur_region(BlurRegion::Rects(&[rect(5.0, 5.0, 50.0, 20.0)]));
                self.primary = Some(layer);
            }
            Scenario::EarlyPopupRetirement => {
                assert!(cx.capabilities().supports_background_blur);
                let layer = open_layer(cx, 200.0, 40.0);
                self.primary = Some(layer);
            }
        }
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, id: SurfaceId, event: SurfaceEvent) {
        match self.scenario {
            Scenario::ManagerWithBlur => {
                let primary = self.primary.as_ref().expect("primary layer");
                let secondary = self.secondary.as_ref().expect("secondary toplevel");
                if id == primary.id() {
                    if let SurfaceEvent::Resized(size) = event {
                        if self.stage == 2 {
                            // Compositor sent a resize while Full is active: Full must survive resize!
                            assert_eq!(size.width.0, 400.0);
                            self.stage = 3;
                            // Clear blur on idle surface -> must send set_blur_region(None) + commit!
                            primary.set_blur_region(BlurRegion::None);
                        }
                    } else if matches!(event, SurfaceEvent::RedrawRequested) {
                        match self.stage {
                            0 => {
                                // First redraw commits the pre-configured Rects blur via finish_redraw.
                                self.stage = 1;
                            }
                            1 => {
                                // Triggered after compositor answers frame callback & sends wake/resize.
                            }
                            3 => {
                                // Clear commit happens in finish_redraw; next stage blurs secondary.
                                self.stage = 4;
                            }
                            _ => {}
                        }
                    }
                } else if id == secondary.id() && matches!(event, SurfaceEvent::RedrawRequested) {
                    if self.stage == 5 {
                        self.stage = 6;
                    }
                }
            }
            Scenario::AbsentManager | Scenario::ZeroFlagsManager => {
                if matches!(event, SurfaceEvent::RedrawRequested) {
                    assert!(!cx.capabilities().supports_background_blur);
                    self.done = true;
                    cx.request_exit();
                }
            }
            Scenario::DynamicCapabilitiesAndRecreation => {
                if matches!(event, SurfaceEvent::RedrawRequested) {
                    let primary_id = self.primary.as_ref().map(|s| s.id());
                    let secondary_id = self.secondary.as_ref().map(|s| s.id());
                    if Some(id) == primary_id {
                        match self.stage {
                            0 => {
                                // Initial frame while blur is unavailable.
                                assert!(!cx.capabilities().supports_background_blur);
                                self.stage = 1;
                            }
                            1 => {
                                // Delayed grant arrived and scheduled redraw!
                                assert!(cx.capabilities().supports_background_blur);
                                self.stage = 2;
                            }
                            2 => {
                                // Withdrawal arrived and scheduled legal commit!
                                assert!(!cx.capabilities().supports_background_blur);
                                // Change intent while unavailable -> must be retained for regain!
                                self.primary
                                    .as_ref()
                                    .expect("primary")
                                    .set_blur_region(BlurRegion::Full);
                                self.stage = 3;
                            }
                            3 => {
                                // Regain on rebound manager arrived!
                                assert!(cx.capabilities().supports_background_blur);
                                self.stage = 4;
                            }
                            _ => {}
                        }
                    } else if Some(id) == secondary_id && self.stage == 5 {
                        self.stage = 6;
                    }
                }
            }
            Scenario::EarlyPopupRetirement => {
                let primary_id = self.primary.as_ref().expect("parent").id();
                if id == primary_id && matches!(event, SurfaceEvent::RedrawRequested) && self.stage == 0 {
                    // Mark parent mapped so popups may draw/commit.
                    self.primary.as_ref().expect("parent").pre_present_notify();
                    let p1 = open_popup(cx, primary_id);
                    p1.set_blur_region(BlurRegion::Full);
                    self.popup1 = Some(p1);
                    self.stage = 1;
                } else if Some(id) == self.popup1.as_ref().map(|s| s.id())
                    && matches!(event, SurfaceEvent::RedrawRequested)
                    && self.stage == 1
                {
                    // popup1 commits its blur on finish_redraw; next wake retires popup1 and opens popup2.
                    self.stage = 2;
                } else if Some(id) == self.popup2.as_ref().map(|s| s.id())
                    && matches!(event, SurfaceEvent::RedrawRequested)
                    && self.stage == 3
                {
                    self.stage = 4;
                }
            }
        }
    }

    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        if !matches!(reason, WakeReason::MonitorsChanged) {
            return;
        }
        match self.scenario {
            Scenario::ManagerWithBlur => {
                match self.stage {
                    1 => {
                        // Compositor nudged us via output update once surface is idle after Rects commit.
                        self.stage = 2;
                        self.primary
                            .as_ref()
                            .expect("primary")
                            .set_blur_region(BlurRegion::Full);
                    }
                    4 => {
                        // Now blur secondary toplevel to prove two-surface isolation.
                        self.stage = 5;
                        self.secondary
                            .as_ref()
                            .expect("secondary")
                            .set_blur_region(BlurRegion::Rects(&[rect(1.0, 2.0, 30.0, 40.0)]));
                    }
                    6 => {
                        self.done = true;
                        cx.request_exit();
                    }
                    _ => {}
                }
            }
            Scenario::DynamicCapabilitiesAndRecreation => {
                if self.stage == 4 {
                    // Recreate surface: destroy S1, open S2 with fresh protocol identity.
                    let old_id = self.primary.take().expect("old surface").id();
                    cx.destroy_surface(old_id);
                    let fresh = open_layer(cx, 180.0, 36.0);
                    fresh.set_blur_region(BlurRegion::Rects(&[rect(2.0, 4.0, 40.0, 10.0)]));
                    self.secondary = Some(fresh);
                    self.stage = 5;
                } else if self.stage == 6 {
                    self.done = true;
                    cx.request_exit();
                }
            }
            Scenario::EarlyPopupRetirement => {
                if self.stage == 2 {
                    let parent_id = self.primary.as_ref().expect("parent").id();
                    let p1 = self.popup1.take().expect("popup1");
                    // Keep Arc<dyn Surface> alive until after popup2 opens to force early role retirement
                    // (Live::release_retiring_popups) rather than waiting for Drop!
                    cx.destroy_surface(p1.id());
                    let p2 = open_popup(cx, parent_id);
                    p2.set_blur_region(BlurRegion::Rects(&[rect(0.0, 0.0, 60.0, 30.0)]));
                    self.popup2 = Some(p2);
                    drop(p1);
                    self.stage = 3;
                } else if self.stage == 4 {
                    self.done = true;
                    cx.request_exit();
                }
            }
            _ => {}
        }
    }

    fn shutting_down(&mut self, _cx: &dyn PlatformCx) {
        self.popup2 = None;
        self.popup1 = None;
        self.secondary = None;
        self.primary = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Object {
    Registry,
    Compositor,
    Shm,
    XdgShell,
    LayerShell,
    Output,
    EffectManager(u32), // global name
    Surface,
    Region,
    Layer(u32),       // wl_surface id
    XdgSurface(u32),  // wl_surface id
    XdgToplevel(u32), // wl_surface id
    XdgPopup(u32),    // wl_surface id
    XdgPositioner,
    EffectSurface(u32), // wl_surface id
    FrameCallback(u32), // wl_surface id
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CommittedBlur {
    None,
    Rects(Vec<[i32; 4]>),
}

struct Compositor {
    stream: UnixStream,
    scenario: Scenario,
    objects: HashMap<u32, Object>,
    registry: u32,
    output_proxy: u32,
    output_x: i32,
    effect_managers: Vec<(u32, u32)>, // (global_name, proxy_id)
    regions: HashMap<u32, Vec<[i32; 4]>>,
    destroyed_regions: HashSet<u32>,
    pending_blur: HashMap<u32, Option<CommittedBlur>>,
    committed_blurs: HashMap<u32, Vec<CommittedBlur>>,
    effect_creations: Vec<(u32, u32, u32)>, // (manager_proxy, effect_id, wl_surface_id)
    effect_for_surface: HashMap<u32, Vec<u32>>,
    surface_order: Vec<u32>,
    configured_surfaces: HashSet<u32>,
    input_region_calls: u32,
    opaque_region_calls: u32,
    destroy_log: Vec<(u32, &'static str, u32)>, // (obj_id, kind, wl_surface_id)
    pending_callbacks: Vec<(u32, u32)>,         // (cb_id, wl_surface_id)
    phase: u32,
    serial: u32,
}

impl Compositor {
    fn new(stream: UnixStream, scenario: Scenario) -> Self {
        Self {
            stream,
            scenario,
            objects: HashMap::new(),
            registry: 0,
            output_proxy: 0,
            output_x: 0,
            effect_managers: Vec::new(),
            regions: HashMap::new(),
            destroyed_regions: HashSet::new(),
            pending_blur: HashMap::new(),
            committed_blurs: HashMap::new(),
            effect_creations: Vec::new(),
            effect_for_surface: HashMap::new(),
            surface_order: Vec::new(),
            configured_surfaces: HashSet::new(),
            input_region_calls: 0,
            opaque_region_calls: 0,
            destroy_log: Vec::new(),
            pending_callbacks: Vec::new(),
            phase: 0,
            serial: 100,
        }
    }

    fn next_serial(&mut self) -> u32 {
        self.serial += 1;
        self.serial
    }

    fn run(&mut self) {
        loop {
            let mut header = [0u8; 8];
            match self.stream.read_exact(&mut header) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return,
                Err(error) => panic!("reading Wayland request: {error}"),
            }
            let id = word(&header, 0);
            let descriptor = word(&header, 1);
            let opcode = descriptor & 0xffff;
            let len = (descriptor >> 16) as usize;
            let mut body = vec![0u8; len - 8];
            self.stream.read_exact(&mut body).expect("Wayland request body");
            self.request(id, opcode, &body);
        }
    }

    fn nudge_client(&mut self) {
        self.output_x += 10;
        self.geometry(self.output_proxy, self.output_x);
        self.event(self.output_proxy, 2, &[]); // wl_output.done
    }

    fn answer_callbacks(&mut self, wl: u32) {
        let mut remaining = Vec::new();
        for (cb, surface) in std::mem::take(&mut self.pending_callbacks) {
            if surface == wl {
                self.event(cb, 0, &words(&[1]));
                self.event(1, 1, &words(&[cb]));
            } else {
                remaining.push((cb, surface));
            }
        }
        self.pending_callbacks = remaining;
    }

    fn request(&mut self, id: u32, opcode: u32, body: &[u8]) {
        let arg = |offset| word(body, offset);
        if id == 1 {
            match opcode {
                0 => {
                    let cb = arg(0);
                    self.event(cb, 0, &words(&[0]));
                    self.event(1, 1, &words(&[cb]));
                }
                1 => {
                    self.registry = arg(0);
                    self.objects.insert(self.registry, Object::Registry);
                    self.global(GLOBAL_COMPOSITOR, "wl_compositor", 6);
                    self.global(GLOBAL_SHM, "wl_shm", 1);
                    self.global(GLOBAL_XDG, "xdg_wm_base", 6);
                    self.global(GLOBAL_LAYER, "zwlr_layer_shell_v1", 4);
                    self.global(GLOBAL_OUTPUT, "wl_output", 4);
                    if self.scenario != Scenario::AbsentManager {
                        self.global(GLOBAL_EFFECT_1, "ext_background_effect_manager_v1", 1);
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
            (Object::Registry, 0) => {
                let global = arg(0);
                let version_index = 2 + (arg(1) as usize).div_ceil(4);
                let bound = arg(version_index + 1);
                let object = match global {
                    GLOBAL_COMPOSITOR => Object::Compositor,
                    GLOBAL_SHM => Object::Shm,
                    GLOBAL_XDG => Object::XdgShell,
                    GLOBAL_LAYER => Object::LayerShell,
                    GLOBAL_OUTPUT => {
                        self.output_proxy = bound;
                        Object::Output
                    }
                    GLOBAL_EFFECT_1 | GLOBAL_EFFECT_2 => {
                        self.effect_managers.push((global, bound));
                        Object::EffectManager(global)
                    }
                    _ => panic!("unexpected global {global}"),
                };
                self.objects.insert(bound, object);
                match object {
                    Object::Output => {
                        self.geometry(bound, self.output_x);
                        self.event(bound, 1, &words(&[3, 1920, 1080, 60_000]));
                        self.event(bound, 3, &words(&[1]));
                        self.event(bound, 4, &string("TEST-1"));
                        self.event(bound, 2, &[]);
                    }
                    Object::EffectManager(GLOBAL_EFFECT_1) => match self.scenario {
                        Scenario::ManagerWithBlur | Scenario::EarlyPopupRetirement => {
                            self.event(bound, 0, &words(&[1])); // Capability::Blur
                        }
                        Scenario::ZeroFlagsManager => {
                            self.event(bound, 0, &words(&[0])); // No capabilities
                        }
                        Scenario::DynamicCapabilitiesAndRecreation => {
                            // Deliberately withhold capabilities initially to verify false until advertised!
                        }
                        Scenario::AbsentManager => unreachable!(),
                    },
                    Object::EffectManager(GLOBAL_EFFECT_2) => {
                        // Send a late Capability::Blur on the RETIRED first manager (must be ignored!)
                        // and then Capability::Blur on the newly bound second manager.
                        let old_mgr = self.effect_managers[0].1;
                        self.event(old_mgr, 0, &words(&[1]));
                        self.event(bound, 0, &words(&[1]));
                    }
                    _ => {}
                }
            }
            (Object::Compositor, 0) => {
                let wl = arg(0);
                self.objects.insert(wl, Object::Surface);
                self.surface_order.push(wl);
            }
            (Object::Compositor, 1) => {
                let reg = arg(0);
                self.objects.insert(reg, Object::Region);
                self.regions.insert(reg, Vec::new());
            }
            (Object::Region, 0) => {
                self.destroyed_regions.insert(id);
                self.objects.remove(&id);
            }
            (Object::Region, 1) => {
                let rect = [arg(0) as i32, arg(1) as i32, arg(2) as i32, arg(3) as i32];
                self.regions.entry(id).or_default().push(rect);
            }
            (Object::LayerShell, 0) => {
                let layer_id = arg(0);
                let wl = arg(1);
                self.objects.insert(layer_id, Object::Layer(wl));
            }
            (Object::Layer(wl), 6) => {
                // zwlr_layer_surface_v1.ack_configure
                if self.scenario == Scenario::DynamicCapabilitiesAndRecreation
                    && self.phase == 0
                    && wl == self.surface_order[0]
                {
                    // Surface configured while blur is unavailable (zero effect objects created).
                    // Now send delayed grant!
                    assert!(
                        self.effect_creations.is_empty(),
                        "zero effect objects before delayed grant"
                    );
                    self.phase = 1;
                    let mgr1 = self.effect_managers[0].1;
                    self.event(mgr1, 0, &words(&[1]));
                }
            }
            (Object::Layer(wl), 7) => {
                self.destroy_log.push((id, "layer", wl));
                self.objects.remove(&id);
            }
            (Object::XdgShell, 1) => {
                self.objects.insert(arg(0), Object::XdgPositioner);
            }
            (Object::XdgShell, 2) => {
                let xdg_id = arg(0);
                let wl = arg(1);
                self.objects.insert(xdg_id, Object::XdgSurface(wl));
            }
            (Object::XdgPositioner, 0) => {
                self.objects.remove(&id);
            }
            (Object::XdgSurface(wl), 0) => {
                self.destroy_log.push((id, "xdg_surface", wl));
                self.objects.remove(&id);
            }
            (Object::XdgSurface(wl), 1) => {
                self.objects.insert(arg(0), Object::XdgToplevel(wl));
            }
            (Object::XdgSurface(wl), 2) => {
                self.objects.insert(arg(0), Object::XdgPopup(wl));
            }
            (Object::XdgToplevel(wl), 0) => {
                self.destroy_log.push((id, "xdg_toplevel", wl));
                self.objects.remove(&id);
            }
            (Object::XdgPopup(wl), 0) => {
                self.destroy_log.push((id, "xdg_popup", wl));
                self.objects.remove(&id);
            }
            (Object::EffectManager(_), 0) => {
                self.objects.remove(&id);
            }
            (Object::EffectManager(_), 1) => {
                // get_background_effect(id: new_id, surface: wl_surface)
                let effect_id = arg(0);
                let wl = arg(1);
                self.objects.insert(effect_id, Object::EffectSurface(wl));
                self.effect_creations.push((id, effect_id, wl));
                self.effect_for_surface.entry(wl).or_default().push(effect_id);
            }
            (Object::EffectSurface(wl), 0) => {
                self.destroy_log.push((id, "effect_surface", wl));
                self.objects.remove(&id);
            }
            (Object::EffectSurface(wl), 1) => {
                let region_id = arg(0);
                let blur = if region_id == 0 {
                    CommittedBlur::None
                } else {
                    let rects = self.regions.get(&region_id).cloned().unwrap_or_default();
                    CommittedBlur::Rects(rects)
                };
                self.pending_blur.insert(wl, Some(blur));
            }
            (Object::Surface, 0) => {
                self.destroy_log.push((id, "wl_surface", id));
                self.objects.remove(&id);
            }
            (Object::Surface, 3) => {
                let cb = arg(0);
                self.objects.insert(cb, Object::FrameCallback(id));
                self.pending_callbacks.push((cb, id));
            }
            (Object::Surface, 4) => {
                self.opaque_region_calls += 1;
            }
            (Object::Surface, 5) => {
                self.input_region_calls += 1;
            }
            (Object::Surface, 6) => {
                self.handle_commit(id);
            }
            _ => {}
        }
    }

    fn handle_commit(&mut self, wl: u32) {
        if !self.configured_surfaces.contains(&wl) {
            // Initial empty commit asking for configure: must NOT carry premature blur!
            assert!(
                self.pending_blur.get(&wl).and_then(|b| b.as_ref()).is_none(),
                "{:?}: surface {wl} committed blur state before initial configure",
                self.scenario
            );
            assert!(
                self.effect_for_surface.get(&wl).map_or(0, Vec::len) == 0,
                "{:?}: surface {wl} created effect object before initial configure",
                self.scenario
            );
            self.configured_surfaces.insert(wl);
            self.send_initial_configure(wl);
            return;
        }

        if let Some(blur) = self.pending_blur.remove(&wl).flatten() {
            // Every temporary wl_region used for blur must already be destroyed before commit.
            for &reg in self.regions.keys() {
                assert!(
                    self.destroyed_regions.contains(&reg),
                    "{:?}: temporary wl_region {reg} was not destroyed before commit",
                    self.scenario
                );
            }
            self.committed_blurs.entry(wl).or_default().push(blur);
        }

        self.answer_callbacks(wl);
        self.advance_scenario(wl);
    }

    fn send_initial_configure(&mut self, wl: u32) {
        let serial = self.next_serial();
        let role = self
            .objects
            .iter()
            .find_map(|(&id, &obj)| match obj {
                Object::Layer(s) if s == wl => Some((id, obj)),
                Object::XdgToplevel(s) if s == wl => Some((id, obj)),
                Object::XdgPopup(s) if s == wl => Some((id, obj)),
                _ => None,
            });
        match role {
            Some((layer_id, Object::Layer(_))) => {
                self.event(layer_id, 0, &words(&[serial, 200, 40]));
            }
            Some((toplevel_id, Object::XdgToplevel(_))) => {
                let xdg_id = self.find_xdg_surface(wl);
                self.event(toplevel_id, 0, &words(&[320, 240, 0]));
                self.event(xdg_id, 0, &words(&[serial]));
            }
            Some((popup_id, Object::XdgPopup(_))) => {
                let xdg_id = self.find_xdg_surface(wl);
                self.event(popup_id, 0, &words(&[4, 24, 120, 80]));
                self.event(xdg_id, 0, &words(&[serial]));
            }
            _ => {}
        }
    }

    fn find_xdg_surface(&self, wl: u32) -> u32 {
        self.objects
            .iter()
            .find_map(|(&id, &obj)| matches!(obj, Object::XdgSurface(s) if s == wl).then_some(id))
            .expect("xdg_surface exists")
    }

    fn find_layer(&self, wl: u32) -> u32 {
        self.objects
            .iter()
            .find_map(|(&id, &obj)| matches!(obj, Object::Layer(s) if s == wl).then_some(id))
            .expect("layer exists")
    }

    fn advance_scenario(&mut self, wl: u32) {
        match self.scenario {
            Scenario::ManagerWithBlur => {
                let primary = self.surface_order[0];
                let secondary = self.surface_order.get(1).copied();
                if wl == primary {
                    let count = self.committed_blurs.get(&primary).map_or(0, Vec::len);
                    match (self.phase, count) {
                        (0, 1) => {
                            // First commit: Rects committed. Surface is now idle -> nudge to switch to Full.
                            self.phase = 1;
                            self.nudge_client();
                        }
                        (1, 2) => {
                            // Second commit: Full committed -> send resize configure (400x60) to verify Full survives resize!
                            self.phase = 2;
                            let layer_id = self.find_layer(primary);
                            let serial = self.next_serial();
                            self.event(layer_id, 0, &words(&[serial, 400, 60]));
                        }
                        (2, 3) => {
                            // Third commit: Clear (None) committed -> nudge to blur secondary toplevel!
                            self.phase = 3;
                            self.nudge_client();
                        }
                        _ => {}
                    }
                } else if Some(wl) == secondary {
                    let count = self.committed_blurs.get(&wl).map_or(0, Vec::len);
                    if self.phase == 3 && count == 1 {
                        self.phase = 4;
                        self.nudge_client();
                    }
                }
            }
            Scenario::DynamicCapabilitiesAndRecreation => {
                let primary = self.surface_order[0];
                let secondary = self.surface_order.get(1).copied();
                let mgr1 = self.effect_managers[0].1;
                if wl == primary {
                    match self.phase {
                        1 if self.committed_blurs.get(&primary).map_or(0, Vec::len) == 1 => {
                            // Delayed grant applied! Now withdraw capability (flags = 0).
                            self.phase = 2;
                            self.event(mgr1, 0, &words(&[0]));
                        }
                        2 => {
                            // Legal commit after withdrawal completed; client updated intent to Full while unavailable.
                            // Now remove manager 1 and announce manager 2!
                            self.phase = 3;
                            self.event(self.registry, 1, &words(&[GLOBAL_EFFECT_1]));
                            self.global(GLOBAL_EFFECT_2, "ext_background_effect_manager_v1", 1);
                        }
                        3 if self.committed_blurs.get(&primary).map_or(0, Vec::len) == 2 => {
                            // Regain applied Full on manager 2! Nudge client to recreate surface.
                            self.phase = 4;
                            self.nudge_client();
                        }
                        _ => {}
                    }
                } else if Some(wl) == secondary && self.phase == 4 {
                    if self.committed_blurs.get(&wl).map_or(0, Vec::len) == 1 {
                        self.phase = 5;
                        self.nudge_client();
                    }
                }
            }
            Scenario::EarlyPopupRetirement => {
                let popup1 = self.surface_order.get(1).copied();
                let popup2 = self.surface_order.get(2).copied();
                if Some(wl) == popup1 && self.phase == 0 {
                    if self.committed_blurs.get(&wl).map_or(0, Vec::len) == 1 {
                        self.phase = 1;
                        self.nudge_client();
                    }
                } else if Some(wl) == popup2 && self.phase == 1 {
                    if self.committed_blurs.get(&wl).map_or(0, Vec::len) == 1 {
                        self.phase = 2;
                        self.nudge_client();
                    }
                }
            }
            Scenario::AbsentManager | Scenario::ZeroFlagsManager => {}
        }
    }

    fn verify(&self) {
        assert_eq!(self.input_region_calls, 0, "{:?}: blur must not touch input region", self.scenario);
        assert_eq!(self.opaque_region_calls, 0, "{:?}: blur must not touch opaque region", self.scenario);

        // Universal destroy ordering invariant: for every surface that ever created an effect surface,
        // every effect_surface.destroy() MUST strictly precede that surface's wl_surface.destroy().
        for (&wl, effects) in &self.effect_for_surface {
            let wl_pos = self
                .destroy_log
                .iter()
                .position(|&(id, kind, s)| id == wl && kind == "wl_surface" && s == wl)
                .unwrap_or_else(|| panic!("{:?}: wl_surface {wl} was never destroyed", self.scenario));
            for &effect_id in effects {
                let eff_pos = self
                    .destroy_log
                    .iter()
                    .position(|&(id, kind, s)| id == effect_id && kind == "effect_surface" && s == wl)
                    .unwrap_or_else(|| {
                        panic!("{:?}: effect_surface {effect_id} for wl_surface {wl} was never destroyed", self.scenario)
                    });
                assert!(
                    eff_pos < wl_pos,
                    "{:?}: effect_surface {effect_id} destroyed at {eff_pos}, after wl_surface {wl} at {wl_pos}",
                    self.scenario
                );
            }
        }

        match self.scenario {
            Scenario::ManagerWithBlur => {
                let primary = self.surface_order[0];
                let secondary = self.surface_order[1];
                assert_eq!(
                    self.effect_for_surface.get(&primary).map_or(0, Vec::len),
                    1,
                    "rects -> full -> clear must reuse a single ext_background_effect_surface_v1"
                );
                assert_eq!(
                    self.effect_for_surface.get(&secondary).map_or(0, Vec::len),
                    1,
                    "secondary toplevel must get its own isolated effect surface"
                );
                assert_eq!(
                    self.committed_blurs.get(&primary),
                    Some(&vec![
                        CommittedBlur::Rects(vec![[-3, 3, 11, 9], [40, 5, 20, 12]]),
                        CommittedBlur::Rects(vec![[0, 0, i32::MAX, i32::MAX]]),
                        CommittedBlur::None,
                    ]),
                    "primary must commit outward-rounded rects, full (surviving resize), then clear"
                );
                assert_eq!(
                    self.committed_blurs.get(&secondary),
                    Some(&vec![CommittedBlur::Rects(vec![[1, 2, 30, 40]])]),
                    "secondary must remain isolated from primary"
                );
            }
            Scenario::AbsentManager | Scenario::ZeroFlagsManager => {
                assert!(
                    self.effect_creations.is_empty(),
                    "{:?}: must allocate zero ext_background_effect_surface_v1 objects",
                    self.scenario
                );
                assert!(
                    self.committed_blurs.is_empty(),
                    "{:?}: must send zero set_blur_region requests",
                    self.scenario
                );
            }
            Scenario::DynamicCapabilitiesAndRecreation => {
                let s1 = self.surface_order[0];
                let s2 = self.surface_order[1];
                assert_eq!(self.effect_managers.len(), 2, "must bind both initial and rebound managers");
                let mgr1 = self.effect_managers[0].1;
                let mgr2 = self.effect_managers[1].1;
                assert_eq!(
                    self.committed_blurs.get(&s1),
                    Some(&vec![
                        CommittedBlur::Rects(vec![[5, 5, 50, 20]]),
                        CommittedBlur::Rects(vec![[0, 0, i32::MAX, i32::MAX]]),
                    ]),
                    "S1 must apply initial intent on delayed grant and updated Full intent on regain"
                );
                // S1 got one effect object on mgr1 (destroyed on withdrawal) and one on mgr2 (on regain).
                let s1_creations: Vec<_> = self
                    .effect_creations
                    .iter()
                    .filter(|&&(_, _, wl)| wl == s1)
                    .map(|&(mgr, _, _)| mgr)
                    .collect();
                assert_eq!(s1_creations, vec![mgr1, mgr2]);
                // Recreated S2 got a fresh protocol identity on mgr2.
                let s2_creations: Vec<_> = self
                    .effect_creations
                    .iter()
                    .filter(|&&(_, _, wl)| wl == s2)
                    .map(|&(mgr, _, _)| mgr)
                    .collect();
                assert_eq!(s2_creations, vec![mgr2]);
                assert_eq!(
                    self.committed_blurs.get(&s2),
                    Some(&vec![CommittedBlur::Rects(vec![[2, 4, 40, 10]])]),
                );
            }
            Scenario::EarlyPopupRetirement => {
                let popup1 = self.surface_order[1];
                let popup2 = self.surface_order[2];
                let eff1 = self.effect_for_surface[&popup1][0];
                let eff1_pos = self
                    .destroy_log
                    .iter()
                    .position(|&(id, kind, s)| id == eff1 && kind == "effect_surface" && s == popup1)
                    .expect("popup1 effect destroyed");
                let popup1_role_pos = self
                    .destroy_log
                    .iter()
                    .position(|&(_, kind, s)| kind == "xdg_popup" && s == popup1)
                    .expect("popup1 xdg_popup destroyed");
                let popup1_wl_pos = self
                    .destroy_log
                    .iter()
                    .position(|&(_, kind, s)| kind == "wl_surface" && s == popup1)
                    .expect("popup1 wl_surface destroyed");
                assert!(
                    eff1_pos < popup1_role_pos && popup1_role_pos < popup1_wl_pos,
                    "early popup retirement must destroy effect_surface ({eff1_pos}) before xdg_popup ({popup1_role_pos}) and wl_surface ({popup1_wl_pos})"
                );
                assert_eq!(
                    self.committed_blurs.get(&popup1),
                    Some(&vec![CommittedBlur::Rects(vec![[0, 0, i32::MAX, i32::MAX]])]),
                );
                assert_eq!(
                    self.committed_blurs.get(&popup2),
                    Some(&vec![CommittedBlur::Rects(vec![[0, 0, 60, 30]])]),
                );
            }
        }
    }

    fn global(&mut self, name: u32, interface: &str, version: u32) {
        let mut body = Vec::new();
        body.extend_from_slice(&name.to_ne_bytes());
        body.extend_from_slice(&string(interface));
        body.extend_from_slice(&version.to_ne_bytes());
        self.event(self.registry, 0, &body);
    }

    fn geometry(&mut self, output: u32, x: i32) {
        let mut body = Vec::new();
        for value in [x, 0, 500, 300, 0] {
            body.extend_from_slice(&value.to_ne_bytes());
        }
        body.extend_from_slice(&string("Test"));
        body.extend_from_slice(&string("Fixture"));
        body.extend_from_slice(&0i32.to_ne_bytes());
        self.event(output, 0, &body);
    }

    fn event(&mut self, object: u32, opcode: u32, body: &[u8]) {
        let length = u32::try_from(body.len() + 8).expect("small Wayland event");
        let mut message = Vec::with_capacity(length as usize);
        message.extend_from_slice(&object.to_ne_bytes());
        message.extend_from_slice(&((length << 16) | opcode).to_ne_bytes());
        message.extend_from_slice(body);
        self.stream.write_all(&message).expect("write Wayland event");
    }
}

fn word(bytes: &[u8], index: usize) -> u32 {
    let start = index * 4;
    u32::from_ne_bytes(bytes[start..start + 4].try_into().expect("32-bit word"))
}

fn words(values: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    bytes
}

fn string(text: &str) -> Vec<u8> {
    let length = text.len() + 1;
    let padded = (length + 3) & !3;
    let mut bytes = Vec::with_capacity(4 + padded);
    bytes.extend_from_slice(&u32::try_from(length).expect("short string").to_ne_bytes());
    bytes.extend_from_slice(text.as_bytes());
    bytes.resize(4 + padded, 0);
    bytes
}
