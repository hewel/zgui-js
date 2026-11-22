//! The platform itself: a clock, a clipboard, a waker and its offscreen surfaces.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use zgui_geom::{DevicePx, Size};
use zgui_platform::{
    Clipboard, Clock, ColorScheme, FocusGrabId, MonitorInfo, MonitorTarget, PlatformCapabilities,
    PlatformCx, PlatformError, ScrollSettings, SessionLockState as PlatformLockState, Surface,
    SurfaceAttributes, SurfaceId, SurfaceRole, Unsupported, VirtualClock, WakeReason, Waker,
};

use crate::clipboard::MemoryClipboard;
use crate::surface::OffscreenSurface;
use crate::waker::RecordingWaker;

/// The extent of a surface created without one, in physical pixels.
const DEFAULT_SIZE: Size<DevicePx, zgui_geom::Device> = Size::new(DevicePx(640.0), DevicePx(480.0));

/// Where the session lock of a [`Headless`] platform is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionLockState {
    /// No lock was asked for, or the last one ended.
    #[default]
    Unlocked,
    /// The application asked for a lock and the test has not answered.
    Requested,
    /// The test confirmed the lock with [`Headless::confirm_lock`].
    Locked,
}

/// The fixture compositor's decision on the next session-lock request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionLockDecision {
    /// Acquire the lock.
    #[default]
    Lock,
    /// Refuse the lock before acquisition.
    Refuse,
}

/// Observable session-lock evidence from the headless compositor.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SessionLockSnapshot {
    /// The latest compositor decision.
    pub state: PlatformLockState,
    /// The count of explicitly acknowledged unlocks.
    pub unlock_count: u64,
    /// Named outputs with a platform-owned role, sorted by name.
    pub covered_outputs: Vec<String>,
    /// Covered outputs currently showing opaque fallback pixels, sorted by name.
    pub fallback_outputs: Vec<String>,
}

#[derive(Default)]
struct SessionLock {
    state: PlatformLockState,
    next_decision: SessionLockDecision,
    unlock_count: u64,
    outputs: BTreeMap<String, LockOutput>,
}

struct LockOutput {
    target: MonitorTarget,
    monitor: MonitorInfo,
    current: Arc<OffscreenSurface>,
    replacement: Option<Arc<OffscreenSurface>>,
}

impl LockOutput {
    fn surfaces(&self) -> impl Iterator<Item = &Arc<OffscreenSurface>> {
        std::iter::once(&self.current).chain(self.replacement.as_ref())
    }

    fn remove(&self) {
        for surface in self.surfaces() {
            surface.remove_lock();
        }
    }

    fn release(&mut self, id: SurfaceId) -> bool {
        if self.current.id() == id {
            if let Some(replacement) = self.replacement.take() {
                self.current.remove_lock();
                self.current = replacement;
            } else {
                self.current.release_lock();
            }
        } else if let Some(replacement) = self.replacement.take_if(|surface| surface.id() == id) {
            replacement.remove_lock();
        } else {
            return false;
        }
        true
    }
}

fn monitor_key_and_target(monitor: &MonitorInfo) -> Option<(String, MonitorTarget)> {
    if let Some(name) = monitor.name.as_deref() {
        Some((name.to_owned(), MonitorTarget::Name(name.to_owned())))
    } else {
        monitor
            .id
            .map(|id| (format!("id:{}", id.get()), MonitorTarget::Id(id)))
    }
}

