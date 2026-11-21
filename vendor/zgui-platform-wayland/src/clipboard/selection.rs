//! What is on each clipboard, and what this application has put there.

use std::sync::{Arc, Mutex, MutexGuard};

use smithay_client_toolkit::data_device_manager::DataDeviceManagerState;
use smithay_client_toolkit::data_device_manager::data_device::DataDevice;
use smithay_client_toolkit::data_device_manager::data_source::{CopyPasteSource, DragSource};
use smithay_client_toolkit::data_device_manager::{ReadPipe, WritePipe};
use smithay_client_toolkit::primary_selection::PrimarySelectionManagerState;
use smithay_client_toolkit::primary_selection::device::PrimarySelectionDevice;
use smithay_client_toolkit::primary_selection::selection::PrimarySelectionSource;
use smithay_client_toolkit::reexports::client::Connection;
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device_manager::DndAction;
use smithay_client_toolkit::reexports::client::protocol::wl_data_source::WlDataSource;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::protocols::wp::primary_selection::zv1::client::zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1;
use zgui_platform::{
    ClipboardData, ClipboardError, ClipboardFormat, ClipboardKind, ClipboardSerial,
    ClipboardWriteOptions, OutboundDragPayload, SurfaceId, Unsupported, Waker,
};

use crate::clipboard::{deliver, mime, pipe};
use crate::driver::WaylandState;

/// The devices, the offers on them, and what this application owns.
///
/// Shared and thread-safe because a read is answered on a thread of its own and a write can be
/// asked for from anywhere. The loop writes the device half; every request reads it.
#[derive(Debug, Default)]
pub struct Selections {
    /// Everything under one lock, because a read consults all of it at once.
    inner: Mutex<Inner>,
}

/// The changing half.
#[derive(Default)]
struct Inner {
    /// The connection every request is flushed on.
    conn: Option<Connection>,
    /// The queue the sources this application makes are dispatched on.
    qh: Option<QueueHandle<WaylandState>>,
    /// Where an answer to a read is delivered.
    waker: Option<Arc<dyn Waker>>,
    /// The standard clipboard's device and manager.
    standard: Option<Standard>,
    /// The selection clipboard's, where the compositor offers one.
    primary: Option<Primary>,
    /// The serial a claim on a selection may quote, which is always from a press.
    serial: Option<u32>,
    /// What this application has put on each clipboard.
    held: [Option<Held>; 2],
    /// Currently active outbound `Copy`-only drag source, if any.
    outbound_drag: Option<ActiveOutboundDrag>,
}

/// An active outbound `Copy`-only drag source owned by this client until finished or cancelled.
#[derive(Debug)]
struct ActiveOutboundDrag {
    surface: SurfaceId,
    source: DragSource,
    items: Vec<(String, Vec<u8>)>,
    accepted_mime: Option<String>,
    selected_action: DndAction,
    dropped: bool,
}

impl Drop for ActiveOutboundDrag {
    fn drop(&mut self) {
        // `DragSource` derives `Clone` and does not destroy its proxy on drop;
        // destroy explicitly so retiring or cancelling tears down the Wayland drag cleanly.
        self.source.inner().destroy();
    }
}

/// The ordinary clipboard's objects.
#[derive(Debug)]
struct Standard {
    /// The factory for sources.
    manager: DataDeviceManagerState,
    /// This seat's device.
    device: DataDevice,
}

/// The selection clipboard's objects.
#[derive(Debug)]
struct Primary {
    /// The factory for sources.
    manager: PrimarySelectionManagerState,
    /// This seat's device.
    device: PrimarySelectionDevice,
}

/// A selection this application owns, and the source that serves it.
#[derive(Debug)]
struct Held {
    /// What was put there.
    data: ClipboardData,
    /// Whether clipboard managers should exclude this selection from history.
    secret: bool,
    /// The source serving it, kept alive for as long as the selection is owned.
    source: Source,
}

/// The source object, whichever clipboard it belongs to.
#[derive(Debug)]
enum Source {
    /// A source on the ordinary clipboard.
    Standard(CopyPasteSource),
    /// A source on the selection clipboard.
    Primary(PrimarySelectionSource),
}

