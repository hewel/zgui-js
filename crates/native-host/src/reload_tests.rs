//! Real compositor gate: full TSX generations, failed candidates and native recovery.
use super::*;
use crate::{session::{ResourceCounts, Session}, test_package::Fixture};
use std::{cell::Cell, path::Path, process::Command, time::Instant};
use zgui::{geom::{Css, CssPx, Device, DevicePx, Point, Scale}, vocab::{Modifiers, PointerAction, PointerButton, PointerEvent}};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage { Initial, Reloaded, Fallback, Recovering, StaleDelivery, Finished }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trigger { Frame, Wake, Other }
struct ReloadGate {
    inner: Application,
    good: Rc<Package>,
    bad_eval: Fixture,
    bad_mount: Fixture,
    stage: Stage,
    deadline: Option<Instant>,
    retry_at: Option<Instant>,
    failed_generation: u64,
    recovered_generation: u64,
    retired: Rc<RefCell<Vec<Rc<Session>>>>,
    failures: Rc<RefCell<Vec<String>>>,
    completed: Rc<Cell<bool>>,
}
impl ReloadGate {
    fn current(&self) -> TestResult<Rc<Session>> {
        self.inner.host.sessions().next().cloned().ok_or_else(|| "active session is absent".into())
    }
    fn stop(&mut self, cx: &dyn PlatformCx, error: impl std::fmt::Display) {
        self.failures.borrow_mut().push(format!("{:?}: {error}", self.stage));
        self.stage = Stage::Finished;
        cx.request_exit();
    }
    fn retire(&self, session: Rc<Session>) -> TestResult {
        if session.counts() != ResourceCounts::default() { return Err(format!("retired generation {} retained {:?}", session.generation, session.counts()).into()); }
        self.retired.borrow_mut().push(session);
        Ok(())
    }
    fn click_retry(&mut self, cx: &dyn PlatformCx) -> TestResult {
        let window = self.inner.native.windows().first().ok_or("window missing")?;
        let bounds = window.box_with_class("zgui-host-fallback__retry").ok_or("retry control has no native box")?;
        if bounds.size.width.0 <= 8.0 || bounds.size.height.0 <= 8.0 { return Err(format!("invalid retry geometry: {bounds:?}").into()); }
        let point: Point<DevicePx, Device> = Point::new(DevicePx(bounds.origin.x.0 + bounds.size.width.0 - 4.0), DevicePx(bounds.origin.y.0 + bounds.size.height.0 - 4.0));
        let scale: Scale<Css, Device> = Scale::new(window.surface().scale_factor() as f32);
        let position: Point<CssPx, Css> = point * scale.invert();
        let surface = window.surface().id();
        for action in [PointerAction::Moved, PointerAction::Pressed, PointerAction::Released] {
            self.inner.surface_event(cx, surface, SurfaceEvent::Pointer {
                action, event: PointerEvent::mouse(position).with_button(PointerButton::Primary),
                modifiers: Modifiers::NONE, timestamp: cx.clock().timestamp(),
            });
        }
        Ok(())
    }
    fn drive(&mut self, cx: &dyn PlatformCx, trigger: Trigger) -> TestResult {
        let native_frame = trigger == Trigger::Frame;
        if self.stage == Stage::Finished || cx.is_exiting() { return Ok(()); }
        if let Some(error) = self.inner.failure.borrow().as_ref() { return Err(error.clone().into()); }
        let Some(window) = self.inner.native.windows().first() else { return Ok(()); };
        if window.is_occluded() { return Ok(()); }
        let document = window.document_id();
        match self.stage {
            Stage::Initial if native_frame => {
                let active = self.current()?;
                let before = active.region.dom.text_content(active.region.root);
                if !before.contains("Service: Rust task completed") { return Ok(()); }
                let outcome = self.inner.replace_session(cx, document, SessionModule::package(self.bad_eval.package()));
                if !matches!(outcome, ReloadOutcome::Rejected(_)) { return Err(format!("evaluation failure did not reject: {outcome:?}").into()); }
                if !Rc::ptr_eq(&active, &self.current()?) || active.region.dom.text_content(active.region.root) != before {
                    return Err("preflight failure changed the active app".into());
                }
                let mut expected = None;
                for _ in 0..16 {
                    let previous = self.current()?;
                    let outcome = self.inner.replace_session(cx, document, SessionModule::package(self.good.clone()));
                    if !matches!(outcome, ReloadOutcome::Mounted(_)) { return Err(format!("reload failed: {outcome:?}").into()); }
                    self.retire(previous)?;
                    let counts = self.current()?.counts();
                    if let Some(expected) = &expected {
                        if &counts != expected { return Err(format!("fresh-generation resources grew: {counts:?} vs {expected:?}").into()); }
                    } else { expected = Some(counts); }
                }
                self.stage = Stage::Reloaded;
                eprintln!("native-reload-cycles: 16 retired generations have zero resources");
            }
            Stage::Reloaded if native_frame => {
                let active = self.current()?;
                if !active.region.dom.text_content(active.region.root).contains("Service: Rust task completed") { return Ok(()); }
                if self.inner.native.windows()[0].nodes_with_class("promise-layout-row").len() != 1 {
                    return Err("native frame reclamation retained prior-generation layout rows".into());
                }
                let outcome = self.inner.replace_session(cx, document, SessionModule::package(self.bad_mount.package()));
                if !matches!(outcome, ReloadOutcome::Fallback(_)) { return Err(format!("mount failure missed fallback: {outcome:?}").into()); }
                self.retire(active)?;
                if self.inner.host.sessions().next().is_some() { return Err("failed generation remained active".into()); }
                self.failed_generation = self.inner.host.fallback_generation(document).ok_or("Rust fallback missing")?;
                let region = self.inner.host.region(document).ok_or("mount region missing")?;
                let text = region.dom.text_content(region.root);
                if !text.contains("Application error") || !text.contains("mount exploded") { return Err(format!("fallback lost original failure: {text}").into()); }
                self.stage = Stage::Fallback;
            }
            Stage::Fallback => {
                if self.retry_at.is_none() && native_frame {
                    let window = &self.inner.native.windows()[0];
                    if window.box_with_class("zgui-host-fallback__retry").is_none() { return Ok(()); }
                    // A short native-clock interval exposes the actual fallback for
                    // visual inspection; it is not used to synchronize worker races.
                    self.retry_at = Some(cx.clock().now() + Duration::from_secs(3));
                    eprintln!("native-reload-fallback: Rust diagnostic and retry control have native geometry");
                }
                if self.retry_at.is_some_and(|at| cx.clock().now() >= at) {
                    self.retry_at = None;
                    self.stage = Stage::Recovering;
                    self.click_retry(cx)?;
                }
            }
            Stage::Recovering if native_frame => {
                // Native input is dispatched by a frame; its Retry message is
                // consumed by a later owner wake, not by that same input frame.
                let Some(active) = self.inner.host.session(document).cloned() else { return Ok(()); };
                if !active.region.dom.text_content(active.region.root).contains("Service: Rust task completed") { return Ok(()); }
                if self.inner.host.fallback_generation(document).is_some() || active.generation <= self.failed_generation {
                    return Err("native retry did not create a clean successful generation".into());
                }
                self.recovered_generation = active.generation;
                let old = self.retired.borrow()[0].generation;
                self.inner.sender.try_send(Completion::Update { document, generation: old })?;
                self.inner.sender.try_send(Completion::Retry { document, generation: self.failed_generation })?;
                self.stage = Stage::StaleDelivery;
                cx.waker().wake(WakeReason::AppWork);
            }
            Stage::StaleDelivery if trigger == Trigger::Wake => {
                let active = self.current()?;
                if active.generation != self.recovered_generation || self.inner.host.fallback_generation(document).is_some() {
                    return Err("stale delivery changed the recovered generation".into());
                }
                if !active.region.dom.text_content(active.region.root).contains("Service: Rust task completed") { return Err("stale delivery changed recovered text".into()); }
                self.retired.borrow_mut().push(active);
                self.completed.set(true);
                self.stage = Stage::Finished;
                eprintln!("native-reload-recovered: generation={} stale deliveries ignored", self.recovered_generation);
                cx.request_exit();
            }
            _ => {}
        }
        Ok(())
    }
}
impl AppHandler for ReloadGate {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        self.deadline = Some(cx.clock().now() + Duration::from_secs(30));
        self.inner.surfaces_available(cx);
    }
    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) { self.inner.surfaces_lost(cx); if self.stage != Stage::Finished { self.stop(cx, "native surfaces lost"); } }
    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        if self.stage == Stage::Finished { return; }
        let frame = matches!(event, SurfaceEvent::RedrawRequested);
        self.inner.surface_event(cx, surface, event);
        if let Err(error) = self.drive(cx, if frame { Trigger::Frame } else { Trigger::Other }) { self.stop(cx, error); }
    }
    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        if self.stage == Stage::Finished { return; }
        self.inner.wake(cx, reason);
        if let Err(error) = self.drive(cx, Trigger::Wake) { self.stop(cx, error); }
    }
    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy {
        let mut policy = self.inner.idle(cx);
        if let Some(at) = self.deadline { policy = policy.merge(IdlePolicy::BlockUntil(at)); }
        if let Some(at) = self.retry_at { policy = policy.merge(IdlePolicy::BlockUntil(at)); }
        policy
    }
    fn deadline_reached(&mut self, cx: &dyn PlatformCx) {
        if self.stage == Stage::Finished { return; }
        self.inner.deadline_reached(cx);
        if let Err(error) = self.drive(cx, Trigger::Other) { self.stop(cx, error); }
        if self.stage != Stage::Finished && self.deadline.is_some_and(|at| cx.clock().now() >= at) { self.stop(cx, "native reload gate timed out; requires an unlocked visible desktop"); }
    }
    fn shutting_down(&mut self, cx: &dyn PlatformCx) { self.inner.shutting_down(cx); }
}

