//! End-to-end Seam 1 verification for gesture-bound `requestActivationToken`,
//! `NativeEvent.modifiers.{capsLock, numLock}`, `getCapsLock()`, and `useCapsLock()`.

use super::*;
use crate::{
    package::{Capabilities, Package},
    session::{ResourceCounts, SessionModule},
    test_package::Fixture,
};
use std::{
    path::Path,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use zgui::{
    geom::{Css, CssPx, Device, DevicePx, Point, Rect, Scale, Size},
    platform::{
        ActivationToken, ActivationTokenSerial, ActivationTokenSerials, AppHandler, BlurRegion,
        Clipboard, Clock, ColorScheme, CursorStyle, Decorations, FocusGrabId, FullscreenMode,
        GpuSurface, IdlePolicy, InputGrant, InputRegion, InputSerial, LayerPatch, LayerPlacement,
        MonitorId, MonitorInfo, OutboundDragPayload, PendingActivationToken, PlatformCapabilities,
        PlatformCx, PlatformError, PopupPlacement, PresentPacing, PresentationTiming, ResizeEdge,
        ScrollSettings, SessionLockState as PlatformLockState, SubpixelLayout, Surface,
        SurfaceAttributes, SurfaceEvent, SurfaceId, SurfaceRole, TextInput, Unsupported,
        WakeReason, Waker, WindowIcon, WindowLevel,
    },
    prelude::IntoView,
    render::Renderer,
    vocab::{
        ImeEvent, Key, KeyEvent, KeyState, Modifiers, NamedKey, PhysicalKey, PointerAction,
        PointerButton, PointerEvent,
    },
};

#[derive(Clone, Debug)]
struct MintRecord {
    serial: ActivationTokenSerial,
    surface: SurfaceId,
    grant: InputGrant,
    app_id: Option<String>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
struct MintLog {
    serials: Arc<ActivationTokenSerials>,
    records: Arc<Mutex<Vec<MintRecord>>>,
    refuse: Arc<AtomicBool>,
}

impl MintLog {
    fn take_records(&self) -> Vec<MintRecord> {
        std::mem::take(&mut *self.records.lock().expect("mint records lock"))
    }
}

struct GrantingSurface {
    inner: Arc<dyn Surface>,
    is_lock: bool,
    log: MintLog,
}

impl Surface for GrantingSurface {
    fn id(&self) -> SurfaceId { self.inner.id() }
    fn size(&self) -> Size<DevicePx, Device> { self.inner.size() }
    fn scale_factor(&self) -> f64 { self.inner.scale_factor() }
    fn subpixel_layout(&self) -> SubpixelLayout { self.inner.subpixel_layout() }
    fn refresh_rate_millihertz(&self) -> Option<u32> { self.inner.refresh_rate_millihertz() }
    fn present_pacing(&self) -> PresentPacing { self.inner.present_pacing() }
    fn crops_buffers(&self) -> bool { self.inner.crops_buffers() }
    fn presentation_timing(&self) -> Option<PresentationTiming> { self.inner.presentation_timing() }
    fn reports_presentation(&self) -> bool { self.inner.reports_presentation() }
    fn request_size(&self, size: Size<CssPx, Css>) -> Option<Size<DevicePx, Device>> { self.inner.request_size(size) }
    fn set_min_size(&self, size: Option<Size<CssPx, Css>>) { self.inner.set_min_size(size) }
    fn set_max_size(&self, size: Option<Size<CssPx, Css>>) { self.inner.set_max_size(size) }
    fn request_redraw(&self) { self.inner.request_redraw() }
    fn pre_present_notify(&self) { self.inner.pre_present_notify() }
    fn present_now(&self) -> bool { self.inner.present_now() }
    fn pre_present_frame(&self, size: Size<i32, Device>, scale: f64) { self.inner.pre_present_frame(size, scale) }
    fn redraw_declined(&self) { self.inner.redraw_declined() }
    fn set_title(&self, title: &str) { self.inner.set_title(title) }
    fn set_visible(&self, visible: bool) { self.inner.set_visible(visible) }
    fn set_decorations(&self, decorations: Decorations) { self.inner.set_decorations(decorations) }
    fn set_resizable(&self, resizable: bool) { self.inner.set_resizable(resizable) }
    fn set_maximized(&self, maximized: bool) { self.inner.set_maximized(maximized) }
    fn set_minimized(&self, minimized: bool) { self.inner.set_minimized(minimized) }
    fn set_fullscreen(&self, mode: Option<FullscreenMode>) { self.inner.set_fullscreen(mode) }
    fn set_position(&self, position: Point<CssPx, Css>) { self.inner.set_position(position) }
    fn position(&self) -> Option<Point<CssPx, Css>> { self.inner.position() }
    fn is_maximized(&self) -> bool { self.inner.is_maximized() }
    fn is_minimized(&self) -> Option<bool> { self.inner.is_minimized() }
    fn fullscreen(&self) -> Option<FullscreenMode> { self.inner.fullscreen() }
    fn set_window_level(&self, level: WindowLevel) { self.inner.set_window_level(level) }
    fn set_icon(&self, icon: Option<&WindowIcon>) { self.inner.set_icon(icon) }
    fn set_theme(&self, theme: Option<ColorScheme>) { self.inner.set_theme(theme) }
    fn focus(&self) { self.inner.focus() }
    fn input_serial(&self) -> Option<InputSerial> { self.inner.input_serial() }
    fn request_attention(&self, urgent: bool) { self.inner.request_attention(urgent) }
    fn title_bar_double_click(&self) { self.inner.title_bar_double_click() }
    fn begin_move_drag(&self) -> Result<(), Unsupported> { self.inner.begin_move_drag() }
    fn begin_resize_drag(&self, edge: ResizeEdge) -> Result<(), Unsupported> { self.inner.begin_resize_drag(edge) }
    fn set_cursor(&self, cursor: CursorStyle) { self.inner.set_cursor(cursor) }
    fn set_layer_placement(&self, patch: &LayerPatch) -> Result<(), Unsupported> { self.inner.set_layer_placement(patch) }
    fn set_pointer_passthrough(&self, passthrough: bool) -> Result<(), Unsupported> { self.inner.set_pointer_passthrough(passthrough) }
    fn set_input_region(&self, region: Option<&[Rect<CssPx, Css>]>) -> Result<(), Unsupported> { self.inner.set_input_region(region) }
    fn set_opaque_region(&self, region: Option<&[Rect<CssPx, Css>]>) { self.inner.set_opaque_region(region) }
    fn prefers_blur_shapes(&self) -> bool { self.inner.prefers_blur_shapes() }
    fn set_blur_region(&self, region: BlurRegion<'_>) { self.inner.set_blur_region(region) }
    fn logical_extent_for(&self, size: Size<i32, Device>, scale: f64) -> Size<CssPx, Css> { self.inner.logical_extent_for(size, scale) }
    fn stage_blur_region_for_present(&self, region: BlurRegion<'_>) { self.inner.stage_blur_region_for_present(region) }
    fn clear_staged_blur_region(&self) { self.inner.clear_staged_blur_region() }
    fn finalize_presented_blur(&self) { self.inner.finalize_presented_blur() }
    fn rollback_unpresented_blur(&self) { self.inner.rollback_unpresented_blur() }
    fn apply_idle_blur_region(&self, region: BlurRegion<'_>) { self.inner.apply_idle_blur_region(region) }
    fn set_shell_input_region(&self, region: &InputRegion) -> Result<(), PlatformError> { self.inner.set_shell_input_region(region) }
    fn reconfigure_layer(&self, placement: &LayerPlacement, size: Size<CssPx, Css>) -> Result<(), PlatformError> {
        self.inner.reconfigure_layer(placement, size)
    }
    fn reconfigure_popup(&self, placement: &PopupPlacement, size: Size<CssPx, Css>) -> Result<(), PlatformError> {
        self.inner.reconfigure_popup(placement, size)
    }
    fn popup_dismissed(&self) -> bool { self.inner.popup_dismissed() }
    fn supports_popup_reposition(&self) -> bool { self.inner.supports_popup_reposition() }
    fn begin_outbound_drag(&self, payload: OutboundDragPayload) -> Result<(), PlatformError> {
        self.inner.begin_outbound_drag(payload)
    }
    fn cancel_outbound_drag(&self) { self.inner.cancel_outbound_drag() }
    fn request_activation_token(
        &self,
        grant: InputGrant,
        app_id: Option<&str>,
    ) -> Option<PendingActivationToken> {
        if self.is_lock
            || self.inner.popup_dismissed()
            || self.log.refuse.load(Ordering::Relaxed)
            || grant.surface() != self.inner.id()
        {
            return None;
        }
        let serial = self.log.serials.take();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.log.records.lock().expect("mint records lock").push(MintRecord {
            serial,
            surface: self.inner.id(),
            grant,
            app_id: app_id.map(ToOwned::to_owned),
            cancelled: Arc::clone(&cancelled),
        });
        Some(PendingActivationToken::new(
            serial,
            Some(Arc::new(move || {
                cancelled.store(true, Ordering::Relaxed);
            })),
        ))
    }
    fn set_text_input(&self, state: Option<TextInput>) { self.inner.set_text_input(state) }
    fn reset_dead_keys(&self) { self.inner.reset_dead_keys() }
    fn push_a11y_update(&self, build: &mut dyn FnMut() -> accesskit::TreeUpdate) { self.inner.push_a11y_update(build) }
    fn gpu(&self) -> Option<&dyn GpuSurface> { self.inner.gpu() }
    fn gpu_shared(self: Arc<Self>) -> Option<Arc<dyn GpuSurface>> { Arc::clone(&self.inner).gpu_shared() }
}

struct GrantingPlatform {
    headless: zgui_platform_headless::Headless,
    surfaces: Mutex<HashMap<SurfaceId, Arc<GrantingSurface>>>,
    log: MintLog,
}

impl GrantingPlatform {
    fn new(capabilities: PlatformCapabilities, monitors: Vec<MonitorInfo>) -> Self {
        Self {
            headless: zgui_platform_headless::Headless::new()
                .with_capabilities(capabilities)
                .with_monitors(monitors),
            surfaces: Mutex::new(HashMap::new()),
            log: MintLog::default(),
        }
    }

    fn wrapped_or_insert(&self, inner: Arc<dyn Surface>, is_lock: bool) -> Arc<dyn Surface> {
        let mut map = self.surfaces.lock().unwrap();
        let entry = map.entry(inner.id()).or_insert_with(|| {
            Arc::new(GrantingSurface {
                inner,
                is_lock,
                log: self.log.clone(),
            })
        });
        Arc::clone(entry) as Arc<dyn Surface>
    }
}

impl PlatformCx for GrantingPlatform {
    fn create_surface(&self, attributes: &SurfaceAttributes) -> Result<Arc<dyn Surface>, PlatformError> {
        let surface = self.headless.create_surface(attributes)?;
        let is_lock = matches!(attributes.role, SurfaceRole::Lock(_));
        Ok(self.wrapped_or_insert(surface, is_lock))
    }
    fn destroy_surface(&self, id: SurfaceId) {
        self.headless.destroy_surface(id);
        self.surfaces
            .lock()
            .unwrap()
            .retain(|&sid, _| self.headless.surface(sid).is_some());
    }
    fn surface(&self, id: SurfaceId) -> Option<Arc<dyn Surface>> {
        let inner = self.headless.surface(id)?;
        Some(self.wrapped_or_insert(inner, false))
    }
    fn surfaces(&self) -> Vec<Arc<dyn Surface>> {
        self.headless
            .surfaces()
            .into_iter()
            .map(|s| self.wrapped_or_insert(s, false))
            .collect()
    }
    fn monitors(&self) -> Vec<MonitorInfo> { self.headless.monitors() }
    fn primary_monitor(&self) -> Option<MonitorInfo> { self.headless.primary_monitor() }
    fn create_focus_grab(&self) -> Option<FocusGrabId> { self.headless.create_focus_grab() }
    fn set_focus_grab(&self, grab: FocusGrabId, surfaces: &[SurfaceId]) { self.headless.set_focus_grab(grab, surfaces) }
    fn destroy_focus_grab(&self, grab: FocusGrabId) { self.headless.destroy_focus_grab(grab) }
    fn lock_session(&self) -> Result<(), PlatformError> { self.headless.lock_session() }
    fn request_session_lock(&self) -> Result<(), PlatformError> { self.headless.request_session_lock() }
    fn session_lock_state(&self) -> PlatformLockState { self.headless.session_lock_state() }
    fn unlock_session(&self) { self.headless.unlock_session() }
    fn try_unlock_session(&self) -> Result<(), PlatformError> { self.headless.try_unlock_session() }
    fn color_scheme(&self) -> Option<ColorScheme> { self.headless.color_scheme() }
    fn clipboard(&self) -> &dyn Clipboard { self.headless.clipboard() }
    fn capabilities(&self) -> &PlatformCapabilities { self.headless.capabilities() }
    fn scroll_settings(&self) -> ScrollSettings { self.headless.scroll_settings() }
    fn clock(&self) -> Arc<dyn Clock> { self.headless.clock() }
    fn waker(&self) -> Arc<dyn Waker> { self.headless.waker() }
    fn request_exit(&self) { self.headless.request_exit() }
    fn is_exiting(&self) -> bool { self.headless.is_exiting() }
}

fn no_retry() -> Rc<dyn Fn(DocumentId, u64)> {
    Rc::new(|_, _| {})
}

fn shell_capabilities() -> PlatformCapabilities {
    let mut caps = PlatformCapabilities::none();
    caps.window_controls = true;
    caps.layer_surfaces = true;
    caps.native_popup_surfaces = true;
    caps.input_regions = true;
    caps.session_lock = true;
    caps.drag_source = true;
    caps
}

fn default_monitors() -> Vec<MonitorInfo> {
    let mut m = MonitorInfo::new(
        Point::new(DevicePx(0.0), DevicePx(0.0)),
        Size::new(DevicePx(1920.0), DevicePx(1080.0)),
        1.0,
    );
    m.id = Some(MonitorId::new(1));
    m.name = Some("DP-1".into());
    vec![m]
}

struct ActivationRig {
    host: SessionHost,
    runtime: zgui::runtime::Runtime,
    cx: GrantingPlatform,
}

impl ActivationRig {
    fn primary(package: Rc<Package>) -> Self {
        let pending = Rc::new(RefCell::new(Vec::new()));
        let roots = pending.clone();
        let fonts = zgui::app::Fonts::shipped_only();
        let mut runtime = zgui::runtime::App::new()
            .with_size(400.0, 300.0)
            .with_renderer(Box::new(|_, target| {
                let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
                Renderer::configure(&mut renderer, target);
                Ok(Box::new(renderer))
            }))
            .into_handler(move |cx| {
                zgui::reactive::provide_context(fonts.clone());
                Box::new(Root(roots.clone()).build(cx))
            })
            .expect("create runtime");
        let cx = GrantingPlatform::new(shell_capabilities(), default_monitors());
        runtime.surfaces_available(&cx);
        let mut host = SessionHost::new(
            pending,
            SessionModule::package(package),
            SessionPolicy::live(host_services::Executor::new(2, 64).unwrap()),
            RecoveryPolicy::Propagate,
            SurfaceOpacity::Transparent,
        );
        let monitors = Monitor::snapshot(&cx.monitors());
        host.set_monitor_snapshot(&monitors);
        host.attach(
            &mut RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly),
            cx.waker(),
            no_retry(),
        )
        .expect("attach primary session");
        let mut rig = Self { host, runtime, cx };
        rig.settle();
        rig
    }

    fn pure_shell(package: Rc<Package>) -> Self {
        let pending = Rc::new(RefCell::new(Vec::new()));
        let fonts = zgui::app::Fonts::shipped_only();
        let mut runtime = zgui::runtime::App::new()
            .without_primary_window()
            .with_exit_policy(zgui::runtime::ExitPolicy::Explicit)
            .with_renderer(Box::new(|_, target| {
                let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
                Renderer::configure(&mut renderer, target);
                Ok(Box::new(renderer))
            }))
            .into_handler(move |cx| {
                zgui::reactive::provide_context(fonts.clone());
                Box::new(zgui::elements::column().into_view().build(cx))
            })
            .expect("create pure-shell runtime");
        let cx = GrantingPlatform::new(shell_capabilities(), default_monitors());
        runtime.surfaces_available(&cx);
        let mut host = SessionHost::new(
            pending,
            SessionModule::package(package),
            SessionPolicy::live(host_services::Executor::new(2, 64).unwrap()),
            RecoveryPolicy::Propagate,
            SurfaceOpacity::Transparent,
        );
        host.set_pure_shell(zgui::app::Fonts::shipped_only());
        let monitors = Monitor::snapshot(&cx.monitors());
        host.set_monitor_snapshot(&monitors);
        host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &monitors, &no_retry())
            .expect("mount pure shell");
        let mut rig = Self { host, runtime, cx };
        rig.settle();
        rig
    }

    fn session(&self) -> Rc<Session> {
        self.host.sessions().next().expect("active session").clone()
    }

    fn surface_id(&self, name: &str) -> SurfaceId {
        self.session()
            .mount_regions()
            .into_iter()
            .find(|(region, _)| region == name)
            .unwrap_or_else(|| panic!("missing mount region {name:?}"))
            .1
            .surface
    }

    fn redraw_all(&mut self) {
        let ids: Vec<SurfaceId> = self.runtime.windows().iter().map(|w| w.surface().id()).collect();
        for id in ids {
            self.host
                .surface_event(&mut self.runtime, &self.cx, id, SurfaceEvent::RedrawRequested)
                .expect("redraw");
        }
    }

    fn pump_wakes(&mut self) {
        loop {
            let wakes = self.cx.headless.drain_wakes();
            if wakes.is_empty() {
                break;
            }
            for reason in wakes {
                match reason {
                    WakeReason::ActivationToken { serial, token } => {
                        self.host.resolve_activation_token(serial, token);
                    }
                    WakeReason::ClipboardRead { serial, result } => {
                        if let Err(result) = self.host.resolve_clipboard_read(serial, result) {
                            self.runtime.wake(&self.cx, WakeReason::ClipboardRead { serial, result });
                        }
                    }
                    other => self.runtime.wake(&self.cx, other),
                }
            }
            let monitors = Monitor::snapshot(&self.cx.monitors());
            if let Err(err) = self.host.turn(
                &mut self.runtime,
                &self.cx,
                ImageChoice::PackageOnly,
                &monitors,
                &no_retry(),
            ) {
                match err {
                    TurnError::Checkpoint(e) => panic!("host turn checkpoint failed: {e}"),
                    TurnError::Shell(e) => panic!("host turn shell failed: {e}"),
                }
            }
        }
    }

    fn settle(&mut self) {
        let monitors = Monitor::snapshot(&self.cx.monitors());
        if let Err(err) = self.host.turn(
            &mut self.runtime,
            &self.cx,
            ImageChoice::PackageOnly,
            &monitors,
            &no_retry(),
        ) {
            match err {
                TurnError::Checkpoint(e) => panic!("initial turn checkpoint failed: {e}"),
                TurnError::Shell(e) => panic!("initial turn shell failed: {e}"),
            }
        }
        self.redraw_all();
        self.pump_wakes();
        self.redraw_all();
    }

    fn send(&mut self, surface: SurfaceId, event: SurfaceEvent) {
        self.host
            .surface_event(&mut self.runtime, &self.cx, surface, event)
            .expect("surface event");
    }

    fn center_of_class(&self, surface: SurfaceId, class: &str) -> Point<CssPx, Css> {
        let window = self
            .runtime
            .windows()
            .iter()
            .find(|w| w.surface().id() == surface)
            .expect("window for surface");
        let bounds = window
            .box_with_class(class)
            .unwrap_or_else(|| panic!("missing settled box for class {class:?}"));
        let device: Point<DevicePx, Device> = Point::new(
            DevicePx(bounds.origin.x.0 + bounds.size.width.0 * 0.5),
            DevicePx(bounds.origin.y.0 + bounds.size.height.0 * 0.5),
        );
        let scale: Scale<Css, Device> = Scale::new(window.surface().scale_factor() as f32);
        device * scale.invert()
    }

    fn click_with_grant(&mut self, surface: SurfaceId, class: &str, serial: u32) {
        let pos = self.center_of_class(surface, class);
        let ts = self.cx.clock().timestamp();
        self.send(
            surface,
            SurfaceEvent::Pointer {
                action: PointerAction::Moved,
                event: PointerEvent::mouse(pos),
                modifiers: Modifiers::NONE,
                timestamp: ts,
            },
        );
        self.send(
            surface,
            SurfaceEvent::InputGrant(InputGrant::new(1, surface, InputSerial::new(serial))),
        );
        self.send(
            surface,
            SurfaceEvent::Pointer {
                action: PointerAction::Pressed,
                event: PointerEvent::mouse(pos).with_button(PointerButton::Primary),
                modifiers: Modifiers::NONE,
                timestamp: ts,
            },
        );
        self.send(
            surface,
            SurfaceEvent::Pointer {
                action: PointerAction::Released,
                event: PointerEvent::mouse(pos).with_button(PointerButton::Primary),
                modifiers: Modifiers::NONE,
                timestamp: ts,
            },
        );
        self.settle();
    }

    fn key_with_grant(
        &mut self,
        surface: SurfaceId,
        key: Key,
        serial: Option<u32>,
        repeat: bool,
        modifiers: Modifiers,
    ) {
        let ts = self.cx.clock().timestamp();
        if let Some(raw) = serial {
            self.send(
                surface,
                SurfaceEvent::InputGrant(InputGrant::new(1, surface, InputSerial::new(raw))),
            );
        }
        let make_ev = |k: &Key| match k {
            Key::Named(named) => KeyEvent::named(*named, PhysicalKey::Unidentified(0)),
            Key::Character(ch) => KeyEvent::character(ch.clone()),
            _ => unreachable!("unsupported test key"),
        };
        let mut ev = make_ev(&key);
        ev.repeat = repeat;
        self.send(
            surface,
            SurfaceEvent::Key {
                state: KeyState::Pressed,
                event: ev,
                modifiers,
                timestamp: ts,
            },
        );
        let up = make_ev(&key);
        self.send(
            surface,
            SurfaceEvent::Key {
                state: KeyState::Released,
                event: up,
                modifiers,
                timestamp: ts,
            },
        );
        self.settle();
    }

    fn report_json(&self, surface: SurfaceId, class: &str) -> serde_json::Value {
        let window = self
            .runtime
            .windows()
            .iter()
            .find(|w| w.surface().id() == surface)
            .expect("window");
        let nodes = window.nodes_with_class(class);
        assert_eq!(nodes.len(), 1, "expected one .{class} node");
        let region = self
            .session()
            .region_for_surface(surface)
            .expect("region for surface");
        let text = region.region.dom.text_content(nodes[0]);
        serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("invalid JSON report {text:?}: {err}"))
    }
}

impl Drop for ActivationRig {
    fn drop(&mut self) {
        let mut surface = RuntimeSurface::new(&mut self.runtime, &self.cx, ImageChoice::PackageOnly);
        self.host.stop(&mut surface);
        self.runtime.shutting_down(&self.cx);
        zgui::reactive::shutdown();
    }
}

fn build_tsx_package(tsx_source: &str, capabilities_json: &str) -> Fixture {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("zgui-act-tsx-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let entry_path = temp.join("entry.tsx");
    let out_js = temp.join("bundle.mjs");
    std::fs::write(&entry_path, tsx_source).unwrap();
    let script = r#"
        import { writeFile } from "node:fs/promises";
        import { bundle } from "./packages/build/src/build.ts";
        const entry = process.env.ZGUI_TEST_ENTRY;
        const out = process.env.ZGUI_TEST_OUT;
        const built = await bundle(entry);
        await writeFile(out, built.files.get("app.mjs"));
        const styles = [];
        for (const [name, buf] of built.files.entries()) {
            if (name.endsWith(".css")) styles.push({ css: buf.toString("utf8") });
        }
        await writeFile(out + ".css", JSON.stringify(styles), "utf8");
    "#;
    let output = Command::new("bun")
        .args(["-e", script])
        .env("ZGUI_TEST_ENTRY", &entry_path)
        .env("ZGUI_TEST_OUT", &out_js)
        .current_dir(root)
        .output()
        .expect("run bun bundler");
    assert!(
        output.status.success(),
        "bun bundle failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let code = std::fs::read_to_string(&out_js).unwrap();
    let styles_raw: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(format!("{}.css", out_js.display())).unwrap())
            .unwrap();
    let _ = std::fs::remove_dir_all(&temp);
    let owned_styles: Vec<(String, String)> = styles_raw
        .into_iter()
        .enumerate()
        .map(|(idx, s)| {
            let css = s["css"].as_str().unwrap_or_default().to_owned();
            (format!("s{idx}.css"), css)
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = owned_styles
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    if capabilities_json.trim() == "{}" {
        Fixture::with_styles(&code, &borrowed)
    } else {
        let caps: Capabilities = serde_json::from_str(capabilities_json).unwrap();
        Fixture::with_styles(&code, &borrowed).with_capabilities(caps)
    }
}

#[test]
fn headless_degrades_to_null_immediately_without_scheduling_timers() {
    let fixture = Fixture::new(
        r#"
        export const hostApiVersion = 1;
        let settled = 'unset';
        export function mount(root) {
            const btn = __zgui.createElement('control');
            __zgui.setProperty(btn, 'class', 'act-btn');
            __zgui.setProperty(btn, 'tabIndex', 0);
            __zgui.setProperty(btn, 'onClick', () => {
                __zguiShell.requestActivationToken({ appId: 'org.gnome.Calculator' }).then(tok => {
                    settled = String(tok);
                });
            });
            const label = __zgui.createTextNode('btn');
            __zgui.insertNode(btn, label, null);
            __zgui.insertNode(root, btn, null);
            return () => __zgui.clearRoot(root);
        }
        export function assertNull() {
            if (settled !== 'null') throw new Error('expected null in headless, got ' + settled);
        }
        "#,
        ".act-btn { width: 80px; height: 32px; }",
    );

    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let fonts = zgui::app::Fonts::shipped_only();
    let mut runtime = zgui::runtime::App::new()
        .with_size(200.0, 120.0)
        .with_renderer(Box::new(|_, target| {
            let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
            Renderer::configure(&mut renderer, target);
            Ok(Box::new(renderer))
        }))
        .into_handler(move |cx| {
            zgui::reactive::provide_context(fonts.clone());
            Box::new(Root(roots.clone()).build(cx))
        })
        .unwrap();
    let cx = zgui_platform_headless::Headless::new();
    runtime.surfaces_available(&cx);
    let mut host = SessionHost::new(
        pending,
        SessionModule::package(fixture.package()),
        SessionPolicy::live(host_services::Executor::new(2, 64).unwrap()),
        RecoveryPolicy::Propagate,
        SurfaceOpacity::Opaque,
    );
    host.attach(
        &mut RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly),
        cx.waker(),
        no_retry(),
    )
    .unwrap();
    let surface = runtime.windows()[0].surface().id();
    host.surface_event(&mut runtime, &cx, surface, SurfaceEvent::RedrawRequested)
        .unwrap();
    let idle_before = runtime.idle(&cx);
    let now = cx.clock().now();
    let ts = cx.clock().timestamp();
    let pos = Point::new(CssPx(20.0), CssPx(16.0));
    host.surface_event(
        &mut runtime,
        &cx,
        surface,
        SurfaceEvent::InputGrant(InputGrant::new(1, surface, InputSerial::new(55))),
    )
    .unwrap();
    host.surface_event(
        &mut runtime,
        &cx,
        surface,
        SurfaceEvent::Pointer {
            action: PointerAction::Pressed,
            event: PointerEvent::mouse(pos).with_button(PointerButton::Primary),
            modifiers: Modifiers::NONE,
            timestamp: ts,
        },
    )
    .unwrap();
    host.surface_event(
        &mut runtime,
        &cx,
        surface,
        SurfaceEvent::Pointer {
            action: PointerAction::Released,
            event: PointerEvent::mouse(pos).with_button(PointerButton::Primary),
            modifiers: Modifiers::NONE,
            timestamp: ts,
        },
    )
    .unwrap();
    assert!(
        host.turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[], &no_retry())
            .is_ok()
    );
    host.surface_event(&mut runtime, &cx, surface, SurfaceEvent::RedrawRequested)
        .unwrap();
    let idle_after = runtime.idle(&cx);
    assert_ne!(
        idle_after,
        IdlePolicy::BlockUntil(now + Duration::from_millis(500)),
        "headless null fallback must not schedule a 500 ms AppTimers timeout"
    );
    assert_eq!(
        idle_after, idle_before,
        "headless null fallback must leave idle scheduling unchanged"
    );
    let session = host.sessions().next().unwrap().clone();
    session.invoke("assertNull").unwrap();
    host.stop(&mut RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly));
    runtime.shutting_down(&cx);
    zgui::reactive::shutdown();
}

#[test]
fn real_tsx_components_mint_tokens_across_button_menu_command_palette_and_field_enter() {
    let tsx = r#"
        import {
            createSignal,
            getCapsLock,
            renderApp,
            requestActivationToken,
            setReducedMotion,
            useCapsLock,
            type NativeNode,
        } from "@hewel/zgui";
        import {
            Button,
            CommandPalette,
            CommandPaletteProvider,
            DropdownMenu,
            DropdownMenuContent,
            DropdownMenuTrigger,
            MenuItem,
            ThemeProvider,
        } from "@hewel/zgui/components";
        import {
            MenuItemPrimitive,
            MenuListPrimitive,
            MenuRootPrimitive,
        } from "@hewel/zgui/primitives";

        export const hostApiVersion = 1;

        export function mount(root: NativeNode) {
            return renderApp(() => {
                setReducedMotion(true);
                const [results, setResults] = createSignal<string[]>([]);
                const [paletteOpen, setPaletteOpen] = createSignal(false);
                const caps = useCapsLock();

                const record = (tag: string, p: Promise<string | null>) => {
                    void p.then(tok => setResults(prev => [...prev, `${tag}:${tok ?? "null"}`]));
                };

                // Validate synchronous TypeError contract at runtime too.
                let syncErrors = 0;
                for (const bad of [null, 42, [], () => {}, { appId: 123 }, { appId: "bad\napp" }, { appId: "x".repeat(257) }]) {
                    try {
                        // @ts-expect-error intentional runtime validation check
                        requestActivationToken(bad);
                    } catch (err) {
                        if (err instanceof TypeError) syncErrors += 1;
                    }
                }

                return (
                    <ThemeProvider scheme="dark">
                        <CommandPaletteProvider
                            commands={[
                                {
                                    id: "launch-term",
                                    label: "Launch Terminal",
                                    onSelect: () => {
                                        record("palette", requestActivationToken({ appId: "org.gnome.Ptyxis" }));
                                    },
                                },
                            ]}
                        >
                            <column style="width: 380px; height: 280px; gap: 4px">
                                <Button
                                    class="tsx-btn"
                                    onClick={() => {
                                        record("btn1", requestActivationToken({ appId: "org.mozilla.firefox" }));
                                        // Second call in same gesture must resolve null!
                                        record("btn2", requestActivationToken({ appId: "org.mozilla.firefox" }));
                                    }}
                                >
                                    Launch Browser
                                </Button>

                                <DropdownMenu defaultOpen>
                                    <DropdownMenuTrigger>
                                        <Button class="menu-trigger">Menu</Button>
                                    </DropdownMenuTrigger>
                                    <DropdownMenuContent>
                                        <MenuItem
                                            class="tsx-menu-item"
                                            onSelect={() => {
                                                record("menuitem", requestActivationToken({ appId: "" }));
                                            }}
                                        >
                                            Open Files
                                        </MenuItem>
                                    </DropdownMenuContent>
                                </DropdownMenu>

                                <MenuRootPrimitive defaultOpen>
                                    <MenuListPrimitive style="width: 160px; height: 32px">
                                        <MenuItemPrimitive
                                            class="tsx-prim-item"
                                            style="width: 160px; height: 28px"
                                            onSelect={() => {
                                                record("prim", requestActivationToken());
                                            }}
                                        >
                                            <text>Primitive Action</text>
                                        </MenuItemPrimitive>
                                    </MenuListPrimitive>
                                </MenuRootPrimitive>

                                <field
                                    class="tsx-field"
                                    style="width: 160px; height: 28px"
                                    onKeyDown={ev => {
                                        if (ev.key === "Enter") {
                                            record("field", requestActivationToken({ appId: "dev.zed.Zed" }));
                                        }
                                    }}
                                />

                                <Button class="open-palette" onClick={() => setPaletteOpen(true)}>
                                    Open Palette
                                </Button>

                                <CommandPalette open={paletteOpen()} onOpenChange={setPaletteOpen} />

                                <text class="report">
                                    {JSON.stringify({
                                        syncErrors,
                                        capsReactive: caps(),
                                        capsSnapshot: getCapsLock(),
                                        results: results(),
                                    })}
                                </text>
                            </column>
                        </CommandPaletteProvider>
                    </ThemeProvider>
                );
            }, root);
        }
    "#;

    let fixture = build_tsx_package(tsx, "{}");
    let mut rig = ActivationRig::primary(fixture.package());
    let main = rig.surface_id("main");

    let initial = rig.report_json(main, "report");
    assert_eq!(initial["syncErrors"], 7, "all 7 malformed inputs must throw synchronous TypeError");
    assert_eq!(initial["capsReactive"], false);
    assert_eq!(initial["capsSnapshot"], false);

    // 1. Click MenuItem (uses activateNow() on pointerdown) -> appId: "" normalized to None!
    rig.click_with_grant(main, "tsx-menu-item", 11);
    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 1, "MenuItem press-activate must mint 1 token");
    assert_eq!(recs[0].grant.serial().raw(), 11);
    assert_eq!(recs[0].app_id, None, "empty string appId must normalize to None");
    rig.host
        .resolve_activation_token(recs[0].serial, ActivationToken::new("tok-menu"));
    rig.settle();

    // 2. Click MenuItemPrimitive -> omitted options -> appId None!
    rig.click_with_grant(main, "tsx-prim-item", 12);
    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 1, "MenuItemPrimitive must mint 1 token");
    assert_eq!(recs[0].grant.serial().raw(), 12);
    assert_eq!(recs[0].app_id, None);
    rig.host
        .resolve_activation_token(recs[0].serial, ActivationToken::new("tok-prim"));
    rig.settle();

    // 3. Click Button -> first call mints, second call in same handler resolves null!
    rig.click_with_grant(main, "tsx-btn", 13);
    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 1, "single physical press must mint at most 1 token");
    assert_eq!(recs[0].grant.serial().raw(), 13);
    assert_eq!(recs[0].app_id.as_deref(), Some("org.mozilla.firefox"));
    rig.host
        .resolve_activation_token(recs[0].serial, ActivationToken::new("tok-btn-click"));
    rig.settle();

    // 4. Button still focused -> Space keyboard activation mints a token!
    rig.key_with_grant(main, Key::Named(NamedKey::Space), Some(14), false, Modifiers::NONE);
    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 1, "Space activation on focused Button must mint 1 token");
    assert_eq!(recs[0].grant.serial().raw(), 14);
    rig.host
        .resolve_activation_token(recs[0].serial, ActivationToken::new("tok-btn-space"));
    rig.settle();

    // 5. Focus <field> and press Enter -> onKeyDown mints token!
    rig.click_with_grant(main, "tsx-field", 15);
    rig.key_with_grant(main, Key::Named(NamedKey::Enter), Some(16), false, Modifiers::NONE);
    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 1, "<field onKeyDown={{Enter}}> must mint 1 token");
    assert_eq!(recs[0].grant.serial().raw(), 16);
    assert_eq!(recs[0].app_id.as_deref(), Some("dev.zed.Zed"));
    rig.host
        .resolve_activation_token(recs[0].serial, ActivationToken::new("tok-field"));
    rig.settle();

    // 6. Open CommandPalette (autofocuses search input) and press Enter -> onSelect mints token BEFORE close!
    rig.click_with_grant(main, "open-palette", 17);
    assert!(rig.cx.log.take_records().is_empty());
    rig.key_with_grant(main, Key::Named(NamedKey::Enter), Some(18), false, Modifiers::NONE);
    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 1, "CommandPalette Enter selection must mint 1 token");
    assert_eq!(recs[0].grant.serial().raw(), 18);
    assert_eq!(recs[0].app_id.as_deref(), Some("org.gnome.Ptyxis"));
    rig.host
        .resolve_activation_token(recs[0].serial, ActivationToken::new("tok-palette"));
    rig.settle();

    // 7. Toggle Caps Lock via ModifiersChanged and verify useCapsLock() + getCapsLock() update!
    rig.send(main, SurfaceEvent::ModifiersChanged(Modifiers::CAPS_LOCK));
    rig.settle();

    let final_report = rig.report_json(main, "report");
    assert_eq!(final_report["capsReactive"], true);
    assert_eq!(final_report["capsSnapshot"], true);
    assert_eq!(
        final_report["results"],
        serde_json::json!([
            "menuitem:tok-menu",
            "prim:tok-prim",
            "btn2:null",
            "btn1:tok-btn-click",
            "btn2:null",
            "btn1:tok-btn-space",
            "field:tok-field",
            "palette:tok-palette",
        ])
    );
}