/// A read that has been asked for and not yet answered.
#[derive(Debug)]
pub struct Pending {
    /// The pipe the other process writes into.
    reader: ReadPipe,
    /// Which clipboard it came from, for the error that says it was empty.
    kind: ClipboardKind,
    /// Which representation was requested.
    format: ClipboardFormat,
}

impl Pending {
    /// Waits for the answer here, up to `patience`.
    pub fn answer_now(
        self,
        patience: std::time::Duration,
    ) -> Result<ClipboardData, ClipboardError> {
        finish(self.kind, self.format, pipe::read(self.reader, patience))
    }

    /// Waits for the answer on a thread of its own, and delivers it to the loop.
    ///
    /// A thread rather than a source on the loop, because the wait is on another process and the
    /// point of this backend is that the loop's own thread never waits on one.
    pub fn answer_on_a_thread(self, selections: Arc<Selections>, serial: ClipboardSerial) {
        let kind = self.kind;
        let format = self.format;
        let answering = Arc::clone(&selections);
        let started = std::thread::Builder::new()
            .name("zgui-clipboard".to_owned())
            .spawn(move || {
                let answer = finish(kind, format, pipe::read(self.reader, pipe::PATIENCE));
                answering.answer(serial, answer);
            });
        if let Err(error) = started {
            // Nobody is going to answer, so the request is answered here instead: a read that is
            // never answered leaves whatever asked for it waiting for ever.
            selections.answer(serial, Err(ClipboardError::Backend(error.to_string())));
        }
    }
}

/// What a completed transfer means.
fn finish(
    kind: ClipboardKind,
    format: ClipboardFormat,
    taken: Result<Vec<u8>, pipe::Failed>,
) -> Result<ClipboardData, ClipboardError> {
    let bytes = taken?;
    if bytes.is_empty() {
        return Err(ClipboardError::Empty(kind));
    }
    match format {
        ClipboardFormat::Text => {
            // Lossy rather than refused: a selection whose owner declared UTF-8 and wrote
            // something else is still mostly readable, and refusing the paste entirely helps nobody.
            let text = String::from_utf8_lossy(&bytes).into_owned();
            Ok(ClipboardData::Text(text.into()))
        }
        ClipboardFormat::Image => Ok(ClipboardData::Image {
            media_type: mime::PNG.into(),
            bytes: bytes.into(),
        }),
        other => Err(ClipboardError::UnsupportedFormat(other)),
    }
}

impl core::fmt::Debug for Inner {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Selections")
            .field("standard", &self.standard.is_some())
            .field("primary", &self.primary.is_some())
            .field(
                "owns",
                &self.held.iter().filter(|held| held.is_some()).count(),
            )
            .finish_non_exhaustive()
    }
}

impl Selections {
    /// Records the connection and the queue everything is made on.
    pub(crate) fn attach(
        &self,
        conn: Connection,
        qh: QueueHandle<WaylandState>,
        waker: Arc<dyn Waker>,
    ) {
        let mut inner = self.lock();
        inner.conn = Some(conn);
        inner.qh = Some(qh);
        inner.waker = Some(waker);
    }

    /// Records this seat's devices.
    pub(crate) fn devices(
        &self,
        standard: Option<(DataDeviceManagerState, DataDevice)>,
        primary: Option<(PrimarySelectionManagerState, PrimarySelectionDevice)>,
    ) {
        let mut inner = self.lock();
        inner.standard = standard.map(|(manager, device)| Standard { manager, device });
        inner.primary = primary.map(|(manager, device)| Primary { manager, device });
    }

    /// Records the serial a claim on a selection may quote.
    ///
    /// Any input serial, not only a press. A pop-up grab and an interactive drag are declined
    /// against anything but a press, but a selection is not: the protocol asks for the serial of
    /// the event that caused the request, and a copy caused by the window merely being focused —
    /// a menu command, a shortcut on a freshly focused window — has only that one to give. Being
    /// stricter here means a copy that silently does nothing until the user has clicked.
    pub(crate) fn observed(&self, serial: u32) {
        self.lock().serial = Some(serial);
    }

