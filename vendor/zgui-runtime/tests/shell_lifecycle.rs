//! A shell application can stay alive, open layers, and schedule work without a primary surface.

mod support;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use zgui_geom::{Css, CssPx, DevicePx, Edges, Point, Size};
use zgui_platform::{
    AppHandler, InputRect, InputRegion, LayerPlacement, MonitorInfo, PlatformCapabilities,
    PlatformCx, SessionLockState, Surface, SurfaceAttributes, SurfaceEvent, SurfaceRole,
};
use zgui_platform_headless::Headless;
use zgui_runtime::{App, WindowContent, WindowSpec};
use zgui_view::{Anchor, BuildCx, IntoView, View};

fn empty(cx: &mut BuildCx<'_>) -> Box<dyn Anchor> {
    Box::new(zgui_elements::column().into_view().build(cx))
}

fn dimensions(width: f32, height: f32) -> Size<CssPx, Css> {
    Size::new(CssPx(width), CssPx(height))
}

fn monitor(name: &str) -> MonitorInfo {
    MonitorInfo::new(
        Point::new(DevicePx(0.0), DevicePx(0.0)),
        Size::new(DevicePx(1080.0), DevicePx(1920.0)),
        1.0,
    )
    .with_name(name)
}

#[test]
fn named_layers_open_update_suspend_and_close_without_a_primary() {
    let platform = Headless::new().with_monitors(vec![monitor("A"), monitor("B")]);
    let mut runtime = App::new()
        .with_renderer(Box::new(support::capture))
        .into_handler(|_| panic!("a pure shell must never construct a primary view"))
        .expect("install the runtime");
    runtime.without_primary_surface();
    runtime.surfaces_available(&platform);
    assert!(
        runtime.primary_surface_attributes_mut().is_none(),
        "a pure shell has no primary specification"
    );
    assert!(
        platform.surfaces().is_empty(),
        "pure-shell startup must not create a platform surface"
    );
    assert_eq!(runtime.monitors().len(), 2, "startup publishes both named monitors");

    let open = |output: &str| {
        let mut attrs = SurfaceAttributes::new(output);
        attrs.size = Some(dimensions(1080.0, 32.0));
        attrs.role = SurfaceRole::Layer(LayerPlacement {
            anchors: Edges::new(true, true, false, true),
            monitor: Some(output.to_owned()),
            ..LayerPlacement::default()
        });
        WindowSpec::new(attrs, WindowContent::default(), Box::new(empty))
    };
    let a = runtime.open_surface(&platform, open("A")).expect("open first layer");
    let b = runtime.open_surface(&platform, open("B")).expect("open second layer");
    let first_id = runtime.surface_id(a).expect("synchronous surface lookup");
    assert_ne!(
        Some(first_id),
        runtime.surface_id(b),
        "each layer must have a distinct platform ID"
    );
    assert_eq!(
        runtime.windows().len(), 2,
        "both layer windows must be live after synchronous open"
    );

    let vertical = LayerPlacement {
        anchors: Edges::new(true, true, true, false),
        monitor: Some("A".to_owned()),
        ..LayerPlacement::default()
    };
    let extent = dimensions(44.0, 1080.0);
    runtime.reconfigure_layer(a, vertical.clone(), extent).expect("flip live layer");
    let first = platform.offscreen(first_id).expect("same offscreen surface");
    assert_eq!(
        first.size(),
        Size::new(DevicePx(44.0), DevicePx(1080.0)),
        "live reconfiguration resizes the same offscreen surface"
    );
    let region = InputRegion::Rects(vec![InputRect { x: 1, y: 2, width: 3, height: 4 }]);
    first.set_input_region(&region).expect("configure input region");
    assert_eq!(
        first.input_region(), region,
        "the offscreen layer retains the configured input rectangle"
    );

    runtime.surfaces_lost(&platform);
    for surface in platform.surfaces() {
        platform.destroy_surface(surface.id());
    }
    assert_eq!(
        runtime.surface_id(a),
        None,
        "a suspended layer token must not resolve to its old ID"
    );
    runtime.surfaces_available(&platform);
    let reopened_id = runtime.surface_id(a).expect("retained layer was recreated");
    assert_ne!(
        first_id, reopened_id,
        "reopening a retained layer creates a new platform surface"
    );
    let reopened = platform.offscreen(reopened_id).expect("reopened layer");
    assert_eq!(
        reopened.requested_attributes().role,
        SurfaceRole::Layer(vertical),
        "the recreated layer keeps its most recent placement"
    );
    assert_eq!(
        reopened.requested_attributes().size,
        Some(extent),
        "the recreated layer keeps its most recent requested extent"
    );

    runtime.close_surface(&platform, a);
    runtime.close_surface(&platform, b);
    assert!(
        runtime.windows().is_empty(),
        "closing both layer tokens removes both runtime windows"
    );
    assert!(
        platform.surfaces().is_empty(),
        "closing both layers destroys their platform surfaces"
    );
    assert!(!platform.is_exiting(), "closing the last shell surface does not quit");
}

