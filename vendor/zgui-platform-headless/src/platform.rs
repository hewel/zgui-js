//! The platform itself: a clock, a clipboard, a waker and its offscreen surfaces.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use zgui_geom::{DevicePx, Size};
use zgui_platform::{
    Clipboard, Clock, ColorScheme, MonitorInfo, PlatformCapabilities, PlatformCx, PlatformError,
    ScrollSettings, SessionLockState, Surface, SurfaceAttributes, SurfaceId, SurfaceRole,
    Unsupported, VirtualClock, WakeReason, Waker,
};

use crate::clipboard::MemoryClipboard;
use crate::surface::OffscreenSurface;
use crate::waker::RecordingWaker;

/// The extent of a surface created without one, in physical pixels.
const DEFAULT_SIZE: Size<DevicePx, zgui_geom::Device> = Size::new(DevicePx(640.0), DevicePx(480.0));

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
    pub state: SessionLockState,
    /// The count of explicitly acknowledged unlocks.
    pub unlock_count: u64,
    /// Named outputs with a platform-owned role, sorted by name.
    pub covered_outputs: Vec<String>,
    /// Covered outputs currently showing opaque fallback pixels, sorted by name.
    pub fallback_outputs: Vec<String>,
}

#[derive(Default)]
struct SessionLock {
    state: SessionLockState,
    next_decision: SessionLockDecision,
    unlock_count: u64,
    outputs: BTreeMap<String, LockOutput>,
}

