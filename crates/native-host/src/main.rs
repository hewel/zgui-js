mod package;
mod session;
mod work;
mod watcher;
mod fallback;
mod shell;
mod preview;
#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod test_package;
#[cfg(test)]
mod reload_tests;
#[cfg(test)]
mod editing_tests;

use std::{cell::RefCell, collections::HashMap, error::Error, path::PathBuf, rc::{Rc, Weak}, sync::{mpsc, Arc}, time::Duration};
use js_runtime::{JsRuntime, RuntimeLimits};
use package::Package;
use session::Session;
use zgui::{platform::{AppHandler, IdlePolicy, PlatformCx, SurfaceEvent, SurfaceId, WakeReason}, runtime::HostBinding, view::{Anchor, BuildCx, DocumentId, DomHandle, ElementName, HostHandle, NodeId, View, host::Repeat}};

#[derive(Clone)]
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
        // These are editing semantics, not application chrome. Keep them owned
        // by the Rust root across JS generations; application CSS may override.
        let host = cx.host_handle();
        host.install_stylesheet("zgui-js-controls", "field { white-space: pre; } editor { white-space: pre-wrap; }");
        self.0.borrow_mut().push(MountRegion { document: cx.document(), root, dom: cx.dom_handle(), host });
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

enum Completion {
    Update { document: DocumentId, generation: u64 },
    Retry { document: DocumentId, generation: u64 },
    Exit,
}

#[derive(Debug)]
enum ReloadOutcome { Mounted(u64), Rejected(String), Fallback(String) }

fn report_reload(outcome: ReloadOutcome) {
    match outcome {
        ReloadOutcome::Mounted(generation) => eprintln!("reload-mounted: generation={generation}"),
        ReloadOutcome::Rejected(error) => eprintln!("reload-preflight-rejected: {error}"),
        ReloadOutcome::Fallback(error) => eprintln!("reload-fallback: {error}"),
    }
}

struct Recovery { generation: u64, view: fallback::Fallback }
struct Application {
    native: zgui::runtime::Runtime,
    pending: Rc<RefCell<Vec<MountRegion>>>,
    sessions: HashMap<DocumentId, Rc<Session>>,
    regions: HashMap<DocumentId, MountRegion>,
    fallbacks: HashMap<DocumentId, Recovery>,
    sender: mpsc::SyncSender<Completion>,
    inbox: mpsc::Receiver<Completion>,
    exit_after: Option<Duration>,
    exit_armed: bool,
    failure: Rc<RefCell<Option<String>>>,
    package: Option<Rc<Package>>,
    last_good: Option<Rc<Package>>,
    watch_path: Option<PathBuf>,
    watcher: Option<watcher::PackageWatcher>,
    exercise: bool,
    generation: u64,
    executor: host_services::Executor,
}
impl Application {
    fn new(native: zgui::runtime::Runtime, pending: Rc<RefCell<Vec<MountRegion>>>, package: Option<Rc<Package>>) -> std::io::Result<Self> {
        let (sender, inbox) = mpsc::sync_channel(8);
        Ok(Self {
            native, pending, sessions: HashMap::new(), regions: HashMap::new(), fallbacks: HashMap::new(),
            sender, inbox, exit_after: None, exit_armed: false, failure: Rc::new(RefCell::new(None)),
            package, last_good: None, watch_path: None, watcher: None, exercise: false, generation: 0,
            executor: host_services::Executor::new(2, 64)?,
        })
    }

    fn fail(&self, cx: &dyn PlatformCx, error: impl std::fmt::Display) {
        eprintln!("native-host error: {error}");
        *self.failure.borrow_mut() = Some(error.to_string());
        cx.request_exit();
    }

    fn request_frame(&self, document: DocumentId) {
        if let Some(window) = self.native.windows().iter().find(|window| window.document_id() == document) { window.request_frame(); }
    }

    fn clear_fallback(&mut self, document: DocumentId) {
        if let Some(mut previous) = self.fallbacks.remove(&document) { previous.view.clear(); }
    }

    fn show_fallback(&mut self, cx: &dyn PlatformCx, document: DocumentId, generation: u64, diagnostic: &str) {
        if let Some(session) = self.sessions.remove(&document) { session.shutdown(); }
        self.clear_fallback(document);
        let Some(region) = self.regions.get(&document).cloned() else { return; };
        let retry = self.last_good.as_ref().map(|_| {
            let sender = self.sender.clone();
            let waker = cx.waker();
            Rc::new(move || {
                match sender.try_send(Completion::Retry { document, generation }) {
                    Ok(()) => waker.wake(WakeReason::AppWork),
                    Err(error) => eprintln!("retry-not-queued: {error}"),
                }
            }) as Rc<dyn Fn()>
        });
        let view = fallback::Fallback::new(region, diagnostic, retry);
        self.fallbacks.insert(document, Recovery { generation, view });
        self.request_frame(document);
    }

