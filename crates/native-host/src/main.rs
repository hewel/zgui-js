mod package;
mod session;
#[cfg(test)]
mod native_tests;

use std::{cell::RefCell, collections::HashMap, error::Error, path::PathBuf, rc::{Rc, Weak}, sync::mpsc, time::Duration};
use js_runtime::{JsRuntime, RuntimeLimits};
use package::Package;
use session::Session;
use zgui::{platform::{AppHandler, IdlePolicy, PlatformCx, SurfaceEvent, SurfaceId, WakeReason}, runtime::HostBinding, view::{Anchor, BuildCx, DocumentId, DomHandle, ElementName, HostHandle, NodeId, View, host::Repeat}};

struct MountRegion {
    document: DocumentId,
    root: NodeId,
    dom: DomHandle,
    host: HostHandle,
}

struct Root(Rc<RefCell<Vec<MountRegion>>>);
struct RootAnchor(NodeId);
impl View for Root {
    type State = RootAnchor;
    fn build(self, cx: &mut BuildCx<'_>) -> RootAnchor {
        let root = cx.dom().create_element(ElementName::new("column"));
        self.0.borrow_mut().push(MountRegion { document: cx.document(), root, dom: cx.dom_handle(), host: cx.host_handle() });
        RootAnchor(root)
    }
    fn rebuild(self, _state: &mut RootAnchor, _cx: &mut BuildCx<'_>) {}
}
impl Anchor for RootAnchor {
    fn mount(&mut self, dom: &DomHandle, parent: NodeId, before: Option<NodeId>) { dom.insert(parent, self.0, before); }
    fn unmount(&mut self, dom: &DomHandle) { dom.detach(self.0); }
    fn first_node(&self) -> Option<NodeId> { Some(self.0) }
}

struct Binding(Weak<Session>);
impl HostBinding for Binding {
    fn checkpoint(&mut self) {
        if let Some(session) = self.0.upgrade() { session.frame_checkpoint(); }
    }
    fn before_paint(&mut self, _timestamp: zgui::vocab::Timestamp) {
        if let Some(session) = self.0.upgrade() {
            session.frames.set(session.frames.get() + 1);
            if session.frames.get() == 1 { eprintln!("first-native-frame: {}", session.region.dom.text_content(session.region.root)); }
        }
    }
    fn shutting_down(&mut self) { if let Some(session) = self.0.upgrade() { session.shutdown(); } }
}

enum Completion { Update(DocumentId), Exit }
struct Application {
    native: zgui::runtime::Runtime,
    pending: Rc<RefCell<Vec<MountRegion>>>,
    sessions: HashMap<DocumentId, Rc<Session>>,
    sender: mpsc::SyncSender<Completion>,
    inbox: mpsc::Receiver<Completion>,
    exit_after: Option<Duration>,
    exit_armed: bool,
    failure: Rc<RefCell<Option<String>>>,
    package: Option<Rc<Package>>,
    exercise: bool,
    generation: u64,
}
impl Application {
    fn fail(&self, cx: &dyn PlatformCx, error: impl std::fmt::Display) {
        eprintln!("native-host error: {error}");
        *self.failure.borrow_mut() = Some(error.to_string());
        cx.request_exit();
    }
    fn attach(&mut self, cx: &dyn PlatformCx) -> Result<(), Box<dyn Error>> {
        let regions = std::mem::take(&mut *self.pending.borrow_mut());
        for region in regions {
            let document = region.document;
            self.generation = self.generation.checked_add(1).ok_or("session generation exhausted")?;
            let surface = self.native.windows().iter().find(|window| window.document_id() == document).ok_or("mount region has no window")?.surface().id();
            let session = Session::prepare(region, self.package.clone(), self.generation, cx.waker(), surface)?;
            if let Some(window) = self.native.windows_mut().iter_mut().find(|window| window.document_id() == document) {
                window.install_binding(Box::new(Binding(Rc::downgrade(&session))));
            }
            eprintln!("binding-installed: {document:?}");
            session.mount()?;
            if self.package.is_none() || self.exercise {
                let weak = Rc::downgrade(&session);
                let failure = self.failure.clone();
                let remaining = std::cell::Cell::new(if self.exercise { 4 } else { 1 });
                let repeat = if self.exercise { Repeat::Every } else { Repeat::Once };
                session.timer.set(Some(session.region.host.schedule(Duration::from_secs(1), repeat, Rc::new(move || {
                    if let Some(session) = weak.upgrade() {
                        if let Err(error) = session.advance(false) {
                            eprintln!("timer error: {error}");
                            *failure.borrow_mut() = Some(error.to_string());
                            remaining.set(1);
                        }
                        remaining.set(remaining.get() - 1);
                        if remaining.get() == 0 && let Some(timer) = session.timer.take() { session.region.host.cancel_timer(timer); }
                    }
                }))));
            }
            if let Some(window) = self.native.windows().iter().find(|window| window.document_id() == document) { window.request_frame(); }
            if self.package.is_none() {
                let waker = cx.waker();
                let sender = self.sender.clone();
                // Only owned IDs/data and the platform waker cross the thread boundary.
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    if sender.send(Completion::Update(document)).is_ok() {
                        waker.wake(WakeReason::ReactiveWork { surfaces: vec![surface].into_boxed_slice() });
                    }
                });
            }
            self.sessions.insert(document, session);
        }
        self.sessions.retain(|document, _| self.native.windows().iter().any(|window| window.document_id() == *document));
        if !self.exit_armed && let Some(after) = self.exit_after {
            self.exit_armed = true;
            let sender = self.sender.clone();
            let waker = cx.waker();
            std::thread::spawn(move || {
                std::thread::sleep(after);
                if sender.send(Completion::Exit).is_ok() { waker.wake(WakeReason::AppWork); }
            });
        }
        Ok(())
    }
    fn after_native(&mut self, cx: &dyn PlatformCx) {
        if let Err(error) = self.attach(cx) { self.fail(cx, error); }
        for session in self.sessions.values() {
            match session.checkpoint() {
                Ok(true) => {
                    if let Some(window) = self.native.windows().iter().find(|window| window.document_id() == session.region.document) { window.request_frame(); }
                }
                Ok(false) => {}
                Err(error) => self.fail(cx, error),
            }
        }
    }
}
impl AppHandler for Application {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) { self.native.surfaces_available(cx); self.after_native(cx); }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) { self.native.surfaces_lost(cx); self.sessions.clear(); }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        self.native.surface_event(cx, surface, event); self.after_native(cx);
    }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        self.native.wake(cx, reason);
        self.after_native(cx);
        // Delivery runs on wake even when painting is suppressed by occlusion.
        for completion in self.inbox.try_iter().take(8) {
            match completion {
                Completion::Exit => cx.request_exit(),
                Completion::Update(document) => {
                    if let Some(session) = self.sessions.get(&document) {
                        if let Err(error) = session.advance(true) { self.fail(cx, error); }
                        if let Some(window) = self.native.windows().iter().find(|window| window.document_id() == document) { window.request_frame(); }
                    }
                }
            }
        }
    }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy { self.native.idle(cx) }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) { self.native.deadline_reached(cx); self.after_native(cx); }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) { self.native.shutting_down(cx); self.sessions.clear(); }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut exit_after = None;
    let mut bundle: Option<PathBuf> = None;
    let mut exercise = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke-module" => {
                let path = args.next().ok_or("--smoke-module requires a path")?;
                let mut engine = JsRuntime::new(RuntimeLimits::default())?;
                let result = engine.run_smoke_module_file(path)?;
                if result != "solid-smoke:ok" { return Err(format!("unexpected smoke result: {result}").into()); }
                println!("{result}");
                return Ok(());
            }
            "--exit-after-ms" => { exit_after = Some(Duration::from_millis(args.next().ok_or("--exit-after-ms requires milliseconds")?.parse()?)); }
            "--bundle" => { bundle = Some(args.next().ok_or("--bundle requires a directory")?.into()); }
            "--exercise" => { exercise = true; }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    let package = bundle.as_deref().map(Package::load).transpose()?.map(Rc::new);
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let native = zgui::app().with_title(package.as_ref().map_or("zgui-js integration baseline", |package| package.manifest.id.as_str())).with_application_id("dev.zgui.js")
        .with_size(560.0, 180.0)
        .with_stylesheet(":root { background: #171b24; color: #edf2fa; font-family: sans-serif; font-size: 22px; padding: 24px; }")
        .into_handler(move || Root(roots.clone()))?.into_runtime();
    let native_failure = native.failure();
    let failure = Rc::new(RefCell::new(None));
    let (sender, inbox) = mpsc::sync_channel(8);
    let application = Application { native, pending, sessions: HashMap::new(), sender, inbox, exit_after, exit_armed: false, failure: failure.clone(), package, exercise, generation: 0 };
    eprintln!("zgui=80baa3c7 rquickjs=73fcd606 QuickJS-NG=6d46d07d fontique=patched platform-selection={}", std::env::var("ZGUI_PLATFORM").unwrap_or_else(|_| "desktop-auto".into()));
    let result = zgui::app::desktop()(Box::new(application));
    zgui::reactive::shutdown();
    result?;
    if let Some(error) = native_failure.take() { return Err(error.into()); }
    if let Some(error) = failure.borrow_mut().take() { return Err(error.into()); }
    eprintln!("native-host: clean shutdown");
    Ok(())
}