    /// What this application has put on `kind`, when it is the owner.
    pub fn owned(
        &self,
        kind: ClipboardKind,
        format: ClipboardFormat,
    ) -> Option<Result<ClipboardData, ClipboardError>> {
        self.lock().held[index(kind)].as_ref().map(|held| {
            if held.data.format() == format {
                Ok(held.data.clone())
            } else {
                Err(ClipboardError::UnavailableFormat(format))
            }
        })
    }

    /// Gives up ownership of `kind`, because the compositor gave it to somebody else.
    pub(crate) fn lost(&self, source: &WlDataSource) {
        let mut inner = self.lock();
        for held in &mut inner.held {
            if matches!(held.as_ref().map(|held| &held.source), Some(Source::Standard(owned)) if owned.inner() == source)
            {
                *held = None;
            }
        }
    }

    /// The same, for the selection clipboard.
    pub(crate) fn lost_primary(&self, source: &ZwpPrimarySelectionSourceV1) {
        let mut inner = self.lock();
        for held in &mut inner.held {
            if matches!(held.as_ref().map(|held| &held.source), Some(Source::Primary(owned)) if owned.inner() == source)
            {
                *held = None;
            }
        }
    }

    /// Serves a request for what this application put on a clipboard.
    ///
    /// On a thread, because the destination is another process and a destination that stops
    /// reading would otherwise hold the loop for as long as it liked.
    pub(crate) fn serve(&self, source: &WlDataSource, requested_mime: &str, destination: WritePipe) {
        let payload = {
            let inner = self.lock();
            if let Some(drag) = inner
                .outbound_drag
                .as_ref()
                .filter(|drag| drag.source.inner() == source)
            {
                drag.items
                    .iter()
                    .find(|(m, _)| m.eq_ignore_ascii_case(requested_mime))
                    .map(|(_, bytes)| bytes.clone())
            } else {
                inner.held.iter().flatten().find_map(|held| {
                    matches!(&held.source, Source::Standard(owned) if owned.inner() == source)
                        .then(|| payload_for_mime(&held.data, held.secret, requested_mime))
                        .flatten()
                })
            }
        };
        serve_bytes(payload, destination);
    }

    /// The same, for the selection clipboard.
    pub(crate) fn serve_primary(
        &self,
        source: &ZwpPrimarySelectionSourceV1,
        requested_mime: &str,
        destination: WritePipe,
    ) {
        let payload = self.lock().held.iter().flatten().find_map(|held| {
            matches!(&held.source, Source::Primary(owned) if owned.inner() == source)
                .then(|| payload_for_mime(&held.data, held.secret, requested_mime))
                .flatten()
        });
        serve_bytes(payload, destination);
    }

    /// Asks the owner of `kind` for `format`, answering with the pipe it will write into.
    pub(crate) fn start_read(
        &self,
        kind: ClipboardKind,
        format: ClipboardFormat,
    ) -> Result<Pending, ClipboardError> {
        let inner = self.lock();
        let select_mime = |offered: &[String]| match format {
            ClipboardFormat::Text => mime::best_text(offered),
            ClipboardFormat::Image => mime::best_image(offered),
            _ => None,
        };
        let reader = match kind {
            ClipboardKind::Primary => {
                let primary = inner
                    .primary
                    .as_ref()
                    .ok_or(ClipboardError::UnsupportedKind(kind))?;
                let offer = primary
                    .device
                    .data()
                    .selection_offer()
                    .ok_or(ClipboardError::Empty(kind))?;
                let wanted = offer
                    .with_mime_types(select_mime)
                    .ok_or(ClipboardError::UnavailableFormat(format))?;
                offer
                    .receive(wanted)
                    .map_err(|error| ClipboardError::Backend(error.to_string()))?
            }
            _ => {
                let standard = inner.standard.as_ref().ok_or(ClipboardError::Empty(kind))?;
                let offer = standard
                    .device
                    .data()
                    .selection_offer()
                    .ok_or(ClipboardError::Empty(kind))?;
                let wanted = offer
                    .with_mime_types(select_mime)
                    .ok_or(ClipboardError::UnavailableFormat(format))?;
                offer
                    .receive(wanted)
                    .map_err(|error| ClipboardError::Backend(error.to_string()))?
            }
        };
        // The request has to reach the compositor before anything waits on the pipe, and the loop
        // is not going to flush until it next parks — which is after this returns.
        if let Some(conn) = &inner.conn {
            let _ = conn.flush();
        }
        Ok(Pending {
            reader,
            kind,
            format,
        })
    }

