//! One compositor lock, with a native black surface per output and disposable UI claims.

use std::cell::RefCell;
use std::sync::Arc;

use smithay_client_toolkit::globals::GlobalData;
use smithay_client_toolkit::registry::RegistryHandler;
use smithay_client_toolkit::shm::raw::RawPool;
use smithay_client_toolkit::subcompositor::SubcompositorState;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_output::WlOutput, wl_shm, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use zgui_geom::{Css, CssPx, Size};
use zgui_platform::{
    PlatformError, SessionLockState, Surface, SurfaceAttributes, SurfaceEvent, SurfaceId,
    Unsupported, WakeReason, Waker,
};

use crate::driver::WaylandState;
use crate::surface::role::{Role, lock::LockContent};
use crate::surface::{Fractional, WaylandSurface};

#[derive(Debug, Default)]
enum Phase {
    #[default]
    Unlocked,
    Pending(ExtSessionLockV1),
    Locked(ExtSessionLockV1),
    Refused,
    Finished,
    Unlocking,
}

impl Phase {
    fn active(&self) -> Option<&ExtSessionLockV1> {
        match self {
            Self::Pending(lock) | Self::Locked(lock) => Some(lock),
            _ => None,
        }
    }

    fn status(&self) -> SessionLockState {
        match self {
            Self::Unlocked => SessionLockState::Unlocked,
            Self::Pending(_) => SessionLockState::Pending,
            Self::Locked(_) => SessionLockState::Locked,
            Self::Refused => SessionLockState::Refused,
            Self::Finished => SessionLockState::Finished,
            Self::Unlocking => SessionLockState::Unlocking,
        }
    }
}

#[derive(Debug)]
pub(super) struct SessionLock {
    protocols: Option<LockProtocols>,
    phase: Phase,
    outputs: Vec<LockOutput>,
    black: Option<RawPool>,
    focus_changed: bool,
}

#[derive(Debug)]
struct LockProtocols {
    manager: ExtSessionLockManagerV1,
    subcompositor: SubcompositorState,
}

enum LockRetireMode {
    KeepFallback,
    DestroyFallback,
}

#[derive(Debug)]
struct LockOutput {
    output: WlOutput,
    role: ExtSessionLockSurfaceV1,
    wl: WlSurface,
    viewport: Option<WpViewport>,
    extent: Option<[i32; 2]>,
    buffer: Option<wl_buffer::WlBuffer>,
    claim: Option<SurfaceId>,
    replacement: Option<SurfaceId>,
}

impl LockOutput {
    fn cover(
        &mut self,
        serial: u32,
        [width, height]: [u32; 2],
        pool: &mut Option<RawPool>,
        state: &WaylandState,
    ) -> Result<[i32; 2], PlatformError> {
        let axis = |value| {
            i32::try_from(value)
                .ok()
                .filter(|&v| v > 0)
                .ok_or_else(|| lock_error("invalid lock surface extent"))
        };
        let extent @ [width, height] = [axis(width)?, axis(height)?];
        let view = self.viewport.as_ref();
        if self.buffer.is_none() || (view.is_none() && self.extent != Some(extent)) {
            let [buffer_width, buffer_height] = if view.is_some() { [1, 1] } else { extent };
            let Some((stride, length)) = buffer_width.checked_mul(4).and_then(|stride| {
                Some((
                    stride,
                    usize::try_from(stride.checked_mul(buffer_height)?).ok()?,
                ))
            }) else {
                return Err(lock_error(
                    "lock surface extent does not fit a shared-memory buffer",
                ));
            };
            // The file starts zero-filled. All fallbacks alias its immutable black pixels; no
            // application or renderer can write them, so no buffer release needs to be awaited.
            if let Some(black) = pool
                && let Err(error) = black.resize(length)
            {
                // RawPool may have updated its cached length before resizing failed. Existing
                // buffers retain the old pool on the server; never allocate from it again.
                *pool = None;
                return Err(lock_error(format!("resize lock fallback: {error}")));
            }
            let black = match pool {
                Some(black) => black,
                None => pool.insert(
                    RawPool::new(length, &state.shm)
                        .map_err(|error| lock_error(format!("allocate lock fallback: {error}")))?,
                ),
            };
            let buffer = black.create_buffer(
                0,
                buffer_width,
                buffer_height,
                stride,
                wl_shm::Format::Xrgb8888,
                BlackBuffer,
                &state.qh,
            );
            if !buffer.is_alive() {
                return Err(lock_error("the lock fallback buffer is no longer available"));
            }
            if let Some(old) = self.buffer.replace(buffer) {
                old.destroy();
            }
        }
        let Some(buffer) = &self.buffer else {
            return Err(lock_error("missing lock fallback buffer"));
        };
        // Do not acknowledge an extent unless a matching opaque buffer is ready to commit.
        self.role.ack_configure(serial);
        if let Some(view) = view {
            view.set_destination(width, height);
        }
        self.wl.attach(Some(buffer), 0, 0);
        self.wl.damage(0, 0, width, height);
        self.wl.commit();
        self.extent = Some(extent);
        Ok(extent)
    }

