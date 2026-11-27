//! Outbound Copy/Move drag source lifecycle on a private Wayland socket.

#![cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]

#[path = "support/dnd_wire.rs"]
mod wire;

use wire::run_client;

use zgui_geom::{CssPx, Size};
use zgui_platform::{
    AppHandler, DragAction, DragActions, DragEvent, OutboundDragPayload, PlatformCx,
    SurfaceAttributes, SurfaceEvent, SurfaceId, WakeReason,
};
use zgui_platform_wayland::WaylandApp;
use zgui_vocab::PointerAction;

const CLIENT: &str = "ZGUI_DND_SOURCE_PROTOCOL_CLIENT";
const TEST: &str = "outbound_drag_source_lifecycle_on_the_wire";

#[test]
fn outbound_drag_source_lifecycle_on_the_wire() {
    if let Ok(selected) = std::env::var(CLIENT) {
        let mut app = WaylandApp::new(DragClient::default())
            .expect("connect to the private compositor");
        app.run().expect("Wayland client loop");
        let h = app.handler();
        assert!(h.refused_before_press, "must refuse outbound drag before press");
        let expected = if selected == "2" { DragAction::Move } else { DragAction::Copy };
        assert_eq!(h.finished_action, Some(expected));
        assert!(h.refused_after_release, "must refuse outbound drag after release");
        assert!(h.cancelled_second, "must cancel second outbound drag cleanly");
        return;
    }

    for selected in [1, 2] {
        run_client(CLIENT, &["--exact", TEST], Some(selected));
    }
}


#[derive(Default)]
struct DragClient {
    surface: Option<SurfaceId>,
    press_count: u32,
    refused_before_press: bool,
    finished_action: Option<DragAction>,
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
                    )])
                    .with_allowed_actions(DragActions::CopyMove);
                    surface
                        .begin_outbound_drag(payload)
                        .expect("start Copy/Move drag during live press #1");
                } else if self.press_count == 2 {
                    let payload = OutboundDragPayload::new(vec![(
                        "text/plain".into(),
                        b"second-payload".to_vec(),
                    )]);
                    surface
                        .begin_outbound_drag(payload)
                        .expect("start Copy-only drag during live press #2");
                }
            }
            SurfaceEvent::Drag(DragEvent::OutboundFinished { action }) => {
                if self.press_count == 1 {
                    self.finished_action = Some(action);
                } else {
                    assert_eq!(action, DragAction::None);
                    self.cancelled_second = true;
                    cx.request_exit();
                }
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

