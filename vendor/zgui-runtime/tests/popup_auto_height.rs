//! Native content-driven auto-height settling for pop-up surfaces.

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use zgui_a11y::TreeUpdate;
use zgui_geom::{Css, CssPx, Device, DevicePx, Point, Rect, Size};
use zgui_platform::{
    Anchor, AppHandler, Clipboard, Clock, ColorScheme, Constrain, CursorStyle, Decorations,
    FullscreenMode, InputRegion, LayerPlacement, MonitorInfo, PlatformCapabilities, PlatformCx,
    PlatformError, PopupPlacement, SessionLockState, Surface, SurfaceAttributes, SurfaceEvent,
    SurfaceId, SurfaceRole, TextInput, Waker,
};
use zgui_platform_headless::Headless;
use zgui_reactive::RwSignal;
use zgui_reactive::prelude::{Get, Set};
use zgui_runtime::{App, PopupAutoHeight, WindowContent, WindowSpec};
use zgui_view::{Anchor as ViewAnchor, BuildCx, IntoView, View};

fn popup_placement(parent: SurfaceId) -> PopupPlacement {
    PopupPlacement {
        parent,
        anchor_rect: Rect::new(
            Point::new(CssPx(10.0), CssPx(10.0)),
            Size::new(CssPx(40.0), CssPx(24.0)),
        ),
        anchor: Anchor::BottomLeft,
        gravity: Anchor::BottomRight,
        offset: Point::new(CssPx(0.0), CssPx(4.0)),
        constrain: Constrain::default(),
        grab: None,
        reactive: false,
    }
}

/// Surface wrapper that defers `reconfigure_popup` size changes until the test delivers a resize,
/// modeling Wayland's asynchronous `xdg_popup.reposition` + `configure` round-trip.
struct AsyncPopupSurface {
    inner: Arc<dyn Surface>,
    size: Mutex<Size<DevicePx, Device>>,
    repositioned: Arc<Mutex<Vec<Size<CssPx, Css>>>>,
}

impl Surface for AsyncPopupSurface {
    fn id(&self) -> SurfaceId {
        self.inner.id()
    }
    fn size(&self) -> Size<DevicePx, Device> {
        *self.size.lock().expect("size lock")
    }
    fn scale_factor(&self) -> f64 {
        self.inner.scale_factor()
    }
    fn request_size(&self, size: Size<CssPx, Css>) -> Option<Size<DevicePx, Device>> {
        self.inner.request_size(size)
    }
    fn set_min_size(&self, min: Option<Size<CssPx, Css>>) {
        self.inner.set_min_size(min);
    }
    fn set_max_size(&self, max: Option<Size<CssPx, Css>>) {
        self.inner.set_max_size(max);
    }
    fn request_redraw(&self) {
        self.inner.request_redraw();
    }
    fn pre_present_notify(&self) {
        self.inner.pre_present_notify();
    }
    fn set_visible(&self, visible: bool) {
        self.inner.set_visible(visible);
    }
    fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }
    fn set_resizable(&self, resizable: bool) {
        self.inner.set_resizable(resizable);
    }
    fn set_decorations(&self, decorations: Decorations) {
        self.inner.set_decorations(decorations);
    }
    fn set_maximized(&self, maximized: bool) {
        self.inner.set_maximized(maximized);
    }
    fn set_minimized(&self, minimized: bool) {
        self.inner.set_minimized(minimized);
    }
    fn set_fullscreen(&self, mode: Option<FullscreenMode>) {
        self.inner.set_fullscreen(mode);
    }
    fn set_cursor(&self, cursor: CursorStyle) {
        self.inner.set_cursor(cursor);
    }
    fn set_shell_input_region(&self, region: &InputRegion) -> Result<(), PlatformError> {
        self.inner.set_shell_input_region(region)
    }
    fn reconfigure_layer(
        &self,
        placement: &LayerPlacement,
        size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        self.inner.reconfigure_layer(placement, size)
    }
    fn reconfigure_popup(
        &self,
        _placement: &PopupPlacement,
        size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        self.repositioned.lock().expect("reposition lock").push(size);
        Ok(())
    }
    fn supports_popup_reposition(&self) -> bool {
        true
    }
    fn set_text_input(&self, state: Option<TextInput>) {
        self.inner.set_text_input(state);
    }
    fn push_a11y_update(&self, build: &mut dyn FnMut() -> TreeUpdate) {
        self.inner.push_a11y_update(build);
    }
}