    // Only used on refusal, output removal, or after the unlock sync. On connection failure
    // or normal process exit the compositor cleans up the remaining native roles.
    fn destroy(self) {
        if let Some(viewport) = &self.viewport {
            viewport.destroy();
        }
        self.role.destroy();
        self.wl.destroy();
        if let Some(buffer) = &self.buffer {
            buffer.destroy();
        }
    }
}

impl SessionLock {
    pub(super) fn bind(
        globals: &GlobalList,
        qh: &QueueHandle<WaylandState>,
        state: &smithay_client_toolkit::compositor::CompositorState,
    ) -> RefCell<Self> {
        let protocols = (|| {
            let manager = globals.bind(qh, 1..=1, GlobalData).ok()?;
            // An advertised lock must be usable for authentication, not just for blanking.
            let subcompositor =
                SubcompositorState::bind(state.wl_compositor().clone(), globals, qh).ok()?;
            Some(LockProtocols { manager, subcompositor })
        })();
        RefCell::new(Self {
            protocols,
            phase: Phase::Unlocked,
            outputs: Vec::new(),
            black: None,
            focus_changed: false,
        })
    }

    pub(super) fn offered(&self) -> bool {
        self.protocols.is_some()
    }
}

impl WaylandState {
    pub(crate) fn request_session_lock(&self) -> Result<(), PlatformError> {
        {
            let mut session = self.session_lock.borrow_mut();
            match session.phase {
                Phase::Pending(_) | Phase::Locked(_) => return Ok(()),
                Phase::Finished => {
                    return Err(lock_error(
                        "cannot replace a finished lock on this connection",
                    ));
                }
                Phase::Unlocking => {
                    return Err(lock_error(
                        "session unlock is still awaiting acknowledgement",
                    ));
                }
                Phase::Unlocked | Phase::Refused => {}
            }
            let protocols = session.protocols.as_ref().ok_or(Unsupported)?;
            session.phase = Phase::Pending(protocols.manager.lock(&self.qh, ()));
        }
        self.cover_lock_outputs();
        self.lock_changed();
        Ok(())
    }

    pub(crate) fn session_lock_state(&self) -> SessionLockState {
        self.session_lock.borrow().phase.status()
    }

    pub(crate) fn unlock_session(&self) -> Result<(), PlatformError> {
        let mut session = self.session_lock.borrow_mut();
        let Phase::Locked(lock) = &session.phase else {
            return Err(lock_error("only an acquired session lock can be unlocked"));
        };
        lock.unlock_and_destroy();
        self.conn.display().sync(&self.qh, UnlockSync);
        session.phase = Phase::Unlocking;
        self.lock_changed();
        Ok(())
    }

    fn lock_changed(&self) {
        self.live.waker.wake(WakeReason::AppWork);
    }

    fn cover_lock_outputs(&self) {
        let mut session = self.session_lock.borrow_mut();
        let SessionLock { phase, outputs, .. } = &mut *session;
        let Some(lock) = phase.active() else {
            return;
        };
        for output in self.outputs.outputs() {
            if outputs.iter().any(|held| held.output == output) {
                continue;
            }
            let wl = self.compositor.create_surface(&self.qh);
            let role = lock.get_lock_surface(&wl, &output, &self.qh, ());
            let viewport = self
                .extras
                .viewporter
                .as_ref()
                .map(|manager| manager.get_viewport(&wl, &self.qh, ()));
            // No initial commit: unlike the xdg and layer shells, the role sends configure itself.
            outputs.push(LockOutput {
                output,
                role,
                wl,
                viewport,
                extent: None,
                buffer: None,
                claim: None,
                replacement: None,
            });
        }
    }

