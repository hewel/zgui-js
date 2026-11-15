//! Real compositor gate; run with:
//! ZGUI_PLATFORM=wayland cargo test -p native-host native_tests::native_button_actions -- --ignored --exact --nocapture

use super::*;
use session::Session;
use std::{fs, path::Path, process::Command, time::{SystemTime, UNIX_EPOCH}};
use serde_json::Value;
use zgui::{
    geom::{Css, CssPx, Device, DevicePx, Point, Scale},
    reactive::prelude::GetUntracked,
    vocab::{KeyEvent, KeyState, Modifiers, NamedKey, PhysicalKey, PointerAction, PointerButton, PointerEvent},
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const NORMAL: &[&str] = &[
    "outer-capture:capture", "middle-capture:capture", "primary-capture:target",
    "primary-bubble:target", "middle-bubble:bubble", "outer-bubble:bubble",
];
const GUARDED_CLICK: &[&str] = &[
    "outer-capture:capture", "middle-capture:capture", "middle-bubble:bubble", "outer-bubble:bubble",
];
const DISABLED_ANCESTOR_CLICK: &[&str] = &[
    "outer-capture:capture", "middle-capture:target", "middle-bubble:target", "outer-bubble:bubble",
];


struct NativeControls {
    inner: Application,
    step: usize,
    finished: bool,
    deadline: Option<std::time::Instant>,
    failures: Rc<RefCell<Vec<String>>>,
    completed: Rc<std::cell::Cell<bool>>,
}

impl NativeControls {
    fn session(&self) -> TestResult<Rc<Session>> {
        self.inner.host.sessions().next().cloned().ok_or_else(|| "fixture session is not attached".into())
    }

    fn report(&self) -> TestResult<Value> {
        let session = self.session()?;
        let window = self.inner.native.windows().first().ok_or("fixture window is absent")?;
        let nodes = window.nodes_with_class("report");
        if nodes.len() != 1 { return Err(format!("expected one native report node, found {}", nodes.len()).into()); }
        let text = session.region.dom.text_content(nodes[0]);
        serde_json::from_str(&text).map_err(|error| format!("invalid native tree report {text:?}: {error}").into())
    }

    fn expect(&self, label: &str, mode: u64, primary: u64, guarded: u64, replacement: u64, log: Option<&[&str]>) -> TestResult {
        let actual = self.report()?;
        for (name, expected) in [("mode", mode), ("primary", primary), ("guarded", guarded), ("disabled", 0), ("replacement", replacement)] {
            if actual[name].as_u64() != Some(expected) {
                self.failures.borrow_mut().push(format!("{label}: {name} expected {expected}; native report {actual}"));
            }
        }
        if let Some(expected) = log {
            let expected = serde_json::json!(expected);
            if actual["log"] != expected {
                self.failures.borrow_mut().push(format!("{label}: capture/bubble expected {expected}; native report {actual}"));
            }
        }
        eprintln!("native-button gate {label}: {actual}");
        Ok(())
    }

    fn expect_focus(&self, class: &str) -> TestResult {
        let session = self.session()?;
        let window = self.inner.native.windows().first().ok_or("fixture window is absent")?;
        let nodes = window.nodes_with_class(class);
        if nodes.len() != 1 { return Err(format!("expected one {class} control, found {}", nodes.len()).into()); }
        let actual = session.region.host.focused().get_untracked();
        if actual != Some(nodes[0]) {
            self.failures.borrow_mut().push(format!("step {}: native focus expected {class} ({:?}), got {actual:?}", self.step, nodes[0]));
        }
        Ok(())
    }

    fn send(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) -> TestResult {
        // Exercise the same router/default-action path as compositor-delivered input.
        self.inner.surface_event(cx, surface, event);
        if let Some(error) = self.inner.failure.borrow().as_ref() { return Err(error.clone().into()); }
        Ok(())
    }

    fn click(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, class: &str) -> TestResult {
        let window = self.inner.native.windows().first().ok_or("fixture window is absent")?;
        let bounds = window.box_with_class(class).ok_or_else(|| format!("{class} has no settled native box"))?;
        if bounds.size.width.0 <= 8.0 || bounds.size.height.0 <= 8.0 {
            return Err(format!("{class} has unusable native box {bounds:?}").into());
        }
        // Hit the control's padding rather than its text child, making the control
        // itself the target for the capture-vs-bubble propagation checks.
        let device: Point<DevicePx, Device> = Point::new(
            DevicePx(bounds.origin.x.0 + bounds.size.width.0 - 4.0),
            DevicePx(bounds.origin.y.0 + bounds.size.height.0 - 4.0),
        );
        let to_device: Scale<Css, Device> = Scale::new(window.surface().scale_factor() as f32);
        let position: Point<CssPx, Css> = device * to_device.invert();
        eprintln!("native-button pointer {class}: box={bounds:?} scale={} css={position:?}", to_device.get());
        for action in [PointerAction::Moved, PointerAction::Pressed, PointerAction::Released] {
            let event = PointerEvent::mouse(position).with_button(PointerButton::Primary);
            self.send(cx, surface, SurfaceEvent::Pointer { action, event, modifiers: Modifiers::NONE, timestamp: cx.clock().timestamp() })?;
        }
        Ok(())
    }

    fn key(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, key: NamedKey, repeat: bool) -> TestResult {
        let mut event = KeyEvent::named(key, PhysicalKey::Unidentified(0));
        event.repeat = repeat;
        self.send(cx, surface, SurfaceEvent::Key {
            state: KeyState::Pressed, event: event.clone(), modifiers: Modifiers::NONE, timestamp: cx.clock().timestamp(),
        })?;
        event.repeat = false;
        self.send(cx, surface, SurfaceEvent::Key {
            state: KeyState::Released, event, modifiers: Modifiers::NONE, timestamp: cx.clock().timestamp(),
        })
    }

    fn advance(&self) -> TestResult { crate::integration::advance(&*self.session()?).map_err(Into::into) }

    fn drive(&mut self, cx: &dyn PlatformCx, surface: SurfaceId) -> TestResult {
        match self.step {
            0 => { self.expect("mounted", 0, 0, 0, 0, Some(&[]))?; self.click(cx, surface, "primary")?; }
            1 => { self.expect("pointer exactly once", 0, 1, 0, 0, Some(NORMAL))?; self.expect_focus("primary")?; self.key(cx, surface, NamedKey::Enter, false)?; }
            2 => { self.expect("Enter exactly once", 0, 2, 0, 0, None)?; self.key(cx, surface, NamedKey::Space, false)?; }
            3 => { self.expect("Space exactly once", 0, 3, 0, 0, None)?; self.key(cx, surface, NamedKey::Enter, true)?; self.key(cx, surface, NamedKey::Space, true)?; }
            4 => { self.expect("key repeats ignored", 0, 3, 0, 0, None)?; self.click(cx, surface, "disabled")?; }
            5 => { self.expect("disabled pointer ignored", 0, 3, 0, 0, None)?; self.key(cx, surface, NamedKey::Enter, false)?; self.key(cx, surface, NamedKey::Space, false)?; self.advance()?; }
            6 => { self.expect("disabled keyboard ignored", 1, 3, 0, 0, Some(&[]))?; self.click(cx, surface, "primary")?; }
            7 => { self.expect("restore focus", 1, 4, 0, 0, Some(NORMAL))?; self.expect_focus("primary")?; self.key(cx, surface, NamedKey::Enter, false)?; self.key(cx, surface, NamedKey::Space, false)?; }
            8 => { self.expect("keydown preventDefault", 1, 4, 0, 0, Some(NORMAL))?; self.advance()?; }
            9 => { self.expect("pointer cancellation mode", 2, 4, 0, 0, Some(&[]))?; self.click(cx, surface, "guarded")?; }
            // Pointerdown owns focus; the later uncancelled pointerup independently owns activation.
            10 => { self.expect("pointerdown cancellation allows independent release", 2, 4, 1, 0, Some(GUARDED_CLICK))?; self.expect_focus("primary")?; self.key(cx, surface, NamedKey::Enter, false)?; }
            11 => { self.expect("pointerdown preserves prior focus", 2, 5, 1, 0, Some(&[
                "outer-capture:capture", "middle-capture:capture", "middle-bubble:bubble", "outer-bubble:bubble",
                "outer-capture:capture", "middle-capture:capture", "primary-capture:target",
                "primary-bubble:target", "middle-bubble:bubble", "outer-bubble:bubble",
            ]))?; self.advance()?; }
            12 => { self.expect("focused control disabled", 3, 5, 1, 0, Some(&[]))?; self.key(cx, surface, NamedKey::Enter, false)?; self.key(cx, surface, NamedKey::Space, false)?; self.click(cx, surface, "primary")?; }
            13 => { self.expect("disabled retained focus cannot activate", 3, 5, 1, 0, Some(DISABLED_ANCESTOR_CLICK))?; self.click(cx, surface, "guarded")?; }
            // Conversely, cancelling pointerup suppresses activation but cannot undo pointerdown focus.
            14 => { self.expect("pointerup cancellation prevents activation", 3, 5, 1, 0, Some(DISABLED_ANCESTOR_CLICK))?; self.expect_focus("guarded")?; self.advance()?; }
            15 => { self.expect("stopPropagation mode", 4, 5, 1, 0, Some(&[]))?; self.click(cx, surface, "primary")?; }
            16 => { self.expect("stopPropagation keeps same-target listener", 4, 6, 1, 0, Some(&NORMAL[..4]))?; self.advance()?; }
            17 => { self.expect("stopImmediatePropagation mode", 5, 6, 1, 0, Some(&[]))?; self.click(cx, surface, "primary")?; }
            18 => { self.expect("stopImmediatePropagation stops same-target listener", 5, 6, 1, 0, Some(&NORMAL[..3]))?; self.advance()?; }
            19 => { self.expect("ancestor stop mode", 6, 6, 1, 0, Some(&[]))?; self.click(cx, surface, "primary")?; }
            20 => { self.expect("ancestor stops descendant handlers", 6, 6, 1, 0, Some(&NORMAL[..2]))?; self.advance()?; }
            21 => { self.expect("replacement mode", 7, 6, 1, 0, Some(&[]))?; self.click(cx, surface, "primary")?; }
            22 => { self.expect("handler replaced exactly once", 7, 6, 1, 1, Some(&[
                "outer-capture:capture", "middle-capture:capture", "primary-capture:target",
                "replacement-bubble:target", "middle-bubble:bubble", "outer-bubble:bubble",
            ]))?; self.advance()?; }
            23 => { self.expect("null removal mode", 8, 6, 1, 1, Some(&[]))?; self.click(cx, surface, "primary")?; }
            24 => { self.expect("null removes only the handler", 8, 6, 1, 1, Some(&[
                "outer-capture:capture", "middle-capture:capture", "primary-capture:target",
                "middle-bubble:bubble", "outer-bubble:bubble",
            ]))?; self.advance()?; }
            25 => {
                self.expect("retained cancellation expired", 9, 6, 1, 1, Some(&[]))?;
                self.expect_focus("primary")?;
                self.key(cx, surface, NamedKey::Enter, false)?;
            }
            26 => {
                self.expect("keydown disables before its own activation default", 9, 6, 1, 1, Some(&[]))?;
                let actual = self.report()?;
                if actual["expired"].as_u64() != Some(48) {
                    self.failures.borrow_mut().push(format!("expected all 48 retained cancellation attempts to reject; native report {actual}"));
                }
                if actual["disabledInKeydown"].as_bool() != Some(true) || actual["disableKeydowns"].as_u64() != Some(1) {
                    self.failures.borrow_mut().push(format!("expected exactly one keydown handler to disable its control synchronously; native report {actual}"));
                }
                self.completed.set(true);
                self.finished = true;
                cx.request_exit();
                return Ok(());
            }
            _ => return Err("native input script advanced past completion".into()),
        }
        self.step += 1;
        if let Some(window) = self.inner.native.windows().first() { window.request_frame(); }
        Ok(())
    }

    fn stop(&mut self, cx: &dyn PlatformCx, error: impl std::fmt::Display) {
        self.failures.borrow_mut().push(format!("step {}: {error}", self.step));
        self.finished = true;
        cx.request_exit();
    }
}

impl AppHandler for NativeControls {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        self.deadline = Some(cx.clock().now() + Duration::from_secs(20));
        self.inner.surfaces_available(cx);
    }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) { self.inner.surfaces_lost(cx); }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        let redraw = matches!(event, SurfaceEvent::RedrawRequested);
        self.inner.surface_event(cx, surface, event);
        if redraw && !self.finished && !cx.is_exiting() {
            if let Err(error) = self.drive(cx, surface) { self.stop(cx, error); }
        }
    }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) { self.inner.wake(cx, reason); }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy {
        let policy = self.inner.idle(cx);
        self.deadline.map_or(policy, |deadline| policy.merge(IdlePolicy::BlockUntil(deadline)))
    }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) {
        self.inner.deadline_reached(cx);
        if !self.finished && self.deadline.is_some_and(|deadline| cx.clock().now() >= deadline) {
            self.stop(cx, "timed out waiting for a real settled Wayland frame");
        }
    }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) { self.inner.shutting_down(cx); }
}

