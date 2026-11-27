//! The data device: what is on a clipboard, and what is being dragged over a window.
//!
//! Two protocols and one object, which is the protocol's own shape rather than a choice here: a
//! selection and a drag arrive on the same device and are told apart by which events carry them.

use smithay_client_toolkit::data_device_manager::data_device::DataDeviceHandler;
use smithay_client_toolkit::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use smithay_client_toolkit::data_device_manager::data_source::DataSourceHandler;
use smithay_client_toolkit::data_device_manager::WritePipe;
use smithay_client_toolkit::primary_selection::device::PrimarySelectionDeviceHandler;
use smithay_client_toolkit::primary_selection::selection::PrimarySelectionSourceHandler;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device::WlDataDevice;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device_manager::DndAction;
use smithay_client_toolkit::reexports::client::protocol::wl_data_source::WlDataSource;
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::primary_selection::zv1::client::zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1;
use smithay_client_toolkit::reexports::protocols::wp::primary_selection::zv1::client::zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1;
use smithay_client_toolkit::{delegate_data_device, delegate_primary_selection};

use zgui_platform::{DragAction, DragEvent, SurfaceEvent};

use crate::driver::WaylandState;

impl WaylandState {
    /// Opens this seat's data devices, once there is a seat to open them on.
    ///
    /// Both are optional in different ways. The ordinary clipboard exists on every compositor but
    /// is bound here rather than at start-up because a device belongs to a seat. The selection
    /// clipboard is a protocol a compositor may simply not have, and its absence is a capability
    /// rather than a failure.
    pub(crate) fn open_data_devices(&mut self, seat: &WlSeat) {
        let standard = self.data.take().map(|manager| {
            let device = manager.get_data_device(&self.qh, seat);
            (manager, device)
        });
        let primary = self.primary.take().map(|manager| {
            let device = manager.get_selection_device(&self.qh, seat);
            (manager, device)
        });
        self.clipboard.selections().devices(standard, primary);
    }
}

impl DataDeviceHandler for WaylandState {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        device: &WlDataDevice,
        x: f64,
        y: f64,
        surface: &WlSurface,
    ) {
        let Some(id) = self.identify(surface) else {
            return;
        };
        let Some(offer) = device
            .data::<smithay_client_toolkit::data_device_manager::data_device::DataDeviceData>()
            .and_then(smithay_client_toolkit::data_device_manager::data_device::DataDeviceData::drag_offer)
        else {
            return;
        };
        let offer_id = offer.inner().id().protocol_id();
        if let Some((prev_id, prev_event)) =
            self.drag
                .entered(id, offer_id, offer, crate::input::pointer::position(x, y))
        {
            self.report(prev_id, SurfaceEvent::Drag(prev_event));
        }
        self.start_drag_read();
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _device: &WlDataDevice,
        x: f64,
        y: f64,
    ) {
        let at = crate::input::pointer::position(x, y);
        if let Some((id, event)) = self.drag.moved(at) {
            self.report_drag(id, event);
        }
    }

    fn leave(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _device: &WlDataDevice) {
        if let Some((id, event)) = self.drag.left() {
            self.report(id, SurfaceEvent::Drag(event));
        }
    }

    fn drop_performed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _device: &WlDataDevice,
    ) {
        if let Some((id, event)) = self.drag.dropped(self.drag.drop_action()) {
            self.report_drag(id, event);
        }
    }

    fn selection(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _device: &WlDataDevice) {
        // Nothing to do: what is on the clipboard is read when somebody asks, not when it changes,
        // and reading every selection as it appears would ask every application that copies
        // anything for its content whether or not this one ever pastes.
    }
}

impl DataOfferHandler for WaylandState {
    fn source_actions(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        offer: &mut DragOffer,
        actions: DndAction,
    ) {
        self.drag.source_actions(offer, actions);
    }

    fn selected_action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        offer: &mut DragOffer,
        action: DndAction,
    ) {
        self.drag.selected_action(offer, action);
    }
}

impl DataSourceHandler for WaylandState {
    fn send_request(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        destination: WritePipe,
    ) {
        self.clipboard.selections().serve(source, &mime, destination);
    }

    fn cancelled(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, source: &WlDataSource) {
        if let Some(surface) = self.clipboard.selections().outbound_drag_cancelled(source) {
            self.report(
                surface,
                SurfaceEvent::Drag(DragEvent::OutboundFinished { action: DragAction::None }),
            );
            return;
        }
        // Somebody else took the selection. Letting go here is what stops this application from
        // answering for a clipboard it no longer owns.
        self.clipboard.selections().lost(source);
    }

    fn accept_mime(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &WlDataSource,
        _mime: Option<String>,
    ) {
    }

    fn dnd_dropped(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _source: &WlDataSource) {}

    fn dnd_finished(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &WlDataSource,
    ) {
        if let Some((surface, action)) = self.clipboard.selections().outbound_drag_finished(source) {
            self.report(
                surface,
                SurfaceEvent::Drag(DragEvent::OutboundFinished { action }),
            );
        }
    }

    fn action(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &WlDataSource,
        action: DndAction,
    ) {
        self.clipboard
            .selections()
            .outbound_drag_action(source, action);
    }
}

impl PrimarySelectionDeviceHandler for WaylandState {
    fn selection(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _device: &ZwpPrimarySelectionDeviceV1,
    ) {
        // As above: read when asked, not when it changes.
    }
}

impl PrimarySelectionSourceHandler for WaylandState {
    fn send_request(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
        mime: String,
        destination: WritePipe,
    ) {
        self.clipboard
            .selections()
            .serve_primary(source, &mime, destination);
    }

    fn cancelled(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
    ) {
        self.clipboard.selections().lost_primary(source);
    }
}

delegate_data_device!(WaylandState);
delegate_primary_selection!(WaylandState);