#[test]
fn timers_fire_and_cancel_while_no_surface_exists() {
    let platform = Headless::new();
    let mut runtime = App::new().into_handler(empty).expect("install the runtime");
    runtime.without_primary_surface();
    runtime.surfaces_available(&platform);
    let timers = runtime.app_timers();
    let hits = Rc::new(Cell::new(0));
    let due = Duration::from_millis(50);
    let capture = Rc::clone(&hits);
    timers.schedule(due, Rc::new(move || capture.set(capture.get() + 1))).expect("active timer clock");
    let cancelled = timers.schedule(due, Rc::new(|| panic!("cancelled callback ran"))).expect("active timer clock");
    timers.cancel(cancelled);
    assert_eq!(
        runtime.idle(&platform).deadline(),
        Some(platform.clock().now() + due),
        "the live application timer schedules a wake even with no surfaces"
    );
    platform.virtual_clock().advance(due);
    runtime.deadline_reached(&platform);
    assert_eq!(
        hits.get(), 1,
        "the application timer fires exactly once at its deadline"
    );
    assert_eq!(
        runtime.idle(&platform).deadline(),
        None,
        "neither timer leaves a pending deadline"
    );
    assert!(platform.surfaces().is_empty(), "timer delivery does not create a surface");
}

#[test]
fn retiring_or_suspending_a_lock_window_leaves_platform_fallback() {
    let mut capabilities = PlatformCapabilities::none();
    capabilities.session_lock = true;
    let platform = Headless::new()
        .with_capabilities(capabilities)
        .with_monitors(vec![monitor("A")]);
    let mut runtime = App::new()
        .with_renderer(Box::new(support::capture))
        .into_handler(empty)
        .expect("install the runtime");
    runtime.without_primary_surface();
    runtime.surfaces_available(&platform);
    runtime.request_session_lock(&platform).expect("lock the platform");
    assert_eq!(
        runtime.session_lock_state(&platform),
        SessionLockState::Locked,
        "the runtime reports acquisition before mounting lock content"
    );

    let mut attrs = SurfaceAttributes::new("App lock");
    attrs.role = SurfaceRole::Lock("A".to_owned());
    let token = runtime
        .open_surface(&platform, WindowSpec::new(attrs, WindowContent::default(), Box::new(empty)))
        .expect("claim A for app content");
    let id = runtime.surface_id(token).expect("lock window");
    let offscreen = platform.offscreen(id).expect("platform-owned role");
    runtime.surface_event(&platform, id, SurfaceEvent::RedrawRequested);
    assert!(offscreen.lock_fallback_frame().is_none(), "a presented app frame replaces fallback");

    let mut attrs = SurfaceAttributes::new("Replacement lock");
    attrs.role = SurfaceRole::Lock("A".to_owned());
    attrs.replacing_lock = Some(id);
    let replacement = runtime
        .open_surface(
            &platform,
            WindowSpec::new(attrs, WindowContent::default(), Box::new(empty)),
        )
        .expect("stage the replacement while the first window is live");
    assert!(
        offscreen.lock_fallback_frame().is_none(),
        "staging the replacement does not hide the presented predecessor"
    );
    runtime.close_surface(&platform, token);
    let token = replacement;
    let id = runtime.surface_id(token).expect("the promoted lock window");
    let offscreen = platform.offscreen(id).expect("the promoted platform role");

    runtime.surfaces_lost(&platform);
    assert_eq!(runtime.surface_id(token), None, "suspension releases the UI claim");
    assert_eq!(
        platform.session_lock_snapshot().fallback_outputs,
        ["A"],
        "suspension exposes fallback on the retained output"
    );
    runtime.surfaces_available(&platform);
    assert_eq!(
        runtime.surface_id(token),
        Some(id),
        "resume reclaims the platform role without reusing the stale predecessor ID"
    );
    runtime.surface_event(&platform, id, SurfaceEvent::RedrawRequested);
    assert!(
        offscreen.lock_fallback_frame().is_none(),
        "the restored window replaces fallback after presenting again"
    );

    runtime.close_surface(&platform, token);
    assert_eq!(runtime.surface_id(token), None, "closing the window retires its claim");
    offscreen.set_visible(true);
    assert_eq!(
        offscreen.lock_fallback_frame().and_then(|frame| frame.pixel(1079, 1919)),
        Some([0, 0, 0, 255]),
        "the retained role displays fallback instead of the retired window",
    );
    assert_eq!(
        runtime.session_lock_state(&platform),
        SessionLockState::Locked,
        "closing the last lock window does not release the compositor lock"
    );
    assert_eq!(
        platform.session_lock_snapshot().unlock_count,
        0,
        "surface suspension and retirement must not send unlock requests"
    );
    runtime.unlock_session(&platform).expect("explicit unlock");
    assert_eq!(
        runtime.session_lock_state(&platform),
        SessionLockState::Unlocked,
        "explicit unlock releases the retained lock"
    );
    assert_eq!(
        platform.session_lock_snapshot().unlock_count,
        1,
        "only the explicit unlock reaches the compositor"
    );
}