impl SessionLock {
    fn sync_outputs(&mut self, monitors: &[MonitorInfo], next_id: impl Fn() -> SurfaceId) {
        self.outputs.retain(|key, output| {
            let present = monitors
                .iter()
                .any(|monitor| monitor_key_and_target(monitor).is_some_and(|(k, _)| k == *key));
            if !present {
                output.remove();
            }
            present
        });
        for monitor in monitors {
            let Some((key, target)) = monitor_key_and_target(monitor) else {
                continue;
            };
            if let Some(output) = self.outputs.get_mut(&key) {
                output.monitor = monitor.clone();
                for surface in output.surfaces() {
                    if surface.size() != monitor.size {
                        surface.resize(monitor.size);
                    }
                    surface.set_scale_factor(monitor.scale_factor);
                    surface.set_refresh_rate_millihertz(monitor.refresh_rate_millihertz);
                }
            } else {
                let surface = OffscreenSurface::new_lock(next_id(), monitor.size);
                let mut attributes = SurfaceAttributes::new("Session lock fallback");
                attributes.role = SurfaceRole::Lock(target.clone());
                surface.set_requested_attributes(&attributes);
                surface.set_scale_factor(monitor.scale_factor);
                surface.set_refresh_rate_millihertz(monitor.refresh_rate_millihertz);
                self.outputs.insert(
                    key,
                    LockOutput {
                        target,
                        monitor: monitor.clone(),
                        current: Arc::new(surface),
                        replacement: None,
                    },
                );
            }
        }
    }

    fn find_output_mut(&mut self, target: &MonitorTarget) -> Option<&mut LockOutput> {
        self.outputs
            .values_mut()
            .find(|output| target.matches(&output.monitor) || output.target == *target)
    }

    fn unlock_internal(&mut self) -> bool {
        if self.state != PlatformLockState::Locked {
            return false;
        }
        self.state = PlatformLockState::Unlocked;
        self.unlock_count += 1;
        for (_, output) in std::mem::take(&mut self.outputs) {
            output.remove();
        }
        true
    }
}

#[derive(Default)]
struct Desktop {
    surfaces: Vec<Arc<OffscreenSurface>>,
    monitors: Vec<MonitorInfo>,
    session_lock: SessionLock,
}

/// A platform with no windowing system behind it.
///
/// Held by a [`Harness`](crate::Harness), which is what drives an application against it. It is
/// also usable on its own by anything that wants the contract's answers without a loop.
pub struct Headless {
    /// The clock, which only moves when it is told to.
    clock: Arc<VirtualClock>,
    /// The clipboards, which are two values in memory.
    clipboard: MemoryClipboard,
    /// What this platform declares it can do.
    capabilities: PlatformCapabilities,
    /// Where a wake from another thread is queued.
    waker: Arc<RecordingWaker>,
    /// The surfaces and monitor inventory, together so a locked output is never published bare.
    desktop: Mutex<Desktop>,
    /// The next surface number, never reused.
    next_surface: AtomicU64,
    /// The desktop's light or dark preference, when one has been declared.
    color_scheme: Mutex<Option<ColorScheme>>,
    /// What this platform says a scroll from its devices means.
    scroll: Mutex<ScrollSettings>,
    /// Whether the loop has been asked to finish.
    exiting: AtomicBool,
    /// Every focus grab that exists, with the surfaces it limits focus to.
    grabs: Mutex<Vec<(FocusGrabId, Vec<SurfaceId>)>>,
    /// The next focus grab number, never reused.
    next_grab: AtomicU64,
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}

impl Headless {
    /// A platform with no surfaces, a clock at its origin and empty clipboards.
    pub fn new() -> Self {
        let waker = Arc::new(RecordingWaker::default());
        let clipboard = MemoryClipboard::default();
        // A read started through the loop is answered through the loop, which needs the waker the
        // platform hands out.
        clipboard.attach_waker(Arc::clone(&waker) as Arc<dyn Waker>);
        let mut capabilities = PlatformCapabilities::none();
        capabilities.window_controls = true;
        Self {
            clock: Arc::new(VirtualClock::new()),
            clipboard,
            capabilities,
            waker,
            desktop: Mutex::new(Desktop::default()),
            next_surface: AtomicU64::new(1),
            color_scheme: Mutex::new(None),
            scroll: Mutex::new(ScrollSettings::desktop()),
            exiting: AtomicBool::new(false),
            grabs: Mutex::new(Vec::new()),
            next_grab: AtomicU64::new(1),
        }
    }

