//! Real compositor regression for named-output startup and input-free IPC delivery.
use super::*;
use crate::{Root, Session, package::Package, session::ResourceCounts};
use std::{cell::{Cell, RefCell}, path::Path, rc::Rc};

struct Proof {
    host: LayerHost,
    until: Option<Instant>,
    live: Rc<Cell<bool>>,
    sessions: Rc<RefCell<Vec<Rc<Session>>>>,
    lifecycle: bool,
}
impl Proof {
    fn inspect(&mut self) {
        for session in self.host.inner.sessions.values() {
            if self.sessions.borrow().iter().any(|seen| seen.generation == session.generation) { continue; }
            let text = session.region.dom.text_content(session.region.root);
            if text.contains("Live") && text.contains("Workspaces (") && text.contains("Windows (") {
                assert_eq!(session.counts().subscriptions, 1);
                assert!(!text.contains("Waiting for initial"));
                self.sessions.borrow_mut().push(session.clone());
                self.live.set(true);
                eprintln!("panel-live-native-tree: generation={}; one subscription; real workspace/window rows; no input", session.generation);
                if self.lifecycle && self.sessions.borrow().len() >= 2 {
                    self.until = Some(Instant::now() + Duration::from_secs(3));
                }
            }
        }
    }
}
impl AppHandler for Proof {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        self.until = Some(cx.clock().now() + Duration::from_secs(if self.lifecycle { 40 } else { 5 }));
        self.host.surfaces_available(cx);
        self.inspect();
    }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) { self.host.surfaces_lost(cx); }
    fn surface_event(&mut self, cx: &dyn PlatformCx, id: SurfaceId, event: SurfaceEvent) {
        assert!(!matches!(event, SurfaceEvent::Focused(true)), "noninteractive panel acquired focus");
        self.host.surface_event(cx, id, event);
        self.inspect();
    }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) { self.host.wake(cx, reason); self.inspect(); }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy {
        self.until.map_or(self.host.idle(cx), |at| self.host.idle(cx).merge(IdlePolicy::BlockUntil(at)))
    }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) {
        self.host.deadline_reached(cx);
        self.inspect();
        if self.until.is_some_and(|at| cx.clock().now() >= at) { cx.request_exit(); }
    }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) { self.host.shutting_down(cx); }
}

#[test]
#[ignore = "requires real niri/Wayland and a prebuilt panel package; opens a noninteractive layer"]
fn real_panel_layer() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let config = LayerConfig::load(&root.join("apps/examples/panel/layer.json"))?;
    let package = Rc::new(Package::load(&root.join("apps/examples/panel/dist"))?);
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let native = zgui::app().with_application_id(config.namespace.as_str())
        .into_handler(move || Root(roots.clone()))?.into_runtime();
    let native_failure = native.failure();
    let inner = Application::new(native, pending, Some(package))?;
    let failure = inner.failure.clone();
    let live = Rc::new(Cell::new(false));
    let sessions = Rc::new(RefCell::new(Vec::new()));
    let lifecycle = std::env::var("ZGUI_SHELL_LIFECYCLE").as_deref() == Ok("1");
    let proof = Proof { host: LayerHost::new(inner, config), until: None, live: live.clone(), sessions: sessions.clone(), lifecycle };
    let result = zgui::app::desktop()(Box::new(proof));
    zgui::reactive::shutdown();
    result?;
    assert!(native_failure.take().is_none());
    assert!(failure.borrow().is_none(), "{:?}", failure.borrow());
    assert!(live.get(), "real niri snapshot did not reach native TSX rows");
    if lifecycle { assert!(sessions.borrow().len() >= 2, "output return did not mount a fresh live generation"); }
    for session in sessions.borrow().iter() { assert_eq!(session.counts(), ResourceCounts::default()); }
    Ok(())
}