    fn replace_session(&mut self, cx: &dyn PlatformCx, document: DocumentId, package: Option<Rc<Package>>) -> ReloadOutcome {
        let Some(region) = self.regions.get(&document).cloned() else { return ReloadOutcome::Rejected("mount region no longer exists".into()); };
        let Some(surface) = self.native.windows().iter().find(|window| window.document_id() == document).map(|window| window.surface().id()) else {
            return ReloadOutcome::Rejected("mount region has no window".into());
        };
        let Some(generation) = self.generation.checked_add(1) else { return ReloadOutcome::Rejected("session generation exhausted".into()); };
        self.generation = generation;
        // Candidate evaluation owns no native children or stylesheet and cannot
        // create work. Rejection must leave the active session untouched.
        let candidate = match Session::prepare(region, package.clone(), generation, cx.waker(), surface, self.executor.clone(), session::SessionPolicy::Live) {
            Ok(candidate) => candidate,
            Err(error) => {
                let error = error.to_string();
                if !self.sessions.contains_key(&document) { self.show_fallback(cx, document, generation, &error); }
                return ReloadOutcome::Rejected(error);
            }
        };
        if let Some(previous) = self.sessions.remove(&document) { previous.shutdown(); }
        self.clear_fallback(document);
        // Binding replacement itself neither shuts down the old owner nor
        // requests a frame. Both are explicit at this owner-thread boundary.
        let window = self.native.windows_mut().iter_mut().find(|window| window.document_id() == document)
            .expect("pure preflight cannot destroy its native window");
        window.replace_image_policy(package.as_ref().map_or_else(
            zgui::runtime::ImagePolicy::default,
            |package|zgui::runtime::ImagePolicy::package_assets(package.image_assets()),
        ));
        window.install_binding(Box::new(Binding(Rc::downgrade(&candidate))));
        eprintln!("binding-installed: {document:?} generation={generation}");
        if let Err(error) = candidate.mount() {
            candidate.shutdown();
            let error = error.to_string();
            self.show_fallback(cx, document, generation, &error);
            return ReloadOutcome::Fallback(error);
        }
        self.package = package.clone();
        if let Some(package) = package { self.last_good = Some(package); }
        self.arm_harness(cx, &candidate, surface);
        self.sessions.insert(document, candidate);
        self.request_frame(document);
        ReloadOutcome::Mounted(generation)
    }

    fn arm_harness(&self, cx: &dyn PlatformCx, session: &Rc<Session>, surface: SurfaceId) {
        if self.package.is_none() || self.exercise {
            let weak = Rc::downgrade(session);
            let remaining = std::cell::Cell::new(if self.exercise { 4 } else { 1 });
            let repeat = if self.exercise { Repeat::Every } else { Repeat::Once };
            session.timer.set(Some(session.region.host.schedule(Duration::from_secs(1), repeat, Rc::new(move || {
                if let Some(session) = weak.upgrade() {
                    if let Err(error) = session.advance(false) {
                        session.record_fault(format!("exercise callback: {error}"));
                        remaining.set(1);
                    }
                    remaining.set(remaining.get() - 1);
                    if remaining.get() == 0 && let Some(timer) = session.timer.take() { session.region.host.cancel_timer(timer); }
                }
            }))));
        }
        if self.package.is_none() {
            let waker = cx.waker();
            let sender = self.sender.clone();
            let document = session.region.document;
            let generation = session.generation;
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                if sender.send(Completion::Update { document, generation }).is_ok() {
                    waker.wake(WakeReason::ReactiveWork { surfaces: vec![surface].into_boxed_slice() });
                }
            });
        }
    }

    fn attach(&mut self, cx: &dyn PlatformCx) -> Result<(), Box<dyn Error>> {
        let regions = std::mem::take(&mut *self.pending.borrow_mut());
        for region in regions {
            let document = region.document;
            self.regions.insert(document, region);
            let outcome = self.replace_session(cx, document, self.package.clone());
            report_reload(outcome);
        }
        let native = &self.native;
        self.sessions.retain(|document, session| {
            let present = native.windows().iter().any(|window| window.document_id() == *document);
            if !present { session.shutdown(); }
            present
        });
        self.fallbacks.retain(|document, recovery| {
            let present = native.windows().iter().any(|window| window.document_id() == *document);
            if !present { recovery.view.clear(); }
            present
        });
        self.regions.retain(|document, _| native.windows().iter().any(|window| window.document_id() == *document));
        if self.watcher.is_none() && let Some(path) = &self.watch_path {
            let initial = self.package.as_ref().ok_or("--watch requires a package")?.directory.clone();
            let waker = cx.waker();
            self.watcher = Some(watcher::PackageWatcher::start(path.clone(), initial, Arc::new(move || waker.wake(WakeReason::AppWork)))?);
        }
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
        if let Err(error) = self.attach(cx) { self.fail(cx, error); return; }
        let mut faults = Vec::new();
        for session in self.sessions.values() {
            match session.checkpoint() {
                Ok(true) => self.request_frame(session.region.document),
                Ok(false) => {}
                Err(error) => faults.push((session.region.document, session.generation, error.to_string())),
            }
        }
        for (document, generation, error) in faults {
            eprintln!("session-fault: generation={generation} {error}");
            self.show_fallback(cx, document, generation, &error);
        }
    }

    fn poll_reload(&mut self, cx: &dyn PlatformCx) {
        let Some(publication) = self.watcher.as_ref().and_then(watcher::PackageWatcher::next) else { return; };
        match publication {
            Ok(package) => {
                let package = Rc::new(package);
                // The initial host has one native window. A publication arriving
                // while surfaces are absent is prepared when its root returns.
                if let Some(document) = self.regions.keys().next().copied() {
                    let outcome = self.replace_session(cx, document, Some(package));
                    report_reload(outcome);
                } else {
                    self.package = Some(package);
                }
            }
            Err(error) => eprintln!("reload-package-rejected: {error}"),
        }
    }

    fn stop_sessions(&mut self) {
        for session in self.sessions.values() { session.shutdown(); }
        for recovery in self.fallbacks.values_mut() { recovery.view.clear(); }
        self.sessions.clear();
        self.fallbacks.clear();
        self.regions.clear();
    }
}
impl AppHandler for Application {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) { self.native.surfaces_available(cx); self.after_native(cx); }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) { self.stop_sessions(); self.native.surfaces_lost(cx); }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        self.native.surface_event(cx, surface, event); self.after_native(cx);
    }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        self.native.wake(cx, reason);
        self.after_native(cx);
        self.poll_reload(cx);
        for _ in 0..8 {
            let Ok(completion) = self.inbox.try_recv() else { break; };
            match completion {
                Completion::Exit => cx.request_exit(),
                Completion::Update { document, generation } => {
                    if let Some(session) = self.sessions.get(&document).filter(|session| session.generation == generation) {
                        match session.advance(true) {
                            Ok(()) => self.request_frame(document),
                            Err(error) => self.show_fallback(cx, document, generation, &error.to_string()),
                        }
                    }
                }
                Completion::Retry { document, generation } => {
                    if self.fallbacks.get(&document).is_some_and(|recovery| recovery.generation == generation)
                        && let Some(package) = self.last_good.clone()
                    {
                        let outcome = self.replace_session(cx, document, Some(package));
                        report_reload(outcome);
                    }
                }
            }
        }
    }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy { self.native.idle(cx) }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) { self.native.deadline_reached(cx); self.after_native(cx); }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) {
        self.watcher.take();
        self.stop_sessions();
        self.native.shutting_down(cx);
    }
}