    /// The same platform declaring `capabilities`.
    ///
    /// Enabling `session_lock` opts into the deterministic offscreen compositor lock, not a
    /// lock on the machine running the fixture.
    pub fn with_capabilities(mut self, capabilities: PlatformCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// The same platform reporting `monitors`.
    pub fn with_monitors(self, monitors: Vec<MonitorInfo>) -> Self {
        let mut desktop = self.desktop();
        if matches!(
            desktop.session_lock.state,
            PlatformLockState::Locked | PlatformLockState::Pending
        ) {
            desktop
                .session_lock
                .sync_outputs(&monitors, || self.next_surface_id());
        }
        desktop.monitors = monitors;
        drop(desktop);
        self
    }

    /// Replaces the reported monitor inventory, syncing lock outputs and waking the loop with
    /// [`WakeReason::MonitorsChanged`].
    pub fn set_monitors(&self, monitors: Vec<MonitorInfo>) {
        let mut desktop = self.desktop();
        if matches!(
            desktop.session_lock.state,
            PlatformLockState::Locked | PlatformLockState::Pending
        ) {
            desktop
                .session_lock
                .sync_outputs(&monitors, || self.next_surface_id());
        }
        desktop.monitors = monitors;
        drop(desktop);
        self.waker.wake(WakeReason::MonitorsChanged);
    }

    /// Selects the next fresh request's decision; subsequent requests default to acquisition.
    pub fn set_session_lock_decision(&self, decision: SessionLockDecision) {
        self.desktop().session_lock.next_decision = decision;
    }

    /// Observes lock ownership and presentation without changing them or advancing the fixture.
    #[must_use]
    pub fn session_lock_snapshot(&self) -> SessionLockSnapshot {
        let desktop = self.desktop();
        let lock = &desktop.session_lock;
        SessionLockSnapshot {
            state: lock.state,
            unlock_count: lock.unlock_count,
            covered_outputs: lock.outputs.keys().cloned().collect(),
            fallback_outputs: lock
                .outputs
                .iter()
                .filter(|(_, output)| output.current.lock_fallback_frame().is_some())
                .map(|(name, _)| name.clone())
                .collect(),
        }
    }

    /// Where the session lock is, mapped to the three-state headless test view.
    pub fn session_lock(&self) -> SessionLockState {
        match self.desktop().session_lock.state {
            PlatformLockState::Pending => SessionLockState::Requested,
            PlatformLockState::Locked => SessionLockState::Locked,
            _ => SessionLockState::Unlocked,
        }
    }

    /// Confirms the lock the application asked for, as a desktop does once it hides everything
    /// else, and wakes the loop with [`WakeReason::SessionLocked`].
    ///
    /// Does nothing unless a lock was asked for.
    pub fn confirm_lock(&self) {
        let mut desktop = self.desktop();
        let Desktop {
            monitors,
            session_lock,
            ..
        } = &mut *desktop;
        if session_lock.state == PlatformLockState::Pending {
            session_lock.sync_outputs(monitors, || self.next_surface_id());
            session_lock.state = PlatformLockState::Locked;
            drop(desktop);
            self.waker.wake(WakeReason::SessionLocked);
        }
    }

    /// Refuses or ends the lock, as a desktop does, and wakes the loop with
    /// [`WakeReason::SessionLockFinished`].
    ///
    /// Does nothing while no lock is asked for or held.
    pub fn finish_lock(&self) {
        let mut desktop = self.desktop();
        let lock = &mut desktop.session_lock;
        if matches!(
            lock.state,
            PlatformLockState::Pending | PlatformLockState::Locked
        ) {
            lock.state = PlatformLockState::Unlocked;
            for (_, output) in std::mem::take(&mut lock.outputs) {
                output.remove();
            }
            drop(desktop);
            self.waker.wake(WakeReason::SessionLockFinished);
        }
    }

    /// The surfaces focus grab `grab` limits focus to, while it exists.
    pub fn focus_grab(&self, grab: FocusGrabId) -> Option<Vec<SurfaceId>> {
        self.grabs
            .lock()
            .expect("the grabs are not poisoned")
            .iter()
            .find(|(held, _)| *held == grab)
            .map(|(_, surfaces)| surfaces.clone())
    }

    /// Every focus grab that exists, oldest first.
    pub fn focus_grabs(&self) -> Vec<FocusGrabId> {
        self.grabs
            .lock()
            .expect("the grabs are not poisoned")
            .iter()
            .map(|(grab, _)| *grab)
            .collect()
    }

    /// Clears focus grab `grab`, as a desktop does when the user presses outside its surfaces,
    /// and wakes the loop with [`WakeReason::FocusGrabCleared`].
    pub fn clear_focus_grab(&self, grab: FocusGrabId) {
        let cleared = {
            let mut grabs = self.grabs.lock().expect("the grabs are not poisoned");
            grabs
                .iter_mut()
                .find(|(held, surfaces)| *held == grab && !surfaces.is_empty())
                .map(|(_, surfaces)| surfaces.clear())
                .is_some()
        };
        if cleared {
            self.waker.wake(WakeReason::FocusGrabCleared(grab));
        }
    }

    /// The same platform declaring what a scroll from its devices means.
    pub fn with_scroll_settings(self, settings: ScrollSettings) -> Self {
        self.set_scroll_settings(settings);
        self
    }

    /// Declares what a scroll from this platform's devices means, after it has been built.
    pub fn set_scroll_settings(&self, settings: ScrollSettings) {
        *self.scroll.lock().expect("the settings are not poisoned") = settings;
    }

    /// Declares the desktop's light or dark preference.
    pub fn set_color_scheme(&self, scheme: Option<ColorScheme>) {
        *self
            .color_scheme
            .lock()
            .expect("the preference is not poisoned") = scheme;
    }

    /// The clock, as itself rather than as the contract's view of it.
    pub fn virtual_clock(&self) -> &VirtualClock {
        &self.clock
    }

    /// The in-memory clipboard, as itself rather than as the contract's view of it.
    pub fn memory_clipboard(&self) -> &MemoryClipboard {
        &self.clipboard
    }

    /// The offscreen surface with this identifier, as itself.
    pub fn offscreen(&self, id: SurfaceId) -> Option<Arc<OffscreenSurface>> {
        let desktop = self.desktop();
        desktop
            .surfaces
            .iter()
            .chain(
                desktop
                    .session_lock
                    .outputs
                    .values()
                    .flat_map(LockOutput::surfaces),
            )
            .find(|surface| surface.id() == id)
            .map(Arc::clone)
    }

    /// Every offscreen surface that exists, as itself.
    pub fn offscreens(&self) -> Vec<Arc<OffscreenSurface>> {
        let desktop = self.desktop();
        desktop
            .surfaces
            .iter()
            .chain(
                desktop
                    .session_lock
                    .outputs
                    .values()
                    .flat_map(LockOutput::surfaces),
            )
            .map(Arc::clone)
            .collect()
    }

    /// Takes every wake delivered since the last call.
    pub fn drain_wakes(&self) -> Vec<WakeReason> {
        self.waker.drain()
    }

    /// Whether anything is waiting to be delivered as a wake.
    pub fn has_pending_wakes(&self) -> bool {
        self.waker.pending() > 0
    }

    fn desktop(&self) -> MutexGuard<'_, Desktop> {
        self.desktop.lock().expect("the desktop is not poisoned")
    }