struct AsyncPopupPlatform {
    inner: Headless,
    popup_surfaces: Mutex<Vec<Arc<AsyncPopupSurface>>>,
    repositioned: Arc<Mutex<Vec<Size<CssPx, Css>>>>,
}

impl PlatformCx for AsyncPopupPlatform {
    fn create_surface(&self, attrs: &SurfaceAttributes) -> Result<Arc<dyn Surface>, PlatformError> {
        let is_popup = matches!(attrs.role, SurfaceRole::Popup(_));
        let surface = self.inner.create_surface(attrs)?;
        if !is_popup {
            return Ok(surface);
        }
        let wrapped = Arc::new(AsyncPopupSurface {
            size: Mutex::new(surface.size()),
            inner: surface,
            repositioned: self.repositioned.clone(),
        });
        self.popup_surfaces
            .lock()
            .expect("popup surfaces lock")
            .push(wrapped.clone());
        Ok(wrapped)
    }

    fn destroy_surface(&self, id: SurfaceId) {
        self.inner.destroy_surface(id);
    }
    fn surfaces(&self) -> Vec<Arc<dyn Surface>> {
        self.inner
            .surfaces()
            .into_iter()
            .map(|surface| self.surface(surface.id()).unwrap_or(surface))
            .collect()
    }
    fn surface(&self, id: SurfaceId) -> Option<Arc<dyn Surface>> {
        if let Some(found) = self
            .popup_surfaces
            .lock()
            .expect("popup surfaces lock")
            .iter()
            .find(|s| s.id() == id)
            .cloned()
        {
            return Some(found);
        }
        self.inner.surface(id)
    }
    fn monitors(&self) -> Vec<MonitorInfo> {
        self.inner.monitors()
    }
    fn primary_monitor(&self) -> Option<MonitorInfo> {
        self.inner.primary_monitor()
    }
    fn color_scheme(&self) -> Option<ColorScheme> {
        self.inner.color_scheme()
    }
    fn clipboard(&self) -> &dyn Clipboard {
        self.inner.clipboard()
    }
    fn capabilities(&self) -> &PlatformCapabilities {
        self.inner.capabilities()
    }
    fn clock(&self) -> Arc<dyn Clock> {
        self.inner.clock()
    }
    fn waker(&self) -> Arc<dyn Waker> {
        self.inner.waker()
    }
    fn request_exit(&self) {
        self.inner.request_exit();
    }
    fn is_exiting(&self) -> bool {
        self.inner.is_exiting()
    }
    fn session_lock_state(&self) -> SessionLockState {
        self.inner.session_lock_state()
    }
    fn request_session_lock(&self) -> Result<(), PlatformError> {
        self.inner.request_session_lock()
    }
    fn try_unlock_session(&self) -> Result<(), PlatformError> {
        self.inner.try_unlock_session()
    }
    fn unlock_session(&self) {
        self.inner.unlock_session();
    }
}

