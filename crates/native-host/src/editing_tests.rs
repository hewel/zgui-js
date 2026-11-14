//! Real native editing model, layout, clipboard and JS-controlled-value gate.
use super::*;
use std::{cell::Cell, io::Write, path::Path, process::{Command, Stdio}, time::Instant};
use zgui::{
    geom::{Css, CssPx, Device, DevicePx, Point, Scale},
    reactive::prelude::GetUntracked,
    view::NodeId,
    vocab::{ImeEvent, KeyEvent, KeyState, Modifiers, NamedKey, PhysicalKey, PointerAction, PointerButton, PointerEvent, Role, SemanticFlags},
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step { Setup, AwaitKey, Start, Position, Type, Preedit, ChinesePreedit, Commit, Delete, Undo, SelectCopy, Paste, Cut, RestorePaste, DefaultInput, DefaultUpdate, Reset, ReadOnly, Disabled, Tab, AreaEdit, AreaCaret, AreaVertical, AreaCommit, WaylandIme, Hold, Reload, Finished }

// Preserve one offered MIME representation without logging the user's clipboard.
// Capture before this application owns the selection: a synchronous read from
// its own Wayland data source would deadlock the owner thread.
struct ClipboardGuard { saved: Option<(String, Vec<u8>)>, restored: bool }
impl ClipboardGuard {
    fn capture() -> TestResult<Self> {
        let types = Command::new("wl-paste").arg("--list-types").output()?;
        if !types.status.success() {
            if String::from_utf8_lossy(&types.stderr).contains("Nothing is copied") {
                return Ok(Self { saved: None, restored: false });
            }
            return Err("cannot preserve clipboard before native editing gate".into());
        }
        let types = String::from_utf8(types.stdout)?;
        let mime = types.lines().find(|mime| *mime == "text/plain;charset=utf-8")
            .or_else(|| types.lines().find(|mime| *mime == "image/png"))
            .or_else(|| types.lines().next()).ok_or("clipboard offered no MIME type")?.to_owned();
        let data = Command::new("wl-paste").args(["--no-newline", "--type", &mime]).output()?;
        if !data.status.success() { return Err("cannot preserve offered clipboard data".into()); }
        Ok(Self { saved: Some((mime, data.stdout)), restored: false })
    }
    fn restore(&mut self) -> TestResult {
        if self.restored { return Ok(()); }
        let status = if let Some((mime, bytes)) = &self.saved {
            let mut child = Command::new("wl-copy").args(["--type", mime]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
            child.stdin.take().ok_or("clipboard restore stdin missing")?.write_all(bytes)?;
            child.wait()?
        } else { Command::new("wl-copy").arg("--clear").status()? };
        if !status.success() { return Err("clipboard restoration failed".into()); }
        self.restored = true;
        Ok(())
    }
}
impl Drop for ClipboardGuard {
    fn drop(&mut self) { if !self.restored && let Err(error) = self.restore() { eprintln!("clipboard restore: {error}"); } }
}

struct EditingGate {
    inner: Application,
    package: Rc<Package>,
    step: Step,
    deadline: Option<Instant>,
    hold_until: Option<Instant>,
    initial_ids: Vec<NodeId>,
    initial_counts: Option<session::ResourceCounts>,
    retired: Rc<RefCell<Vec<Rc<Session>>>>,
    failures: Rc<RefCell<Vec<String>>>,
    completed: Rc<Cell<bool>>,
    wayland_ime: bool,
    ime_preedit: bool,
    ime_commit: Option<String>,
}
impl EditingGate {
    fn current(&self) -> TestResult<Rc<Session>> {
        self.inner.sessions.values().next().cloned().ok_or_else(|| "editing session absent".into())
    }
    fn node(&self, class: &str) -> TestResult<NodeId> {
        let window = self.inner.native.windows().first().ok_or("native editing window absent")?;
        let nodes = window.nodes_with_class(class);
        if nodes.len() != 1 { return Err(format!("expected one {class}, found {}", nodes.len()).into()); }
        Ok(nodes[0])
    }
    fn text(&self, class: &str) -> TestResult<String> {
        Ok(self.current()?.region.dom.text_content(self.node(class)?))
    }
    fn expect_text(&self, class: &str, expected: &str) -> TestResult {
        let actual = self.text(class)?;
        if actual != expected { return Err(format!("{class}: expected {expected:?}, got {actual:?}").into()); }
        Ok(())
    }
    fn selection(&self, class: &str, expected: std::ops::Range<usize>) -> TestResult {
        let actual = self.current()?.region.host.selection(self.node(class)?);
        if actual != Some(expected.clone()) { return Err(format!("{class}: expected UTF-8 selection {expected:?}, got {actual:?}").into()); }
        Ok(())
    }
    fn focused(&self, class: &str) -> TestResult {
        if self.current()?.region.host.focused().get_untracked() != Some(self.node(class)?) {
            return Err(format!("native focus is not on {class}").into());
        }
        Ok(())
    }
    fn input_count(&self, expected: usize) -> TestResult {
        self.expect_text("editing-input-count", &expected.to_string())?;
        self.expect_text("editing-target-valid", "true")
    }
    fn semantics(&self) -> TestResult<Vec<NodeId>> {
        let window = self.inner.native.windows().first().ok_or("window absent")?;
        let mut ids = Vec::new();
        for (class, role, label, flag) in [
            ("editing-field", Role::TextInput, "Controlled single-line field", SemanticFlags::NONE),
            ("editing-area", Role::MultilineTextInput, "Controlled multiline area", SemanticFlags::NONE),
            ("editing-default", Role::TextInput, "Uncontrolled field", SemanticFlags::NONE),
            ("editing-readonly", Role::TextInput, "Read-only field", SemanticFlags::READ_ONLY),
            ("editing-disabled", Role::TextInput, "Disabled field", SemanticFlags::DISABLED),
        ] {
            let node = self.node(class)?;
            let semantics = window.dom().semantics(node).ok_or_else(|| format!("{class} has no native semantics"))?;
            if semantics.role != role || semantics.label.as_deref() != Some(label) || !semantics.flags.contains(flag) {
                return Err(format!("{class}: native accessibility semantics do not match its live control").into());
            }
            ids.push(node);
        }
        Ok(ids)
    }
    fn send(&mut self, cx: &dyn PlatformCx, event: SurfaceEvent) -> TestResult {
        let surface = self.inner.native.windows().first().ok_or("window absent")?.surface().id();
        self.inner.surface_event(cx, surface, event);
        Ok(())
    }
    fn key(&mut self, cx: &dyn PlatformCx, event: KeyEvent, modifiers: Modifiers) -> TestResult {
        for state in [KeyState::Pressed, KeyState::Released] {
            self.send(cx, SurfaceEvent::Key { state, event: event.clone(), modifiers, timestamp: cx.clock().timestamp() })?;
        }
        Ok(())
    }
    fn named(&mut self, cx: &dyn PlatformCx, key: NamedKey, modifiers: Modifiers) -> TestResult {
        self.key(cx, KeyEvent::named(key, PhysicalKey::Unidentified(0)), modifiers)
    }
    fn chord(&mut self, cx: &dyn PlatformCx, key: &str) -> TestResult { self.key(cx, KeyEvent::character(key), Modifiers::CONTROL) }
    fn click(&mut self, cx: &dyn PlatformCx, class: &str) -> TestResult {
        let window = self.inner.native.windows().first().ok_or("window absent")?;
        let bounds = window.box_with_class(class).ok_or_else(|| format!("{class} has no native box"))?;
        if bounds.size.width.0 <= 8.0 || bounds.size.height.0 <= 8.0 { return Err(format!("{class} has unusable bounds {bounds:?}").into()); }
        let point: Point<DevicePx, Device> = Point::new(DevicePx(bounds.origin.x.0 + bounds.size.width.0 - 4.0), DevicePx(bounds.origin.y.0 + bounds.size.height.0 - 4.0));
        let scale: Scale<Css, Device> = Scale::new(window.surface().scale_factor() as f32);
        let position: Point<CssPx, Css> = point * scale.invert();
        for action in [PointerAction::Moved, PointerAction::Pressed, PointerAction::Released] {
            self.send(cx, SurfaceEvent::Pointer { action, event: PointerEvent::mouse(position).with_button(PointerButton::Primary), modifiers: Modifiers::NONE, timestamp: cx.clock().timestamp() })?;
        }
        Ok(())
    }
    fn drive(&mut self, cx: &dyn PlatformCx, frame: bool) -> TestResult {
        if self.step == Step::Finished || cx.is_exiting() { return Ok(()); }
        if let Some(error) = self.inner.failure.borrow().as_ref() { return Err(error.clone().into()); }
        let Some(window) = self.inner.native.windows().first() else { return Ok(()); };
        if window.is_occluded() { return Ok(()); }
        if self.step == Step::Hold {
            if self.hold_until.is_some_and(|at| cx.clock().now() >= at) {
                self.hold_until = None;
                let document = window.document_id();
                let old = self.current()?;
                if !matches!(self.inner.replace_session(cx, document, Some(self.package.clone())), ReloadOutcome::Mounted(_)) { return Err("editing reload failed".into()); }
                if old.counts() != session::ResourceCounts::default() { return Err("editing generation retained resources after reload".into()); }
                self.retired.borrow_mut().push(old);
                self.step = Step::Reload;
            }
            return Ok(());
        }
        if !frame { return Ok(()); }
        eprintln!("native-editing-step: {:?}", self.step);
        match self.step {
            Step::Setup => {
                if self.text("editing-field")? != "A中B" { return Ok(()); }
                self.step = Step::AwaitKey;
                eprintln!("native-editing-input-ready: press a real key to authorize Wayland clipboard access");
            }
            Step::Start => {
                if self.text("editing-field")? != "A中B" { return Ok(()); }
                self.input_count(0)?;
                self.expect_text("editing-change-count", "0")?;
                self.initial_ids = self.semantics()?;
                self.initial_counts = Some(self.current()?.counts());
                self.click(cx, "editing-field")?;
                self.named(cx, NamedKey::Home, Modifiers::NONE)?;
                self.named(cx, NamedKey::ArrowRight, Modifiers::NONE)?;
                self.step = Step::Position;
            }
            Step::Position => {
                self.focused("editing-field")?;
                self.selection("editing-field", 1..1)?;
                self.key(cx, KeyEvent::character("x"), Modifiers::NONE)?;
                self.step = Step::Type;
            }
            Step::Type => {
                self.expect_text("editing-field", "Ax中B")?;
                self.selection("editing-field", 2..2)?;
                self.input_count(1)?;
                self.send(cx, SurfaceEvent::Ime(ImeEvent::Enabled))?;
                self.send(cx, SurfaceEvent::Ime(ImeEvent::Preedit { text: "ni".into(), cursor: Some(2..2) }))?;
                self.step = Step::Preedit;
            }
            Step::Preedit => {
                self.expect_text("editing-field", "Axni中B")?;
                self.input_count(2)?;
                self.named(cx, NamedKey::ArrowLeft, Modifiers::NONE)?;
                self.send(cx, SurfaceEvent::Ime(ImeEvent::Preedit { text: "你".into(), cursor: Some(3..3) }))?;
                self.step = Step::ChinesePreedit;
            }
            Step::ChinesePreedit => {
                self.expect_text("editing-field", "Ax你中B")?;
                self.input_count(3)?;
                self.send(cx, SurfaceEvent::Ime(ImeEvent::Commit("你好".into())))?;
                self.send(cx, SurfaceEvent::Ime(ImeEvent::Disabled))?;
                self.step = Step::Commit;
            }
            Step::Commit => {
                self.expect_text("editing-field", "Ax你好中B")?;
                self.selection("editing-field", 8..8)?;
                self.input_count(4)?;
                self.named(cx, NamedKey::Backspace, Modifiers::NONE)?;
                self.step = Step::Delete;
            }
            Step::Delete => {
                self.expect_text("editing-field", "Ax你中B")?;
                self.selection("editing-field", 5..5)?;
                self.input_count(5)?;
                self.chord(cx, "z")?;
                self.step = Step::Undo;
            }
            Step::Undo => {
                self.expect_text("editing-field", "Ax你好中B")?;
                self.input_count(6)?;
                self.named(cx, NamedKey::Home, Modifiers::NONE)?;
                for _ in 0..2 { self.named(cx, NamedKey::ArrowRight, Modifiers::NONE)?; }
                for _ in 0..2 { self.named(cx, NamedKey::ArrowRight, Modifiers::SHIFT)?; }
                self.chord(cx, "c")?;
                self.step = Step::SelectCopy;
            }
            Step::SelectCopy => {
                self.selection("editing-field", 2..8)?;
                self.input_count(6)?;
                self.named(cx, NamedKey::End, Modifiers::NONE)?;
                self.chord(cx, "v")?;
                self.step = Step::Paste;
            }
            Step::Paste => {
                if self.text("editing-field")? != "Ax你好中B你好" { return Ok(()); }
                self.selection("editing-field", 18..18)?;
                self.input_count(7)?;
                self.chord(cx, "a")?;
                self.chord(cx, "x")?;
                self.step = Step::Cut;
            }
            Step::Cut => {
                self.expect_text("editing-field", "")?;
                self.input_count(8)?;
                self.chord(cx, "v")?;
                self.step = Step::RestorePaste;
            }
            Step::RestorePaste => {
                if self.text("editing-field")? != "Ax你好中B你好" { return Ok(()); }
                self.input_count(9)?;
                self.click(cx, "editing-default")?;
                self.named(cx, NamedKey::End, Modifiers::NONE)?;
                self.key(cx, KeyEvent::character("!"), Modifiers::NONE)?;
                self.step = Step::DefaultInput;
            }
            Step::DefaultInput => {
                self.expect_text("editing-default", "seed!")?;
                self.input_count(10)?;
                self.click(cx, "editing-change-default")?;
                self.step = Step::DefaultUpdate;
            }
            Step::DefaultUpdate => {
                self.expect_text("editing-default", "seed!")?;
                self.input_count(10)?;
                self.click(cx, "editing-reset")?;
                self.step = Step::Reset;
            }
            Step::Reset => {
                self.expect_text("editing-field", "Reset 中")?;
                self.selection("editing-field", 9..9)?;
                self.input_count(10)?;
                self.click(cx, "editing-readonly")?;
                self.chord(cx, "a")?;
                self.key(cx, KeyEvent::character("X"), Modifiers::NONE)?;
                self.named(cx, NamedKey::Delete, Modifiers::NONE)?;
                self.step = Step::ReadOnly;
            }
            Step::ReadOnly => {
                self.focused("editing-readonly")?;
                self.expect_text("editing-readonly", "Read only")?;
                self.selection("editing-readonly", 0..9)?;
                self.input_count(10)?;
                self.click(cx, "editing-disabled")?;
                self.key(cx, KeyEvent::character("Y"), Modifiers::NONE)?;
                self.step = Step::Disabled;
            }
            Step::Disabled => {
                self.expect_text("editing-disabled", "Disabled")?;
                self.expect_text("editing-readonly", "Read only")?;
                self.input_count(10)?;
                self.named(cx, NamedKey::Tab, Modifiers::NONE)?;
                self.step = Step::Tab;
            }
            Step::Tab => {
                self.focused("editing-field")?;
                self.click(cx, "editing-area")?;
                self.named(cx, NamedKey::End, Modifiers::CONTROL)?;
                self.named(cx, NamedKey::Enter, Modifiers::NONE)?;
                self.key(cx, KeyEvent::character("你好世界"), Modifiers::NONE)?;
                self.step = Step::AreaEdit;
            }
            Step::AreaEdit => {
                self.expect_text("editing-last-value", "one\n二\n你好世界")?;
                self.expect_text("editing-last-selection", "20:20")?;
                self.selection("editing-area", 20..20)?;
                self.input_count(12)?;
                self.named(cx, NamedKey::ArrowLeft, Modifiers::NONE)?;
                self.named(cx, NamedKey::ArrowLeft, Modifiers::NONE)?;
                self.step = Step::AreaCaret;
            }
            Step::AreaCaret => {
                // Two native grapheme steps cross two three-byte CJK characters.
                self.selection("editing-area", 14..14)?;
                self.named(cx, NamedKey::ArrowUp, Modifiers::NONE)?;
                self.step = Step::AreaVertical;
            }
            Step::AreaVertical => {
                // Moving up clamps to the end of the shorter preceding line.
                // This fails if CSS collapses the model's newlines into one line.
                self.selection("editing-area", 7..7)?;
                self.input_count(12)?;
                self.named(cx, NamedKey::Tab, Modifiers::NONE)?;
                self.step = Step::AreaCommit;
            }
            Step::AreaCommit => {
                self.focused("editing-default")?;
                self.expect_text("editing-change-count", "3")?;
                self.input_count(12)?;
                if self.semantics()? != self.initial_ids { return Err("editing replaced public control nodes or semantic identities".into()); }
                let current = self.current()?.counts();
                let initial = self.initial_counts.as_ref().ok_or("initial resource counts missing")?;
                if current.nodes != initial.nodes || current.wrappers != initial.wrappers || current.callbacks != initial.callbacks {
                    return Err(format!("editing leaked public resources: {initial:?} -> {current:?}").into());
                }
                if self.wayland_ime {
                    self.click(cx, "editing-field")?;
                    self.chord(cx, "a")?;
                    self.step = Step::WaylandIme;
                    eprintln!("native-wayland-ime-ready: type and commit Chinese through the real compositor/IME");
                } else {
                    self.hold_until = Some(cx.clock().now() + Duration::from_secs(3));
                    self.step = Step::Hold;
                }
                eprintln!("native-editing-complete: controlled caret, Chinese composition, clipboard, undo, multiline, readonly, focus and live semantics verified");
            }
            Step::WaylandIme => {
                let Some(value) = self.ime_commit.as_deref() else { return Ok(()); };
                if value.is_ascii() { return Err(format!("Wayland IME did not commit Chinese: {value:?}").into()); }
                if !self.ime_preedit { return Err("Wayland IME committed without an observed preedit".into()); }
                self.expect_text("editing-field", value)?;
                self.expect_text("editing-last-value", value)?;
                self.expect_text("editing-target-valid", "true")?;
                self.selection("editing-field", value.len()..value.len())?;
                self.hold_until = Some(cx.clock().now() + Duration::from_secs(3));
                self.step = Step::Hold;
                eprintln!("native-wayland-ime-complete: real preedit/commit reached the controlled native field: {value:?}");
            }
            Step::Reload => {
                if self.text("editing-field")? != "A中B" { return Ok(()); }
                self.input_count(0)?;
                self.expect_text("editing-change-count", "0")?;
                let current = self.current()?;
                if self.initial_ids.iter().any(|node| current.region.dom.is_live(*node)) { return Err("old editable controls survived native frame reclamation".into()); }
                self.semantics()?;
                self.retired.borrow_mut().push(current);
                self.completed.set(true);
                self.step = Step::Finished;
                cx.request_exit();
            }
            Step::AwaitKey | Step::Hold | Step::Finished => {}
        }
        Ok(())
    }
    fn stop(&mut self, cx: &dyn PlatformCx, error: impl std::fmt::Display) {
        self.failures.borrow_mut().push(format!("{:?}: {error}", self.step));
        self.step = Step::Finished;
        cx.request_exit();
    }
}
impl AppHandler for EditingGate {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) { self.deadline = Some(cx.clock().now() + Duration::from_secs(40)); self.inner.surfaces_available(cx); }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) { self.inner.surfaces_lost(cx); if self.step != Step::Finished { self.stop(cx, "native surfaces lost"); } }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        if self.step == Step::Finished { return; }
        let frame = matches!(event, SurfaceEvent::RedrawRequested);
        if self.step == Step::AwaitKey && matches!(event, SurfaceEvent::Key { state: KeyState::Pressed, .. }) {
            self.step = Step::Start;
            if let Some(window) = self.inner.native.windows().first() { window.request_frame(); }
        }
        if self.step == Step::WaylandIme {
            match &event {
                SurfaceEvent::Ime(ImeEvent::Preedit { text, .. }) if !text.is_empty() => {
                    self.ime_preedit = true;
                    eprintln!("native-wayland-ime-preedit: {text}");
                }
                SurfaceEvent::Ime(ImeEvent::Commit(text)) if !text.is_empty() => {
                    self.ime_commit = Some(text.to_string());
                }
                _ => {}
            }
        }
        self.inner.surface_event(cx, surface, event);
        if let Err(error) = self.drive(cx, frame) { self.stop(cx, error); }
    }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) { self.inner.wake(cx, reason); }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy {
        let policy = self.inner.idle(cx);
        let policy = self.deadline.map_or(policy, |at| policy.merge(IdlePolicy::BlockUntil(at)));
        self.hold_until.map_or(policy, |at| policy.merge(IdlePolicy::BlockUntil(at)))
    }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) {
        if self.step == Step::Finished { return; }
        self.inner.deadline_reached(cx);
        if let Err(error) = self.drive(cx, false) { self.stop(cx, error); }
        if self.step != Step::Finished && self.deadline.is_some_and(|at| cx.clock().now() >= at) { self.stop(cx, "native editing gate timed out"); }
    }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) { self.inner.shutting_down(cx); }
}

