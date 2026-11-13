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
    Hiding,
    Occluded,
    Visible,
    Finished,
}

struct NativeAsync {
    inner: Application,
    stage: Stage,
    deadline: Option<Instant>,
    placement: Option<Placement>,
    settled_at: Option<Instant>,
    probe_at: Option<Instant>,
    failures: Rc<RefCell<Vec<String>>>,
    completed: Rc<Cell<bool>>,
}

struct Placement { window: u64, original_workspace: u64, hidden_workspace: u64 }

fn niri_query(kind: &str) -> TestResult<serde_json::Value> {
    let output = Command::new("niri").args(["msg", "--json", kind]).output()?;
    if !output.status.success() { return Err(format!("niri {kind}: {}", String::from_utf8_lossy(&output.stderr)).into()); }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn move_owned_window(window: u64, workspace: u64) -> TestResult {
    use std::io::{BufRead, BufReader, Write};
    // CLI indexes are relative to an output and can change. IPC IDs identify
    // exactly the previously selected workspace without moving user focus.
    let mut socket = std::os::unix::net::UnixStream::connect(
        std::env::var_os("NIRI_SOCKET").ok_or("NIRI_SOCKET is absent")?)?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    serde_json::to_writer(&mut socket, &serde_json::json!({
        "Action": { "MoveWindowToWorkspace": {
            "window_id": window, "reference": { "Id": workspace }, "focus": false
        }}
    }))?;
    socket.write_all(b"\n")?;
    let mut line = String::new();
    BufReader::new(socket).read_line(&mut line)?;
    let reply: serde_json::Value = serde_json::from_str(&line)?;
    if reply != serde_json::json!({"Ok": "Handled"}) {
        return Err(format!("cannot move owned gate window: {reply}").into());
    }
    Ok(())
}

impl Placement {
    fn hide_mapped_window() -> TestResult<Option<Self>> {
        let windows = niri_query("windows")?;
        let owned: Vec<_> = windows.as_array().ok_or("niri windows is not an array")?.iter()
            .filter(|window| window["app_id"].as_str() == Some("dev.zgui.js.native-async")
                && window["pid"].as_u64() == Some(u64::from(std::process::id()))).collect();
        if owned.is_empty() { return Ok(None); }
        if owned.len() != 1 { return Err(format!("expected one verified owned gate window, got {}", owned.len()).into()); }
        let window = owned[0]["id"].as_u64().ok_or("window ID missing")?;
        let original_workspace = owned[0]["workspace_id"].as_u64().ok_or("window workspace missing")?;
        let workspaces = niri_query("workspaces")?;
        let workspaces = workspaces.as_array().ok_or("niri workspaces is not an array")?;
        let original = workspaces.iter().find(|workspace| workspace["id"].as_u64() == Some(original_workspace)).ok_or("original workspace missing")?;
        let empty = workspaces.iter().find(|workspace| workspace["id"].as_u64() != Some(original_workspace)
            && workspace["output"] == original["output"] && workspace["is_active"].as_bool() == Some(false)
            && workspace.get("active_window_id").is_some_and(serde_json::Value::is_null)).ok_or("no inactive empty workspace on the same output")?;
        let hidden_workspace = empty["id"].as_u64().ok_or("empty workspace ID missing")?;
        move_owned_window(window, hidden_workspace)?;
        eprintln!("async-native-placement: owned-window={window} original={original_workspace} hidden-workspace={hidden_workspace}");
        let placement = Self { window, original_workspace, hidden_workspace };
        Ok(Some(placement))
    }

    fn verify_hidden(&self) -> TestResult {
        let windows = niri_query("windows")?;
        let window = windows.as_array().ok_or("niri windows is not an array")?.iter()
            .find(|window| window["id"].as_u64() == Some(self.window)).ok_or("owned window disappeared")?;
        let workspaces = niri_query("workspaces")?;
        let hidden = workspaces.as_array().ok_or("niri workspaces is not an array")?.iter()
            .find(|workspace| workspace["id"].as_u64() == Some(self.hidden_workspace)).ok_or("hidden workspace disappeared")?;
        if window["workspace_id"].as_u64() != Some(self.hidden_workspace) || hidden["is_active"].as_bool() != Some(false) {
            return Err(format!("owned window is not on its inactive target workspace: window={window}, target={hidden}").into());
        }
        Ok(())
    }

    fn restore(&self) -> TestResult { move_owned_window(self.window, self.original_workspace) }
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
            return Err("a JS-bound native frame ran before actual compositor occlusion".into());
        }
        let text = self.text()?;
        if text != INITIAL_TEXT {
            return Err(format!("unexpected initial native text: {text:?}").into());
        }
        let window = self.inner.native.windows().first().ok_or("async window is absent")?;
        if window.box_with_class("promise-layout-row").is_some() {
            return Err("Promise row has a native box before its frame callback".into());
        }
        if !window.is_occluded() { return Err("compositor has not occluded the owned test window".into()); }
        self.stage = Stage::Occluded;
        eprintln!("async-native-occluded: initial native text={text:?}");
        self.observe(cx, false)
    }

    fn observe(&mut self, cx: &dyn PlatformCx, native_frame: bool) -> TestResult {
        if matches!(self.stage, Stage::Mounting | Stage::Hiding | Stage::Finished) || cx.is_exiting() {
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
                    self.stage = Stage::Visible;
                    self.placement.as_ref().ok_or("original window placement is absent")?.restore()?;
                    // Only the compositor's real visible edge releases rAF.
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
            Stage::Mounting | Stage::Hiding | Stage::Visible | Stage::Finished => {}
        }
        Ok(())
    }
}