#[test]
fn headless_popup_auto_height_settles_on_first_frame_and_tracks_growth_and_clamping() {
    let platform = Headless::new();
    let mut runtime = App::new()
        .with_renderer(Box::new(support::capture))
        .into_handler(|cx| Box::new(zgui_elements::column().into_view().build(cx)))
        .expect("install runtime");
    runtime.surfaces_available(&platform);
    let parent = runtime.windows()[0].surface().id();

    let placement = popup_placement(parent);
    let mut attrs = SurfaceAttributes::popup(placement.clone());
    attrs.size = Some(Size::new(CssPx(320.0), CssPx(600.0)));
    attrs.transparent = true;
    let css = r#"
        :root { display: block; width: 100%; height: 100%; margin: 0; padding: 0; }
        .card { display: flex; flex-direction: column; max-height: 100%; }
        .scroller { display: block; flex: 1; min-height: 0; overflow-y: auto; }
        .body { width: 100%; background: #222; }
    "#;
    let mut content = WindowContent::default();
    content.window_stylesheets.push(css.into());

    let body_height = Rc::new(RefCell::new(None::<RwSignal<u32>>));
    let body_slot = body_height.clone();
    let token = runtime
        .open_surface(
            &platform,
            WindowSpec::new(
                attrs,
                content,
                Box::new(move |cx: &mut BuildCx<'_>| {
                    let h = RwSignal::new(120u32);
                    *body_slot.borrow_mut() = Some(h);
                    let view = zgui_elements::column().class("card").child(
                        zgui_elements::column().class("scroller").child(
                            zgui_elements::column()
                                .class("body")
                                .style_text(move || Some(format!("height: {}px;", h.get()))),
                        ),
                    );
                    Box::new(view.into_view().build(cx)) as Box<dyn ViewAnchor>
                }),
            ),
        )
        .expect("open auto-height popup");
    let popup_id = runtime.surface_id(token).expect("popup surface id");
    runtime
        .set_popup_auto_height(
            token,
            Some(PopupAutoHeight {
                placement: placement.clone(),
                width: CssPx(320.0),
                min_height: CssPx(48.0),
                max_height: CssPx(600.0),
            }),
        )
        .expect("enable auto-height");

    let run_popup_frame = |rt: &mut zgui_runtime::Runtime| {
        rt.surface_event(&platform, popup_id, SurfaceEvent::RedrawRequested);
    };

    // Frame 1 shrinks synchronously from 600px to 120px inside the very first frame.
    run_popup_frame(&mut runtime);
    let surface = platform.offscreen(popup_id).expect("headless popup");
    assert_eq!(surface.size(), Size::new(DevicePx(320.0), DevicePx(120.0)));
    assert_eq!(
        runtime
            .windows()
            .iter()
            .find(|w| w.surface().id() == popup_id)
            .expect("popup window")
            .popup_auto_height_resolved(),
        Some(120.0)
    );

    // Growing content inside `<scroll style="max-height: 100%">` grows the popup to 260px.
    let signal = body_height.borrow().expect("height signal");
    signal.set(260);
    run_popup_frame(&mut runtime);
    assert_eq!(surface.size(), Size::new(DevicePx(320.0), DevicePx(260.0)));

    // Exceeding maxHeight clamps cleanly to 600px.
    signal.set(900);
    run_popup_frame(&mut runtime);
    assert_eq!(surface.size(), Size::new(DevicePx(320.0), DevicePx(600.0)));

    // Shrinking back down contracts the popup to 80px.
    signal.set(80);
    run_popup_frame(&mut runtime);
    assert_eq!(surface.size(), Size::new(DevicePx(320.0), DevicePx(80.0)));

    // Content below minHeight clamps to minHeight (48px).
    signal.set(16);
    run_popup_frame(&mut runtime);
    assert_eq!(surface.size(), Size::new(DevicePx(320.0), DevicePx(48.0)));
}

#[test]
fn fixed_max_height_scroller_does_not_inflate_auto_height_popup() {
    let platform = Headless::new();
    let mut runtime = App::new()
        .with_renderer(Box::new(support::capture))
        .into_handler(|cx| Box::new(zgui_elements::column().into_view().build(cx)))
        .expect("install runtime");
    runtime.surfaces_available(&platform);
    let parent = runtime.windows()[0].surface().id();

    let placement = popup_placement(parent);
    let mut attrs = SurfaceAttributes::popup(placement.clone());
    attrs.size = Some(Size::new(CssPx(320.0), CssPx(600.0)));
    attrs.transparent = true;
    let css = r#"
        :root { display: block; width: 100%; height: 100%; margin: 0; padding: 0; }
        .card { display: flex; flex-direction: column; }
        .header { height: 40px; }
        .capped { display: block; max-height: 150px; overflow-y: auto; }
        .tall { height: 500px; }
    "#;
    let mut content = WindowContent::default();
    content.window_stylesheets.push(css.into());
    let token = runtime
        .open_surface(
            &platform,
            WindowSpec::new(
                attrs,
                content,
                Box::new(|cx| {
                    let view = zgui_elements::column()
                        .class("card")
                        .child(zgui_elements::column().class("header"))
                        .child(
                            zgui_elements::column()
                                .class("capped")
                                .child(zgui_elements::column().class("tall")),
                        );
                    Box::new(view.into_view().build(cx))
                }),
            ),
        )
        .expect("open popup with capped scroller");
    let popup_id = runtime.surface_id(token).expect("popup surface id");
    runtime
        .set_popup_auto_height(
            token,
            Some(PopupAutoHeight {
                placement,
                width: CssPx(320.0),
                min_height: CssPx(48.0),
                max_height: CssPx(600.0),
            }),
        )
        .expect("enable auto-height");

    runtime.surface_event(&platform, popup_id, SurfaceEvent::RedrawRequested);
    let surface = platform.offscreen(popup_id).expect("headless popup");
    assert_eq!(surface.size(), Size::new(DevicePx(320.0), DevicePx(190.0)));
    // Second frame at 190px must stay at 190px and never bounce up to 540px.
    runtime.surface_event(&platform, popup_id, SurfaceEvent::RedrawRequested);
    assert_eq!(surface.size(), Size::new(DevicePx(320.0), DevicePx(190.0)));
}