fn build_fixture(root: &Path) -> TestResult<PathBuf> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let directory = root.join("target").join(format!("native-controls-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("target"))?;
    let output = Command::new("node").arg(root.join("tests/native-controls/build.mjs")).arg(&directory).current_dir(root).output()?;
    if !output.status.success() {
        return Err(format!("native TSX fixture build failed ({}):\n{}\n{}", output.status, String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(directory.join("dist"))
}

#[test]
#[ignore = "opens a real Wayland window; requires Node dependencies and a working GPU/compositor"]
fn native_button_actions() -> TestResult {
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
    let native = zgui::app().with_title("zgui-js native Button acceptance gate").with_application_id("dev.zgui.js.native-controls")
        .with_size(760.0, 440.0).into_handler(move || Root(roots.clone()))?.into_runtime();
    let native_failure = native.failure();
    let failures = Rc::new(RefCell::new(Vec::new()));
    let completed = Rc::new(std::cell::Cell::new(false));
    let application = Application::new(native, pending, Some(package))?;
    let failure = application.failure.clone();
    let driver = NativeControls {
        inner: application, step: 0, finished: false, deadline: None,
        failures: failures.clone(), completed: completed.clone(),
    };
    let result = zgui::app::desktop()(Box::new(driver));
    zgui::reactive::shutdown();
    result?;
    if let Some(error) = native_failure.take() { failures.borrow_mut().push(error.to_string()); }
    if let Some(error) = failure.borrow_mut().take() { failures.borrow_mut().push(error); }
    if !completed.get() { failures.borrow_mut().push("native input script did not complete".into()); }
    let failures = failures.borrow();
    if !failures.is_empty() { return Err(failures.join("\n").into()); }
    Ok(())
}