#[test]
#[ignore = "requires real unlocked Wayland/niri, GPU, Node, wl-copy and wl-paste; temporarily uses the clipboard"]
fn native_text_editing() -> TestResult {
    if std::env::var("ZGUI_PLATFORM").as_deref() != Ok("wayland") { return Err("run with ZGUI_PLATFORM=wayland".into()); }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).ok_or("workspace root absent")?;
    let build = Command::new("node").arg(root.join("packages/build/src/cli.mjs")).args(["--app", "editing"]).current_dir(root).output()?;
    if !build.status.success() { return Err(format!("editing package build failed: {}", String::from_utf8_lossy(&build.stderr)).into()); }
    let package = Rc::new(Package::load(&root.join("apps/examples/editing/dist"))?);
    let mut clipboard = ClipboardGuard::capture()?;
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let native = zgui::app().with_title("zgui-js native editing acceptance").with_application_id("dev.zgui.js.native-editing")
        .with_size(800.0, 1000.0).into_handler(move || Root(roots.clone()))?.into_runtime();
    let native_failure = native.failure();
    let application = Application::new(native, pending, Some(package.clone()))?;
    let failure = application.failure.clone();
    let failures = Rc::new(RefCell::new(Vec::new()));
    let completed = Rc::new(Cell::new(false));
    let retired = Rc::new(RefCell::new(Vec::new()));
    let driver = EditingGate { inner: application, package, step: Step::Setup, deadline: None, hold_until: None, initial_ids: Vec::new(), initial_counts: None, retired: retired.clone(), failures: failures.clone(), completed: completed.clone(), wayland_ime: std::env::var("ZGUI_EDITING_WAYLAND_IME").as_deref() == Ok("1"), ime_preedit: false, ime_commit: None };
    let result = zgui::app::desktop()(Box::new(driver));
    zgui::reactive::shutdown();
    let restored = clipboard.restore();
    result?;
    restored?;
    if let Some(error) = native_failure.take() { failures.borrow_mut().push(error.to_string()); }
    if let Some(error) = failure.borrow_mut().take() { failures.borrow_mut().push(error); }
    if !completed.get() { failures.borrow_mut().push("native editing gate did not complete".into()); }
    for session in retired.borrow().iter() {
        if session.counts() != session::ResourceCounts::default() { failures.borrow_mut().push(format!("generation {} retained {:?}", session.generation, session.counts())); }
    }
    if !failures.borrow().is_empty() { return Err(failures.borrow().join("\n").into()); }
    Ok(())
}