    fn next_surface_id(&self) -> SurfaceId {
        SurfaceId::new(self.next_surface.fetch_add(1, Ordering::Relaxed))
    }
}

impl PlatformCx for Headless {
    fn create_surface(
        &self,
        attributes: &SurfaceAttributes,
    ) -> Result<Arc<dyn Surface>, PlatformError> {
        if let SurfaceRole::Lock(target) = &attributes.role {
            if !self.capabilities.session_lock {
                return Err(Unsupported.into());
            }
            let mut desktop = self.desktop();
            let output = desktop
                .session_lock
                .find_output_mut(target)
                .ok_or_else(|| {
                    PlatformError::SurfaceCreation(format!(
                        "no session-lock surface on {target:?}"
                    ))
                })?;
            let surface = if let Some(previous) = attributes.replacing_lock {
                if output.current.id() != previous
                    || !output.current.lock_is_claimed()
                    || output.replacement.is_some()
                {
                    return Err(PlatformError::SurfaceCreation(format!(
                        "no replaceable lock claim on {target:?}"
                    )));
                }
                let surface = Arc::new(OffscreenSurface::new_lock(
                    self.next_surface_id(),
                    output.current.size(),
                ));
                surface.set_scale_factor(output.current.scale_factor());
                surface.set_refresh_rate_millihertz(output.current.refresh_rate_millihertz());
                output.replacement.insert(surface)
            } else {
                &output.current
            };
            if !surface.claim_lock() {
                return Err(PlatformError::SurfaceCreation(format!(
                    "the session-lock surface on {target:?} is already claimed"
                )));
            }
            surface.set_title(attributes.title.as_str());
            surface.set_requested_attributes(attributes);
            return Ok(Arc::clone(surface) as Arc<dyn Surface>);
        }
        let mut desktop = self.desktop();
        let size = crate::role::extent(attributes, &desktop.monitors, DEFAULT_SIZE, |parent| {
            desktop
                .surfaces
                .iter()
                .any(|surface| surface.id() == parent && surface.can_parent_popup())
        })?;
        let surface = Arc::new(OffscreenSurface::new(self.next_surface_id(), size));
        surface.set_title(attributes.title.as_str());
        surface.set_requested_attributes(attributes);
        desktop.surfaces.push(Arc::clone(&surface));
        Ok(surface as Arc<dyn Surface>)
    }