#[test]
#[ignore = "opens a real Wayland window; requires an unlocked desktop and Node dependencies"]
fn native_full_reload() -> TestResult {
    if std::env::var("ZGUI_PLATFORM").as_deref() != Ok("wayland") { return Err("run with ZGUI_PLATFORM=wayland".into()); }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).ok_or("workspace missing")?;
    let output = Command::new("node").arg(root.join("packages/build/src/cli.mjs")).args(["build", "--app", "async"]).current_dir(root).output()?;
    if !output.status.success() { return Err(format!("async build failed: {}", String::from_utf8_lossy(&output.stderr)).into()); }
    let good = Rc::new(Package::load(&root.join("apps/examples/async/dist"))?);
    let bad_eval = Fixture::new("throw Error('evaluation exploded'); export const hostApiVersion = 1; export function mount() { return () => {}; }", "");
    let bad_mount = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            __zgui.insertNode(root, __zgui.createTextNode('partial'), null);
            setTimeout(() => {}, 60000);
            requestAnimationFrame(() => {});
            __zguiServices.readTextAsset('test.txt');
            throw Error('mount exploded');
        }
    "#, "text { color: red }");
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let native = zgui::app().with_title("zgui-js native reload acceptance gate").with_application_id("dev.zgui.js.native-reload")
        .with_size(760.0, 560.0).with_stylesheet(":root { background: #171b24; color: #edf2fa; font-size: 20px; padding: 16px; }")
        .into_handler(move || Root(roots.clone()))?.into_runtime();
    let native_failure = native.failure();
    let inner = Application::new(native, pending, Some(good.clone()))?;
    let failure = inner.failure.clone();
    let failures = Rc::new(RefCell::new(Vec::new()));
    let completed = Rc::new(Cell::new(false));
    let retired = Rc::new(RefCell::new(Vec::new()));
    let driver = ReloadGate { inner, good, bad_eval, bad_mount, stage: Stage::Initial, deadline: None, retry_at: None,
        failed_generation: 0, recovered_generation: 0, retired: retired.clone(), failures: failures.clone(), completed: completed.clone() };
    let result = zgui::app::desktop()(Box::new(driver));
    zgui::reactive::shutdown();
    result?;
    if let Some(error) = native_failure.take() { failures.borrow_mut().push(error.to_string()); }
    if let Some(error) = failure.borrow_mut().take() { failures.borrow_mut().push(error); }
    if !completed.get() { failures.borrow_mut().push("native reload acceptance did not complete".into()); }
    for session in retired.borrow().iter() {
        if session.counts() != ResourceCounts::default() { failures.borrow_mut().push(format!("shutdown retained generation {}: {:?}", session.generation, session.counts())); }
    }
    if !failures.borrow().is_empty() { return Err(failures.borrow().join("\n").into()); }
    Ok(())
}
