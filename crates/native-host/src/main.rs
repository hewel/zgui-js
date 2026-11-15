mod package;
mod session;
mod session_host;
mod integration;
mod work;
mod assets;
mod environment;
mod schema;
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

use std::{cell::RefCell, error::Error, path::PathBuf, rc::Rc, sync::{mpsc, Arc}, time::Duration};
use js_runtime::{JsRuntime, RuntimeLimits};
use integration::{ExternalWake, IntegrationHarness};
use package::Package;
use session::{SessionModule, SessionPolicy};
use session_host::{ImageChoice, RecoveryPolicy, ReloadOutcome, Root, RuntimeSurface, SessionHost, SessionSurface};
use zgui::{platform::{AppHandler, IdlePolicy, PlatformCx, SurfaceEvent, SurfaceId, WakeReason}, view::DocumentId};

enum Completion {
    Update { document: DocumentId, generation: u64 },
    Retry { document: DocumentId, generation: u64 },
    Exit,
}

struct Application {
    native: zgui::runtime::Runtime,
    host: SessionHost,
    harness: IntegrationHarness,
    sender: mpsc::SyncSender<Completion>,
    inbox: mpsc::Receiver<Completion>,
    exit_after: Option<Duration>,
    exit_armed: bool,
    failure: Rc<RefCell<Option<String>>>,
    watch_path: Option<PathBuf>,
    watcher: Option<watcher::PackageWatcher>,
}
impl Application {
    fn new(native: zgui::runtime::Runtime, pending: Rc<RefCell<Vec<session_host::MountRegion>>>, package: Option<Rc<Package>>) -> std::io::Result<Self> {
        let (sender, inbox) = mpsc::sync_channel(8);
        let module = package.map_or_else(integration::baseline_module, SessionModule::package);
        let host = SessionHost::new(pending, module, SessionPolicy::live(host_services::Executor::new(2, 64)?), RecoveryPolicy::Fallback);
        Ok(Self {
            native, host, harness: IntegrationHarness::new(),
            sender, inbox, exit_after: None, exit_armed: false, failure: Rc::new(RefCell::new(None)),
            watch_path: None, watcher: None,
        })
    }

    fn fail(&self, cx: &dyn PlatformCx, error: impl std::fmt::Display) {
        eprintln!("native-host error: {error}");
        *self.failure.borrow_mut() = Some(error.to_string());
        cx.request_exit();
    }

    fn retry_sender(&self, cx: &dyn PlatformCx) -> Rc<dyn Fn(DocumentId, u64)> {
        let sender = self.sender.clone();
        let waker = cx.waker();
        Rc::new(move |document, generation| {
            match sender.try_send(Completion::Retry { document, generation }) {
                Ok(()) => waker.wake(WakeReason::AppWork),
                Err(error) => eprintln!("retry-not-queued: {error}"),
            }
        })
    }

    fn external_wake(&self, cx: &dyn PlatformCx) -> ExternalWake {
        let sender = self.sender.clone();
        let waker = cx.waker();
        std::sync::Arc::new(move |document, generation, surface| {
            if sender.send(Completion::Update { document, generation }).is_ok() {
                waker.wake(WakeReason::ReactiveWork { surfaces: vec![surface].into_boxed_slice() });
            }
        })
    }

    /// After any host operation, the harness cancels registrations owned by
    /// retired generations and arms new baseline/exercise sessions.
    fn reconcile_harness(&mut self, cx: &dyn PlatformCx) {
        let external = self.external_wake(cx);
        self.harness.reconcile(&self.host, &external);
    }

    fn replace_session(&mut self, cx: &dyn PlatformCx, document: DocumentId, module: SessionModule) -> ReloadOutcome {
        let retry = self.retry_sender(cx);
        let waker = cx.waker();
        let outcome = {
            let mut surface = RuntimeSurface::new(&mut self.native, ImageChoice::Live);
            self.host.replace(&mut surface, waker, document, module, retry)
        };
        self.reconcile_harness(cx);
        outcome
    }

    fn attach(&mut self, cx: &dyn PlatformCx) -> Result<(), Box<dyn Error>> {
        {
            let retry = self.retry_sender(cx);
            let waker = cx.waker();
            let mut surface = RuntimeSurface::new(&mut self.native, ImageChoice::Live);
            self.host.attach(&mut surface, waker, retry)?;
        }
        self.reconcile_harness(cx);
        if self.watcher.is_none() && let Some(path) = &self.watch_path {
            let initial = self.host.module().package.as_ref().ok_or("--watch requires a package")?.directory.clone();
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
        let retry = self.retry_sender(cx);
        {
            let mut surface = RuntimeSurface::new(&mut self.native, ImageChoice::Live);
            if let Err(error) = self.host.checkpoint(&mut surface, retry) { self.fail(cx, error); }
        }
        // Checkpoint faults can retire a generation; cancel its registrations
        // before the event loop continues.
        self.reconcile_harness(cx);
    }

    fn poll_reload(&mut self, cx: &dyn PlatformCx) {
        let Some(publication) = self.watcher.as_ref().and_then(watcher::PackageWatcher::next) else { return; };
        match publication {
            Ok(package) => {
                let package = Rc::new(package);
                // The initial host has one native window. A publication arriving
                // while surfaces are absent is prepared when its root returns.
                let document = self.host.region_documents().next();
                match document {
                    Some(document) => { self.replace_session(cx, document, SessionModule::package(package)); }
                    None => self.host.set_module(SessionModule::package(package)),
                }
            }
            Err(error) => eprintln!("reload-package-rejected: {error}"),
        }
    }

    fn stop_sessions(&mut self) {
        self.harness.stop();
        self.host.stop();
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
                    if let Some(session) = self.host.session(document).filter(|session| session.generation == generation).cloned() {
                        match session.invoke("external") {
                            Ok(()) => RuntimeSurface::new(&mut self.native, ImageChoice::Live).request_frame(document),
                            Err(error) => {
                                session.record_fault(error);
                                let retry = self.retry_sender(cx);
                                {
                                    let mut surface = RuntimeSurface::new(&mut self.native, ImageChoice::Live);
                                    if let Err(error) = self.host.checkpoint(&mut surface, retry) { self.fail(cx, error); }
                                }
                                self.reconcile_harness(cx);
                            }
                        }
                    }
                }
                Completion::Retry { document, generation } => {
                    let retry = self.retry_sender(cx);
                    let waker = cx.waker();
                    let outcome = {
                        let mut surface = RuntimeSurface::new(&mut self.native, ImageChoice::Live);
                        self.host.retry(&mut surface, waker, document, generation, retry)
                    };
                    if outcome.is_some() { self.reconcile_harness(cx); }
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
    application.harness.exercise = exercise;
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
