mod bounded_fs;
mod control_cli;
mod dev_banner;
mod dev_state;
mod dnd;
mod inspector;
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
    dev_inspector: Option<Rc<inspector::DevInspector>>,
    dev_inspect_path: Option<PathBuf>,
    dev_inspect_server: Option<inspector::DevInspectorServer>,
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
            dev_inspector: None,
            dev_inspect_path: None,
            dev_inspect_server: None,
        })
    }

    fn drain_dev_inspector(&mut self) {
        let Some(inspector) = &self.dev_inspector else { return; };
        if let Some(session) = self.host.sessions().next() {
            inspector.record_checkpoint(
                session.generation,
                session.checkpoint_queue_depth(),
                session.live_telemetry(),
            );
        }
        let Some(server) = &self.dev_inspect_server else { return; };
        while let Some(req) = server.try_recv() {
            let session = self.host.sessions().next().map(AsRef::as_ref);
            let response = match &req.action {
                inspector::InspectAction::Snapshot { include_tree, include_profile } => inspector
                    .capture_snapshot(
                        &self.native,
                        session,
                        *include_tree,
                        *include_profile,
                        req.surface.as_deref(),
                    ),
                inspector::InspectAction::Pick { action, token } => inspector.handle_pick_command(
                    &self.native,
                    session,
                    action,
                    req.surface.as_deref(),
                    token.as_deref(),
                ),
            };
            req.reply(&response);
        }
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
            if let Err(error) = self.host.checkpoint(&mut surface, retry.clone()) { self.fail(cx, error); return; }
            if self.host.drain_clipboard(cx)
                && let Err(error) = self.host.checkpoint(&mut surface, retry)
            {
                self.fail(cx, error);
                return;
            }
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
        if let Some(status) = self.watcher.as_ref().and_then(watcher::PackageWatcher::next_status) {
            let document = self.host.region_documents().next();
            if let Some(document) = document {
                let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live);
                match status {
                    watcher::DevBuildStatus::Failed { title, detail } => {
                        self.host.show_dev_banner(&mut surface, document, &title, &detail);
                    }
                    watcher::DevBuildStatus::Clear => {
                        self.host.clear_dev_banner(&mut surface, document);
                    }
                }
            }
        }
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
                        let outcome = self.replace_session(cx, document, module);
                        let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live);
                        match &outcome {
                            ReloadOutcome::Mounted { .. } => {
                                self.host.clear_dev_banner(&mut surface, document);
                            }
                            ReloadOutcome::Rejected(error) | ReloadOutcome::Fallback(error) => {
                                self.host.show_dev_banner(&mut surface, document, "Reload rejected", &error.to_string());
                            }
                        }
                        self.after_shell(cx);
                    }
                    None => self.host.set_module(module),
                }
            }
            Err(error) => {
                eprintln!("reload-package-rejected: {error}");
                let document = self.host.region_documents().next();
                if let Some(document) = document {
                    let mut surface = RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live);
                    self.host.show_dev_banner(&mut surface, document, "Package rejected", &error);
                }
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
        if let Some(inspector) = &self.dev_inspector {
            inspector.clear_overlay();
        }
        self.harness.stop();
        self.host.stop(&mut RuntimeSurface::new(&mut self.native, cx, ImageChoice::Live));
    }
}
impl AppHandler for Application {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        self.native.surfaces_available(cx);
        self.monitors = shell::Monitor::snapshot(self.native.monitors());
        self.host.set_monitor_snapshot(&self.monitors);
        if self.dev_inspect_server.is_none()
            && let Some(path) = self.dev_inspect_path.clone()
        {
            match inspector::DevInspectorServer::start(path, cx.waker()) {
                Ok(server) => self.dev_inspect_server = Some(server),
                Err(error) => eprintln!("dev-inspector-error: {error}"),
            }
        }
        self.after_native(cx);
        self.drain_dev_inspector();
        self.reconcile_control(cx);
    }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) {
        self.stop_control(cx);
        self.stop_sessions(cx);
        self.native.surfaces_lost(cx);
        if self.failure.borrow().is_some() { cx.request_exit(); }
    }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        if let Some(inspector) = &self.dev_inspector {
            let session = self.host.sessions().next().map(AsRef::as_ref);
            let consumed = match &event {
                SurfaceEvent::Key {
                    state,
                    event: key_ev,
                    modifiers,
                    ..
                } => inspector.handle_key_event(&self.native, session, *state, key_ev, *modifiers),
                SurfaceEvent::Pointer { action, event: ptr_ev, .. } => {
                    inspector.handle_pointer_event(&self.native, session, surface, *action, ptr_ev)
                }
                _ => false,
            };
            if consumed {
                return;
            }
        }
        if let Err(error) = self.host.surface_event(&mut self.native, cx, surface, event) {
            self.shell_fault(cx, error);
            self.reconcile_control(cx);
            return;
        }
        self.after_native(cx);
        self.reconcile_control(cx);
    }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        match reason {
            WakeReason::ClipboardRead { serial, result } => {
                if let Err(result) = self.host.resolve_clipboard_read(serial, result) {
                    self.native.wake(cx, WakeReason::ClipboardRead { serial, result });
                }
            }
            other => self.native.wake(cx, other),
        }
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
        self.drain_dev_inspector();
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
        if let Some(mut server) = self.dev_inspect_server.take() {
            server.stop();
        }
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
       native-host inspect --socket <PATH> [--op snapshot|tree|profile] [--surface <NAME>]
       native-host compile-bytecode --input <JS> --output <QJSB> [--module-name <NAME>]

Options:
  -h, --help                   Show this help message and exit
  -V, --version [--json]       Print native-host version and ABI handshake metadata
  --bundle <DIR>               Load a built zgui-js application package directory
  --watch                      Watch --bundle for new generations and hot-reload
  --dev-inspect <PATH>         Enable dev-only read-only inspector/profiler on Unix socket PATH
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

