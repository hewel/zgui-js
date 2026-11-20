//! Realize one generation's shell requests outside JavaScript and native window frames.
use std::collections::{BTreeMap, BTreeSet};

use zgui::{app::Fonts, geom::{Css, CssPx, Size}, platform::{InputRegion, PlatformCx, SessionLockState, SurfaceAttributes, SurfaceId, SurfaceRole}, runtime::{Runtime, WindowContent, WindowSpec, WindowToken}, view::View};

use super::{ImageChoice, MountRegion, PredecessorLeak, ReloadOutcome, Root, RuntimeSurface, SessionHost, SessionSurface, SurfaceOpacity};
use crate::{session::{ResourceCounts, Result, Session, SessionModule, SessionRetirement}, shell::{LayerSpec, LockCommand, LockSpec, Monitor}};
use std::{cell::RefCell, rc::Rc};

mod popup;
mod window;
use popup::RunningPopup;
use window::RunningWindow;

struct RunningLayer {
    token: WindowToken,
    surface: SurfaceId,
    spec: LayerSpec,
}

struct RunningLock {
    token: WindowToken,
    surface: SurfaceId,
    spec: LockSpec,
}

enum ShellKind { Layer, Lock }

#[derive(Clone, Copy)]
enum LockReconcile<'a> { Current, Stage(&'a ShellWindows), Release }

#[derive(Default)]
pub(crate) struct ShellWindows {
    pub(super) generation: Option<u64>,
    opacity: SurfaceOpacity,
    running: BTreeMap<u32, RunningLayer>,
    dismissed: BTreeSet<u32>,
    windows: BTreeMap<u32, RunningWindow>,
    dismissed_windows: BTreeSet<u32>,
    popups: Vec<RunningPopup>,
    dismissed_popups: BTreeSet<u32>,
    popups_waiting_for_geometry: bool,
    locks: BTreeMap<u32, RunningLock>,
    lock_ui_suspended: bool,
}

impl SessionHost {
    /// Configure a generation that launches without a primary mount region.
    pub(crate) fn set_pure_shell(&mut self, fonts: Fonts) { self.pure_fonts = Some(fonts); }

    /// Whether package replacement goes through the zero-primary startup path.
    pub(crate) fn is_pure_shell(&self) -> bool { self.pure_fonts.is_some() }

    /// Supply the initial monitor inventory before candidate mounting.
    pub(crate) fn set_monitor_snapshot(&mut self, monitors: &[Monitor]) { self.monitors.clear(); self.monitors.extend_from_slice(monitors); }

    /// Mount a candidate and its shell surfaces before replacing the active generation.
    pub(crate) fn replace_shell(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, module: SessionModule) -> ReloadOutcome {
        let outcome = self.mount_shell(runtime, cx, images, &module);
        self.record_outcome(&module, None, &outcome);
        outcome
    }

    fn mount_shell(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, module: &SessionModule) -> ReloadOutcome {
        self.pure_attempted = true;
        let Some(fonts) = &self.pure_fonts else { return ReloadOutcome::Rejected("pure-shell startup was not configured".into()); };
        let Some(generation) = Self::allocate_generation(&mut self.generation) else { return ReloadOutcome::Rejected("session generation exhausted".into()); };
        let policy = self.policy.clone().with_session_lock(platform_lock_state(cx));
        let candidate = match Session::prepare_shell(fonts.clone(), runtime.app_timers(), module.clone(), generation, cx.waker(), policy) {
            Ok(candidate) => candidate,
            Err(error) => return ReloadOutcome::Rejected(error),
        };
        let mut staged = self.shell_windows.empty_successor();
        let result = candidate.update_monitors(&self.monitors)
            .and_then(|_| candidate.mount())
            .and_then(|()| staged.mount(runtime, cx, images, &self.shell_windows, &self.monitors, &candidate));
        if let Err(error) = result {
            staged.retire(&candidate, |token, id| close_native_surface(runtime, cx, token, id));
            if self.pure_session.is_none() { self.pure_session = Some(candidate); }
            return ReloadOutcome::Rejected(error);
        }
        let predecessor_leak = self.pure_session.take().and_then(|previous| {
            let report = self.shell_windows.retire(&previous, |token, id| close_native_surface(runtime, cx, token, id));
            staged.fence_after_predecessor(report, &candidate, platform_lock_state(cx), |token, id| close_native_surface(runtime, cx, token, id))
        });
        self.shell_windows = staged;
        self.module = module.clone();
        self.last_good = Some(module.clone());
        self.pure_session = Some(candidate);
        ReloadOutcome::Mounted { generation, predecessor_leak }
    }