    /// Claims `kind` for this application, serving `data`, or gives it up when there is none.
    pub(crate) fn take_selection(
        &self,
        kind: ClipboardKind,
        data: Option<ClipboardData>,
        options: ClipboardWriteOptions,
    ) -> Result<(), ClipboardError> {
        let mut inner = self.lock();
        let serial = inner.serial.ok_or_else(|| {
            // A compositor grants a selection only against a serial from something the user did.
            // Before the first press there is nothing to quote, and claiming without one is
            // declined silently — so it is reported rather than appearing to work.
            ClipboardError::Backend(
                "a selection can only be claimed after the user has pressed something".to_owned(),
            )
        })?;
        let qh = inner
            .qh
            .clone()
            .ok_or_else(|| ClipboardError::Backend("the loop has not started".to_owned()))?;

        let secret = options.exclude_from_history;
        let held = match (kind, data) {
            (_, None) => {
                match kind {
                    ClipboardKind::Primary => {
                        if let Some(primary) = &inner.primary {
                            primary.device.unset_selection(serial);
                        }
                    }
                    _ => {
                        if let Some(standard) = &inner.standard {
                            standard.device.unset_selection(serial);
                        }
                    }
                }
                None
            }
            (ClipboardKind::Primary, Some(data)) => {
                let primary = inner
                    .primary
                    .as_ref()
                    .ok_or(ClipboardError::UnsupportedKind(kind))?;
                let offered = offered_mimes(&data, secret);
                let source = primary.manager.create_selection_source(&qh, offered);
                source.set_selection(&primary.device, serial);
                Some(Held {
                    data,
                    secret,
                    source: Source::Primary(source),
                })
            }
            (_, Some(data)) => {
                let standard = inner.standard.as_ref().ok_or_else(|| {
                    ClipboardError::Backend("this compositor offers no clipboard".to_owned())
                })?;
                let offered = offered_mimes(&data, secret);
                let source = standard.manager.create_copy_paste_source(&qh, offered);
                source.set_selection(&standard.device, serial);
                Some(Held {
                    data,
                    secret,
                    source: Source::Standard(source),
                })
            }
        };
        // Replaced rather than dropped first: dropping the old source destroys it, and destroying
        // the source that currently owns the selection before the new one has claimed it leaves a
        // gap in which the clipboard is empty.
        inner.held[index(kind)] = held;
        if let Some(conn) = &inner.conn {
            let _ = conn.flush();
        }
        Ok(())
    }

    /// Delivers an answer to whoever asked for it.
    pub(crate) fn answer(
        &self,
        serial: ClipboardSerial,
        result: Result<ClipboardData, ClipboardError>,
    ) {
        let waker = self.lock().waker.clone();
        deliver(waker.as_ref(), serial, result);
    }

    /// Starts an outbound `Copy`-only Wayland drag on `origin` using the verified live press `serial`.
    ///
    /// Never advertises `DndAction::Move`.
    pub(crate) fn start_outbound_drag(
        &self,
        surface: SurfaceId,
        origin: &WlSurface,
        serial: u32,
        payload: OutboundDragPayload,
    ) -> Result<(), Unsupported> {
        if payload.items.is_empty() {
            return Err(Unsupported);
        }
        let mut inner = self.lock();
        let qh = inner.qh.clone().ok_or(Unsupported)?;
        let standard = inner.standard.as_ref().ok_or(Unsupported)?;
        let mimes: Vec<&str> = payload.items.iter().map(|(m, _)| m.as_str()).collect();
        // Copy-only semantics: never advertise Move without a destination-commit contract.
        let source = standard
            .manager
            .create_drag_and_drop_source(&qh, mimes, DndAction::Copy);
        source.start_drag(&standard.device, origin, None, serial);
        inner.outbound_drag = Some(ActiveOutboundDrag {
            surface,
            source,
            items: payload.items,
            accepted_mime: None,
            selected_action: DndAction::None,
            dropped: false,
        });
        if let Some(conn) = &inner.conn {
            let _ = conn.flush();
        }
        Ok(())
    }