    fn destroy_surface(&self, id: SurfaceId) {
        let mut desktop = self.desktop();
        if desktop
            .session_lock
            .outputs
            .values_mut()
            .any(|output| output.release(id))
        {
            return;
        }
        // A child can only have been created after its parent, so a single forward scan
        // discovers every descendant. Remove the snapshot in the opposite order.
        let mut descendants = Vec::new();
        for surface in &desktop.surfaces {
            if surface
                .popup_parent()
                .is_some_and(|parent| parent == id || descendants.contains(&parent))
            {
                descendants.push(surface.id());
            }
        }
        for closing in descendants.into_iter().rev().chain(std::iter::once(id)) {
            if let Some(index) = desktop
                .surfaces
                .iter()
                .position(|surface| surface.id() == closing)
            {
                desktop.surfaces.remove(index);
            }
        }
    }

    fn surface(&self, id: SurfaceId) -> Option<Arc<dyn Surface>> {
        self.offscreen(id)
            .map(|surface| surface as Arc<dyn Surface>)
    }

    fn surfaces(&self) -> Vec<Arc<dyn Surface>> {
        self.offscreens()
            .into_iter()
            .map(|surface| surface as Arc<dyn Surface>)
            .collect()
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        self.desktop().monitors.clone()
    }

    fn primary_monitor(&self) -> Option<MonitorInfo> {
        self.desktop().monitors.first().cloned()
    }

    fn create_focus_grab(&self) -> Option<FocusGrabId> {
        if !self.capabilities.focus_grab {
            return None;
        }
        let grab = FocusGrabId::new(self.next_grab.fetch_add(1, Ordering::Relaxed));
        self.grabs
            .lock()
            .expect("the grabs are not poisoned")
            .push((grab, Vec::new()));
        Some(grab)
    }

    fn set_focus_grab(&self, grab: FocusGrabId, surfaces: &[SurfaceId]) {
        let existing: Vec<SurfaceId> = surfaces
            .iter()
            .copied()
            .filter(|surface| self.offscreen(*surface).is_some())
            .collect();
        if let Some((_, held)) = self
            .grabs
            .lock()
            .expect("the grabs are not poisoned")
            .iter_mut()
            .find(|(held, _)| *held == grab)
        {
            *held = existing;
        }
    }

    fn destroy_focus_grab(&self, grab: FocusGrabId) {
        self.grabs
            .lock()
            .expect("the grabs are not poisoned")
            .retain(|(held, _)| *held != grab);
    }