#[test]
fn async_repositionable_translucent_popup_suppresses_first_paint_until_repositioned() {
    let frames = Arc::new(Mutex::new(Vec::new()));
    let frames_ref = frames.clone();
    let repositioned = Arc::new(Mutex::new(Vec::new()));
    let platform = AsyncPopupPlatform {
        inner: Headless::new(),
        popup_surfaces: Mutex::new(Vec::new()),
        repositioned: repositioned.clone(),
    };
    let mut runtime = App::new()
        .with_renderer(Box::new(move |surface, target| {
            if surface.supports_popup_reposition() {
                Ok(Box::new(support::FrameRecorder::new(
                    target,
                    frames_ref.clone(),
                )))
            } else {
                support::capture(surface, target)
            }
        }))
        .into_handler(|cx| Box::new(zgui_elements::column().into_view().build(cx)))
        .expect("install runtime");
    runtime.surfaces_available(&platform);
    let parent = runtime.windows()[0].surface().id();

    let placement = popup_placement(parent);
    let mut attrs = SurfaceAttributes::popup(placement.clone());
    attrs.size = Some(Size::new(CssPx(320.0), CssPx(600.0)));
    attrs.transparent = true;
    let mut content = WindowContent::default();
    content.window_stylesheets.push(
        ":root { display: block; width: 100%; height: 100%; } .panel { height: 120px; background: #336699; }".into(),
    );

    let token = runtime
        .open_surface(
            &platform,
            WindowSpec::new(
                attrs,
                content,
                Box::new(|cx| Box::new(zgui_elements::column().class("panel").into_view().build(cx))),
            ),
        )
        .expect("open async auto-height popup");
    let popup_id = runtime.surface_id(token).expect("popup surface id");
    runtime
        .set_popup_auto_height(
            token,
            Some(PopupAutoHeight {
                placement,
                width: CssPx(320.0),
                min_height: CssPx(48.0),
                max_height: CssPx(600.0),
            }),
        )
        .expect("enable auto-height");

    // Frame 1: surface is still 600px because the compositor hasn't replied yet.
    // Must request reposition(320, 120) and commit an empty transparent frame (0 quads).
    runtime.surface_event(&platform, popup_id, SurfaceEvent::RedrawRequested);
    assert_eq!(
        *repositioned.lock().expect("reposition log"),
        vec![Size::new(CssPx(320.0), CssPx(120.0))],
        "Frame 1 must request xdg_popup.reposition to the measured 120px height"
    );
    assert_eq!(
        *frames.lock().expect("frames log"),
        vec![support::DrawnFrame {
            viewport_height: 600,
            quad_count: 0,
        }],
        "Frame 1 must emit zero quads so the popup never flashes at maxHeight"
    );

    // Compositor replies with Configure(320, 120).
    let popup_surface = platform
        .popup_surfaces
        .lock()
        .expect("popup surfaces")
        .first()
        .expect("async popup surface")
        .clone();
    let settled = Size::new(DevicePx(320.0), DevicePx(120.0));
    *popup_surface.size.lock().expect("size lock") = settled;
    runtime.surface_event(&platform, popup_id, SurfaceEvent::Resized(settled));
    runtime.surface_event(&platform, popup_id, SurfaceEvent::RedrawRequested);

    let logged = frames.lock().expect("frames log");
    assert_eq!(logged.len(), 2, "Frame 2 paints at the repositioned height");
    assert_eq!(logged[1].viewport_height, 120, "Frame 2 viewport height is 120px");
    assert!(logged[1].quad_count > 0, "Frame 2 paints the real popup content");
}