struct LockOutput {
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

impl SessionLock {
    fn sync_outputs(&mut self, monitors: &[MonitorInfo], next_id: impl Fn() -> SurfaceId) {
        self.outputs.retain(|name, output| {
            let present = monitors
                .iter()
                .any(|monitor| monitor.name.as_deref() == Some(name.as_str()));
            if !present {
                output.remove();
            }
            present
        });
        for monitor in monitors {
            let Some(name) = monitor.name.as_deref() else {
                continue;
            };
            if let Some(output) = self.outputs.get(name) {
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
                attributes.role = SurfaceRole::Lock(name.to_owned());
                surface.set_requested_attributes(&attributes);
                surface.set_scale_factor(monitor.scale_factor);
                surface.set_refresh_rate_millihertz(monitor.refresh_rate_millihertz);
                self.outputs.insert(name.to_owned(), LockOutput {
                    current: Arc::new(surface),
                    replacement: None,
                });
            }
        }
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
        Self {
            clock: Arc::new(VirtualClock::new()),
            clipboard,
            capabilities: PlatformCapabilities::none(),
            waker,
            desktop: Mutex::new(Desktop::default()),
            next_surface: AtomicU64::new(1),
            color_scheme: Mutex::new(None),
            scroll: Mutex::new(ScrollSettings::desktop()),
            exiting: AtomicBool::new(false),
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
        self.set_monitors(monitors);
        self
    }

    /// Replaces the reported monitor inventory for the next application callback.
    ///
    /// The fixture driving the handler delivers the corresponding `monitors_changed` callback.
    pub fn set_monitors(&self, monitors: Vec<MonitorInfo>) {
        let mut desktop = self.desktop();
        if desktop.session_lock.state == SessionLockState::Locked {
            desktop.session_lock.sync_outputs(&monitors, || self.next_surface_id());
        }
        desktop.monitors = monitors;
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

    /// The same platform declaring what a scroll from its devices means.
    ///
    /// The headless backend has no devices and no desktop, so every answer here is a *stated* one —
    /// which is exactly what makes the scrolling contract testable. A wheel that arrives inverted,
    /// a desktop that means five lines by one detent and a platform that animates its own detents
    /// are three different behaviours above this seam, and without a backend that can say each of
    /// them in turn none of the three is reachable from a test at all.
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

    /// The offscreen surface with this identifier, as itself.
    ///
    /// A test asserts against what a surface recorded, and the contract deliberately offers no way
    /// to ask.
    pub fn offscreen(&self, id: SurfaceId) -> Option<Arc<OffscreenSurface>> {
        let desktop = self.desktop();
        desktop
            .surfaces
            .iter()
            .chain(desktop.session_lock.outputs.values().flat_map(LockOutput::surfaces))
            .find(|surface| surface.id() == id)
            .map(Arc::clone)
    }

    /// Every offscreen surface that exists, as itself.
    pub fn offscreens(&self) -> Vec<Arc<OffscreenSurface>> {
        let desktop = self.desktop();
        desktop
            .surfaces
            .iter()
            .chain(desktop.session_lock.outputs.values().flat_map(LockOutput::surfaces))
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
        if let SurfaceRole::Lock(name) = &attributes.role {
            if !self.capabilities.session_lock {
                return Err(Unsupported.into());
            }
            let mut desktop = self.desktop();
            let output = desktop.session_lock.outputs.get_mut(name).ok_or_else(|| {
                PlatformError::SurfaceCreation(format!("no session-lock surface on {name:?}"))
            })?;
            let surface = if let Some(previous) = attributes.replacing_lock {
                if output.current.id() != previous || !output.current.lock_is_claimed() || output.replacement.is_some() {
                    return Err(PlatformError::SurfaceCreation(format!("no replaceable lock claim on {name:?}")));
                }
                let surface = Arc::new(OffscreenSurface::new_lock(self.next_surface_id(), output.current.size()));
                surface.set_scale_factor(output.current.scale_factor());
                surface.set_refresh_rate_millihertz(output.current.refresh_rate_millihertz());
                output.replacement.insert(surface)
            } else {
                &output.current
            };
            if !surface.claim_lock() {
                return Err(PlatformError::SurfaceCreation(format!(
                    "the session-lock surface on {name:?} is already claimed"
                )));
            }
            surface.set_title(attributes.title.as_str());
            surface.set_requested_attributes(attributes);
            return Ok(Arc::clone(surface) as Arc<dyn Surface>);
        }
        let size = attributes.size.map_or(DEFAULT_SIZE, |size| {
            Size::new(DevicePx(size.width.0), DevicePx(size.height.0))
        });
        let surface = Arc::new(OffscreenSurface::new(self.next_surface_id(), size));
        surface.set_title(attributes.title.as_str());
        surface.set_requested_attributes(attributes);
        self.desktop().surfaces.push(Arc::clone(&surface));
        Ok(surface as Arc<dyn Surface>)
    }

    fn destroy_surface(&self, id: SurfaceId) {
        let mut desktop = self.desktop();
        if !desktop.session_lock.outputs.values_mut().any(|output| output.release(id)) {
            desktop.surfaces.retain(|surface| surface.id() != id);
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
            SessionLockState::Locked | SessionLockState::Pending => return Ok(()),
            SessionLockState::Refused | SessionLockState::Unlocked => {}
            _ => {
                return Err(PlatformError::Backend(
                    "cannot replace the active session lock".to_owned(),
                ));
            }
        }
        let next = match std::mem::take(&mut session_lock.next_decision) {
            SessionLockDecision::Lock => {
                session_lock.sync_outputs(monitors, || self.next_surface_id());
                SessionLockState::Locked
            }
            SessionLockDecision::Refuse => SessionLockState::Refused,
        };
        if next != session_lock.state {
            session_lock.state = next;
            self.waker.wake(WakeReason::AppWork);
        }
        Ok(())
    }

    fn session_lock_state(&self) -> SessionLockState {
        self.desktop().session_lock.state
    }

    fn unlock_session(&self) -> Result<(), PlatformError> {
        if !self.capabilities.session_lock {
            return Err(Unsupported.into());
        }
        let mut desktop = self.desktop();
        let lock = &mut desktop.session_lock;
        if lock.state != SessionLockState::Locked {
            return Err(PlatformError::Backend("no acquired session lock to unlock".to_owned()));
        }
        // This compositor acknowledges synchronously; only now can the output roles go away.
        lock.state = SessionLockState::Unlocked;
        lock.unlock_count += 1;
        for (_, output) in std::mem::take(&mut lock.outputs) {
            output.remove();
        }
        self.waker.wake(WakeReason::AppWork);
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
}