    pub(super) fn claim_lock_output(
        &self,
        name: &str,
        attributes: &SurfaceAttributes,
    ) -> Result<Arc<WaylandSurface>, PlatformError> {
        let mut session = self.session_lock.borrow_mut();
        if session.phase.active().is_none() {
            return Err(lock_error("no pending or acquired session lock"));
        }
        let SessionLock {
            protocols,
            outputs,
            ..
        } = &mut *session;
        let subcompositor = &protocols.as_ref().ok_or(Unsupported)?.subcompositor;
        let (output, info) = outputs
            .iter_mut()
            .find_map(|output| {
                let info = self.outputs.info(&output.output)?;
                (info.name.as_deref() == Some(name)).then_some((output, info))
            })
            .ok_or_else(|| {
                PlatformError::SurfaceCreation(format!("lock output {name:?} is not available"))
            })?;
        let replacing = match (output.claim, attributes.replacing_lock, output.replacement) {
            (None, None, None) => false,
            (Some(claim), Some(previous), None) if claim == previous => true,
            _ => {
                return Err(PlatformError::SurfaceCreation(format!(
                    "lock output {name:?} does not accept this application claim"
                )));
            }
        };
        let id = self.live.next_id();
        let (child, wl) = subcompositor.create_subsurface(output.wl.clone(), &self.qh);
        let fractional = Fractional::attach(&self.extras, &self.qh, &wl, id);
        let role = Role::LockContent(LockContent::new(wl, child));
        let initial = output
            .extent
            .map_or(Size::new(CssPx(0.0), CssPx(0.0)), logical_extent);
        let surface =
            WaylandSurface::new(id, role, fractional, initial, None, initial, self.links());
        {
            let mut shared = surface.shared();
            shared.ahead =
                crate::frame::Ahead::new(crate::frame::ahead::mode(attributes.compose_ahead));
            shared.ladder.outputs(Some(info.scale_factor));
            shared.outputs.push(output.output.clone());
            if output.extent.is_some() {
                let scale = shared.ladder.factor();
                shared.resized(initial, scale);
                shared.visibility.configured = true;
                // Apply the child addition. The parent is already configured with an opaque buffer.
                output.wl.commit();
            }
        }
        if replacing {
            // Keep input routed to the predecessor until the host releases it.
            output.replacement = Some(id);
        } else {
            output.claim = Some(id);
            session.focus_changed = true;
        }
        self.live.surfaces.borrow_mut().push(Arc::clone(&surface));
        self.declare_rate(id);
        surface.request_redraw();
        Ok(surface)
    }

    pub(super) fn release_lock_claim(&self, id: SurfaceId) -> bool {
        let mut session = self.session_lock.borrow_mut();
        let Some(output) = session
            .outputs
            .iter_mut()
            .find(|out| out.claim == Some(id) || out.replacement == Some(id))
        else {
            return false;
        };
        if output.claim == Some(id) {
            output.claim = output.replacement.take();
            session.focus_changed = true;
        } else {
            output.replacement = None;
        }
        self.detach_lock_claim(id);
        true
    }

    fn detach_lock_claim(&self, id: SurfaceId) {
        if let Some(surface) = self.live.surface(id) {
            if let Role::LockContent(content) = surface.role() {
                content.detach();
            }
            self.live.close(id);
        }
    }

    pub(super) fn lock_output_removed(&mut self, removed: &WlOutput) {
        let claims = {
            let mut session = self.session_lock.borrow_mut();
            // Keep even a departing role alive until the requested unlock is acknowledged.
            if matches!(session.phase, Phase::Unlocking) {
                return;
            }
            let Some(index) = session
                .outputs
                .iter()
                .position(|out| &out.output == removed)
            else {
                return;
            };
            let output = session.outputs.swap_remove(index);
            let claims = [output.claim, output.replacement];
            for id in claims.into_iter().flatten() {
                self.detach_lock_claim(id);
                session.focus_changed = true;
            }
            output.destroy();
            claims
        };
        for id in claims.into_iter().flatten() {
            self.report(id, SurfaceEvent::Destroyed);
        }
    }

    pub(super) fn lock_output_scale(&mut self, output: &WlOutput) {
        let claims = self
            .session_lock
            .borrow()
            .outputs
            .iter()
            .find(|out| out.output == *output)
            .map(|out| [out.claim, out.replacement]);
        if let Some(claims) = claims {
            let scale = self.outputs.info(output).map(|info| info.scale_factor);
            for id in claims.into_iter().flatten() {
                self.scale_changed(id, |ladder| ladder.outputs(scale));
            }
        }
    }

    pub(super) fn take_lock_focus_changed(&self) -> bool {
        std::mem::take(&mut self.session_lock.borrow_mut().focus_changed)
    }

    pub(super) fn lock_keyboard_target(&self, parent: &WlSurface) -> Option<Option<SurfaceId>> {
        self.session_lock
            .borrow()
            .outputs
            .iter()
            .find(|out| out.wl == *parent)
            .map(|out| out.claim)
    }