    fn lock_session(&self) -> Result<(), PlatformError> {
        if !self.capabilities.session_lock {
            return Err(Unsupported.into());
        }
        let mut desktop = self.desktop();
        let Desktop {
            monitors,
            session_lock,
            ..
        } = &mut *desktop;
        if matches!(
            session_lock.state,
            PlatformLockState::Pending | PlatformLockState::Locked | PlatformLockState::Unlocking
        ) {
            return Err(PlatformError::Backend(
                "this application holds a session lock already".to_owned(),
            ));
        }
        session_lock.sync_outputs(monitors, || self.next_surface_id());
        session_lock.state = PlatformLockState::Pending;
        Ok(())
    }

    fn request_session_lock(&self) -> Result<(), PlatformError> {
        if !self.capabilities.session_lock {
            return Err(Unsupported.into());
        }
        let mut desktop = self.desktop();
        let Desktop {
            monitors,
            session_lock,
            ..
        } = &mut *desktop;
        match session_lock.state {
            PlatformLockState::Locked | PlatformLockState::Pending => return Ok(()),
            PlatformLockState::Refused
            | PlatformLockState::Unlocked
            | PlatformLockState::Finished => {}
            _ => {
                return Err(PlatformError::Backend(
                    "cannot replace the active session lock".to_owned(),
                ));
            }
        }
        let (next, wake) = match std::mem::take(&mut session_lock.next_decision) {
            SessionLockDecision::Lock => {
                session_lock.sync_outputs(monitors, || self.next_surface_id());
                (PlatformLockState::Locked, WakeReason::SessionLocked)
            }
            SessionLockDecision::Refuse => {
                (PlatformLockState::Refused, WakeReason::SessionLockFinished)
            }
        };
        if next != session_lock.state {
            session_lock.state = next;
            drop(desktop);
            self.waker.wake(wake);
        }
        Ok(())
    }

    fn session_lock_state(&self) -> PlatformLockState {
        self.desktop().session_lock.state
    }

    fn unlock_session(&self) {
        if !self.capabilities.session_lock {
            return;
        }
        let mut desktop = self.desktop();
        desktop.session_lock.unlock_internal();
    }

    fn try_unlock_session(&self) -> Result<(), PlatformError> {
        if !self.capabilities.session_lock {
            return Err(Unsupported.into());
        }
        let mut desktop = self.desktop();
        if !desktop.session_lock.unlock_internal() {
            return Err(PlatformError::Backend(
                "no acquired session lock to unlock".to_owned(),
            ));
        }
        drop(desktop);
        self.waker.wake(WakeReason::SessionLockFinished);
        Ok(())
    }

    fn color_scheme(&self) -> Option<ColorScheme> {
        *self
            .color_scheme
            .lock()
            .expect("the preference is not poisoned")
    }

    fn clipboard(&self) -> &dyn Clipboard {
        &self.clipboard
    }

    fn capabilities(&self) -> &PlatformCapabilities {
        &self.capabilities
    }

    fn scroll_settings(&self) -> ScrollSettings {
        *self.scroll.lock().expect("the settings are not poisoned")
    }