    /// Deliver output changes, retire superseded surfaces and realize pending shell requests.
    pub(crate) fn shell_turn(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, monitors: &[Monitor]) -> Result<()> {
        if self.monitors != monitors { self.set_monitor_snapshot(monitors); }
        if self.is_pure_shell() && !self.pure_attempted {
            match self.replace_shell(runtime, cx, images, self.module.clone()) {
                ReloadOutcome::Mounted { .. } => {}
                ReloadOutcome::Rejected(error) | ReloadOutcome::Fallback(error) => return Err(error),
            }
        }
        let session = self.sessions().find(|session| Some(session.generation) == self.shell_windows.generation).cloned();
        let Some(session) = session else { return Ok(()); };
        let result = self.step_shell(runtime, cx, images, &session);
        if result.is_err() {
            self.shell_windows.retire(&session, |token, id| close_native_surface(runtime, cx, token, id));
        }
        result
    }

    fn step_shell(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, session: &Rc<Session>) -> Result<()> {
        let state = cx.session_lock_state();
        let mut worked = session.update_monitors(&self.monitors)?;
        worked |= session.update_lock_state(state)?;
        self.shell_windows.settle(runtime, cx, images, &self.monitors, session, LockReconcile::Current)?;
        let surface = RuntimeSurface::new(runtime, cx, images);
        if session.primary().is_none() || session.shell.has_lock_command() {
            surface.validate_fonts(session)?;
            worked |= session.checkpoint()?;
        }
        if worked { session.request_frames(|document| surface.request_frame(document)); }
        // A checkpoint can close an existing lock UI or enqueue a child
        // mount. Let that structural work succeed before performing commands.
        if cx.is_exiting() || session.shell.is_dirty() || session.shell.locks_are_dirty() || !session.lock_commands_ready() {
            return Ok(());
        }
        match session.shell.take_lock_command() {
            Some(LockCommand::Request) => cx.request_session_lock().map_err(|error| error.to_string())?,
            Some(LockCommand::Unlock) if cx.session_lock_state() == SessionLockState::Locked => {
                // Return UI claims to the dark fallback before unlocking. A
                // child disposer fault must prevent the protocol request.
                self.shell_windows.settle(runtime, cx, images, &self.monitors, session, LockReconcile::Release)?;
                session.check_unlock_teardown_rejections()?;
                if session.shell.is_dirty() || session.shell.locks_are_dirty() || !session.lock_commands_ready() {
                    return Err("lock UI disposal did not settle before unlock".into());
                }
                if !cx.is_exiting() && cx.session_lock_state() == SessionLockState::Locked {
                    cx.unlock_session().map_err(|error| error.to_string())?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Target the current extent/monitor of a named preview surface without querying Wayland.
    pub(crate) fn shell_geometry(&self, surface: SurfaceId) -> Option<(f64, f64, Option<&str>)> {
        if let Some(lock) = self.shell_windows.locks.values().find(|lock| lock.surface == surface) {
            let monitor = self.monitors.iter().find(|monitor| monitor.id == lock.spec.monitor)?;
            let size = lock_size(monitor);
            return Some((f64::from(size.width.0), f64::from(size.height.0), Some(&lock.spec.monitor)));
        }
        if let Some(popup) = self.shell_windows.popups.iter().find(|popup| popup.surface == surface) {
            let mut parent = popup.parent;
            while let Some(ancestor) = self.shell_windows.popups.iter().find(|popup| popup.surface == parent) {
                parent = ancestor.parent;
            }
            let output = self.shell_windows.running.values().find(|layer| layer.surface == parent)
                .and_then(|layer| layer.spec.placement.monitor.as_deref());
            let size = popup.spec.size();
            return Some((f64::from(size.width.0), f64::from(size.height.0), output));
        }
        if let Some(window) = self.shell_windows.windows.values().find(|window| window.surface == surface) {
            return Some((f64::from(window.spec.size.width.0), f64::from(window.spec.size.height.0), None));
        }
        let running = self.shell_windows.running.values().find(|running| running.surface == surface)?;
        Some((f64::from(running.spec.size.width.0), f64::from(running.spec.size.height.0), running.spec.placement.monitor.as_deref()))
    }

    /// Recreate one named surface for preview while retaining its session and other regions.
    pub(crate) fn recreate_shell_surface(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, monitors: &[Monitor], name: &str) -> Result<()> {
        let session = self.sessions().next().cloned().ok_or("no active session")?;
        let id = self.shell_windows.running.iter().find(|(_, layer)| layer.spec.name == name).map(|(&id, _)| id)
            .ok_or_else(|| format!("shell surface {name:?} is not active"))?;
        let mut spec = session.shell.specs().remove(&id).ok_or("shell surface was closed")?;
        spec.size = used_size(&spec, monitors, images)?;
        let result = self.shell_windows.close(runtime, cx, &session, id)
            .and_then(|()| self.shell_windows.open(runtime, cx, images, &session, id, spec));
        if result.is_err() {
            self.shell_windows.retire(&session, |token, id| close_native_surface(runtime, cx, token, id));
        }
        result
    }

    /// Dispose the shell owner and destroy its native surfaces before checking for leaks.
    pub(crate) fn stop_shell_surfaces(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx) {
        let owner = self.sessions().find(|session| Some(session.generation) == self.shell_windows.generation).cloned();
        if let Some(owner) = owner {
            self.shell_windows.retire(&owner, |token, id| close_native_surface(runtime, cx, token, id));
        }
    }

    /// Restore platform-owned fallback without running application cleanup or losing fault evidence.
    pub(crate) fn hide_lock_ui(&mut self) {
        let owner = self.sessions().find(|session| Some(session.generation) == self.shell_windows.generation).cloned();
        if let Some(owner) = owner {
            self.shell_windows.hide_lock_ui(&owner);
        }
    }
}

impl ShellWindows {
    /// Start with no shell surfaces and retain the same policy across replacements.
    #[must_use]
    pub(super) fn new(opacity: SurfaceOpacity) -> Self {
        Self { opacity, ..Self::default() }
    }

    /// Give a candidate generation empty surface ownership under the same opacity policy.
    #[must_use]
    pub(super) fn empty_successor(&self) -> Self {
        Self::new(self.opacity)
    }

    pub(super) fn mount(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, previous: &Self, monitors: &[Monitor], session: &Rc<Session>) -> Result<()> {
        self.generation = Some(session.generation);
        // Stage replacement claims without dropping the currently presented content.
        self.settle(runtime, cx, images, monitors, session, LockReconcile::Stage(previous))
    }

    fn settle(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, monitors: &[Monitor], session: &Rc<Session>, locks: LockReconcile<'_>) -> Result<()> {
        for _ in 0..=2 * crate::shell::MAX_LAYERS {
            self.reconcile(runtime, cx, images, monitors, session)?;
            self.reconcile_locks(runtime, cx, images, monitors, session, locks)?;
            if !session.shell.is_dirty() && !session.shell.locks_are_dirty() { return Ok(()); }
        }
        Err("shell surface requests did not settle".into())
    }

    pub(super) fn has_windows(&self) -> bool {
        !self.running.is_empty() || !self.windows.is_empty() || !self.popups.is_empty() || !self.locks.is_empty()
    }

    pub(super) fn retire(
        &mut self,
        session: &Session,
        mut close: impl FnMut(WindowToken, Option<SurfaceId>) -> bool,
    ) -> SessionRetirement {
        if self.generation == Some(session.generation) {
            session.dispose(self.popups.iter().rev().map(|popup| popup.id));
            for popup in self.popups.drain(..).rev() {
                if close(popup.token, Some(popup.surface)) { session.shell_surface_closed(); }
            }
            self.dismissed_popups.clear();
            self.popups_waiting_for_geometry = false;
            for lock in std::mem::take(&mut self.locks).into_values() {
                if close(lock.token, None) { session.shell_surface_closed(); }
            }
            for window in std::mem::take(&mut self.windows).into_values() {
                if close(window.token, Some(window.surface)) { session.shell_surface_closed(); }
            }
            self.dismissed_windows.clear();
            for layer in std::mem::take(&mut self.running).into_values() {
                if close(layer.token, Some(layer.surface)) { session.shell_surface_closed(); }
            }
            self.dismissed.clear();
            self.generation = None;
        }
        session.retire()
    }

    pub(super) fn hide_lock_ui(&mut self, session: &Session) {
        if self.generation != Some(session.generation) { return; }
        session.shell.take_lock_command();
        self.lock_ui_suspended = true;
        session.hide_lock_ui();
    }

    pub(super) fn fence_after_predecessor(
        &mut self,
        report: SessionRetirement,
        session: &Session,
        state: Option<SessionLockState>,
        close: impl FnMut(WindowToken, Option<SurfaceId>) -> bool,
    ) -> Option<PredecessorLeak> {
        let leaked = report.counts != ResourceCounts::default();
        if (report.failure.is_some() || leaked) && state.is_some_and(SessionLockState::is_retained) {
            session.shell.fence_lock_authority();
            self.suspend_lock_ui(session, close);
        }
        leaked.then_some(PredecessorLeak {
            code: "E_TEARDOWN", generation: report.generation, resources: report.counts,
        })
    }

    fn suspend_lock_ui(&mut self, session: &Session, mut close: impl FnMut(WindowToken, Option<SurfaceId>) -> bool) {
        self.hide_lock_ui(session);
        for (id, lock) in std::mem::take(&mut self.locks) {
            if let Err(error) = session.detach_layer(id) { session.record_fault(error); }
            if close(lock.token, None) {
                session.shell_surface_closed();
            } else {
                session.record_fault("E_TEARDOWN: lock UI claim remained open".into());
            }
        }
    }

    fn reconcile_locks(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, monitors: &[Monitor], session: &Rc<Session>, mode: LockReconcile<'_>) -> Result<()> {
        let state = match mode {
            LockReconcile::Release => SessionLockState::Unlocking,
            _ => cx.session_lock_state(),
        };
        let mut needs_sync = session.shell.take_lock_dirty();
        while let Some((&id, _)) = self.locks.iter().find(|(id, lock)| {
            state != SessionLockState::Locked || runtime.surface_id(lock.token) != Some(lock.surface) || cx.surface(lock.surface).is_none()
                || !session.shell.owns_lock(**id) || !monitors.iter().any(|output| output.id == lock.spec.monitor)
        }) {
            let lock = self.locks.remove(&id).expect("closing a tracked lock region");
            needs_sync = true;
            if let Some(claim) = cx.surface(lock.surface) { claim.set_visible(false); }
            let cleanup = session.detach_layer(id);
            if !close_native_surface(runtime, cx, lock.token, None) {
                return Err("E_TEARDOWN: lock UI claim remained open".into());
            }
            session.shell_surface_closed();
            cleanup?;
        }
        if state != SessionLockState::Locked || self.lock_ui_suspended || !needs_sync { return Ok(()); }
        for (id, spec) in session.shell.lock_specs() {
            let Some(monitor) = monitors.iter().find(|monitor| monitor.id == spec.monitor) else { continue; };
            if self.locks.contains_key(&id) || !session.shell.owns_lock(id) { continue; }
            let mut attrs = SurfaceAttributes::new(spec.name.as_str());
            attrs.role = SurfaceRole::Lock(spec.monitor.clone());
            if let LockReconcile::Stage(previous) = mode {
                attrs.replacing_lock = previous.locks.values().find(|lock| {
                    lock.spec.monitor == spec.monitor && cx.surface(lock.surface).is_some()
                }).map(|lock| lock.surface);
            }
            attrs.size = Some(lock_size(monitor));
            attrs.transparent = false;
            let (token, surface, region) = open_region(runtime, cx, session, attrs, WindowContent::default())?;
            let document = region.document;
            let name = spec.name.clone();
            self.locks.insert(id, RunningLock { token, surface, spec });
            let mut binding = RuntimeSurface::new(runtime, cx, images);
            binding.attach_shell(session, id, name, region, surface, ShellKind::Lock)?;
            binding.request_frame(document);
        }
        Ok(())
    }

    fn reconcile(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, monitors: &[Monitor], session: &Rc<Session>) -> Result<()> {
        let mut needs_sync = session.shell.take_dirty();
        needs_sync |= self.reconcile_popup_removals(runtime, cx, session)?;
        needs_sync |= self.reconcile_window_removals(runtime, cx, session)?;
        while let Some((&id, _)) = self.running.iter().find(|(_, layer)| runtime.surface_id(layer.token) != Some(layer.surface)) {
            // A compositor closes a role permanently. A disconnected output may
            // return with a new proxy; a dismissed role on a live output must not loop.
            if present(&self.running[&id].spec, monitors) { self.dismissed.insert(id); }
            self.close(runtime, cx, session, id)?;
            needs_sync = true;
        }
        if !needs_sync {
            return if self.popups_waiting_for_geometry { self.reconcile_popups(runtime, cx, images, session) } else { Ok(()) };
        }
        let specs = session.shell.specs();
        self.dismissed.retain(|id| specs.get(id).is_some_and(|spec| present(spec, monitors)));
        while let Some((&id, _)) = self.running.iter().find(|(id, _)| !specs.get(id).is_some_and(|spec| present(spec, monitors))) {
            self.close(runtime, cx, session, id)?;
        }
        for (id, mut spec) in specs {
            if self.dismissed.contains(&id) || !present(&spec, monitors) || !session.shell.owns(id) { continue; }
            spec.size = used_size(&spec, monitors, images)?;
            if let Some(running) = self.running.get_mut(&id) {
                if running.spec.placement != spec.placement || running.spec.size != spec.size {
                    runtime.reconfigure_layer(running.token, spec.placement.clone(), spec.size)
                        .map_err(|error| error.to_string())?;
                }
                if running.spec.input != spec.input {
                    let surface = cx.surface(running.surface).ok_or("layer surface disappeared during update")?;
                    surface.set_input_region(&spec.input).map_err(|error| error.to_string())?;
                }
                running.spec = spec;
            } else {
                self.open(runtime, cx, images, session, id, spec)?;
            }
        }
        self.reconcile_windows(runtime, cx, images, session)?;
        self.reconcile_popups(runtime, cx, images, session)
    }

    fn close(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session, id: u32) -> Result<()> {
        let Some(running) = self.running.get(&id) else { return Ok(()); };
        self.close_child_popups(runtime, cx, session, running.surface)?;
        let running = self.running.remove(&id).expect("layer remains tracked until its popups close");
        let result = session.detach_layer(id);
        if !close_native_surface(runtime, cx, running.token, Some(running.surface)) {
            return Err("E_TEARDOWN: native shell surface remained open".into());
        }
        session.shell_surface_closed();
        result
    }

    fn open(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, session: &Rc<Session>, id: u32, spec: LayerSpec) -> Result<()> {
        if matches!(images, ImageChoice::Live) && !cx.capabilities().layer_surfaces {
            return Err("selected native backend does not support layer surfaces".into());
        }
        let name = spec.name.clone();
        let mut attrs = SurfaceAttributes::new(spec.name.as_str());
        attrs.role = SurfaceRole::Layer(spec.placement.clone());
        attrs.size = Some(spec.size);
        attrs.transparent = self.opacity == SurfaceOpacity::Transparent;
        let content = shell_window_content(attrs.transparent, images);
        let (token, surface, region) = open_region(runtime, cx, session, attrs, content)?;
        let document = region.document;
        self.running.insert(id, RunningLayer { token, surface, spec });
        RuntimeSurface::new(runtime, cx, images).attach_shell(session, id, name, region, surface, ShellKind::Layer)?;
        let running = self.running.get_mut(&id).expect("new layer is tracked before its first frame");
        if let Some(input) = session.shell.updated_input(id, &running.spec.input) { running.spec.input = input; }
        let input = &running.spec.input;
        if *input != InputRegion::Full {
            cx.surface(surface).ok_or("opened shell window has no platform surface")?
                .set_input_region(input).map_err(|error| error.to_string())?;
        }
        RuntimeSurface::new(runtime, cx, images).request_frame(document);
        Ok(())
    }
}

pub(super) fn platform_lock_state(cx: &dyn PlatformCx) -> Option<SessionLockState> {
    cx.capabilities().session_lock.then(|| cx.session_lock_state())
}

fn lock_size(monitor: &Monitor) -> Size<CssPx, Css> {
    Size::new(CssPx(monitor.logical_width().max(1.0) as f32),
        CssPx(monitor.logical_height().max(1.0) as f32))
}

pub(super) fn shell_window_content(transparent: bool, images: ImageChoice) -> WindowContent {
    let mut content = WindowContent::default();
    // Live transparent shell surfaces must not inherit the opaque demo APPLICATION_STYLESHEET
    // from a hybrid primary window. Preview mode injects its own profile surface background.
    if transparent && matches!(images, ImageChoice::Live) {
        content
            .window_stylesheets
            .push(":root { background: transparent; padding: 0; }".into());
    }
    content
}

fn open_region(
    runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session, attrs: SurfaceAttributes, content: WindowContent,
) -> Result<(WindowToken, SurfaceId, MountRegion)> {
    let is_lock = matches!(&attrs.role, SurfaceRole::Lock(_));
    let fonts = session.fonts.clone();
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let token = runtime.open_surface(cx, WindowSpec::new(attrs, content, Box::new(move |cx| {
        zgui::reactive::provide_context(fonts.clone());
        Box::new(Root(roots.clone()).build(cx))
    }))).map_err(|error| error.to_string())?;
    let Some(surface) = runtime.surface_id(token) else {
        runtime.close_surface(cx, token);
        return Err("opened shell window has no platform surface".into());
    };
    session.shell_surface_opened();
    let Some(mut region) = pending.borrow_mut().pop() else {
        if close_native_surface(runtime, cx, token, (!is_lock).then_some(surface)) { session.shell_surface_closed(); }
        return Err("opened shell window has no mount region".into());
    };
    region.root = region.root_anchor.get();
    Ok((token, surface, region))
}

impl RuntimeSurface<'_> {
    fn attach_shell(&mut self, session: &Rc<Session>, id: u32, name: String, region: MountRegion, surface: SurfaceId, kind: ShellKind) -> Result<()> {
        let document = region.document;
        let claim = match kind {
            ShellKind::Lock => Some(std::sync::Arc::downgrade(&self.cx.surface(surface).ok_or("lock UI has no platform claim")?)),
            ShellKind::Layer => None,
        };
        self.bind(document, session);
        let checkpoint = self.image_source_checkpoint(document);
        if let Err(error) = session.attach_layer(id, name, region, surface, claim) {
            self.discard_image_source_writes(document, checkpoint);
            return Err(error);
        }
        let writes = self.seal_image_source_writes(document, checkpoint);
        self.commit_image_policy(document, session, writes);
        Ok(())
    }
}

pub(super) fn close_native_surface(runtime: &mut Runtime, cx: &dyn PlatformCx, token: WindowToken, removed_surface: Option<SurfaceId>) -> bool {
    runtime.close_surface(cx, token);
    runtime.surface_id(token).is_none() && removed_surface.is_none_or(|id| cx.surface(id).is_none())
}

fn present(spec: &LayerSpec, monitors: &[Monitor]) -> bool {
    spec.placement.monitor.as_ref().is_none_or(|id| monitors.iter().any(|monitor| monitor.id == *id))
}

fn used_size(spec: &LayerSpec, monitors: &[Monitor], images: ImageChoice) -> Result<Size<CssPx, Css>> {
    if matches!(images, ImageChoice::Live) { return Ok(spec.size); }
    let monitor = spec.placement.monitor.as_ref().map_or_else(|| monitors.first(), |id| monitors.iter().find(|monitor| monitor.id == *id));
    let margin = &spec.placement.margin;
    let width = if spec.size.width.0 == 0.0 {
        monitor.map_or(1.0, |output| (output.logical_width() - f64::from(margin.left.0 + margin.right.0)).round().clamp(1.0, 16384.0))
    } else { f64::from(spec.size.width.0) };
    let height = if spec.size.height.0 == 0.0 {
        monitor.map_or(1.0, |output| (output.logical_height() - f64::from(margin.top.0 + margin.bottom.0)).round().clamp(1.0, 16384.0))
    } else { f64::from(spec.size.height.0) };
    let scale = monitor.map_or(1.0, |output| output.scale);
    let physical_width = (width * scale).ceil();
    let physical_height = (height * scale).ceil();
    if physical_width > 16384.0 || physical_height > 16384.0 || (physical_width * physical_height).max(width * height) > 16_777_216.0 {
        return Err("preview shell surface exceeds its physical extent budget".into());
    }
    Ok(Size::new(CssPx(width as f32), CssPx(height as f32)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_transparent_shell_windows_override_root_background_and_padding() {
        let live = shell_window_content(true, ImageChoice::Live);
        assert_eq!(live.window_stylesheets.len(), 1);
        assert!(live.window_stylesheets[0].contains("background: transparent"));
        assert!(live.window_stylesheets[0].contains("padding: 0"));

        let opaque = shell_window_content(false, ImageChoice::Live);
        assert!(opaque.window_stylesheets.is_empty());

        let preview = shell_window_content(true, ImageChoice::PackageOnly);
        assert!(preview.window_stylesheets.is_empty());
    }
}
