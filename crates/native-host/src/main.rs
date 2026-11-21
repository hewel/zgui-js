mod bounded_fs;
mod control_cli;
mod package;
mod runtime_capabilities;
mod runtime_paths;
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

use std::{cell::RefCell, collections::BTreeMap, error::Error, path::PathBuf, rc::Rc, sync::{mpsc, Arc}, time::Duration};
use js_runtime::{JsRuntime, RuntimeLimits};
use integration::{ExternalWake, IntegrationHarness};
use package::Package;
use session::{SessionModule, SessionPolicy};
use session_host::{ImageChoice, RecoveryPolicy, ReloadOutcome, Root, RuntimeSurface, SessionHost, SessionSurface, SurfaceOpacity};
use zgui::{platform::{AppHandler, IdlePolicy, PlatformCx, SurfaceEvent, SurfaceId, WakeReason}, view::DocumentId};

enum Completion {
    Update { document: DocumentId, generation: u64 },
    Retry { document: DocumentId, generation: u64 },
    Exit,
}

struct Application {
    native: zgui::runtime::Runtime,
    host: SessionHost,
    control: control_cli::Control,
    harness: IntegrationHarness,
    sender: mpsc::SyncSender<Completion>,
    inbox: mpsc::Receiver<Completion>,
    exit_after: Option<Duration>,
    exit_armed: bool,
    failure: Rc<RefCell<Option<String>>>,
    watch_path: Option<PathBuf>,
    watcher: Option<watcher::PackageWatcher>,
    monitors: Vec<shell::Monitor>,
    layer_primary: bool,
}
impl Application {
    fn new(native: zgui::runtime::Runtime, pending: Rc<RefCell<Vec<session_host::MountRegion>>>, package: Option<Rc<Package>>, socket_bindings: BTreeMap<String, String>) -> std::io::Result<Self> {
        let (sender, inbox) = mpsc::sync_channel(8);
        let module = package.map_or_else(integration::baseline_module, SessionModule::package);
        let failure = Rc::new(RefCell::new(None));
        let teardown_failure = Rc::clone(&failure);
        let policy = SessionPolicy::live(host_services::Executor::new(2, 64)?)
            .with_socket_bindings(socket_bindings)
            .with_teardown_reporter(Rc::new(move |counts| {
                teardown_failure.borrow_mut().get_or_insert_with(|| {
                    format!("E_TEARDOWN: external service resources remained after cleanup: {counts:?}")
                });
            }));
        let host = SessionHost::new(pending, module, policy, RecoveryPolicy::Fallback, SurfaceOpacity::Transparent);
        Ok(Self {
            native, host, harness: IntegrationHarness::new(),
            control: control_cli::Control::default(),
            sender, inbox, exit_after: None, exit_armed: false, failure,
            watch_path: None, watcher: None,
            monitors: Vec::new(),
            layer_primary: false,
        })
    }

    fn fail(&self, cx: &dyn PlatformCx, error: impl std::fmt::Display) {
        self.record_failure(error);
        cx.request_exit();
    }

    fn record_failure(&self, error: impl std::fmt::Display) {
        eprintln!("native-host error: {error}");
        self.failure.borrow_mut().get_or_insert_with(|| error.to_string());
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
        Arc::new(move |document, generation, surface| {
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
            let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live);
            self.host.replace(&mut surface, waker, document, module, retry)
        };
        self.reconcile_harness(cx);
        outcome
    }