#[test]
fn revocations_denials_interleaved_surfaces_timeout_lock_and_pure_shell_teardown() {
    // Part 1: Staged candidate mount denies requestActivationToken during mount()!
    let staged_pkg = Fixture::new(
        r#"
        export const hostApiVersion = 1;
        let mountToken = 'unset';
        export function mount(root) {
            __zguiShell.requestActivationToken().then(t => { mountToken = String(t); });
            const el = __zgui.createElement('text');
            __zgui.setProperty(el, 'class', 'staged-report');
            __zgui.insertNode(el, __zgui.createTextNode('init'), null);
            __zgui.insertNode(root, el, null);
            return () => __zgui.clearRoot(root);
        }
        export function checkStaged() {
            if (mountToken !== 'null') throw new Error('expected staged mount token null, got ' + mountToken);
        }
        "#,
        "",
    );
    {
        let rig = ActivationRig::primary(staged_pkg.package());
        assert!(rig.cx.log.take_records().is_empty(), "staged mount must never mint");
        rig.session().invoke("checkStaged").unwrap();
    }

    // Part 2: Pure-shell multi-surface interleaved batches, drag revocation, repeat/IME denial,
    // 500 ms AppTimers timeout, mid-flight SessionLock revocation, and last-popup close + retirement!
    let shell_src = r#"
        export const hostApiVersion = 1;
        const log = [];
        let popId = null;
        let dragMode = 'none';
        let syncBarLog = null;

        function pushLog(entry) {
            log.push(entry);
            if (syncBarLog) syncBarLog();
        }

        function placePopup() {
            return {
                parent: 'bar', width: 120, height: 80,
                anchorRect: { x: 4, y: 4, width: 24, height: 24 },
                offset: { x: 0, y: 0 }, anchor: 'bottom-left', gravity: 'bottom-right',
                grab: false, slide: true, flip: true, resize: false, reactive: false,
                geometryInsets: { top: 0, right: 0, bottom: 0, left: 0 },
            };
        }

        export function setDragInternal() { dragMode = 'internal'; }
        export function setDragOutbound() { dragMode = 'outbound'; }
        export function clearDragMode() { dragMode = 'none'; }
        export function clearLog() { log.length = 0; if (syncBarLog) syncBarLog(); }

        export function mount() {
            __zguiShell.openLayer('bar', {
                layer: 'top', anchor: ['top', 'left'], width: 200, height: 48,
                margin: { top: 0, right: 0, bottom: 0, left: 0 }, exclusiveZone: 0, keyboard: 'on-demand',
            }, (root, bridge) => {
                const btn = bridge.createElement('control');
                bridge.setProperty(btn, 'class', 'bar-btn');
                bridge.setProperty(btn, 'tabIndex', 0);
                bridge.setProperty(btn, 'onPointerDown', () => {
                    if (dragMode === 'internal') {
                        const h = __zguiDnd.startInternalDrag(btn, { types: ['text/plain'], payload: 'x' });
                        __zguiShell.requestActivationToken().then(t => pushLog('after-internal-drag:' + t));
                        h.cancel();
                        return;
                    }
                    if (dragMode === 'outbound') {
                        const h = __zguiDnd.startOutboundDrag(btn, { items: [{ mime: 'text/plain', data: 'x' }] });
                        __zguiShell.requestActivationToken().then(t => pushLog('after-outbound-drag:' + t));
                        h.cancel();
                        return;
                    }
                });
                bridge.setProperty(btn, 'onClick', () => {
                    if (dragMode !== 'none') {
                        __zguiShell.requestActivationToken().then(t => pushLog('drag-click:' + t));
                        return;
                    }
                    __zguiShell.requestActivationToken({ appId: 'bar.app' }).then(t => pushLog('bar-click:' + t));
                    Promise.resolve().then(() => {
                        __zguiShell.requestActivationToken().then(t => pushLog('post-await:' + t));
                    });
                    setTimeout(() => {
                        __zguiShell.requestActivationToken().then(t => pushLog('timeout-cb:' + t));
                    }, 0);
                });
                bridge.setProperty(btn, 'onKeyDown', ev => {
                    __zguiShell.requestActivationToken({ appId: 'key.app' }).then(
                        t => pushLog(`key(repeat=${ev.repeat},comp=${ev.isComposing},caps=${ev.modifiers.capsLock},num=${ev.modifiers.numLock}):${t}`)
                    );
                });
                const out = bridge.createElement('text');
                bridge.setProperty(out, 'class', 'log-node');
                const txt = bridge.createTextNode(JSON.stringify(log));
                bridge.insertNode(out, txt, null);
                const mySync = () => bridge.replaceText(txt, JSON.stringify(log));
                syncBarLog = mySync;
                bridge.insertNode(root, btn, null);
                bridge.insertNode(root, out, null);
                return () => {
                    if (syncBarLog === mySync) syncBarLog = null;
                    bridge.clearRoot(root);
                };
            });

            popId = __zguiShell.openPopup('launcher', placePopup(), (root, bridge) => {
                const item = bridge.createElement('control');
                bridge.setProperty(item, 'class', 'pop-btn');
                bridge.setProperty(item, 'tabIndex', 0);
                bridge.setProperty(item, 'onClick', () => {
                    __zguiShell.requestActivationToken({ appId: 'pop.app' }).then(t => pushLog('pop-click:' + t));
                });
                const closeItem = bridge.createElement('control');
                bridge.setProperty(closeItem, 'class', 'pop-close-btn');
                bridge.setProperty(closeItem, 'onClick', () => {
                    __zguiShell.requestActivationToken({ appId: 'closing.app' }).then(t => pushLog('closing:' + t));
                    __zguiShell.closePopup(popId);
                });
                bridge.insertNode(root, item, null);
                bridge.insertNode(root, closeItem, null);
                return () => bridge.clearRoot(root);
            }, () => {});

            return () => {};
        }

        export function outsideGestureCall() {
            __zguiShell.requestActivationToken().then(t => pushLog('outside:' + t));
        }
    "#;

    let pkg = Fixture::new(
        shell_src,
        ".bar-btn { width: 80px; height: 24px; } .pop-btn { width: 80px; height: 24px; } .pop-close-btn { width: 80px; height: 24px; }",
    )
    .with_capabilities(Capabilities {
        shell_surfaces: true,
        session_lock: true,
        ..Default::default()
    });

    let mut rig = ActivationRig::pure_shell(pkg.package());
    let bar = rig.surface_id("bar");
    let pop = rig.surface_id("launcher");

    // 1. Outside gesture -> null!
    rig.session().invoke("outsideGestureCall").unwrap();
    rig.settle();
    assert_eq!(rig.report_json(bar, "log-node"), serde_json::json!(["outside:null"]));
    rig.session().invoke("clearLog").unwrap();

    // 2. Interleaved multi-surface input batch: press+release on `bar` (serial 201) AND press+release on `launcher` (serial 202) before settling!
    let bar_pos = rig.center_of_class(bar, "bar-btn");
    let pop_pos = rig.center_of_class(pop, "pop-btn");
    let ts = rig.cx.clock().timestamp();
    rig.send(bar, SurfaceEvent::InputGrant(InputGrant::new(1, bar, InputSerial::new(201))));
    rig.send(
        bar,
        SurfaceEvent::Pointer {
            action: PointerAction::Pressed,
            event: PointerEvent::mouse(bar_pos).with_button(PointerButton::Primary),
            modifiers: Modifiers::NONE,
            timestamp: ts,
        },
    );
    rig.send(pop, SurfaceEvent::InputGrant(InputGrant::new(1, pop, InputSerial::new(202))));
    rig.send(
        pop,
        SurfaceEvent::Pointer {
            action: PointerAction::Pressed,
            event: PointerEvent::mouse(pop_pos).with_button(PointerButton::Primary),
            modifiers: Modifiers::NONE,
            timestamp: ts,
        },
    );
    rig.send(
        bar,
        SurfaceEvent::Pointer {
            action: PointerAction::Released,
            event: PointerEvent::mouse(bar_pos).with_button(PointerButton::Primary),
            modifiers: Modifiers::NONE,
            timestamp: ts,
        },
    );
    rig.send(
        pop,
        SurfaceEvent::Pointer {
            action: PointerAction::Released,
            event: PointerEvent::mouse(pop_pos).with_button(PointerButton::Primary),
            modifiers: Modifiers::NONE,
            timestamp: ts,
        },
    );
    rig.settle();
    // Advance 10ms for setTimeout(..., 0) to fire too.
    rig.cx.headless.virtual_clock().advance(Duration::from_millis(10));
    rig.runtime.deadline_reached(&rig.cx);
    rig.settle();

    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 2, "interleaved presses on bar and popup must mint 2 independent tokens");
    assert_eq!((recs[0].surface, recs[0].grant.serial().raw(), recs[0].app_id.as_deref()), (bar, 201, Some("bar.app")));
    assert_eq!((recs[1].surface, recs[1].grant.serial().raw(), recs[1].app_id.as_deref()), (pop, 202, Some("pop.app")));

    // Complete recs[0] normally via WakeReason::ActivationToken; let recs[1] time out at 500 ms!
    rig.host
        .resolve_activation_token(recs[0].serial, ActivationToken::new("tok-bar-201"));
    rig.settle();
    assert!(!recs[0].cancelled.load(Ordering::Relaxed));
    assert!(!recs[1].cancelled.load(Ordering::Relaxed));

    rig.cx.headless.virtual_clock().advance(Duration::from_millis(500));
    rig.runtime.deadline_reached(&rig.cx);
    rig.settle();
    assert!(
        recs[1].cancelled.load(Ordering::Relaxed),
        "500 ms AppTimers timeout must cancel PendingActivationToken"
    );

    let batch_log = rig.report_json(bar, "log-node");
    let entries: Vec<String> = serde_json::from_value(batch_log).unwrap();
    assert!(entries.contains(&"post-await:null".to_owned()));
    assert!(entries.contains(&"timeout-cb:null".to_owned()));
    assert!(entries.contains(&"bar-click:tok-bar-201".to_owned()));
    assert!(entries.contains(&"pop-click:null".to_owned()));
    rig.session().invoke("clearLog").unwrap();

    // 3. CancelPressGrants before PointerUp prevents synthesized Click from minting!
    rig.send(bar, SurfaceEvent::InputGrant(InputGrant::new(1, bar, InputSerial::new(203))));
    rig.send(
        bar,
        SurfaceEvent::Pointer {
            action: PointerAction::Pressed,
            event: PointerEvent::mouse(bar_pos).with_button(PointerButton::Primary),
            modifiers: Modifiers::NONE,
            timestamp: ts,
        },
    );
    rig.send(bar, SurfaceEvent::CancelPressGrants);
    rig.send(
        bar,
        SurfaceEvent::Pointer {
            action: PointerAction::Released,
            event: PointerEvent::mouse(bar_pos).with_button(PointerButton::Primary),
            modifiers: Modifiers::NONE,
            timestamp: ts,
        },
    );
    rig.settle();
    assert!(rig.cx.log.take_records().is_empty(), "CancelPressGrants must revoke press grant before release");
    rig.session().invoke("clearLog").unwrap();

    // 4. Drag consumption (startInternalDrag & startOutboundDrag) revokes same-handler grant!
    rig.session().invoke("setDragInternal").unwrap();
    rig.click_with_grant(bar, "bar-btn", 204);
    assert!(rig.cx.log.take_records().is_empty(), "startInternalDrag must revoke active gesture grant");

    rig.session().invoke("setDragOutbound").unwrap();
    rig.cx.headless.offscreen(bar).unwrap().set_live_press(true);
    rig.click_with_grant(bar, "bar-btn", 205);
    rig.cx.headless.offscreen(bar).unwrap().set_live_press(false);
    assert!(rig.cx.log.take_records().is_empty(), "startOutboundDrag must revoke active gesture grant");
    rig.session().invoke("clearDragMode").unwrap();
    rig.session().invoke("clearLog").unwrap();

    // 5. Key repeat, IME Preedit composition, and queue-time CapsLock/NumLock snapshot isolation!
    // Focus bar-btn first:
    rig.click_with_grant(bar, "bar-btn", 206);
    for r in rig.cx.log.take_records() {
        rig.host.resolve_activation_token(r.serial, ActivationToken::new("focus-click"));
    }
    rig.settle();
    rig.session().invoke("clearLog").unwrap();

    // Queue ModifiersChanged(CAPS_LOCK | NUM_LOCK) -> KeyDown('a') -> ModifiersChanged(NONE) in ONE batch before redraw!
    rig.send(bar, SurfaceEvent::ModifiersChanged(Modifiers::CAPS_LOCK | Modifiers::NUM_LOCK));
    rig.send(bar, SurfaceEvent::InputGrant(InputGrant::new(1, bar, InputSerial::new(207))));
    rig.send(
        bar,
        SurfaceEvent::Key {
            state: KeyState::Pressed,
            event: KeyEvent::character("a"),
            modifiers: Modifiers::CAPS_LOCK | Modifiers::NUM_LOCK,
            timestamp: ts,
        },
    );
    rig.send(bar, SurfaceEvent::ModifiersChanged(Modifiers::NONE));
    rig.settle();
    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 1, "non-repeat non-composing keydown must mint");
    rig.host.resolve_activation_token(recs[0].serial, ActivationToken::new("tok-key"));
    rig.settle();

    // Repeat keydown (even if platform sent an InputGrant, Window::key rejects repeat!)
    rig.key_with_grant(bar, Key::Character("a".into()), Some(208), true, Modifiers::NONE);
    assert!(rig.cx.log.take_records().is_empty(), "repeat: true keydown must never mint");

    // IME Preedit composing keydown must never mint!
    rig.send(
        bar,
        SurfaceEvent::Ime(ImeEvent::Preedit {
            text: "ni".into(),
            cursor: Some(2..2),
        }),
    );
    rig.key_with_grant(bar, Key::Character("n".into()), Some(209), false, Modifiers::NONE);
    rig.send(bar, SurfaceEvent::Ime(ImeEvent::Commit("ni".into())));
    rig.settle();
    assert!(rig.cx.log.take_records().is_empty(), "IME composing keydown must never mint");

    let key_log: Vec<String> = serde_json::from_value(rig.report_json(bar, "log-node")).unwrap();
    assert_eq!(
        key_log,
        vec![
            "key(repeat=false,comp=false,caps=true,num=true):tok-key",
            "key(repeat=true,comp=false,caps=false,num=false):null",
            "key(repeat=false,comp=true,caps=false,num=false):null",
        ],
        "queued key event must preserve queue-time capsLock/numLock snapshot and deny repeat/IME"
    );
    rig.session().invoke("clearLog").unwrap();

    // 6. Mid-flight SessionLock retention revokes pending tokens to null and cancels PendingActivationToken
    // EVEN WHEN the session has zero JS requestSessionLock() subscribers!
    rig.click_with_grant(bar, "bar-btn", 210);
    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 1);
    rig.cx.request_session_lock().unwrap();
    rig.settle();
    assert!(
        recs[0].cancelled.load(Ordering::Relaxed),
        "transition to retained SessionLockState must cancel in-flight PendingActivationToken"
    );
    // Unlock so bar and launcher re-attach and display the settled log!
    rig.cx.try_unlock_session().unwrap();
    rig.settle();
    let bar = rig.surface_id("bar");
    let lock_log: Vec<String> = serde_json::from_value(rig.report_json(bar, "log-node")).unwrap();
    assert!(
        lock_log.contains(&"bar-click:null".to_owned()),
        "revoked in-flight token must resolve to null under retained lock: {lock_log:?}"
    );
    rig.session().invoke("clearLog").unwrap();

    // Also verify pre-existing retained lock denies new tokens:
    rig.cx.headless.set_session_lock_decision(zgui_platform_headless::SessionLockDecision::Lock);
    rig.cx.lock_session().unwrap(); // Pending (retained!) while keeping existing surfaces inspectable before Locked suspend
    rig.click_with_grant(bar, "bar-btn", 211);
    assert!(rig.cx.log.take_records().is_empty(), "retained Pending lock must deny new tokens");
    rig.cx.headless.finish_lock();
    rig.settle();
    let bar = rig.surface_id("bar");
    let pop = rig.surface_id("launcher");
    let pending_lock_log: Vec<String> = serde_json::from_value(rig.report_json(bar, "log-node")).unwrap();
    assert!(
        pending_lock_log.contains(&"bar-click:null".to_owned()),
        "new press during Pending lock must resolve null: {pending_lock_log:?}"
    );
    rig.session().invoke("clearLog").unwrap();

    // 7. Same-turn popup close + pure-shell retirement with an in-flight token:
    // Clicking .pop-close-btn mints a token AND calls closePopup(popId) in the same handler,
    // and retiring the session while that token is still in flight produces ZERO E_TEARDOWN leak!
    rig.click_with_grant(pop, "pop-close-btn", 212);
    let recs = rig.cx.log.take_records();
    assert_eq!(recs.len(), 1, "requestActivationToken before same-turn closePopup must mint");
    assert_eq!(recs[0].surface, pop);
    assert!(
        !recs[0].cancelled.load(Ordering::Relaxed),
        "popup component onCleanup/unmount must NOT cancel in-flight activation token!"
    );

    let session = rig.session();
    rig.host.stop(&mut RuntimeSurface::new(
        &mut rig.runtime,
        &rig.cx,
        ImageChoice::PackageOnly,
    ));
    assert!(
        recs[0].cancelled.load(Ordering::Relaxed),
        "whole-Session retirement must cancel remaining in-flight PendingActivationToken"
    );
    let retirement = session.retire();
    assert!(
        retirement.failure.is_none(),
        "retiring session with in-flight activation token must not fault: {:?}",
        retirement.failure
    );
    assert_eq!(
        retirement.counts,
        ResourceCounts::default(),
        "retiring session with in-flight activation token must leak zero resources"
    );
}
