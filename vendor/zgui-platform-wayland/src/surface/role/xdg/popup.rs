//! A pop-up, on the shell this backend binds itself.

use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_popup::{self, XdgPopup};
use wayland_protocols::xdg::shell::client::xdg_positioner::XdgPositioner;
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface;
use zgui_geom::{Css, CssPx, Rect, Size};
use zgui_platform::{CloseReason, PopupPlacement, SurfaceEvent, SurfaceId};

use crate::driver::WaylandState;
use crate::surface::role::popup as placement;
use crate::surface::role::xdg::XdgShell;

/// The version at which a pop-up can be moved without being made again.
const REPOSITION_SINCE: u32 = 3;

/// Where a pop-up goes: what it is placed against, and how large each of the two is.
///
/// One value rather than four arguments, because the compositor answers one question with all of
/// them — where a rectangle of this size fits against that one — and it is asked again, unchanged
/// but for the size, every time the pop-up grows.
#[derive(Clone, Debug)]
pub struct Placed<'a> {
    /// Where it hangs from, and what the compositor may do to fit it.
    pub place: &'a PopupPlacement,
    /// The validated, quantized translation from the anchor.
    pub offset: placement::Offset,
    /// The window geometry (origin and visual extent) of the surface it is placed against.
    pub parent: Rect<CssPx, Css>,
    /// The visual extent of the pop-up itself.
    pub size: Size<CssPx, Css>,
}

/// A pop-up: the surface, the shell object over it, and the role.
#[derive(Debug)]
pub struct Popup {
    /// The surface underneath.
    wl: WlSurface,
    /// The shell object a configure arrives on.
    xdg: XdgSurface,
    /// The pop-up role.
    popup: XdgPopup,
}

impl Popup {
    /// Makes a pop-up out of `wl`, with an optional foreign layer parent.
    pub fn new(
        shell: &XdgShell,
        qh: &QueueHandle<WaylandState>,
        wl: WlSurface,
        id: SurfaceId,
        parent: &crate::surface::Role,
        placed: &Placed<'_>,
    ) -> Self {
        let positioner = shell.base().create_positioner(qh, ());
        placement::describe(
            &positioner,
            placed.place,
            placed.offset,
            placed.parent,
            placed.size,
        );
        let xdg = shell.base().get_xdg_surface(&wl, qh, id);
        let popup = xdg.get_popup(parent.xdg_surface(), &positioner, qh, id);
        if let Some(layer) = parent.layer() {
            layer.get_popup(&popup);
        }
        positioner.destroy();
        Self { wl, xdg, popup }
    }

    /// The surface underneath.
    pub const fn wl_surface(&self) -> &WlSurface {
        &self.wl
    }

    /// The shell object a configure arrives on.
    pub const fn xdg_surface(&self) -> &XdgSurface {
        &self.xdg
    }

    /// The pop-up role, for a shell layer to adopt as its child.
    pub const fn xdg_popup(&self) -> &XdgPopup {
        &self.popup
    }

    /// Acknowledges a configure.
    pub fn ack(&self, serial: u32) {
        self.xdg.ack_configure(serial);
    }

    /// Takes the pointer and keyboard until the pop-up is dismissed.
    ///
    /// The serial has to be from a press. A grab quoted against anything else is refused, and a
    /// refused grab does not fail: the compositor dismisses the pop-up the instant it opens, which
    /// looks like a menu that will not stay open.
    pub fn grab(&self, seat: &WlSeat, serial: u32) {
        self.popup.grab(seat, serial);
    }

    /// Whether this compositor supports live repositioning.
    #[must_use]
    pub(crate) fn can_reposition(&self) -> bool {
        self.popup.version() >= REPOSITION_SINCE
    }

    /// Moves the pop-up without making it again.
    pub fn reposition(
        &self,
        shell: &XdgShell,
        qh: &QueueHandle<WaylandState>,
        placed: &Placed<'_>,
        token: u32,
    ) {
        let positioner = shell.base().create_positioner(qh, ());
        placement::describe(
            &positioner,
            placed.place,
            placed.offset,
            placed.parent,
            placed.size,
        );
        self.popup.reposition(&positioner, token);
        positioner.destroy();
    }

    /// Releases the shell role even when the client still holds a surface handle.
    pub(crate) fn retire(&self) {
        if self.popup.is_alive() {
            self.popup.destroy();
        }
        if self.xdg.is_alive() {
            self.xdg.destroy();
        }
    }
}

impl Drop for Popup {
    /// Takes the objects down innermost first, which the protocol requires.
    fn drop(&mut self) {
        self.retire();
        self.wl.destroy();
    }
}

impl Dispatch<XdgPopup, SurfaceId> for WaylandState {
    fn event(
        state: &mut Self,
        _popup: &XdgPopup,
        event: <XdgPopup as Proxy>::Event,
        id: &SurfaceId,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            // Stashed like a window's, and applied when the surface says the description is done.
            xdg_popup::Event::Configure { width, height, .. } => {
                state.stash_popup(*id, (width, height));
            }
            // The compositor dismissed it — a press outside a menu, the parent losing focus. There
            // is no way back for this surface; the application closes it and makes another.
            xdg_popup::Event::PopupDone => {
                if state.live.surface(*id).is_some_and(|surface| surface.mark_popup_done()) {
                    state.report(*id, SurfaceEvent::Closed(CloseReason::Dismissed));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgPositioner, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _positioner: &XdgPositioner,
        _event: <XdgPositioner as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Write-only: it describes a placement and says nothing back.
    }
}