    fn attach(&mut self, cx: &dyn PlatformCx) -> Result<(), Box<dyn Error>> {
        {
            let retry = self.retry_sender(cx);
            let waker = cx.waker();
            let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live);
            self.host.attach(&mut surface, waker, retry)?;
        }
        self.reconcile_harness(cx);
        if self.watcher.is_none() && let Some(path) = &self.watch_path {
            let initial = self.host.module().as_package().ok_or("--watch requires a package")?.directory.clone();
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
        if let Some(error) = self.native.failure().take() { self.fail(cx, error); return; }
        if let Err(error) = self.attach(cx) { self.fail(cx, error); return; }
        let retry = self.retry_sender(cx);
        {
            let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live);
            if let Err(error) = self.host.sync_reduced_motion(cx.reduced_motion(), &mut surface) { self.fail(cx, error); return; }
            if let Err(error) = self.host.checkpoint(&mut surface, retry) { self.fail(cx, error); return; }
        }
        self.after_shell(cx);
        // Checkpoint faults can retire a generation; cancel its registrations
        // before the event loop continues.
        self.reconcile_harness(cx);
    }

    fn after_shell(&mut self, cx: &dyn PlatformCx) {
        if cx.is_exiting() || self.failure.borrow().is_some() { return; }
        if let Err(error) = self.host.shell_turn(&mut self.native, cx, ImageChoice::Live, &self.monitors) {
            self.shell_fault(cx, error);
        }
    }

    fn shell_fault(&mut self, cx: &dyn PlatformCx, error: session::SessionError) {
        if cx.session_lock_state().is_retained() {
            eprintln!("session-lock UI fault (compositor lock retained): {error}");
        } else {
            self.fail(cx, error);
        }
    }

    fn poll_reload(&mut self, cx: &dyn PlatformCx) {
        let Some(publication) = self.watcher.as_ref().and_then(watcher::PackageWatcher::next) else { return; };
        match publication {
            Ok(package) => {
                let module = SessionModule::package(Rc::new(package));
                if self.layer_primary && !module.as_package().is_some_and(|package| package.manifest.capabilities.shell_surfaces) {
                    let outcome = ReloadOutcome::Rejected(assets::ContractFailure {
                        code: "E_CAPABILITY", message: "--layer-config requires capabilities.shellSurfaces".into(),
                    }.into());
                    let document = self.host.region_documents().next();
                    self.host.record_outcome(&module, document, &outcome);
                    return;
                }
                if self.host.is_pure_shell() {
                    self.host.replace_shell(&mut self.native, cx, ImageChoice::Live, module);
                    self.after_shell(cx);
                    return;
                }
                let document = self.host.region_documents().next();
                match document {
                    Some(document) => {
                        self.replace_session(cx, document, module);
                        self.after_shell(cx);
                    }
                    None => self.host.set_module(module),
                }
            }
            Err(error) => {
                eprintln!("reload-package-rejected: {error}");
            }
        }
    }

    fn reconcile_control(&mut self, cx: &dyn PlatformCx) {
        if self.failure.borrow().is_some() {
            self.host.hide_lock_ui();
            self.stop_control(cx);
            cx.request_exit();
            return;
        }
        let app_id = self.host.module().as_package()
            .filter(|package| package.manifest.capabilities.control_channel)
            .map(|package| package.manifest.id.as_str())
            .filter(|app_id| self.host.sessions().any(|session| session.control_app_id() == Some(*app_id)));
        if let Some(app_id) = app_id {
            let host = &self.host;
            let sessions = || host.sessions().filter(move |session| session.control_app_id() == Some(app_id));
            let result = self.control.reconcile(app_id, || sessions().map(|session| session.generation), || {
                let waker = cx.waker();
                move || waker.wake(WakeReason::AppWork)
            });
            match result {
                Ok(false) if sessions().any(|session| session.has_control_handler()) => {
                    self.fail(cx, "control handler requires --control-socket or XDG_RUNTIME_DIR");
                }
                Err(error) => self.fail(cx, format_args!("control socket: {error}")),
                Ok(_) => {}
            }
        } else {
            self.stop_control(cx);
        }
    }

    fn drain_control(&self, cx: &dyn PlatformCx) {
        for _ in 0..8 {
            let Some((app_id, generations, request)) = self.control.try_recv() else { return; };
            let mut handlers = self.host.sessions().filter(|session| {
                session.control_app_id() == Some(app_id)
                    && generations.contains(&session.generation)
                    && session.has_control_handler()
            });
            match handlers.next() {
                None => request.reply(Err("no active control handler".into())),
                Some(session) if handlers.any(|other| !Rc::ptr_eq(session, other)) => {
                    request.reply(Err("multiple active control handlers".into()));
                }
                Some(session) => session.offer_control(request),
            }
        }
        cx.waker().wake(WakeReason::AppWork);
    }

    fn stop_control(&mut self, cx: &dyn PlatformCx) {
        if let Err(error) = self.control.stop() {
            self.fail(cx, format_args!("control socket: {error}"));
        }
    }

    fn stop_sessions(&mut self, cx: &dyn PlatformCx) {
        self.harness.stop();
        self.host.stop(&mut RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live));
    }
}
impl AppHandler for Application {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        self.native.surfaces_available(cx);
        self.monitors = shell::Monitor::snapshot(self.native.monitors());
        self.host.set_monitor_snapshot(&self.monitors);
        self.after_native(cx);
        self.reconcile_control(cx);
    }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) {
        self.stop_control(cx);
        self.stop_sessions(cx);
        self.native.surfaces_lost(cx);
        if self.failure.borrow().is_some() { cx.request_exit(); }
    }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        if let Err(error) = self.host.surface_event(&mut self.native, cx, surface, event) {
            self.shell_fault(cx, error);
            self.reconcile_control(cx);
            return;
        }
        self.after_native(cx);
        self.reconcile_control(cx);
    }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        self.native.wake(cx, reason);
        self.after_native(cx);
        if self.failure.borrow().is_none() {
            self.poll_reload(cx);
        }
        if self.failure.borrow().is_some() {
            self.reconcile_control(cx);
            return;
        }
        for _ in 0..8 {
            let Ok(completion) = self.inbox.try_recv() else { break; };
            match completion {
                Completion::Exit => cx.request_exit(),
                Completion::Update { document, generation } => {
                    if let Some(session) = self.host.session(document).filter(|session| session.generation == generation).cloned() {
                        match session.invoke("external") {
                            Ok(()) => RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live).request_frame(document),
                            Err(error) => {
                                session.record_fault(error);
                                let retry = self.retry_sender(cx);
                                {
                                    let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live);
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
                        let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live);
                        self.host.retry(&mut surface, waker, document, generation, retry)
                    };
                    if outcome.is_some() { self.reconcile_harness(cx); }
                }
            }
        }
        // A publication or retry may replace the active generation or revoke
        // its capability. Route only after committing the resulting ownership.
        self.reconcile_control(cx);
        self.drain_control(cx);
    }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy {
        self.native.idle(cx)
    }
    fn monitors_changed(&mut self, cx: &dyn PlatformCx) {
        self.native.monitors_changed(cx);
        self.monitors = shell::Monitor::snapshot(self.native.monitors());
        self.after_native(cx);
        self.reconcile_control(cx);
    }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) {
        self.native.deadline_reached(cx);
        self.after_native(cx);
        self.reconcile_control(cx);
    }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) {
        self.watcher.take();
        if let Err(error) = self.control.close() {
            self.record_failure(format_args!("control socket: {error}"));
        }
        self.stop_sessions(cx);
        self.native.shutting_down(cx);
    }
}

