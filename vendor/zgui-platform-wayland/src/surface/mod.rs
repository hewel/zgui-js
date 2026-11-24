//! One surface: what is drawn into it, what it looks like, and what it owes the compositor.

pub mod a11y;
pub mod chrome;
pub mod fractional;
mod handles;
pub mod region;
pub mod role;
pub mod scale;
pub mod seat;
pub(crate) mod sequence;
pub mod state;

pub use crate::surface::a11y::A11y;
pub use crate::surface::fractional::Fractional;
pub use crate::surface::role::Role;
pub use crate::surface::scale::Scale;
pub use crate::surface::seat::SeatLink;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Instant;

use accesskit::TreeUpdate;
use smithay_client_toolkit::reexports::client::{Connection, Proxy as _, QueueHandle};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;
use zgui_geom::{Css, CssPx, Device, DevicePx, Edges, Point, Rect, Size};
use zgui_platform::{
    BlurRegion, CursorStyle, Decorations, FullscreenMode, GpuSurface, InputRegion, LayerPlacement,
    PlatformError, PopupPlacement, PresentPacing, PresentationTiming, Surface, SurfaceId,
    TextInput, Unsupported,
};

use crate::driver::WaylandState;
use crate::driver::background_effect::BackgroundEffectLink;
use crate::frame::Presentation;
use crate::surface::role::popup::Offset as PopupOffset;
use crate::surface::state::{Bounds, Shared};

/// What a surface needs from the loop it belongs to.
///
/// Every one of these is a handle the loop already holds and hands over by cloning: the connection
/// a graphics API is given, the queue everything this surface creates is dispatched on, the
/// compositor its regions come from, the global its presentation is reported by, and the pipe that
/// ends the loop's park. They travel together because they are handed over together, once.
#[derive(Clone)]
pub struct Links {
    /// The connection everything is created on.
    pub conn: Connection,
    /// The queue everything is dispatched on.
    pub qh: QueueHandle<WaylandState>,
    /// The compositor, for the regions a surface builds.
    pub compositor: WlCompositor,
    /// Where a request for presentation feedback goes.
    pub presentation: Presentation,
    /// How a request made away from the loop's thread reaches it.
    pub ping: calloop::ping::Ping,
    /// How a surface reaches the seat.
    pub seat: Arc<SeatLink>,
    /// Where an assistive technology's questions are answered.
    pub waker: Arc<dyn zgui_platform::Waker>,
    /// The shell a pop-up's later requests are made against.
    pub shell: Arc<crate::surface::role::xdg::XdgShell>,
    /// Shared clipboard and outbound Copy-only drag source controller.
    pub selections: Arc<crate::clipboard::Selections>,
    /// Shared handle to the live `ext_background_effect_manager_v1` global and blur capability.
    pub background_effect: Arc<BackgroundEffectLink>,
}

impl core::fmt::Debug for Links {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("Links")
    }
}

/// A popup's fixed parent and the placement requests made against it.
pub(crate) struct PopupState {
    parent: Weak<WaylandSurface>,
    parent_id: SurfaceId,
    root: SurfaceId,
    grab_seat: Option<WlSeat>,
    desired: Mutex<PopupDesired>,
}

struct PopupDesired {
    placement: PopupPlacement,
    offset: PopupOffset,
    size: Size<CssPx, Css>,
    insets: Edges<i32>,
    last_sent_anchor_rect: (i32, i32, i32, i32),
    phase: PopupPhase,
    pending: bool,
    reposition_in_flight: bool,
    token: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PopupPhase {
    AwaitingConfigure,
    AwaitingBuffer,
    Mapped,
    Dismissed,
    Retired,
}

impl PopupState {
    pub(crate) fn new(
        parent: &Arc<WaylandSurface>,
        placement: PopupPlacement,
        offset: PopupOffset,
        size: Size<CssPx, Css>,
        insets: Edges<i32>,
        last_sent_anchor_rect: (i32, i32, i32, i32),
        grab_seat: Option<WlSeat>,
    ) -> Self {
        Self {
            parent: Arc::downgrade(parent),
            parent_id: parent.id,
            root: parent.popup_root().unwrap_or(parent.id),
            grab_seat,
            desired: Mutex::new(PopupDesired {
                placement,
                offset,
                size,
                insets,
                last_sent_anchor_rect,
                phase: PopupPhase::AwaitingConfigure,
                pending: false,
                reposition_in_flight: false,
                token: 0,
            }),
        }
    }
}

/// A Wayland surface, seen as something to draw into and interact with.
///
/// Shared and thread-safe, because [`Surface::request_redraw`] has to work from anywhere: work
/// finishing on a worker thread is one of the four things that can want a frame, and making it hop
/// to the loop's thread first would put a scheduling delay in front of every one of them.
///
/// That is affordable here because a protocol object is itself shareable — a request from any
/// thread is serialised into the connection's own buffer — so most of this type forwards straight
/// to the compositor with no hop at all. What cannot be done from another thread is *flushing*
/// that buffer, which only the loop does, so a request made from elsewhere is followed by a poke.
pub struct WaylandSurface {
    /// Which surface this is, in the contract's numbering.
    id: SurfaceId,
    /// The shell object it is.
    role: Role,
    /// Fractional scaling, when the compositor offers it.
    fractional: Fractional,
    /// Everything about it that changes.
    shared: Mutex<Shared>,
    /// A redraw asked for from anywhere, waiting to be pulled into the pacer.
    asked: AtomicBool,
    /// Whether this surface still belongs to the loop.
    active: AtomicBool,
    /// How a request made away from the loop's thread reaches it.
    ping: calloop::ping::Ping,
    /// The connection, for the display handle a graphics API needs.
    conn: Connection,
    /// The queue everything this surface creates is dispatched on.
    qh: QueueHandle<WaylandState>,
    /// The compositor, for the regions this surface builds.
    compositor: WlCompositor,
    /// Where a request for presentation feedback goes.
    presentation: Presentation,
    popup: Option<PopupState>,
    /// Live pop-up surfaces whose parent is this surface, so parent geometry changes can
    /// re-evaluate nested anchor translation.
    popup_children: Mutex<Vec<Weak<WaylandSurface>>>,
    /// The shell a reposition is made against.
    shell: Arc<crate::surface::role::xdg::XdgShell>,
    /// The channel an assistive technology talks to.
    a11y: A11y,
    /// The seat's own requests, which belong to the loop rather than to this surface.
    ///
    /// A cursor shape and an interactive drag are both asked of a *seat*, and there is one seat
    /// whichever surface asks. So the loop lends each surface a way to reach it, guarded by
    /// whether that surface is the one the pointer is actually on.
    seat: Arc<SeatLink>,
    /// Shared clipboard and outbound Copy-only drag source controller.
    selections: Arc<crate::clipboard::Selections>,
    /// Shared handle to the live `ext_background_effect_manager_v1` global and blur capability.
    background_effect: Arc<BackgroundEffectLink>,
}

impl WaylandSurface {
    /// A surface over `role`, numbered `id`, wired into the loop by `links`.
    pub(crate) fn new(
        id: SurfaceId,
        role: Role,
        fractional: Fractional,
        wanted: Size<CssPx, Css>,
        popup: Option<PopupState>,
        links: Links,
    ) -> Arc<Self> {
        let mut shared = Shared::new();
        shared.wanted = wanted;
        if let Some(state) = &popup {
            shared.window_geometry_insets = state
                .desired
                .lock()
                .expect("popup placement lock poisoned")
                .insets;
        }
        Arc::new(Self {
            id,
            role,
            fractional,
            shared: Mutex::new(shared),
            asked: AtomicBool::new(false),
            active: AtomicBool::new(true),
            ping: links.ping,
            conn: links.conn,
            qh: links.qh,
            compositor: links.compositor,
            presentation: links.presentation,
            popup,
            popup_children: Mutex::new(Vec::new()),
            shell: links.shell,
            a11y: A11y::open(id, links.waker),
            seat: links.seat,
            selections: links.selections,
            background_effect: links.background_effect,
        })
    }

