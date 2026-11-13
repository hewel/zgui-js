//! Real compositor gate; run with:
//! ZGUI_PLATFORM=wayland cargo test -p native-host async_tests::native_async_delivery -- --ignored --exact --nocapture

use super::*;
use std::{cell::Cell, path::Path, process::Command, time::Instant};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const INITIAL_TEXT: &str = "Async statusTimer: pendingFrame: pendingPromise: pendingService: waiting for timer";
const OCCLUDED_TEXT: &str = "Async statusTimer: firedFrame: pendingPromise: pendingService: Rust task completed";
const FINAL_TEXT: &str = "Async statusTimer: firedFrame: firedPromise: layout updatedService: Rust task completedPromise layout row";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Mounting,
    Occluded,
    Visible,
    Finished,
}

struct NativeAsync {
    inner: Application,
    stage: Stage,
    deadline: Option<Instant>,
    failures: Rc<RefCell<Vec<String>>>,
    completed: Rc<Cell<bool>>,
}

impl NativeAsync {
    fn session(&self) -> TestResult<Rc<Session>> {
        if self.inner.sessions.len() != 1 {
            return Err(format!("expected one async session, found {}", self.inner.sessions.len()).into());
        }
        self.inner.sessions.values().next().cloned().ok_or_else(|| "async session is absent".into())
    }

    fn text(&self) -> TestResult<String> {
        let session = self.session()?;
        Ok(session.region.dom.text_content(session.region.root))
    }

    fn stop(&mut self, cx: &dyn PlatformCx, error: impl std::fmt::Display) {
        self.failures.borrow_mut().push(format!("{:?}: {error}", self.stage));
        self.stage = Stage::Finished;
        cx.request_exit();
    }

    fn occlude(&mut self, cx: &dyn PlatformCx) -> TestResult {
        let session = self.session()?;
        if session.frames.get() != 0 {
            return Err("a native frame ran before controlled initial occlusion".into());
        }
        let text = self.text()?;
        if text != INITIAL_TEXT {
            return Err(format!("unexpected initial native text: {text:?}").into());
        }
        let window = self.inner.native.windows().first().ok_or("async window is absent")?;
        if window.box_with_class("promise-layout-row").is_some() {
            return Err("Promise row has a native box before its frame callback".into());
        }
        let surface = window.surface().id();
        // Only visibility is controlled. Timers, worker completion, redraws and JS jobs
        // must all progress through the actual native event loop without input.
        self.inner.surface_event(cx, surface, SurfaceEvent::Occluded(true));
        self.stage = Stage::Occluded;
        eprintln!("async-native-occluded: initial native text={text:?}");
        self.observe(cx, false)
    }

    fn observe(&mut self, cx: &dyn PlatformCx, native_frame: bool) -> TestResult {
        if matches!(self.stage, Stage::Mounting | Stage::Finished) || cx.is_exiting() {
            return Ok(());
        }
        if let Some(error) = self.inner.failure.borrow().as_ref() {
            return Err(error.clone().into());
        }
        let text = self.text()?;
        let window = self.inner.native.windows().first().ok_or("async window is absent")?;
        match self.stage {
            Stage::Occluded => {
                if !window.is_occluded() {
                    return Err("surface became visible before timer and Rust task completed".into());
                }
                if !text.contains("Frame: pending") || !text.contains("Promise: pending")
                    || text.contains("Promise layout row")
                    || window.box_with_class("promise-layout-row").is_some()
                {
                    return Err(format!("frame/Promise work ran while occluded: {text:?}").into());
                }
                if text.contains("Service: failed") {
                    return Err(format!("real packaged asset request failed: {text:?}").into());
                }
                if text.contains("Service: Rust task completed") {
                    if text != OCCLUDED_TEXT {
                        return Err(format!("unexpected completed housekeeping text: {text:?}").into());
                    }
                    eprintln!("async-native-housekeeping: occluded=true text={text:?}");
                    let surface = window.surface().id();
                    self.stage = Stage::Visible;
                    self.inner.surface_event(cx, surface, SurfaceEvent::Occluded(false));
                    // Unocclusion requests a real redraw; never drive a frame manually.
                }
            }
            Stage::Visible if native_frame && text.contains("Frame: fired") => {
                if window.is_occluded() {
                    return Err("first frame reporting Frame: fired is still occluded".into());
                }
                // Inspect immediately after this native frame, not on a later settling
                // frame: a DOM-only continuation can otherwise hide a one-frame lag.
                if text != FINAL_TEXT {
                    return Err(format!("first native frame reporting Frame: fired missed Promise work: {text:?}").into());
                }
                let nodes = window.nodes_with_class("promise-layout-row");
                if nodes.len() != 1 {
                    return Err(format!("first fired frame has {} Promise layout rows", nodes.len()).into());
                }
                let bounds = window.box_with_class("promise-layout-row")
                    .ok_or("Promise row has no native box in the first frame reporting Frame: fired")?;
                let scale = window.scale().get();
                let expected_height = 32.0 * scale;
                if !scale.is_finite() || scale <= 0.0 || !bounds.size.width.0.is_finite()
                    || bounds.size.width.0 <= 0.0 || !bounds.size.height.0.is_finite()
                    || (bounds.size.height.0 - expected_height).abs() > 0.5
                {
                    return Err(format!("first fired frame has invalid Promise box {bounds:?}; expected height=32 CSS px * scale {scale} = {expected_height} device px").into());
                }
                eprintln!("async-native-first-fired-frame: frames={} scale={scale} promise-box={bounds:?} text={text:?}", self.session()?.frames.get());
                self.completed.set(true);
                self.stage = Stage::Finished;
                cx.request_exit();
            }
            Stage::Mounting | Stage::Visible | Stage::Finished => {}
        }
        Ok(())
    }
}