pub(crate) const APPLICATION_STYLESHEET: &str =
    ":root { background: #171b24; color: #edf2fa; font-family: sans-serif; font-size: 22px; padding: 24px; }";
pub(crate) const PURE_SHELL_STYLESHEET: &str =
    ":root { background: transparent; color: #edf2fa; font-family: sans-serif; padding: 0; }";

const USAGE: &str = "\
Usage: native-host [OPTIONS]
       native-host preview <COMMAND> [OPTIONS]
       native-host msg <APP_ID> [ARGS...]

Options:
  -h, --help                   Show this help message and exit
  --bundle <DIR>               Load a built zgui-js application package directory
  --watch                      Watch --bundle for new generations and hot-reload
  --pure-shell                 Start without a primary window (requires shellSurfaces or sessionLock)
  --layer-config <FILE>        Mount primary surface as a Wayland layer-shell surface
  --width <PX>                 Override initial primary window width (logical px, default 1024)
  --height <PX>                Override initial primary window height (logical px, default 720)
  --title <TEXT>               Override initial primary window title
  --reduced-motion             Force reduced motion on for this session
  --max-js-stack-bytes <N>     Override QuickJS stack size limit in bytes (default 1048576)
  --bind-socket <NAME=PATH>    Bind a declared named Unix socket capability
  --control-socket <PATH>      Override control channel Unix socket path
  --exit-after-ms <MS>         Exit cleanly after MS milliseconds
  --exercise                   Run built-in host self-exercise harness
  --smoke-module <PATH>        Run a standalone JS smoke module and exit";