fn host_platform_id() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{}-{arch}", std::env::consts::OS)
}

fn format_version(json_mode: bool) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let platform = host_platform_id();
    if json_mode {
        serde_json::json!({
            "name": "native-host",
            "version": version,
            "hostApiVersion": package::HOST_API_VERSION,
            "schemaVersion": package::SCHEMA_VERSION,
            "platform": platform,
            "engine": "QuickJS-NG",
            "engineRevision": "6d46d07d",
            "bytecodeFormat": js_runtime::BYTECODE_FORMAT,
            "quickjsAbi": js_runtime::quickjs_abi_tag(),
            "styleProfile": "lightningcss-v1",
        })
        .to_string()
    } else {
        format!(
            "native-host {version} (hostApiVersion={} schemaVersion={} platform={platform})",
            package::HOST_API_VERSION,
            package::SCHEMA_VERSION,
        )
    }
}

fn parse_window_dim(flag: &str, raw: &str) -> Result<f32, Box<dyn Error>> {
    let value: f32 = raw.parse()?;
    if !value.is_finite() || value <= 0.0 || value > 16384.0 {
        return Err(format!("{flag} must be a positive finite number <= 16384").into());
    }
    Ok(value)
}

fn run_compile_bytecode(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut module_name: Option<String> = None;
    let mut source_dir: Option<PathBuf> = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--input" => {
                idx += 1;
                let value = args.get(idx).ok_or("--input requires a JS file path")?;
                input = Some(PathBuf::from(value));
            }
            "--output" => {
                idx += 1;
                let value = args.get(idx).ok_or("--output requires a .qjsb output path")?;
                output = Some(PathBuf::from(value));
            }
            "--module-name" => {
                idx += 1;
                let value = args.get(idx).ok_or("--module-name requires a module identifier")?;
                module_name = Some(value.clone());
            }
            "--source-dir" => {
                idx += 1;
                let value = args.get(idx).ok_or("--source-dir requires a directory path")?;
                source_dir = Some(PathBuf::from(value));
            }
            other => {
                return Err(format!("unknown compile-bytecode option: {other}").into());
            }
        }
        idx += 1;
    }
    let input = input.ok_or("compile-bytecode requires --input <JS>")?;
    let output = output.ok_or("compile-bytecode requires --output <QJSB>")?;
    let name = module_name.unwrap_or_else(|| {
        input
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("app.mjs")
            .to_owned()
    });
    let source = std::fs::read_to_string(&input)?;
    let bytecode =
        js_runtime::compile_module_bytecode_in_dir(&name, &source, source_dir.as_deref())?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = output.with_extension(format!("qjsb.{}.tmp", std::process::id()));
    std::fs::write(&tmp, &bytecode)?;
    if let Err(err) = std::fs::rename(&tmp, &output) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
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
    if args.peek().is_some_and(|arg| arg == "inspect") {
        args.next();
        let rest: Vec<String> = args.collect();
        return inspector::run_cli_inspect(&rest);
    }
    if args.peek().is_some_and(|arg| arg == "compile-bytecode") {
        args.next();
        let rest: Vec<String> = args.collect();
        return run_compile_bytecode(&rest);
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
    let mut dev_inspect_path: Option<PathBuf> = None;
    let mut socket_bindings = BTreeMap::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                let mut json_mode = false;
                for extra in args {
                    match extra.as_str() {
                        "--json" => json_mode = true,
                        other => return Err(format!("unknown argument for --version: {other}").into()),
                    }
                }
                println!("{}", format_version(json_mode));
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
            "--dev-inspect" => {
                if dev_inspect_path.is_some() { return Err("--dev-inspect can only be specified once".into()); }
                dev_inspect_path = Some(args.next().ok_or("--dev-inspect requires a socket path")?.into());
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
    let dev_inspector = dev_inspect_path.as_ref().map(|_| inspector::DevInspector::new());
    if let Some(insp) = &dev_inspector {
        app = app.with_probe(insp.probe());
    }
    let mut native = app.into_handler(move || Root(roots.clone()))?.into_runtime();
    let fonts = pure_shell.then(|| native.scope().with(zgui::reactive::use_context::<zgui::app::Fonts>)
        .expect("the native application provides a font collection"));
    if pure_shell { native.without_primary_surface(); }
    let native_failure = native.failure();
    let mut application = Application::new(native, pending, package, socket_bindings)?;
    application.layer_primary = layer_config.is_some();
    application.dev_inspector = dev_inspector;
    application.dev_inspect_path = dev_inspect_path;
    if let Some(fonts) = fonts { application.host.set_pure_shell(fonts); }
    if reduced_motion { application.host.set_reduced_motion(true); }
    if let Some(bytes) = max_js_stack_bytes { application.host.set_stack_limit_bytes(bytes); }
    if watch {
        application.host.set_dev_state(true);
        if !pure_shell {
            application.host.set_dev_banner_enabled(true);
        }
    }
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

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn version_output_exposes_abi_handshake_fields() {
        let text = format_version(false);
        assert!(text.starts_with("native-host "));
        assert!(text.contains("hostApiVersion=1"));
        assert!(text.contains("schemaVersion=2"));

        let parsed: serde_json::Value = serde_json::from_str(&format_version(true)).expect("valid JSON");
        assert_eq!(parsed["name"], "native-host");
        assert_eq!(parsed["hostApiVersion"], 1);
        assert_eq!(parsed["schemaVersion"], 2);
        assert_eq!(parsed["styleProfile"], "lightningcss-v1");
        assert!(parsed["platform"].as_str().is_some_and(|p| p.contains('-')));
    }
}
