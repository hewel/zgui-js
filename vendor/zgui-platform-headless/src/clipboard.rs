//! A clipboard that is an ordinary value in memory.

use std::sync::{Arc, Mutex};

use zgui_platform::{
    Clipboard, ClipboardData, ClipboardError, ClipboardFormat, ClipboardKind, ClipboardSerial,
    ClipboardSerials, ClipboardWriteOptions, WakeReason, Waker,
};

/// A clipboard that is an ordinary value in memory.
///
/// The standard clipboard and the selection are separate slots. A backend that shared one would
/// pass every test written against copy and paste while destroying the user's selection on every
/// copy, which is a fault nothing but a person notices.
///
/// ```
/// use zgui_platform::{Clipboard, ClipboardData, ClipboardKind, ClipboardWriteOptions};
/// use zgui_platform_headless::MemoryClipboard;
///
/// let clipboard = MemoryClipboard::default();
/// clipboard
///     .write(
///         ClipboardKind::Standard,
///         ClipboardData::from("copied"),
///         ClipboardWriteOptions::default(),
///     )
///     .expect("an in-memory clipboard always accepts text");
///
/// assert!(clipboard.read_blocking(ClipboardKind::Primary, zgui_platform::ClipboardFormat::Text).is_err());
/// ```
#[derive(Default)]
pub struct MemoryClipboard {
    /// Identifiers handed to reads that are answered through the loop.
    serials: ClipboardSerials,
    /// What was last copied.
    standard: Mutex<Option<Slot>>,
    /// What was last selected.
    primary: Mutex<Option<Slot>>,
    /// How a started read delivers its answer.
    waker: Mutex<Option<Arc<dyn Waker>>>,
}

#[derive(Clone, Debug)]
struct Slot {
    data: ClipboardData,
    options: ClipboardWriteOptions,
}

// By hand: a waker is not `Debug`, and the two slots are what a test wants to see anyway.
impl core::fmt::Debug for MemoryClipboard {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MemoryClipboard")
            .field("standard", &self.standard)
            .field("primary", &self.primary)
            .finish_non_exhaustive()
    }
}

impl MemoryClipboard {
    /// Names how a read started through the loop delivers its answer.
    ///
    /// Without one a read takes an identity and is never answered, which is what a real backend
    /// does when it is asked before the loop can hear the reply.
    pub fn attach_waker(&self, waker: Arc<dyn Waker>) {
        *self.waker.lock().expect("the clipboard is not poisoned") = Some(waker);
    }

    /// Returns the last data written to `kind`, if any.
    pub fn snapshot(&self, kind: ClipboardKind) -> Option<ClipboardData> {
        self.slot(kind)
            .lock()
            .expect("the clipboard is not poisoned")
            .as_ref()
            .map(|slot| slot.data.clone())
    }

    /// Returns the write options recorded on the last write to `kind`, if any.
    pub fn last_write_options(&self, kind: ClipboardKind) -> Option<ClipboardWriteOptions> {
        self.slot(kind)
            .lock()
            .expect("the clipboard is not poisoned")
            .as_ref()
            .map(|slot| slot.options)
    }

    /// Which slot a clipboard kind is held in.
    fn slot(&self, kind: ClipboardKind) -> &Mutex<Option<Slot>> {
        match kind {
            ClipboardKind::Primary => &self.primary,
            _ => &self.standard,
        }
    }
}

impl Clipboard for MemoryClipboard {
    fn read(&self, kind: ClipboardKind, format: ClipboardFormat) -> ClipboardSerial {
        let serial = self.serials.take();
        let result = self.read_blocking(kind, format);
        let waker = self
            .waker
            .lock()
            .expect("the clipboard is not poisoned")
            .clone();
        // With no waker the answer is dropped, which is what a real backend does when a read is
        // started before the loop can hear the reply.
        if let Some(waker) = waker {
            waker.wake(WakeReason::ClipboardRead { serial, result });
        }
        serial
    }

    fn read_blocking(
        &self,
        kind: ClipboardKind,
        format: ClipboardFormat,
    ) -> Result<ClipboardData, ClipboardError> {
        let held = self
            .slot(kind)
            .lock()
            .expect("the clipboard is not poisoned")
            .as_ref()
            .map(|slot| slot.data.clone())
            .ok_or(ClipboardError::Empty(kind))?;
        if held.format() == format {
            Ok(held)
        } else {
            Err(ClipboardError::UnavailableFormat(format))
        }
    }

    fn write(
        &self,
        kind: ClipboardKind,
        data: ClipboardData,
        options: ClipboardWriteOptions,
    ) -> Result<(), ClipboardError> {
        *self
            .slot(kind)
            .lock()
            .expect("the clipboard is not poisoned") = Some(Slot { data, options });
        Ok(())
    }