    /// Cancels an active outbound drag originating from `surface`, destroying its `wl_data_source`.
    pub(crate) fn cancel_outbound_drag(&self, surface: SurfaceId) {
        let mut inner = self.lock();
        if inner
            .outbound_drag
            .as_ref()
            .is_some_and(|drag| drag.surface == surface)
        {
            inner.outbound_drag = None;
            if let Some(conn) = &inner.conn {
                let _ = conn.flush();
            }
        }
    }

    /// Records the target's accepted MIME type for the active outbound drag source.
    pub(crate) fn outbound_drag_accept_mime(&self, source: &WlDataSource, mime: Option<String>) {
        let mut inner = self.lock();
        if let Some(drag) = inner
            .outbound_drag
            .as_mut()
            .filter(|drag| drag.source.inner() == source)
        {
            drag.accepted_mime = mime;
        }
    }

    /// Records the compositor-selected action (`Copy` or `None`) for the active outbound drag.
    pub(crate) fn outbound_drag_action(&self, source: &WlDataSource, action: DndAction) {
        let mut inner = self.lock();
        if let Some(drag) = inner
            .outbound_drag
            .as_mut()
            .filter(|drag| drag.source.inner() == source)
        {
            drag.selected_action = if action.contains(DndAction::Copy) {
                DndAction::Copy
            } else {
                DndAction::None
            };
        }
    }

    /// Marks the active outbound drag as dropped while awaiting `dnd_finished` or `cancelled`.
    pub(crate) fn outbound_drag_dropped(&self, source: &WlDataSource) {
        let mut inner = self.lock();
        if let Some(drag) = inner
            .outbound_drag
            .as_mut()
            .filter(|drag| drag.source.inner() == source)
        {
            drag.dropped = true;
        }
    }

    /// Completes and destroys the active outbound drag on `dnd_finished`, returning its surface.
    pub(crate) fn outbound_drag_finished(&self, source: &WlDataSource) -> Option<SurfaceId> {
        let mut inner = self.lock();
        let matches = inner
            .outbound_drag
            .as_ref()
            .is_some_and(|drag| drag.source.inner() == source);
        if matches {
            let drag = inner.outbound_drag.take()?;
            Some(drag.surface)
        } else {
            None
        }
    }

    /// Cancels and destroys the active outbound drag on `cancelled`, returning its surface.
    pub(crate) fn outbound_drag_cancelled(&self, source: &WlDataSource) -> Option<SurfaceId> {
        self.outbound_drag_finished(source)
    }

    /// The state, recovering from a panic on another thread.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// MIME types advertised when claiming a selection.
fn offered_mimes(data: &ClipboardData, secret: bool) -> Vec<&'static str> {
    let mut mimes = match data {
        ClipboardData::Text(_) => mime::OFFERED.to_vec(),
        ClipboardData::Image { .. } => mime::OFFERED_IMAGE.to_vec(),
        _ => Vec::new(),
    };
    if secret {
        mimes.push(mime::PASSWORD_HINT);
    }
    mimes
}

/// Resolves bytes to write into a peer's pipe for `requested_mime`.
fn payload_for_mime(data: &ClipboardData, secret: bool, requested_mime: &str) -> Option<Vec<u8>> {
    if secret && requested_mime.eq_ignore_ascii_case(mime::PASSWORD_HINT) {
        return Some(b"secret".to_vec());
    }
    match data {
        ClipboardData::Text(text)
            if mime::OFFERED
                .iter()
                .any(|offered| offered.eq_ignore_ascii_case(requested_mime))
                || requested_mime.to_ascii_lowercase().starts_with("text/") =>
        {
            Some(text.as_str().as_bytes().to_vec())
        }
        ClipboardData::Image { bytes, .. } if requested_mime.eq_ignore_ascii_case(mime::PNG) => {
            Some(bytes.to_vec())
        }
        _ => None,
    }
}

