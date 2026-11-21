//! The part of the loop's state the borrowed context is allowed to change.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use wayland_client::protocol::wl_seat::WlSeat;

use zgui_platform::{MonitorInfo, PlatformCapabilities, Surface as _, SurfaceId};

use crate::clock::SystemClock;
use crate::surface::WaylandSurface;
use crate::waker::PingWaker;

/// What a callback may change while the loop is inside it.
///
/// The loop's protocol state is held by value and mutated only while the toolkit is dispatching
/// into it. Everything here is different: it is reached through the borrowed context the
/// application is handed, which the contract gives out as `&self`, so each field carries the cell
/// that makes it writable there.
///
/// Keeping the two apart is what lets a protocol event and an application callback never overlap.
/// A dispatch writes protocol state; a delivery writes this; and neither runs inside the other,
/// because events are collected during a dispatch and delivered after it has finished.
pub struct Live {
    /// Every surface that exists.
    pub(crate) surfaces: RefCell<Vec<Arc<WaylandSurface>>>,
    /// Surfaces the application has closed, waiting for the end of the turn.
    ///
    /// A surface is not destroyed inside the callback that closed it: the objects underneath are
    /// the ones the loop is currently dispatching for, and taking them down there is a use after
    /// free. The holders are retired between turns; only popup shell roles may be released
    /// earlier, when otherwise a newly opened popup would stack above one pending destruction.
    pub(crate) retiring: RefCell<Vec<Arc<WaylandSurface>>>,
    /// Sibling branches forced closed by the shell's topmost rule.
    sibling_closes: RefCell<Vec<SurfaceId>>,
    /// Whether the loop has been asked to finish.
    pub(crate) exiting: Cell<bool>,
    /// The number the next surface gets.
    next: Cell<u64>,
    /// What is known about the outputs.
    pub(crate) monitors: RefCell<Vec<MonitorInfo>>,
    /// The identifier the desktop groups this application's windows under.
    ///
    /// Taken from whichever window states one first, because a request to activate names the
    /// application rather than the window: several compositors flash the task-bar entry it
    /// identifies even when they refuse the activation itself.
    pub(crate) app_id: RefCell<Option<String>>,
    /// The desktop's appearance preferences (color scheme and reduced motion), where discoverable.
    ///
    /// Behind a lock rather than a cell, because the portal answers on a thread of its own: the
    /// bus is a socket like any other, and the loop's thread waits on nothing.
    pub(crate) appearance: Arc<std::sync::Mutex<crate::theme::Appearance>>,
    /// What this compositor turned out to be able to do.
    pub(crate) capabilities: PlatformCapabilities,
    /// Where the time comes from.
    pub(crate) clock: Arc<SystemClock>,
    /// How another thread reaches this loop.
    pub(crate) waker: Arc<PingWaker>,
}

impl Live {
    /// The state a loop starts with: no surfaces, nothing closed, nothing known.
    pub(crate) fn new(capabilities: PlatformCapabilities, waker: Arc<PingWaker>) -> Self {
        Self {
            surfaces: RefCell::new(Vec::new()),
            retiring: RefCell::new(Vec::new()),
            sibling_closes: RefCell::new(Vec::new()),
            exiting: Cell::new(false),
            next: Cell::new(1),
            monitors: RefCell::new(Vec::new()),
            app_id: RefCell::new(None),
            appearance: Arc::default(),
            capabilities,
            clock: Arc::new(SystemClock::new()),
            waker,
        }
    }

    /// The number for one more surface.
    ///
    /// Never reused, so that a stale identifier held by something that outlived its surface names
    /// nothing rather than naming whichever window opened next.
    pub(crate) fn next_id(&self) -> SurfaceId {
        let id = self.next.get();
        self.next.set(id + 1);
        SurfaceId::new(id)
    }

    /// The surface with this number, while it still exists.
    pub(crate) fn surface(&self, id: SurfaceId) -> Option<Arc<WaylandSurface>> {
        self.surfaces
            .borrow()
            .iter()
            .find(|surface| surface.id() == id)
            .map(Arc::clone)
    }

    /// Every surface that exists, as a snapshot.
    ///
    /// A copy rather than a borrow, because the caller is the application and what it does with
    /// the answer includes closing one of them.
    pub(crate) fn all(&self) -> Vec<Arc<WaylandSurface>> {
        self.surfaces.borrow().clone()
    }

    /// Whether a new grab can extend the active popup stack.
    #[must_use]
    pub(crate) fn can_grab_popup(&self, parent: &WaylandSurface, seat: &WlSeat) -> bool {
        let surfaces = self.surfaces.borrow();
        let root = parent.popup_root().unwrap_or(parent.id());
        let top = surfaces.iter().rev().find(|surface| surface.popup_root() == Some(root));
        let grabbing = surfaces.iter().rev().find(|surface| surface.popup_grabs(seat));
        match (parent.popup_root(), top, grabbing) {
            (None, None, None) => true,
            (Some(_), Some(top), Some(grabbing)) => {
                top.id() == parent.id() && grabbing.id() == parent.id()
            }
            _ => false,
        }
    }

    /// Close this surface and anything above it in its popup stack, newest first.
    ///
    /// The shell stacks even sibling popups with a common root. Closing just descendants
    /// could leave a sibling above the popup whose role we are about to destroy.
    pub(crate) fn close(&self, id: SurfaceId) {
        let mut surfaces = self.surfaces.borrow_mut();
        let Some(index) = surfaces.iter().position(|surface| surface.id() == id) else {
            return;
        };
        let root = surfaces[index].popup_root().unwrap_or(id);
        let mut retiring = self.retiring.borrow_mut();
        let start = retiring.len();
        retiring.extend(surfaces.extract_if(index.., |surface| {
            let close = surface.id() == id || surface.popup_root() == Some(root);
            if close {
                surface.close();
            }
            close
        }));
        retiring[start..].reverse();
        // A removed popup whose parent is still live starts a sibling branch.
        self.sibling_closes.borrow_mut().extend(
            retiring[start..]
                .iter()
                .filter(|surface| surface.id() != id && surface.popup_parent_live())
                .map(|surface| surface.id()),
        );
    }

    pub(crate) fn take_sibling_closes(&self) -> Vec<SurfaceId> {
        self.sibling_closes.take()
    }

    /// Release pending popup roles before a new popup can stack above them.
    pub(crate) fn release_retiring_popups(&self) {
        for surface in self.retiring.borrow().iter() {
            surface.retire();
        }
    }

    pub(crate) fn retire(&self) {
        for surface in self.retiring.take() {
            surface.retire();
        }
    }

    pub(crate) fn close_all(&self) {
        let mut surfaces = self.surfaces.borrow_mut();
        self.retiring.borrow_mut().extend(surfaces.drain(..).rev().inspect(|surface| {
            surface.close();
        }));
    }
}

impl core::fmt::Debug for Live {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Live")
            .field("surfaces", &self.surfaces.borrow().len())
            .field("retiring", &self.retiring.borrow().len())
            .field("exiting", &self.exiting.get())
            .finish_non_exhaustive()
    }
}