    fn configure_lock_output(
        &mut self,
        role: &ExtSessionLockSurfaceV1,
        serial: u32,
        extent: [u32; 2],
    ) {
        let (surfaces, extent) = {
            let mut session = self.session_lock.borrow_mut();
            if session.phase.active().is_none() {
                return;
            }
            let SessionLock { outputs, black, .. } = &mut *session;
            let Some(output) = outputs.iter_mut().find(|out| out.role == *role) else {
                return;
            };
            let previous = output.extent;
            let surfaces =
                [output.claim, output.replacement].map(|id| id.and_then(|id| self.live.surface(id)));
            if previous.is_some_and(|previous| previous.map(u32::try_from) != extent.map(Ok)) {
                // Subsurfaces are not clipped to their parent. Remove old application buffers
                // before changing the parent extent; the new UI frame will cover it again.
                for surface in surfaces.iter().flatten() {
                    if let Role::LockContent(content) = surface.role() {
                        let mut shared = surface.shared();
                        content.hide();
                        shared.cancel_callbacks();
                        shared.pacer.callback();
                        shared.pacer.cancel_held();
                        shared.visibility.answered();
                    }
                }
            }
            let extent = match output.cover(serial, extent, black, self) {
                Ok(extent) => extent,
                Err(error) => {
                    tracing::error!(%error, "unable to map lock fallback; keeping the session locked");
                    return;
                }
            };
            (surfaces, logical_extent(extent))
        };
        for surface in surfaces.into_iter().flatten() {
            let (visibility, resize) = {
                let mut shared = surface.shared();
                let scale = shared.ladder.factor();
                let resize = shared.resized(extent, scale);
                shared.visibility.configured = !shared.unmapped;
                (shared.visibility_edge(), resize)
            };
            if let Some(event) = visibility {
                self.report(surface.id(), event);
            }
            if let Some(event) = resize {
                self.report(surface.id(), event);
            }
            surface.request_redraw();
        }
    }

    // The backend owns these resources; a JavaScript fault can only shed its own claims.
    fn retire_lock_outputs(&mut self, mode: LockRetireMode) {
        let mut session = self.session_lock.borrow_mut();
        for output in &mut session.outputs {
            for id in [output.claim.take(), output.replacement.take()]
                .into_iter()
                .flatten()
            {
                self.detach_lock_claim(id);
                self.out.push((id, SurfaceEvent::Destroyed));
            }
        }
        session.focus_changed = true;
        if matches!(mode, LockRetireMode::DestroyFallback) {
            for output in session.outputs.drain(..) {
                output.destroy();
            }
            session.black = None;
        }
    }
}

fn lock_error(message: impl Into<String>) -> PlatformError {
    PlatformError::Backend(message.into())
}

fn logical_extent([width, height]: [i32; 2]) -> Size<CssPx, Css> {
    Size::new(CssPx(width as f32), CssPx(height as f32))
}

// Runs after OutputState has bound the new wl_output, without waiting for its name or done event.
impl RegistryHandler<WaylandState> for SessionLock {
    fn new_global(
        state: &mut WaylandState,
        _: &Connection,
        _: &QueueHandle<WaylandState>,
        _: u32,
        interface: &str,
        _: u32,
    ) {
        if interface == "wl_output" {
            state.cover_lock_outputs();
        }
    }
}

impl Dispatch<ExtSessionLockManagerV1, GlobalData> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ExtSessionLockManagerV1,
        _: <ExtSessionLockManagerV1 as Proxy>::Event,
        _: &GlobalData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtSessionLockV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        proxy: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let retire = {
            let mut session = state.session_lock.borrow_mut();
            match (&session.phase, event) {
                (Phase::Pending(lock), ext_session_lock_v1::Event::Locked) if lock == proxy => {
                    session.phase = Phase::Locked(proxy.clone());
                    None
                }
                (Phase::Pending(lock), ext_session_lock_v1::Event::Finished) if lock == proxy => {
                    proxy.destroy();
                    session.phase = Phase::Refused;
                    Some(LockRetireMode::DestroyFallback)
                }
                (Phase::Locked(lock), ext_session_lock_v1::Event::Finished) if lock == proxy => {
                    // The only legal destructor would unlock. Leave the object inert until
                    // disconnect instead: a compositor event is not authorization from the host.
                    session.phase = Phase::Finished;
                    Some(LockRetireMode::KeepFallback)
                }
                _ => return,
            }
        };
        if let Some(mode) = retire {
            state.retire_lock_outputs(mode);
        }
        state.lock_changed();
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        role: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            state.configure_lock_output(role, serial, [width, height]);
        }
    }
}

#[derive(Debug)]
struct UnlockSync;

impl Dispatch<wl_callback::WlCallback, UnlockSync> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &UnlockSync,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. })
            && state.session_lock_state() == SessionLockState::Unlocking
        {
            state.retire_lock_outputs(LockRetireMode::DestroyFallback);
            state.session_lock.borrow_mut().phase = Phase::Unlocked;
            state.lock_changed();
        }
    }
}

#[derive(Debug)]
struct BlackBuffer;

impl Dispatch<wl_buffer::WlBuffer, BlackBuffer> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &BlackBuffer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The shared black pixels are never modified, even after the compositor releases a buffer.
    }
}

smithay_client_toolkit::delegate_subcompositor!(WaylandState);