    fn clock(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.clock) as Arc<dyn Clock>
    }

    fn waker(&self) -> Arc<dyn Waker> {
        Arc::clone(&self.waker) as Arc<dyn Waker>
    }

    fn request_exit(&self) {
        self.exiting.store(true, Ordering::Relaxed);
    }

    fn is_exiting(&self) -> bool {
        self.exiting.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::Headless;
    use zgui_geom::{CssPx, Size};
    use zgui_platform::{PlatformCx, SurfaceAttributes};

    #[test]
    fn a_surface_is_created_hidden_with_the_extent_it_asked_for() {
        let platform = Headless::new();
        let surface = platform
            .create_surface(
                &SurfaceAttributes::new("headless")
                    .with_size(Size::new(CssPx(800.0), CssPx(600.0))),
            )
            .expect("always creatable");
        assert_eq!(surface.size().width.0, 800.0);

        let offscreen = platform.offscreen(surface.id()).expect("the same surface");
        assert!(
            !offscreen.is_visible(),
            "a surface is shown by its first frame, not by creation"
        );
        assert_eq!(offscreen.title(), "headless");
    }

    #[test]
    fn a_shell_layer_and_its_pop_up_are_plain_surfaces_whose_placement_is_recorded() {
        use zgui_geom::{Edges, Point, Rect};
        use zgui_platform::{Layer, LayerPatch, LayerPlacement, PopupPlacement, SurfaceRole};

        let platform = Headless::new();
        let bar = LayerPlacement::new(Layer::Top).with_anchors(Edges::new(true, true, false, true));
        let mut attributes =
            SurfaceAttributes::new("bar").with_size(Size::new(CssPx(0.0), CssPx(34.0)));
        attributes.role = SurfaceRole::Layer(bar.clone());
        let surface = platform
            .create_surface(&attributes)
            .expect("a plain surface");
        assert_eq!(
            surface.size().width.0,
            640.0,
            "stretched across the default extent"
        );

        surface
            .set_layer_placement(&LayerPatch::new().with_layer(Layer::Overlay))
            .expect("a layer takes a patch");
        let offscreen = platform.offscreen(surface.id()).expect("the same surface");
        let placed = offscreen.layer_placement().expect("a layer");
        assert_eq!(placed.layer, Layer::Overlay);
        assert_eq!(placed.anchors, bar.anchors);
        assert_eq!(offscreen.popup_placement(), None);

        let rect = Rect::new(
            Point::new(CssPx(4.0), CssPx(0.0)),
            Size::new(CssPx(20.0), CssPx(34.0)),
        );
        let mut menu = SurfaceAttributes::new("menu");
        menu.role = SurfaceRole::Popup(PopupPlacement::below(surface.id(), rect));
        let popup = platform.create_surface(&menu).expect("the parent exists");
        let placed = platform
            .offscreen(popup.id())
            .and_then(|popup| popup.popup_placement())
            .expect("a pop-up");
        assert_eq!(placed.parent, surface.id());
        assert_eq!(placed.anchor_rect, rect);
        assert!(popup.set_layer_placement(&LayerPatch::new()).is_err());

        platform.destroy_surface(surface.id());
        assert!(
            platform.create_surface(&menu).is_err(),
            "a pop-up of a surface that is gone is refused"
        );
    }

    #[test]
    fn every_surface_gets_its_own_identity() {
        let platform = Headless::new();
        let first = platform
            .create_surface(&SurfaceAttributes::new("a"))
            .expect("creatable");
        let second = platform
            .create_surface(&SurfaceAttributes::new("b"))
            .expect("creatable");
        assert_ne!(first.id(), second.id());
        assert_eq!(platform.surfaces().len(), 2);
    }

    #[test]
    fn a_session_lock_is_asked_for_confirmed_and_unlocked() {
        use super::SessionLockState;
        use zgui_platform::{PlatformCapabilities, WakeReason};

        let platform = Headless::new();
        assert!(platform.lock_session().is_err(), "no capability, no lock");

        let mut capabilities = PlatformCapabilities::none();
        capabilities.session_lock = true;
        let platform = Headless::new().with_capabilities(capabilities);
        platform.lock_session().expect("the lock is asked for");
        assert_eq!(platform.session_lock(), SessionLockState::Requested);
        assert!(platform.lock_session().is_err(), "one lock at a time");

        platform.unlock_session();
        assert_eq!(
            platform.session_lock(),
            SessionLockState::Requested,
            "an unconfirmed lock cannot be unlocked"
        );
        platform.confirm_lock();
        assert!(matches!(
            platform.drain_wakes()[..],
            [WakeReason::SessionLocked]
        ));
        platform.unlock_session();
        assert_eq!(platform.session_lock(), SessionLockState::Unlocked);

        platform.lock_session().expect("a new lock after an unlock");
        platform.finish_lock();
        assert_eq!(platform.session_lock(), SessionLockState::Unlocked);
        assert!(matches!(
            platform.drain_wakes()[..],
            [WakeReason::SessionLockFinished]
        ));
    }
}