fn parse_window_dim(flag: &str, raw: &str) -> Result<f32, Box<dyn Error>> {
    let value: f32 = raw.parse()?;
    if !value.is_finite() || value <= 0.0 || value > 16384.0 {
        return Err(format!("{flag} must be a positive finite number <= 16384").into());
    }
    Ok(value)
}

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "msg") {
        return control_cli::command(std::env::args_os().skip(2));
    }
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_some_and(|arg| arg == "preview") {
        args.next();
        std::process::exit(i32::from(preview::command(args)));
    }
    let mut exit_after = None;
    let mut bundle: Option<PathBuf> = None;
    let mut exercise = false;
    let mut watch = false;
    let mut reduced_motion = false;
    let mut max_js_stack_bytes: Option<usize> = None;
    let mut width_override: Option<f32> = None;
    let mut height_override: Option<f32> = None;
    let mut title_override: Option<String> = None;
    let mut layer_config = None;
    let mut pure_shell = false;
    let mut control_socket = None;
    let mut socket_bindings = BTreeMap::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
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
            "--pure-shell" => { pure_shell = true; }
            "--reduced-motion" => { reduced_motion = true; }
            "--width" => {
                let raw = args.next().ok_or("--width requires logical pixels")?;
                width_override = Some(parse_window_dim("--width", &raw)?);
            }
            "--height" => {
                let raw = args.next().ok_or("--height requires logical pixels")?;
                height_override = Some(parse_window_dim("--height", &raw)?);
            }
            "--title" => {
                let title = args.next().ok_or("--title requires a window title")?;
                if title.trim().is_empty() || title.chars().any(char::is_control) {
                    return Err("--title must be a non-empty string without control characters".into());
                }
                title_override = Some(title);
            }
            "--max-js-stack-bytes" => {
                let bytes: usize = args.next().ok_or("--max-js-stack-bytes requires a byte count")?.parse()?;
                if bytes == 0 { return Err("--max-js-stack-bytes must be positive".into()); }
                max_js_stack_bytes = Some(bytes);
            }
            "--bind-socket" => {
                let binding = args.next().ok_or("--bind-socket requires name=path")?;
                runtime_capabilities::add_socket_binding(&mut socket_bindings, &binding)?;
            }
            "--control-socket" => {
                if control_socket.is_some() { return Err("--control-socket can only be specified once".into()); }
                control_socket = Some(args.next().ok_or("--control-socket requires a path")?.into());
            }
            "--layer-config" => {
                let path: PathBuf = args.next().ok_or("--layer-config requires a JSON file")?.into();
                layer_config = Some(shell::LayerConfig::load(&path)?);
            }
            _ => return Err(format!("unknown argument: {arg} (run with --help for usage)").into()),
        }
    }
    if watch && bundle.is_none() { return Err("--watch requires --bundle".into()); }
    if layer_config.is_some() && bundle.is_none() { return Err("--layer-config requires --bundle".into()); }
    let package = bundle.as_deref().map(Package::load).transpose()?.map(Rc::new);
    if pure_shell && (exercise || layer_config.is_some()) {
        return Err("--pure-shell cannot be combined with --exercise or --layer-config".into());
    }
    let capabilities = package.as_ref().map(|package| &package.manifest.capabilities);
    if layer_config.is_some() && !capabilities.is_some_and(|capabilities| capabilities.shell_surfaces) {
        return Err("E_CAPABILITY: --layer-config requires capabilities.shellSurfaces".into());
    }
    if pure_shell && !capabilities.is_some_and(|capabilities| capabilities.shell_surfaces || capabilities.session_lock) {
        return Err("E_CAPABILITY: --pure-shell requires capabilities.shellSurfaces or capabilities.sessionLock".into());
    }
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let namespace = layer_config.as_ref().map_or("dev.zgui.js", |config| config.namespace.as_str());
    let stylesheet = if pure_shell || layer_config.as_ref().is_some_and(shell::LayerConfig::is_transparent) {
        PURE_SHELL_STYLESHEET
    } else {
        APPLICATION_STYLESHEET
    };
    let manifest_window = package.as_ref().and_then(|pkg| pkg.manifest.window.as_ref());
    let default_title = package.as_ref().map_or("zgui-js integration baseline", |pkg| pkg.manifest.id.as_str());
    let title = title_override
        .as_deref()
        .or_else(|| manifest_window.and_then(|win| win.title.as_deref()))
        .unwrap_or(default_title);
    let width = width_override
        .or_else(|| manifest_window.and_then(|win| win.width).map(|px| px as f32))
        .unwrap_or(1024.0);
    let height = height_override
        .or_else(|| manifest_window.and_then(|win| win.height).map(|px| px as f32))
        .unwrap_or(720.0);
    let mut app = zgui::app()
        .with_title(title)
        .with_application_id(namespace)
        .with_size(width, height)
        .with_stylesheet(stylesheet);
    if let Some(win) = manifest_window {
        if win.min_width.is_some() || win.min_height.is_some() {
            app = app.with_min_size(win.min_width.unwrap_or(0) as f32, win.min_height.unwrap_or(0) as f32);
        }
        if let Some(resizable) = win.resizable {
            app = app.with_resizable(resizable);
        }
    }
    let mut native = app.into_handler(move || Root(roots.clone()))?.into_runtime();
    let fonts = pure_shell.then(|| native.scope().with(zgui::reactive::use_context::<zgui::app::Fonts>)
        .expect("the native application provides a font collection"));
    if pure_shell { native.without_primary_surface(); }
    let native_failure = native.failure();
    let mut application = Application::new(native, pending, package, socket_bindings)?;
    application.layer_primary = layer_config.is_some();
    if let Some(fonts) = fonts { application.host.set_pure_shell(fonts); }
    if reduced_motion { application.host.set_reduced_motion(true); }
    if let Some(bytes) = max_js_stack_bytes { application.host.set_stack_limit_bytes(bytes); }
    application.exit_after = exit_after;
    application.harness.exercise = exercise;
    application.watch_path = watch.then_some(bundle).flatten();
    application.control.override_path = control_socket;
    let failure = application.failure.clone();
    eprintln!("zgui=80baa3c7 rquickjs=73fcd606 QuickJS-NG=6d46d07d fontique=patched platform-selection={}", std::env::var("ZGUI_PLATFORM").unwrap_or_else(|_| "desktop-auto".into()));
    let handler: Box<dyn AppHandler> = match layer_config {
        Some(config) => Box::new(shell::LayerHost::new(application, config)),
        None => Box::new(application),
    };
    let result = zgui::app::desktop()(handler);
    zgui::reactive::shutdown();
    if let Some(error) = failure.borrow_mut().take() { return Err(error.into()); }
    result?;
    if let Some(error) = native_failure.take() { return Err(error.into()); }
    eprintln!("native-host: clean shutdown");
    Ok(())
}