pub(crate) const APPLICATION_STYLESHEET:&str =
    ":root { background: #171b24; color: #edf2fa; font-family: sans-serif; font-size: 22px; padding: 24px; }";

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_some_and(|arg| arg == "preview") {
        args.next();
        std::process::exit(i32::from(preview::command(args)));
    }
    let mut exit_after = None;
    let mut bundle: Option<PathBuf> = None;
    let mut exercise = false;
    let mut watch = false;
    let mut layer_config = None;
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
            "--watch" => { watch = true; }
            "--layer-config" => {
                let path: PathBuf = args.next().ok_or("--layer-config requires a JSON file")?.into();
                layer_config = Some(shell::LayerConfig::load(&path)?);
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    if watch && bundle.is_none() { return Err("--watch requires --bundle".into()); }
    if layer_config.is_some() && bundle.is_none() { return Err("--layer-config requires --bundle".into()); }
    let package = bundle.as_deref().map(Package::load).transpose()?.map(Rc::new);
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let namespace = layer_config.as_ref().map_or("dev.zgui.js", |config| config.namespace.as_str());
    let native = zgui::app().with_title(package.as_ref().map_or("zgui-js integration baseline", |package| package.manifest.id.as_str())).with_application_id(namespace)
        .with_size(560.0, 180.0)
        .with_stylesheet(APPLICATION_STYLESHEET)
        .into_handler(move || Root(roots.clone()))?.into_runtime();
    let native_failure = native.failure();
    let mut application = Application::new(native, pending, package)?;
    application.exit_after = exit_after;
    application.exercise = exercise;
    application.watch_path = watch.then_some(bundle).flatten();
    let failure = application.failure.clone();
    eprintln!("zgui=80baa3c7 rquickjs=73fcd606 QuickJS-NG=6d46d07d fontique=patched platform-selection={}", std::env::var("ZGUI_PLATFORM").unwrap_or_else(|_| "desktop-auto".into()));
    let handler: Box<dyn AppHandler> = match layer_config {
        Some(config) => Box::new(shell::LayerHost::new(application, config)),
        None => Box::new(application),
    };
    let result = zgui::app::desktop()(handler);
    zgui::reactive::shutdown();
    result?;
    if let Some(error) = native_failure.take() { return Err(error.into()); }
    if let Some(error) = failure.borrow_mut().take() { return Err(error.into()); }
    eprintln!("native-host: clean shutdown");
    Ok(())
}