    /// The shell object this surface is.
    pub const fn role(&self) -> &Role {
        &self.role
    }

    /// The surface underneath.
    pub fn wl_surface(&self) -> &WlSurface {
        self.role.wl_surface()
    }

    /// The connection this surface belongs to.
    pub const fn connection(&self) -> &Connection {
        &self.conn
    }

    /// The changing half, recovering from a panic on another thread.
    ///
    /// A poisoned lock means a thread panicked while reading a size. Everything under it is plain
    /// data that cannot be left half-written, and refusing to draw ever again turns one thread's
    /// panic into a window that has stopped.
    pub(crate) fn shared(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records `size` as the extent the application asked for.
    ///
    /// A shell layer's configure names zero along an axis it leaves to the client, and this is
    /// the extent the surface then takes.
    pub(crate) fn set_wanted(&self, size: Size<CssPx, Css>) {
        self.shared().wanted = size;
    }

    /// Changes the placement of this shell layer, to be sent with the next frame.
    pub(crate) fn patch_layer(&self, patch: &zgui_platform::LayerPatch) -> Result<(), Unsupported> {
        if self.role.layer().is_none() {
            return Err(Unsupported);
        }
        {
            let mut shared = self.shared();
            let Some(placement) = shared.layer.as_mut() else {
                return Err(Unsupported);
            };
            patch.apply_to(placement);
            shared.layer_owed = true;
        }
        Surface::request_redraw(self);
        Ok(())
    }

    /// Sends what this shell layer owes the compositor, right before a commit.
    ///
    /// With `all`, everything is sent whether it changed or not, which is what a remap needs: the
    /// shell forgets the placement and the extent of an unmapped surface. Otherwise only a change
    /// made since the last send goes out. Nothing is committed here.
    pub(crate) fn send_layer(&self, all: bool) {
        let Some(layer) = self.role.layer() else {
            return;
        };
        let (placement, extent) = {
            let mut shared = self.shared();
            if !all && !shared.layer_owed {
                return;
            }
            shared.layer_owed = false;
            let Some(placement) = shared.layer.clone() else {
                return;
            };
            let wanted = shared.wanted;
            let extent = Size::new(
                if wanted.width.0 > 0.0 || (placement.anchors.left && placement.anchors.right) {
                    wanted.width
                } else {
                    shared.logical.width
                },
                if wanted.height.0 > 0.0 || (placement.anchors.top && placement.anchors.bottom) {
                    wanted.height
                } else {
                    shared.logical.height
                },
            );
            (placement, extent)
        };
        crate::surface::role::layer::apply_with_extent(layer, &placement, extent);
    }

    /// Sends the input, opaque, and background blur regions asked for since the last send, right
    /// before a commit.
    ///
    /// Nothing is committed here: all three regions are double-buffered state of the surface that
    /// the next commit applies, and updating one never alters the pending state of the others.
    pub(crate) fn send_regions(&self) {
        let (input, opaque) = {
            let mut shared = self.shared();
            (shared.input_owed.take(), shared.opaque_owed.take())
        };
        let wl = self.wl_surface();
        if let Some(input) = input {
            match input {
                None => wl.set_input_region(None),
                Some(rects) => {
                    let region = self.region_of(&rects, region::outward);
                    wl.set_input_region(Some(&region));
                    region.destroy();
                }
            }
        }
        if let Some(opaque) = opaque {
            match opaque {
                Some(rects) if !rects.is_empty() => {
                    let region = self.region_of(&rects, region::inward);
                    wl.set_opaque_region(Some(&region));
                    region.destroy();
                }
                _ => wl.set_opaque_region(None),
            }
        }
        self.send_blur_region();
    }

    /// Whether this surface's logical role may participate in `ext_background_effect_v1`.
    ///
    /// Session lock surfaces (both persistent parents and disposable content subsurfaces) are
    /// unconditionally excluded by logical role.
    const fn is_blur_eligible(&self) -> bool {
        matches!(self.role, Role::Toplevel(_) | Role::Layer(_) | Role::Popup(_))
    }

    /// Re-marks a non-empty desired blur region dirty before a remap commit.
    pub(crate) fn restage_blur_for_remap(&self) {
        if !self.is_blur_eligible() || !self.active() {
            return;
        }
        if !self.background_effect.blur_supported() {
            return;
        }
        let mut shared = self.shared();
        if !shared.blur_desired.is_none() {
            shared.blur_owed = true;
        }
    }

    /// Idempotently destroys any `ext_background_effect_surface_v1` associated with this surface.
    ///
    /// Must run before `wl_surface.destroy()` on both early role retirement and `Drop`.
    pub(crate) fn destroy_blur_surface(&self) {
        let effect = {
            let mut shared = self.shared();
            shared.blur_owed = false;
            shared.blur_staged = None;
            shared.blur_pre_presented = None;
            shared.blur_last_committed = region::OwnedBlurRegion::None;
            shared.blur_surface.take()
        };
        if let Some(effect) = effect
            && effect.is_alive()
        {
            effect.destroy();
        }
    }

    /// Promotes any staged picture-bound blur candidate into `blur_desired` / `blur_owed` right
    /// before `send_regions()` runs inside `chrome::pre_present` (including `present_held`).
    pub(crate) fn promote_staged_blur_for_present(&self) {
        if !self.is_blur_eligible() || !self.active() {
            return;
        }
        let supported = self.background_effect.blur_supported();
        let mut shared = self.shared();
        if let Some(staged) = shared.blur_staged.take() {
            let previous = shared.blur_last_committed.clone();
            shared.blur_pre_presented = Some(previous);
            if shared.blur_desired != staged {
                shared.blur_desired = staged;
                let needs_protocol = (supported
                    && (!shared.blur_desired.is_none() || shared.blur_surface.is_some()))
                    || (!supported && shared.blur_surface.is_some());
                if needs_protocol {
                    shared.blur_owed = true;
                }
            }
        } else {
            shared.blur_pre_presented = None;
        }
    }

    /// Handles capability loss or manager removal: preserves desired intent, retires the protocol
    /// effect object once, and arranges a legal commit so the compositor clears the effect.
    pub(crate) fn background_blur_lost(&self) {
        if !self.is_blur_eligible() || !self.active() {
            return;
        }
        let (retired, needs_commit) = {
            let mut shared = self.shared();
            let retired = shared.blur_surface.take();
            let needs_commit = retired.is_some();
            if needs_commit {
                shared.blur_owed = true;
            }
            (retired, needs_commit)
        };
        if let Some(effect) = retired
            && effect.is_alive()
        {
            effect.destroy();
        }
        if needs_commit {
            Surface::request_redraw(self);
        }
    }

    /// Handles capability gain/regain: marks surfaces with non-empty desired blur intent dirty
    /// and arranges a legal commit to lazily bind and apply the latest desired region.
    pub(crate) fn background_blur_gained(&self) {
        if !self.is_blur_eligible() || !self.active() {
            return;
        }
        let needs_apply = {
            let mut shared = self.shared();
            let needs = !shared.blur_desired.is_none();
            if needs {
                shared.blur_owed = true;
            }
            needs
        };
        if needs_apply {
            Surface::request_redraw(self);
        }
    }

    fn send_blur_region(&self) {
        if !self.is_blur_eligible() || !self.active() {
            return;
        }
        enum BlurAction {
            Clear(ExtBackgroundEffectSurfaceV1),
            Full(ExtBackgroundEffectSurfaceV1),
            Rects(ExtBackgroundEffectSurfaceV1, Vec<Rect<CssPx, Css>>),
            Retire(ExtBackgroundEffectSurfaceV1),
        }
        let manager = self.background_effect.active_manager_for_blur();
        let action = {
            let mut shared = self.shared();
            if !shared.blur_owed {
                return;
            }
            shared.blur_owed = false;
            if shared.blur_pre_presented.is_none() {
                shared.blur_last_committed = shared.blur_desired.clone();
            }
            match manager {
                Some(manager) => match &shared.blur_desired {
                    region::OwnedBlurRegion::None => {
                        shared.blur_surface.clone().map(BlurAction::Clear)
                    }
                    region::OwnedBlurRegion::Full => {
                        let effect = shared
                            .blur_surface
                            .get_or_insert_with(|| {
                                manager.get_background_effect(self.wl_surface(), &self.qh, ())
                            })
                            .clone();
                        Some(BlurAction::Full(effect))
                    }
                    region::OwnedBlurRegion::Rects(rects) => {
                        let rects = rects.clone();
                        let effect = shared
                            .blur_surface
                            .get_or_insert_with(|| {
                                manager.get_background_effect(self.wl_surface(), &self.qh, ())
                            })
                            .clone();
                        Some(BlurAction::Rects(effect, rects))
                    }
                },
                None => shared.blur_surface.take().map(BlurAction::Retire),
            }
        };
        match action {
            Some(BlurAction::Clear(effect)) => {
                if effect.is_alive() {
                    effect.set_blur_region(None);
                }
            }
            Some(BlurAction::Full(effect)) => {
                if effect.is_alive() {
                    let region = self.create_region();
                    region.add(0, 0, i32::MAX, i32::MAX);
                    effect.set_blur_region(Some(&region));
                    region.destroy();
                }
            }
            Some(BlurAction::Rects(effect, rects)) => {
                if effect.is_alive() {
                    let region = self.region_of(&rects, region::outward);
                    effect.set_blur_region(Some(&region));
                    region.destroy();
                }
            }
            Some(BlurAction::Retire(effect)) => {
                if effect.is_alive() {
                    effect.destroy();
                }
            }
            None => {}
        }
    }

    /// A region holding `rects`, each rounded to whole pixels by `round`.
    fn region_of(
        &self,
        rects: &[Rect<CssPx, Css>],
        round: fn(Rect<CssPx, Css>) -> Option<region::Whole>,
    ) -> WlRegion {
        let region = self.create_region();
        for (x, y, width, height) in rects.iter().copied().filter_map(round) {
            region.add(x, y, width, height);
        }
        region
    }
    /// Fractional scaling's objects, for the commit that carries a new extent.
    pub(crate) const fn fractional(&self) -> &Fractional {
        &self.fractional
    }

    /// Takes the redraw asked for from anywhere since the last turn.
    pub(crate) fn take_request(&self) -> bool {
        self.asked.swap(false, Ordering::AcqRel)
    }

    /// How this surface reaches the seat.
    pub(crate) const fn seat(&self) -> &Arc<SeatLink> {
        &self.seat
    }

    /// The channel an assistive technology talks to.
    pub(crate) const fn a11y(&self) -> &A11y {
        &self.a11y
    }

    #[must_use]
    pub(crate) fn popup_root(&self) -> Option<SurfaceId> {
        self.popup.as_ref().map(|popup| popup.root)
    }

    #[must_use]
    pub(crate) fn popup_parent_live(&self) -> bool {
        self.popup.as_ref().is_some_and(|popup| {
            popup.parent.upgrade().is_some_and(|parent| parent.active())
        })
    }

    #[must_use]
    pub(crate) fn popup_grabs(&self, seat: &WlSeat) -> bool {
        self.popup.as_ref().and_then(|popup| popup.grab_seat.as_ref()) == Some(seat)
    }

    #[must_use]
    pub(crate) fn active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(crate) fn close(&self) {
        self.active.store(false, Ordering::Release);
    }

    pub(crate) fn retire(&self) {
        self.destroy_blur_surface();
        if let Some(state) = &self.popup {
            let mut desired = state.desired.lock().expect("popup placement lock poisoned");
            if desired.phase == PopupPhase::Retired {
                return;
            }
            desired.phase = PopupPhase::Retired;
            self.a11y.close();
            self.role.popup().expect("popup state has a popup role").retire();
        }
    }

    #[must_use]
    pub(crate) fn mark_popup_done(&self) -> bool {
        let Some(state) = &self.popup else { return false };
        let mut desired = state.desired.lock().expect("popup placement lock poisoned");
        if matches!(desired.phase, PopupPhase::Dismissed | PopupPhase::Retired) {
            return false;
        }
        desired.phase = PopupPhase::Dismissed;
        true
    }

    #[must_use]
    pub(crate) fn accepts_popups(&self) -> bool {
        self.popup.as_ref().is_none_or(|state| {
            let desired = state.desired.lock().expect("popup placement lock poisoned");
            !matches!(desired.phase, PopupPhase::Dismissed | PopupPhase::Retired)
        })
    }

    /// Bufferless commits may configure a popup, but it cannot map before its parent.
    #[must_use]
    pub(crate) fn parent_mapped(&self) -> bool {
        self.popup.as_ref().is_none_or(|popup| {
            popup.parent.upgrade().is_some_and(|parent| {
                if !parent.active() || !parent.parent_mapped() {
                    return false;
                }
                let shared = parent.shared();
                shared.mapped && !shared.unmapped
            })
        })
    }

    #[must_use]
    pub(crate) fn window_geometry_rect(&self) -> Rect<CssPx, Css> {
        let shared = self.shared();
        let insets = shared.window_geometry_insets;
        let size = if let Some(visual) = shared.configured_visual {
            Size::new(CssPx(visual.width as f32), CssPx(visual.height as f32))
        } else if self.popup.is_some() {
            shared.wanted
        } else if shared.logical.width.0 > 0.0 && shared.logical.height.0 > 0.0 {
            shared.logical
        } else {
            shared.wanted
        };
        Rect::new(
            Point::new(CssPx(insets.left as f32), CssPx(insets.top as f32)),
            size,
        )
    }

    pub(crate) fn add_popup_child(&self, child: &Arc<WaylandSurface>) {
        let mut children = self
            .popup_children
            .lock()
            .expect("popup children lock poisoned");
        children.retain(|weak| weak.strong_count() > 0);
        children.push(Arc::downgrade(child));
    }

    pub(crate) fn notify_popup_children(&self) {
        let live: Vec<Arc<WaylandSurface>> = {
            let mut children = self
                .popup_children
                .lock()
                .expect("popup children lock poisoned");
            let mut live = Vec::new();
            children.retain(|weak| match weak.upgrade() {
                Some(child) if child.active() => {
                    live.push(child);
                    true
                }
                _ => false,
            });
            live
        };
        for child in live {
            child.parent_geometry_changed(self);
        }
    }

    fn parent_geometry_changed(&self, parent: &WaylandSurface) {
        let (Some(state), Some(popup)) = (&self.popup, self.role.popup()) else {
            return;
        };
        if !self.active() || !popup.can_reposition() {
            return;
        }
        let mut desired = state.desired.lock().expect("popup placement lock poisoned");
        if matches!(desired.phase, PopupPhase::Dismissed | PopupPhase::Retired) {
            return;
        }
        let next_anchor = crate::surface::role::popup::anchor_rect(
            desired.placement.anchor_rect,
            parent.window_geometry_rect(),
        );
        if next_anchor == desired.last_sent_anchor_rect {
            return;
        }
        desired.pending = true;
        if desired.phase == PopupPhase::Mapped && self.shared().mapped {
            self.send_popup_placement(parent, &mut desired);
        }
    }

    #[must_use]
    pub(crate) fn popup_desired_insets(&self) -> Edges<i32> {
        self.popup.as_ref().map_or(Edges::new(0, 0, 0, 0), |state| {
            state
                .desired
                .lock()
                .expect("popup placement lock poisoned")
                .insets
        })
    }

    /// Records the configure that makes the popup eligible for its first frame.
    pub(crate) fn popup_configured(&self) {
        let Some(state) = &self.popup else { return };
        let mut desired = state.desired.lock().expect("popup placement lock poisoned");
        desired.reposition_in_flight = false;
        if desired.phase == PopupPhase::AwaitingConfigure {
            desired.phase = PopupPhase::AwaitingBuffer;
        }
    }

    /// Called after the renderer returns, when the buffer notified by pre_present is committed.
    pub(crate) fn popup_frame_committed(&self) {
        let Some(state) = &self.popup else { return };
        if !self.active() || !self.shared().mapped {
            return;
        }
        let mut desired = state.desired.lock().expect("popup placement lock poisoned");
        if desired.phase == PopupPhase::AwaitingBuffer {
            desired.phase = PopupPhase::Mapped;
        }
        if desired.phase == PopupPhase::Mapped && desired.pending
            && let Some(parent) = state.parent.upgrade().filter(|parent| parent.active())
        {
            self.send_popup_placement(&parent, &mut desired);
        }
    }

    #[must_use]
    pub(crate) fn resize_popup(&self, size: Size<CssPx, Css>) -> Option<Size<CssPx, Css>> {
        if !self.role.popup().is_some_and(|popup| popup.can_reposition()) {
            return None;
        }
        self.update_popup(None, size).ok()
    }

    fn apply_popup_insets_locally(&self, next_insets: Edges<i32>) -> Size<CssPx, Css> {
        let (needs_redraw, notify_children, drawable) = {
            let mut shared = self.shared();
            let changed = shared.window_geometry_insets != next_insets;
            shared.window_geometry_insets = next_insets;
            shared.geometry_owed |= changed;
            let visual = shared.configured_visual.unwrap_or_else(|| {
                Size::new(
                    (shared.wanted.width.0.round() as i32).max(1),
                    (shared.wanted.height.0.round() as i32).max(1),
                )
            });
            let drawable = Size::new(
                CssPx(
                    visual
                        .width
                        .saturating_add(next_insets.left)
                        .saturating_add(next_insets.right) as f32,
                ),
                CssPx(
                    visual
                        .height
                        .saturating_add(next_insets.top)
                        .saturating_add(next_insets.bottom) as f32,
                ),
            );
            if shared.configured_visual.is_some() {
                let scale = shared.scale;
                if let Some(event) = shared.resized(drawable, scale) {
                    shared.unreported = Some(event);
                } else {
                    shared.pacer.cancel_held();
                }
                (true, changed, drawable)
            } else {
                (false, changed, drawable)
            }
        };
        if needs_redraw {
            Surface::request_redraw(self);
        }
        if notify_children {
            self.notify_popup_children();
        }
        drawable
    }

    fn update_popup(
        &self,
        placement: Option<&PopupPlacement>,
        size: Size<CssPx, Css>,
    ) -> Result<Size<CssPx, Css>, PlatformError> {
        let (Some(state), Some(popup)) = (&self.popup, self.role.popup()) else {
            return Err(PlatformError::Unsupported(Unsupported));
        };
        let mut desired = state.desired.lock().expect("popup placement lock poisoned");
        if !self.active()
            || matches!(desired.phase, PopupPhase::Dismissed | PopupPhase::Retired)
        {
            return Err(PlatformError::Unsupported(Unsupported));
        }
        let parent = state.parent.upgrade().filter(|parent| parent.active())
            .ok_or(PlatformError::Unsupported(Unsupported))?;
        // Parent and grab belong to role creation, not to a positioner.
        if let Some(placement) = placement
            && (placement.parent != state.parent_id || placement.grab != desired.placement.grab)
        {
            return Err(PlatformError::Unsupported(Unsupported));
        }
        let offset = placement
            .map(|placement| PopupOffset::new(placement.offset))
            .transpose()?;
        let next_placement = placement.unwrap_or(&desired.placement);
        let (visual_i32, next_insets, drawable_i32) = next_placement.expand_visual_size(size)?;
        let visual_size = Size::new(
            CssPx(visual_i32.width as f32),
            CssPx(visual_i32.height as f32),
        );
        let drawable_size = Size::new(
            CssPx(drawable_i32.width as f32),
            CssPx(drawable_i32.height as f32),
        );
        let positioner_changed = visual_size != desired.size
            || placement.is_some_and(|p| {
                p.anchor_rect != desired.placement.anchor_rect
                    || p.anchor != desired.placement.anchor
                    || p.gravity != desired.placement.gravity
                    || p.constrain != desired.placement.constrain
                    || offset.is_some_and(|o| o != desired.offset)
                    || p.reactive != desired.placement.reactive
            });
        let insets_changed = next_insets != desired.insets;
        if !positioner_changed && !insets_changed {
            return Ok(drawable_size);
        }
        // Older shells retain their initial visual placement, while still allowing local inset
        // updates without calling `xdg_popup.reposition` or tearing down a grabbed popup tree.
        if !popup.can_reposition() {
            if !insets_changed {
                return Ok(drawable_size);
            }
            desired.placement.geometry_insets = next_placement.geometry_insets;
            desired.insets = next_insets;
            drop(desired);
            return Ok(self.apply_popup_insets_locally(next_insets));
        }
        if positioner_changed {
            if let Some((placement, offset)) = placement.zip(offset) {
                desired.placement = placement.clone();
                desired.offset = offset;
            }
            desired.size = visual_size;
            desired.insets = next_insets;
            desired.pending = true;
            self.shared().wanted = visual_size;
            if desired.phase == PopupPhase::Mapped && self.shared().mapped {
                self.send_popup_placement(&parent, &mut desired);
            }
            return Ok(drawable_size);
        }
        // Inset-only change: preserve the compositor-accepted visual size without issuing
        // `xdg_popup.reposition`.
        desired.placement.geometry_insets = next_placement.geometry_insets;
        desired.insets = next_insets;
        let deferred = desired.pending || desired.reposition_in_flight;
        drop(desired);
        if deferred {
            Ok(drawable_size)
        } else {
            Ok(self.apply_popup_insets_locally(next_insets))
        }
    }

    fn send_popup_placement(&self, parent: &WaylandSurface, desired: &mut PopupDesired) {
        let parent_geom = parent.window_geometry_rect();
        desired.last_sent_anchor_rect = crate::surface::role::popup::anchor_rect(
            desired.placement.anchor_rect,
            parent_geom,
        );
        desired.pending = false;
        desired.reposition_in_flight = true;
        desired.token = desired.token.wrapping_add(1);
        self.role.popup().expect("popup state has a popup role").reposition(
            &self.shell,
            &self.qh,
            &crate::surface::role::xdg::popup::Placed {
                place: &desired.placement,
                offset: desired.offset,
                parent: parent_geom,
                size: desired.size,
            },
            desired.token,
        );
        self.ping.ping();
    }

    /// The smallest and largest extents the user may drag to.
    pub(crate) fn bounds(&self) -> Bounds {
        self.shared().bounds
    }

    /// Records both bounds and states them together.
    pub(crate) fn set_bounds(&self, bounds: Bounds) {
        self.shared().bounds = bounds;
        if let Some(window) = self.role.window() {
            window.set_bounds(bounds.0, bounds.1);
        }
    }

    /// Creates an empty region, which the compositor copies when applied to a surface.
    pub(crate) fn create_region(&self) -> WlRegion {
        self.compositor.create_region(&self.qh, ())
    }

    /// Asks the compositor to report when the frame about to be committed reaches the screen.
    pub(crate) fn ask_for_feedback(&self) {
        self.presentation.ask(&self.qh, self.wl_surface(), self.id);
    }

    /// Asks the compositor to tell us when it is ready for the next frame.
    ///
    /// Requested before the commit that carries a frame, so that it rides that commit. A callback
    /// asked for after the commit belongs to the *next* one, which never comes: that is the shape
    /// of a client that draws one frame and stops.
    pub(crate) fn ask_for_callback(&self) {
        let serial = self.shared().begin_callback();
        let callback = FrameCallback {
            surface: self.id,
            serial,
        };
        self.wl_surface().frame(&self.qh, callback);
    }

    /// Asks the compositor to answer once it has taken every request sent so far.
    ///
    /// Sent right after the commit that unmaps the surface. The answer arrives after every
    /// configure the shell sent before that commit, see [`sequence`].
    pub(crate) fn send_barrier(&self, serial: u64) {
        let barrier = sequence::Barrier {
            surface: self.id,
            serial,
        };
        self.conn.display().sync(&self.qh, barrier);
    }

    /// Ends a delivered [`SurfaceEvent::PresentReady`].
    /// `began` is when the present began.
    pub(crate) fn finish_present(&self, began: Instant, now: Instant) {
        self.shared().end_of_present(began, now);
        self.end_drawing();
    }

    /// Records that a present held for the compositor is about to be delivered.
    pub(crate) fn begin_present(&self) {
        self.shared().drawing = true;
    }

    /// Ends a delivery to the application, carrying out a hide it asked for while it drew.
    fn end_drawing(&self) {
        let hide = {
            let mut shared = self.shared();
            shared.drawing = false;
            shared.hide_owed
        };
        if hide {
            chrome::unmap(self);
        }
    }

    /// Ends a delivered redraw, keeping the frame chain alive wherever a frame actually ran.
    ///
    /// A frame callback rides a commit. Every reason a *ran* frame can end without presenting —
    /// no damage, a lost device, a refused acquisition, a surface the compositor is not drawing —
    /// would otherwise end the turn with no commit, and the compositor would never speak about
    /// this surface again.
    ///
    /// A redraw the application **declined** ends differently: nothing ran, so nothing is
    /// committed and no callback is owed. The application kept the obligation, and its own
    /// deadline asks for the frame — which the pacer, owing nothing, answers in the same turn.
    /// A bufferless commit here would put that deadline behind a compositor round trip.
    ///
    /// A redraw that ran while a callback is owed commits nothing either: that callback keeps the
    /// chain alive, and a held frame is committed when it arrives. Nor does a redraw that ran,
    /// presented nothing and left nothing asked for: no chain needs to stay alive.
    /// `began` is when the redraw began. Its cost decides whether the next one composes ahead.
    pub(crate) fn finish_redraw(&self, began: Instant, now: Instant) {
        // A redraw asked for while this one ran is pulled into the pacer on the next turn.
        let again = self.asked.load(Ordering::Acquire);
        let (end, bare, exact_owed) = {
            let mut shared = self.shared();
            (
                shared.redrawn(began, now, again),
                shared.may_commit_bare(),
                shared.exact_owed,
            )
        };
        let keep_alive = end == crate::surface::state::EndOfRedraw::KeepChainAlive;
        // A surface that is hidden, or about to be, owes the compositor nothing: an empty commit
        // would ask the shell to map it again.
        if keep_alive && bare {
            // A change asked for with no new content to draw still goes out, with this commit.
            self.send_layer(false);
            self.send_regions();
            let bare_geom = self
                .shared()
                .take_bare_window_geometry(self.role.xdg_surface().is_some());
            if let (Some(xdg), Some(geom)) = (self.role.xdg_surface(), bare_geom) {
                xdg.set_window_geometry(geom.x, geom.y, geom.width, geom.height);
            }
            self.ask_for_callback();
            self.role.commit();
        } else if keep_alive {
            // Nothing was committed, so no callback is on its way.
            self.shared().pacer.commit_withheld();
            if exact_owed {
                // A lock surface that still owes a buffer at its new extent draws again next
                // turn.
                zgui_platform::Surface::request_redraw(self);
            }
        }
        self.end_drawing();
    }
}

/// What a frame callback answers for: the surface, and which of its callbacks it is.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameCallback {
    /// The surface the callback was asked for.
    pub(crate) surface: SurfaceId,
    /// The number [`state::Shared::begin_callback`] gave it.
    pub(crate) serial: u64,
}

impl Drop for WaylandSurface {
    /// Destroys the objects that hang off the surface before the surface itself.
    ///
    /// Order is not a preference: a viewport outliving the surface it maps is a protocol error
    /// rather than a leak. This runs before any field is dropped, and the surface is owned by the
    /// role, which is a field — so the ordering holds wherever the last handle happens to go.
    fn drop(&mut self) {
        // Before the objects, because the channel is on another thread and a question answered
        // against a surface that has gone is answered against nothing.
        self.a11y.close();
        self.destroy_blur_surface();
        self.fractional.release();
    }
}

impl core::fmt::Debug for WaylandSurface {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WaylandSurface")
            .field("id", &self.id)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl Surface for WaylandSurface {
    fn id(&self) -> SurfaceId {
        self.id
    }

    fn size(&self) -> Size<DevicePx, Device> {
        self.shared().size
    }

    fn scale_factor(&self) -> f64 {
        self.shared().scale
    }

    fn subpixel_layout(&self) -> zgui_platform::SubpixelLayout {
        self.shared().subpixel
    }

    fn refresh_rate_millihertz(&self) -> Option<u32> {
        self.shared().timing.refresh_rate_millihertz()
    }

    fn present_pacing(&self) -> PresentPacing {
        // The whole reason this backend exists. Presentation that waits for the display waits on
        // the frame callbacks this backend is already waiting on, on the thread that also reads
        // input, and a surface the compositor stops drawing blocks it for a second at a time.
        PresentPacing::Platform
    }

    fn crops_buffers(&self) -> bool {
        self.fractional().crops()
    }

    fn presentation_timing(&self) -> Option<PresentationTiming> {
        Some(self.shared().snapshot())
    }

    fn reports_presentation(&self) -> bool {
        true
    }

    fn request_size(&self, size: Size<CssPx, Css>) -> Option<Size<DevicePx, Device>> {
        chrome::request_size(self, size)
    }

    fn set_min_size(&self, size: Option<Size<CssPx, Css>>) {
        chrome::set_min_size(self, size);
    }

    fn set_max_size(&self, size: Option<Size<CssPx, Css>>) {
        chrome::set_max_size(self, size);
    }

    fn request_redraw(&self) {
        if !self.active() {
            return;
        }
        // Coalescing, and cheap enough to be called from a worker thread on every value that
        // changed: the flag is what the loop reads, and the poke is only there to end a park.
        if !self.asked.swap(true, Ordering::AcqRel) {
            zgui_profile::latency::mark("req.redraw");
        }
        self.ping.ping();
    }

    fn pre_present_notify(&self) {
        let (size, scale) = {
            let shared = self.shared();
            let size = Size::new(
                shared.size.width.0.round() as i32,
                shared.size.height.0.round() as i32,
            );
            (size, shared.scale)
        };
        chrome::pre_present(self, size, scale);
    }

    fn pre_present_frame(&self, size: Size<i32, Device>, scale: f64) {
        chrome::pre_present(self, size, scale);
    }

    fn present_now(&self) -> bool {
        if !self.active() || !self.parent_mapped() {
            return false;
        }
        self.shared().pacer.present_now(Instant::now())
    }

    fn redraw_declined(&self) {
        self.shared().declined = true;
    }

    fn set_title(&self, title: &str) {
        if let Some(window) = self.role.window() {
            window.set_title(title);
        }
    }

    fn set_visible(&self, visible: bool) {
        chrome::set_visible(self, visible);
    }

    fn set_decorations(&self, decorations: Decorations) {
        if let Some(window) = self.role.window() {
            window.request_decorations(decorations);
        }
    }

    fn set_resizable(&self, resizable: bool) {
        chrome::set_resizable(self, resizable);
    }

    fn set_maximized(&self, maximized: bool) {
        if let Some(window) = self.role.window() {
            window.set_maximized(maximized);
        }
    }

    fn set_minimized(&self, minimized: bool) {
        // There is no way back: a compositor tells a client nothing about being minimised and
        // offers no request to undo it. Restoring is the person's, through the desktop.
        if minimized && let Some(window) = self.role.window() {
            window.set_minimized();
        }
    }

    fn set_fullscreen(&self, mode: Option<FullscreenMode>) {
        let Some(window) = self.role.window() else {
            return;
        };
        // A compositor never changes an output's mode for a client, so both kinds of full screen
        // are the same request and the exclusive one is the borderless one.
        window.set_fullscreen(mode.is_some());
    }

    fn is_maximized(&self) -> bool {
        self.shared().maximized
    }

    fn fullscreen(&self) -> Option<FullscreenMode> {
        self.shared()
            .fullscreen
            .then_some(FullscreenMode::Borderless)
    }

    fn focus(&self) {
        self.seat.activate(self.id, true);
    }

    fn input_serial(&self) -> Option<zgui_platform::InputSerial> {
        self.seat
            .press_serial()
            .map(zgui_platform::InputSerial::new)
    }

    fn request_attention(&self, urgent: bool) {
        // Turning it off is not a request this protocol has: a desktop stops drawing attention when
        // the person looks at the window, which is its decision and not the application's.
        if urgent {
            self.seat.activate(self.id, false);
        }
    }

    fn set_cursor(&self, cursor: CursorStyle) {
        // The pointer belongs to the seat rather than to a surface, and there is one of it. What a
        // surface asks for is what the pointer looks like while it is over that surface, so the
        // request is refused unless the pointer is actually there — otherwise a background window
        // changing its cursor would change it under the window the person is using.
        self.seat.set_cursor(self.id, cursor);
    }

    fn set_layer_placement(&self, patch: &zgui_platform::LayerPatch) -> Result<(), Unsupported> {
        self.patch_layer(patch)
    }

    fn set_pointer_passthrough(&self, passthrough: bool) -> Result<(), Unsupported> {
        chrome::set_pointer_passthrough(self, passthrough)
    }

    fn set_shell_input_region(&self, region: &InputRegion) -> Result<(), PlatformError> {
        chrome::set_shell_input_region(self, region);
        Ok(())
    }

    fn reconfigure_layer(
        &self,
        placement: &LayerPlacement,
        size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        chrome::reconfigure_layer(self, placement, size)
    }

    fn reconfigure_popup(
        &self,
        placement: &PopupPlacement,
        size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        if !chrome::valid_size(size) {
            return Err(PlatformError::SurfaceCreation("invalid pop-up extent".to_owned()));
        }
        self.update_popup(Some(placement), size).map(|_| ())
    }

    fn popup_dismissed(&self) -> bool {
        self.popup.as_ref().is_some_and(|state| {
            state.desired.lock().expect("popup placement lock poisoned").phase == PopupPhase::Dismissed
        })
    }

    fn supports_popup_reposition(&self) -> bool {
        self.role.popup().is_some_and(|popup| popup.can_reposition())
    }

    fn set_input_region(&self, region: Option<&[Rect<CssPx, Css>]>) -> Result<(), Unsupported> {
        self.shared().input_owed = Some(region.map(<[_]>::to_vec));
        self.request_redraw();
        Ok(())
    }

    fn set_opaque_region(&self, region: Option<&[Rect<CssPx, Css>]>) {
        self.shared().opaque_owed = Some(region.map(<[_]>::to_vec));
        self.request_redraw();
    }

    fn set_blur_region(&self, region: BlurRegion<'_>) {
        if !self.is_blur_eligible() || !self.active() {
            return;
        }
        let next = region::normalize_blur(region);
        let supported = self.background_effect.blur_supported();
        let should_redraw = {
            let mut shared = self.shared();
            shared.blur_staged = None;
            shared.blur_pre_presented = None;
            if shared.blur_desired == next {
                return;
            }
            shared.blur_desired = next;
            let needs_protocol = (supported
                && (!shared.blur_desired.is_none() || shared.blur_surface.is_some()))
                || (!supported && shared.blur_surface.is_some());
            if needs_protocol {
                shared.blur_owed = true;
            }
            needs_protocol
        };
        if should_redraw {
            self.request_redraw();
        }
    }

    fn logical_extent_for(&self, size: Size<i32, Device>, scale: f64) -> Size<CssPx, Css> {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let logical = {
            let shared = self.shared();
            let matching_current =
                shared.device_extent() == size && (shared.scale - scale).abs() < 1e-5;
            if matching_current {
                shared.logical
            } else {
                Size::new(
                    CssPx((f64::from(size.width.max(1)) / scale) as f32),
                    CssPx((f64::from(size.height.max(1)) / scale) as f32),
                )
            }
        };
        if self.fractional().crops() {
            Size::new(
                CssPx((logical.width.0.round() as i32).max(1) as f32),
                CssPx((logical.height.0.round() as i32).max(1) as f32),
            )
        } else {
            logical
        }
    }

    fn stage_blur_region_for_present(&self, region: BlurRegion<'_>) {
        if !self.is_blur_eligible() || !self.active() {
            return;
        }
        let next = region::normalize_blur(region);
        let mut shared = self.shared();
        shared.blur_staged = Some(next);
    }

    fn clear_staged_blur_region(&self) {
        let mut shared = self.shared();
        shared.blur_staged = None;
        shared.blur_pre_presented = None;
    }

    fn finalize_presented_blur(&self) {
        let mut shared = self.shared();
        shared.blur_pre_presented = None;
        shared.blur_last_committed = shared.blur_desired.clone();
    }

    fn rollback_unpresented_blur(&self) {
        let (should_redraw, drawing) = {
            let mut shared = self.shared();
            shared.blur_staged = None;
            if let Some(previous) = shared.blur_pre_presented.take() {
                let changed = shared.blur_desired != previous;
                shared.blur_desired = previous;
                let supported = self.background_effect.blur_supported();
                let needs_protocol = changed
                    && ((supported
                        && (!shared.blur_desired.is_none() || shared.blur_surface.is_some()))
                        || (!supported && shared.blur_surface.is_some()));
                if needs_protocol {
                    shared.blur_owed = true;
                }
                (needs_protocol, shared.drawing)
            } else {
                (false, false)
            }
        };
        if should_redraw && !drawing {
            self.request_redraw();
        }
    }

    fn apply_idle_blur_region(&self, region: BlurRegion<'_>) {
        if !self.is_blur_eligible() || !self.active() {
            return;
        }
        let next = region::normalize_blur(region);
        let supported = self.background_effect.blur_supported();
        let (should_redraw, drawing) = {
            let mut shared = self.shared();
            shared.blur_staged = None;
            shared.blur_pre_presented = None;
            if shared.blur_desired == next {
                return;
            }
            shared.blur_desired = next;
            let needs_protocol = (supported
                && (!shared.blur_desired.is_none() || shared.blur_surface.is_some()))
                || (!supported && shared.blur_surface.is_some());
            if needs_protocol {
                shared.blur_owed = true;
            }
            (needs_protocol, shared.drawing)
        };
        // During an active redraw delivery (`drawing == true`), `finish_redraw` observes
        // `blur_owed` via `owes_surface_change()` and performs one legal bare commit without
        // queuing a redundant redraw loop.
        if should_redraw && !drawing {
            self.request_redraw();
        }
    }

    fn begin_move_drag(&self) -> Result<(), Unsupported> {
        chrome::begin_move_drag(self)
    }

    fn begin_resize_drag(&self, edge: zgui_platform::ResizeEdge) -> Result<(), Unsupported> {
        chrome::begin_resize_drag(self, edge)
    }

    fn begin_outbound_drag(
        &self,
        payload: zgui_platform::OutboundDragPayload,
    ) -> Result<(), PlatformError> {
        let (_seat, serial) = self.seat.live_drag_serial(self.id)?;
        self.selections
            .start_outbound_drag(self.id, self.wl_surface(), serial, payload)
            .map_err(PlatformError::from)
    }

    fn cancel_outbound_drag(&self) {
        self.selections.cancel_outbound_drag(self.id);
    }

    fn set_text_input(&self, state: Option<TextInput>) {
        self.seat.set_text_input(self.id, state);
    }

    fn reset_dead_keys(&self) {
        self.seat.reset_composition();
    }

    fn push_a11y_update(&self, build: &mut dyn FnMut() -> TreeUpdate) {
        self.a11y.publish(build);
    }

    fn gpu(&self) -> Option<&dyn GpuSurface> {
        Some(self)
    }

    fn gpu_shared(self: Arc<Self>) -> Option<Arc<dyn GpuSurface>> {
        Some(self)
    }
}