impl AppHandler for NativeAsync {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        self.deadline = Some(cx.clock().now() + Duration::from_secs(20));
        // Consume the compositor's initial configured/visible edge before mounting
        // JS, so it cannot overwrite the controlled occlusion before the first rAF.
        self.inner.native.surfaces_available(cx);
    }

    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) {
        self.inner.surfaces_lost(cx);
        if self.stage != Stage::Finished { self.stop(cx, "real compositor surfaces were lost"); }
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        if self.stage == Stage::Finished { return; }
        if self.stage == Stage::Mounting {
            if !event.is_input() {
                eprintln!("async-native-startup: surface={surface:?} event={event:?}");
            }
            let configured_visible = matches!(event, SurfaceEvent::Occluded(false));
            self.inner.native.surface_event(cx, surface, event);
            if configured_visible && !cx.is_exiting() {
                eprintln!("async-native-startup: actual visible edge consumed; mounting package");
                self.inner.after_native(cx);
                if !cx.is_exiting() {
                    if let Err(error) = self.occlude(cx) { self.stop(cx, error); }
                }
            }
            return;
        }
        let redraw = matches!(event, SurfaceEvent::RedrawRequested);
        let before = self.inner.sessions.values().next().map_or(0, |session| session.frames.get());
        self.inner.surface_event(cx, surface, event);
        let after = self.inner.sessions.values().next().map_or(0, |session| session.frames.get());
        if let Err(error) = self.observe(cx, redraw && after > before) { self.stop(cx, error); }
    }

    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        if self.stage == Stage::Finished { return; }
        if self.stage == Stage::Mounting {
            self.inner.native.wake(cx, reason);
        } else {
            self.inner.wake(cx, reason);
        }
        if let Err(error) = self.observe(cx, false) { self.stop(cx, error); }
    }

    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy {
        if self.stage == Stage::Finished { return IdlePolicy::Block; }
        let policy = self.inner.idle(cx);
        // Native idle can itself service an already-due maintenance deadline.
        if let Err(error) = self.observe(cx, false) { self.stop(cx, error); }
        self.deadline.map_or(policy, |deadline| policy.merge(IdlePolicy::BlockUntil(deadline)))
    }

    fn deadline_reached(&mut self, cx: &dyn PlatformCx) {
        if self.stage == Stage::Finished { return; }
        if self.stage == Stage::Mounting {
            self.inner.native.deadline_reached(cx);
        } else {
            self.inner.deadline_reached(cx);
        }
        if let Err(error) = self.observe(cx, false) { self.stop(cx, error); }
        if self.stage != Stage::Finished && self.deadline.is_some_and(|deadline| cx.clock().now() >= deadline) {
            let text = self.text().unwrap_or_else(|error| format!("<unavailable: {error}>"));
            self.stop(cx, format!("timed out waiting for native async delivery; native text={text:?}"));
        }
    }

    fn shutting_down(&mut self, cx: &dyn PlatformCx) { self.inner.shutting_down(cx); }
}

fn build_fixture(root: &Path) -> TestResult<PathBuf> {
    let output = Command::new("node")
        .arg(root.join("packages/build/src/cli.mjs"))
        .args(["--app", "async"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(format!("async app build failed ({}):\n{}\n{}", output.status,
            String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)).into());
    }
    eprint!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(root.join("apps/examples/async/dist"))
}

#[test]
#[ignore = "opens a real Wayland window; requires Node dependencies and a working GPU/compositor"]
fn native_async_delivery() -> TestResult {
    if std::env::var("ZGUI_PLATFORM").as_deref() != Ok("wayland") {
        return Err("run this gate with ZGUI_PLATFORM=wayland; no headless or portable fallback is accepted".into());
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() || std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        return Err("WAYLAND_DISPLAY and XDG_RUNTIME_DIR must name a real compositor session".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).ok_or("workspace root is absent")?;
    let package = Rc::new(Package::load(&build_fixture(root)?)?);
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let native = zgui::app()
        .with_title("zgui-js native async acceptance gate")
        .with_application_id("dev.zgui.js.native-async")
        .with_size(760.0, 440.0)
        .into_handler(move || Root(roots.clone()))?
        .into_runtime();
    let native_failure = native.failure();
    let failure = Rc::new(RefCell::new(None));
    let failures = Rc::new(RefCell::new(Vec::new()));
    let completed = Rc::new(Cell::new(false));
    let (sender, inbox) = mpsc::sync_channel(8);
    let application = Application {
        native, pending, sessions: HashMap::new(), sender, inbox, exit_after: None,
        exit_armed: false, failure: failure.clone(), package: Some(package), exercise: false, generation: 0,
        executor: host_services::Executor::new(2, 64)?,
    };
    let driver = NativeAsync {
        inner: application, stage: Stage::Mounting, deadline: None,
        failures: failures.clone(), completed: completed.clone(),
    };
    let result = zgui::app::desktop()(Box::new(driver));
    zgui::reactive::shutdown();
    result?;
    if let Some(error) = native_failure.take() { failures.borrow_mut().push(error.to_string()); }
    if let Some(error) = failure.borrow_mut().take() { failures.borrow_mut().push(error); }
    if !completed.get() { failures.borrow_mut().push("native async acceptance gate did not complete".into()); }
    let failures = failures.borrow();
    if !failures.is_empty() { return Err(failures.join("\n").into()); }
    Ok(())
}