    fn clear(&self, kind: ClipboardKind) -> Result<(), ClipboardError> {
        *self
            .slot(kind)
            .lock()
            .expect("the clipboard is not poisoned") = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryClipboard;
    use zgui_platform::{
        Clipboard, ClipboardData, ClipboardFormat, ClipboardKind, ClipboardWriteOptions,
    };

    #[test]
    fn copying_does_not_overwrite_the_selection() {
        let clipboard = MemoryClipboard::default();
        clipboard
            .write(
                ClipboardKind::Primary,
                ClipboardData::from("selected"),
                ClipboardWriteOptions::default(),
            )
            .expect("accepted");
        clipboard
            .write(
                ClipboardKind::Standard,
                ClipboardData::from("copied"),
                ClipboardWriteOptions::default(),
            )
            .expect("accepted");

        let selected = clipboard
            .read_blocking(ClipboardKind::Primary, ClipboardFormat::Text)
            .expect("still there");
        assert_eq!(selected.as_text(), Some("selected"));
    }

    #[test]
    fn a_read_started_through_the_loop_is_answered_by_a_wake_naming_it() {
        use std::sync::Arc;
        use zgui_platform::{WakeReason, Waker};

        let waker = Arc::new(crate::waker::RecordingWaker::default());
        let clipboard = MemoryClipboard::default();
        clipboard.attach_waker(Arc::clone(&waker) as Arc<dyn Waker>);
        clipboard
            .write(
                ClipboardKind::Standard,
                ClipboardData::from("held"),
                ClipboardWriteOptions::default(),
            )
            .expect("accepted");

        let serial = clipboard.read(ClipboardKind::Standard, ClipboardFormat::Text);
        let woken = waker.drain();
        assert_eq!(woken.len(), 1, "one read, one answer");
        match &woken[0] {
            WakeReason::ClipboardRead {
                serial: named,
                result,
            } => {
                assert_eq!(*named, serial, "the answer names the read that started it");
                assert_eq!(
                    result.as_ref().ok().and_then(|data| data.as_text()),
                    Some("held")
                );
            }
            other => panic!("a read is answered by a clipboard wake, not {other:?}"),
        }
    }

    #[test]
    fn a_read_of_an_empty_clipboard_is_answered_with_the_reason_it_failed() {
        use std::sync::Arc;
        use zgui_platform::{WakeReason, Waker};

        let waker = Arc::new(crate::waker::RecordingWaker::default());
        let clipboard = MemoryClipboard::default();
        clipboard.attach_waker(Arc::clone(&waker) as Arc<dyn Waker>);

        clipboard.read(ClipboardKind::Standard, ClipboardFormat::Text);
        let woken = waker.drain();
        assert_eq!(woken.len(), 1);
        match &woken[0] {
            WakeReason::ClipboardRead { result, .. } => assert!(
                matches!(result, Err(zgui_platform::ClipboardError::Empty(_))),
                "an empty clipboard answers, and says it was empty"
            ),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_read_with_no_loop_to_answer_it_takes_an_identity_and_nothing_else() {
        let clipboard = MemoryClipboard::default();
        // No waker attached: the read is started and its answer is dropped.
        let first = clipboard.read(ClipboardKind::Standard, ClipboardFormat::Text);
        let second = clipboard.read(ClipboardKind::Standard, ClipboardFormat::Text);
        assert_ne!(first, second);
    }

    #[test]
    fn every_read_started_through_the_loop_gets_its_own_identity() {
        let clipboard = MemoryClipboard::default();
        let first = clipboard.read(ClipboardKind::Standard, ClipboardFormat::Text);
        let second = clipboard.read(ClipboardKind::Standard, ClipboardFormat::Text);
        assert_ne!(first, second);
    }

    #[test]
    fn records_write_options_and_supports_image_roundtrip() {
        let clipboard = MemoryClipboard::default();
        let png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        clipboard
            .write(
                ClipboardKind::Standard,
                ClipboardData::Image {
                    media_type: "image/png".into(),
                    bytes: png.clone().into(),
                },
                ClipboardWriteOptions::SECRET,
            )
            .expect("accepted");

        assert_eq!(
            clipboard.last_write_options(ClipboardKind::Standard),
            Some(ClipboardWriteOptions::SECRET)
        );
        let read = clipboard
            .read_blocking(ClipboardKind::Standard, ClipboardFormat::Image)
            .expect("image present");
        assert_eq!(
            read,
            ClipboardData::Image {
                media_type: "image/png".into(),
                bytes: png.into(),
            }
        );
        assert_eq!(
            clipboard.read_blocking(ClipboardKind::Standard, ClipboardFormat::Text),
            Err(zgui_platform::ClipboardError::UnavailableFormat(
                ClipboardFormat::Text
            ))
        );
    }
}