impl AppHandler for NativeAsync {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        self.deadline = Some(cx.clock().now() + Duration::from_secs(20));
        // Mount JS only after real compositor occlusion has settled. Mapping does
        // not require keyboard focus or an initial Occluded(false) notification.
        self.inner.native.surfaces_available(cx);
    }

    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) {
        self.inner.surfaces_lost(cx);
        if self.stage != Stage::Finished { self.stop(cx, "real compositor surfaces were lost"); }
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        if self.stage == Stage::Finished { return; }
        if matches!(self.stage, Stage::Mounting | Stage::Hiding) {
            let visibility_edge = matches!(event, SurfaceEvent::Occluded(_));
            if !event.is_input() { eprintln!("async-native-startup: stage={:?} surface={surface:?} event={event:?}", self.stage); }
            self.inner.native.surface_event(cx, surface, event);
            if self.stage == Stage::Hiding && visibility_edge {
                self.settled_at = self.inner.native.windows().first().filter(|window| window.is_occluded())
                    .map(|_| cx.clock().now() + Duration::from_millis(200));
                self.probe_at = self.settled_at.is_none().then(|| cx.clock().now() + Duration::from_millis(50));
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
        if matches!(self.stage, Stage::Mounting | Stage::Hiding) {
            self.inner.native.wake(cx, reason);
        } else {
            self.inner.wake(cx, reason);
        }
        if let Err(error) = self.observe(cx, false) { self.stop(cx, error); }
    }

    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy {
        if self.stage == Stage::Finished { return IdlePolicy::Block; }
        if self.stage == Stage::Mounting {
            match Placement::hide_mapped_window() {
                Ok(Some(placement)) => {
                    self.placement = Some(placement);
                    self.stage = Stage::Hiding;
                    self.probe_at = Some(cx.clock().now() + Duration::from_millis(50));
                }
                Ok(None) => {}
                Err(error) => self.stop(cx, error),
            }
        }
        let policy = self.inner.idle(cx);
        // Native idle can itself service an already-due maintenance deadline.
        if let Err(error) = self.observe(cx, false) { self.stop(cx, error); }
        let policy = self.settled_at.map_or(policy, |deadline| policy.merge(IdlePolicy::BlockUntil(deadline)));
        let policy = self.probe_at.map_or(policy, |deadline| policy.merge(IdlePolicy::BlockUntil(deadline)));
        self.deadline.map_or(policy, |deadline| policy.merge(IdlePolicy::BlockUntil(deadline)))
    }

    fn deadline_reached(&mut self, cx: &dyn PlatformCx) {
        if self.stage == Stage::Finished { return; }
        if matches!(self.stage, Stage::Mounting | Stage::Hiding) {
            self.inner.native.deadline_reached(cx);
        } else {
            self.inner.deadline_reached(cx);
        }
        if self.stage == Stage::Hiding && self.probe_at.is_some_and(|deadline| cx.clock().now() >= deadline) {
            // The empty bootstrap has no animation. Give the backend bounded frame
            // demand so its missed-callback occlusion detector can observe silence.
            if let Some(window) = self.inner.native.windows().first() { window.request_frame(); }
            self.probe_at = Some(cx.clock().now() + Duration::from_millis(50));
        }
        if self.stage == Stage::Hiding && self.settled_at.is_some_and(|deadline| cx.clock().now() >= deadline) {
            self.settled_at = None;
            if self.inner.native.windows().first().is_some_and(|window| window.is_occluded()) {
                let result = self.placement.as_ref().ok_or("window placement is absent").map_err(Into::into)
                    .and_then(Placement::verify_hidden);
                if let Err(error) = result { self.stop(cx, error); return; }
                self.inner.after_native(cx);
                if !cx.is_exiting() && let Err(error) = self.occlude(cx) { self.stop(cx, error); }
            }
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
#[ignore = "moves only its owned niri window; requires an unlocked desktop, Node dependencies and GPU"]
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
    let failures = Rc::new(RefCell::new(Vec::new()));
    let completed = Rc::new(Cell::new(false));
    let application = Application::new(native, pending, Some(package))?;
    let failure = application.failure.clone();
    let driver = NativeAsync {
        inner: application, stage: Stage::Mounting, deadline: None, placement: None, settled_at: None, probe_at: None,
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