/// Writes a held selection into `destination`, on a thread.
fn serve_bytes(bytes: Option<Vec<u8>>, destination: WritePipe) {
    let _ = std::thread::Builder::new()
        .name("zgui-clipboard-write".to_owned())
        .spawn(move || {
            // A destination asking for a selection this application no longer holds gets an empty
            // one: closing the pipe with nothing in it is how the protocol says "nothing here".
            pipe::write(
                destination,
                bytes.as_deref().unwrap_or_default(),
                pipe::PATIENCE,
            );
        });
}

/// Which slot a clipboard's state is kept in.
const fn index(kind: ClipboardKind) -> usize {
    match kind {
        ClipboardKind::Primary => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{finish, index, offered_mimes, payload_for_mime};
    use crate::clipboard::mime;
    use crate::clipboard::pipe::Failed;
    use zgui_platform::{ClipboardData, ClipboardError, ClipboardFormat, ClipboardKind};

    #[test]
    fn the_two_clipboards_are_kept_apart() {
        // Writing the selection clipboard by mistake destroys whatever the user had selected
        // elsewhere, so the two must never share a slot.
        assert_ne!(
            index(ClipboardKind::Standard),
            index(ClipboardKind::Primary)
        );
    }

    #[test]
    fn an_empty_transfer_is_an_empty_clipboard_rather_than_a_failure() {
        assert_eq!(
            finish(ClipboardKind::Standard, ClipboardFormat::Text, Ok(Vec::new())),
            Err(ClipboardError::Empty(ClipboardKind::Standard))
        );
    }

    #[test]
    fn what_arrived_is_what_is_answered_with() {
        assert_eq!(
            finish(
                ClipboardKind::Standard,
                ClipboardFormat::Text,
                Ok(b"copied".to_vec()),
            ),
            Ok(ClipboardData::Text("copied".into()))
        );
    }

    #[test]
    fn png_image_transfers_preserve_bytes_and_media_type() {
        let png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        assert_eq!(
            finish(
                ClipboardKind::Standard,
                ClipboardFormat::Image,
                Ok(png.clone()),
            ),
            Ok(ClipboardData::Image {
                media_type: mime::PNG.into(),
                bytes: png.into(),
            })
        );
    }

    #[test]
    fn secret_selections_advertise_and_serve_password_manager_hint() {
        let png = ClipboardData::Image {
            media_type: mime::PNG.into(),
            bytes: vec![0x89, b'P', b'N', b'G'].into(),
        };
        assert_eq!(offered_mimes(&png, false), vec![mime::PNG]);
        assert_eq!(
            offered_mimes(&png, true),
            vec![mime::PNG, mime::PASSWORD_HINT]
        );
        assert_eq!(
            payload_for_mime(&png, true, mime::PASSWORD_HINT).as_deref(),
            Some(b"secret".as_slice())
        );
        assert_eq!(payload_for_mime(&png, false, mime::PASSWORD_HINT), None);
        assert_eq!(
            payload_for_mime(&png, true, mime::PNG).as_deref(),
            Some([0x89, b'P', b'N', b'G'].as_slice())
        );
        assert_eq!(payload_for_mime(&png, true, "text/plain"), None);
    }

    #[test]
    fn text_that_is_not_valid_is_read_as_far_as_it_goes_rather_than_refused() {
        // A selection whose owner declared UTF-8 and wrote something else is still mostly
        // readable, and refusing the paste entirely helps nobody.
        let answered = finish(
            ClipboardKind::Standard,
            ClipboardFormat::Text,
            Ok(vec![b'o', b'k', 0xff]),
        );
        assert!(matches!(answered, Ok(ClipboardData::Text(_))));
    }

    #[test]
    fn an_owner_that_never_answered_is_reported_as_a_timeout() {
        assert_eq!(
            finish(
                ClipboardKind::Primary,
                ClipboardFormat::Text,
                Err(Failed::TimedOut),
            ),
            Err(ClipboardError::TimedOut)
        );
    }
}
