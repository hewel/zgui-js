//! Content dragged over a window from another application.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use smithay_client_toolkit::data_device_manager::data_offer::DragOffer;
use zgui_geom::{Css, CssPx, Point};
use zgui_platform::{DragEvent, SurfaceId};

/// Bounded capacity for out-of-band pipe completions waiting on the main loop.
const MAX_PENDING_READS: usize = 64;

/// Unique identity for one external drag offer so delayed pipe reads from a
/// previous or cancelled offer can never be attributed to a newer offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OfferKey {
    /// Wayland object protocol ID of the `wl_data_offer`.
    pub offer_id: u32,
    /// Monotonic local session generation incremented on every `enter`.
    pub generation: u64,
}

/// Completed pipe read tagged with the offer that initiated it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadCompletion {
    /// Identity of the offer whose pipe produced `paths`.
    pub key: OfferKey,
    /// Decoded local file paths read from `text/uri-list`.
    pub paths: Vec<PathBuf>,
}

/// Minimal lifecycle hook on a drag offer, abstracted for protocol unit tests.
pub trait OfferLifecycle {
    /// Signals successful drop completion (`wl_data_offer.finish`).
    fn finish_offer(&self);
    /// Releases the underlying offer resource (`wl_data_offer.destroy`).
    fn destroy_offer(&self);
}

impl OfferLifecycle for DragOffer {
    fn finish_offer(&self) {
        self.finish();
    }

    fn destroy_offer(&self) {
        self.destroy();
    }
}

/// The drag that is currently over a window, if there is one.
///
/// Every asynchronous pipe read is tagged with `(offer_id, generation)` and queued
/// in `reads`. Completed reads are matched strictly against the active session's
/// `OfferKey`, preventing cross-offer attribution races when a slow source finishes
/// after the pointer leaves or a successor offer enters.
#[derive(Debug)]
pub struct DragTracker<O: OfferLifecycle> {
    /// Next generation number assigned when a new offer enters.
    next_generation: u64,
    /// The active session, while one is over a window.
    session: Option<Session<O>>,
    /// Tagged pipe completions waiting to be drained on the Wayland loop thread.
    reads: Arc<Mutex<VecDeque<ReadCompletion>>>,
}

impl<O: OfferLifecycle> Default for DragTracker<O> {
    fn default() -> Self {
        Self {
            next_generation: 1,
            session: None,
            reads: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

/// Concrete Wayland external drag tracker backed by `smithay_client_toolkit::DragOffer`.
pub type Drag = DragTracker<DragOffer>;

/// One drag over one window.
#[derive(Debug)]
struct Session<O: OfferLifecycle> {
    /// Unique key binding async pipe reads to this exact offer instance.
    key: OfferKey,
    /// The window it is over.
    surface: SurfaceId,
    /// The offer the content is asked for through.
    offer: O,
    /// Where the pointer is.
    at: Point<CssPx, Css>,
    /// Resolved paths once this offer's own pipe read has completed.
    paths: Option<Vec<PathBuf>>,
    /// Whether an async pipe read has been launched for this offer.
    read_started: bool,
    /// Whether the paths have been reported via `Entered`.
    announced: bool,
    /// Whether the user has let go and the drop is waiting on the read.
    dropped: bool,
}

impl<O: OfferLifecycle> DragTracker<O> {
    /// Records a drag arriving over `surface` for `offer_id`.
    ///
    /// If a previous session was still active, it is retired and destroyed; if it
    /// had already been announced, a synthetic `Left` event for the previous
    /// surface is returned so targets can clear hover state cleanly.
    pub fn entered(
        &mut self,
        surface: SurfaceId,
        offer_id: u32,
        offer: O,
        at: Point<CssPx, Css>,
    ) -> Option<(SurfaceId, DragEvent)> {
        let displaced = self.session.take().and_then(|old| {
            old.offer.destroy_offer();
            old.announced.then_some((old.surface, DragEvent::Left))
        });
        let generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        self.session = Some(Session {
            key: OfferKey {
                offer_id,
                generation,
            },
            surface,
            offer,
            at,
            paths: None,
            read_started: false,
            announced: false,
            dropped: false,
        });
        displaced
    }

    /// Returns the active offer and its `OfferKey` if no pipe read has been started yet.
    pub fn to_read(&mut self) -> Option<(OfferKey, &O)> {
        let session = self.session.as_mut()?;
        if session.read_started || session.announced {
            return None;
        }
        session.read_started = true;
        Some((session.key, &session.offer))
    }

    /// Shared queue where pipe reader threads deposit tagged `ReadCompletion`s.
    pub fn answers(&self) -> Arc<Mutex<VecDeque<ReadCompletion>>> {
        Arc::clone(&self.reads)
    }

    /// Enqueues one completed pipe read while keeping the queue bounded.
    pub fn push_completion(
        reads: &Mutex<VecDeque<ReadCompletion>>,
        completion: ReadCompletion,
    ) {
        let mut queue = reads.lock().unwrap_or_else(|held| held.into_inner());
        if queue.len() >= MAX_PENDING_READS {
            queue.pop_front();
        }
        queue.push_back(completion);
    }

    /// Records the pointer moving.
    pub fn moved(&mut self, at: Point<CssPx, Css>) -> Option<(SurfaceId, DragEvent)> {
        let session = self.session.as_mut()?;
        session.at = at;
        session
            .announced
            .then_some((session.surface, DragEvent::Moved { position: at }))
    }

    /// Records the drag leaving or being cancelled by the source without a drop.
    ///
    /// A dropped drag also leaves. Its session stays until its read finishes.
    pub fn left(&mut self) -> Option<(SurfaceId, DragEvent)> {
        // A drop is followed by wl_data_device.leave. The read of a dropped session may still
        // run, so keep the session.
        if self.session.as_ref().is_some_and(|session| session.dropped) {
            return None;
        }
        let session = self.session.take()?;
        session.offer.destroy_offer();
        session
            .announced
            .then_some((session.surface, DragEvent::Left))
    }

    /// Records the user letting go.
    ///
    /// Answers with the drop when this offer's own paths are already known, and with
    /// nothing when its read is still in flight — in which case `read_finished`
    /// produces the drop once the matching `(offer_id, generation)` arrives.
    pub fn dropped(&mut self) -> Option<(SurfaceId, DragEvent)> {
        let session = self.session.as_mut()?;
        session.dropped = true;
        let paths = session.paths.clone()?;
        Some(self.finish(paths))
    }

    /// Records a tagged pipe read finishing.
    ///
    /// Completions whose `(offer_id, generation)` does not match the active session
    /// are strictly discarded so a slow offer A can never contaminate offer B.
    pub fn read_finished(
        &mut self,
        key: OfferKey,
        paths: Vec<PathBuf>,
    ) -> Option<(SurfaceId, DragEvent)> {
        let session = self.session.as_mut()?;
        if session.key != key {
            return None;
        }
        session.paths = Some(paths.clone());
        if session.dropped {
            return Some(self.finish(paths));
        }
        if session.announced {
            return None;
        }
        session.announced = true;
        Some((
            session.surface,
            DragEvent::Entered {
                paths,
                position: session.at,
            },
        ))
    }

    /// Completes the drop and takes the session down.
    fn finish(&mut self, paths: Vec<PathBuf>) -> (SurfaceId, DragEvent) {
        let session = self
            .session
            .take()
            .expect("a drop is only finished while a session exists");
        // The offer is told the transfer worked and then retired. A drop that is never finished
        // leaves the source waiting, which on some desktops leaves the dragged icon on screen.
        session.offer.finish_offer();
        session.offer.destroy_offer();
        (
            session.surface,
            DragEvent::Dropped {
                paths,
                position: session.at,
            },
        )
    }
}

/// The paths a list of file addresses names.
///
/// The format is one address per line with comment lines, and only `file` addresses are paths at
/// all — a drag of a web address carries something no file system can open, and turning it into a
/// path would produce a name nothing can read rather than nothing. An address with a host other
/// than `localhost` names a file on another machine, so it is dropped too.
///
/// Read as bytes, because a Unix file name is bytes and need not be UTF-8.
pub fn paths(bytes: &[u8]) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    bytes
        .split(|byte| *byte == b'\n')
        .map(<[u8]>::trim_ascii)
        .filter(|line| !line.is_empty() && !line.starts_with(b"#"))
        .filter_map(|line| line.strip_prefix(b"file://"))
        .filter_map(|address| {
            let path = address.strip_prefix(b"localhost").unwrap_or(address);
            if !path.starts_with(b"/") {
                return None;
            }
            let bytes = unescape(path);
            (!bytes.contains(&0)).then(|| PathBuf::from(std::ffi::OsString::from_vec(bytes)))
        })
        .collect()
}

/// One address with its percent escapes resolved.
fn unescape(bytes: &[u8]) -> Vec<u8> {
    let digit = |byte: u8| (byte as char).to_digit(16).map(|value| value as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (digit(bytes[index + 1]), digit(bytes[index + 2]))
        {
            out.push(high * 16 + low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{DragTracker, OfferLifecycle, paths};
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use zgui_geom::{Css, CssPx, Point};
    use zgui_platform::{DragEvent, SurfaceId};

    #[derive(Clone, Debug, Default)]
    struct MockOffer {
        finished: Arc<AtomicUsize>,
        destroyed: Arc<AtomicUsize>,
    }

    impl OfferLifecycle for MockOffer {
        fn finish_offer(&self) {
            self.finished.fetch_add(1, Ordering::Relaxed);
        }

        fn destroy_offer(&self) {
            self.destroyed.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn pt(x: f32, y: f32) -> Point<CssPx, Css> {
        Point::new(CssPx(x), CssPx(y))
    }

    #[test]
    fn a_utf8_name_survives_its_escapes() {
        assert_eq!(
            paths("file:///tmp/flight%20%C3%BC.ulg\nfile://localhost/tmp/日本.ulg\n".as_bytes()),
            vec![
                PathBuf::from("/tmp/flight ü.ulg"),
                PathBuf::from("/tmp/日本.ulg")
            ]
        );
    }

    #[test]
    fn a_name_that_is_not_utf8_keeps_its_bytes() {
        let name = |bytes: &[u8]| PathBuf::from(OsStr::from_bytes(bytes));
        assert_eq!(paths(b"file:///tmp/%FF.ulg"), vec![name(b"/tmp/\xFF.ulg")]);
        assert_eq!(paths(b"file:///tmp/\xFE.ulg"), vec![name(b"/tmp/\xFE.ulg")]);
    }

    #[test]
    fn a_split_character_after_a_percent_is_kept() {
        assert_eq!(
            paths("file:///tmp/%aé.ulg".as_bytes()),
            vec![PathBuf::from("/tmp/%aé.ulg")]
        );
        assert_eq!(
            paths("file:///tmp/%é.ulg".as_bytes()),
            vec![PathBuf::from("/tmp/%é.ulg")]
        );
    }

    #[test]
    fn a_file_on_another_host_is_dropped() {
        assert!(paths(b"file://remote/tmp/log.ulg\nfile://localhostfoo/tmp/log.ulg").is_empty());
    }

    #[test]
    fn a_name_with_an_escaped_nul_is_dropped() {
        assert!(paths(b"file:///tmp/bad%00.ulg").is_empty());
    }

    #[test]
    fn a_list_with_nothing_on_it_names_no_files() {
        assert!(paths(b"").is_empty());
        assert!(paths(b"# a comment and nothing else\r\n").is_empty());
    }

    #[test]
    fn every_address_on_the_list_becomes_a_path() {
        let list = b"file:///home/a/one.png\r\nfile:///home/a/two.png\r\n";
        assert_eq!(
            paths(list),
            vec![
                PathBuf::from("/home/a/one.png"),
                PathBuf::from("/home/a/two.png")
            ]
        );
    }

    #[test]
    fn something_that_is_not_a_file_is_dropped_rather_than_turned_into_a_name() {
        let mixed = b"https://example.invalid/a\r\nfile:///home/a/one.png\r\n";
        assert_eq!(paths(mixed), vec![PathBuf::from("/home/a/one.png")]);
    }

    #[test]
    fn an_escaped_name_comes_back_as_the_name_it_stands_for() {
        let escaped = b"file:///home/a/two%20words%21.txt\r\n";
        assert_eq!(
            paths(escaped),
            vec![PathBuf::from("/home/a/two words!.txt")]
        );
    }

    #[test]
    fn an_escape_that_is_not_one_is_left_alone_rather_than_dropping_the_character() {
        let odd = b"file:///home/a/100%.txt\r\n";
        assert_eq!(paths(odd), vec![PathBuf::from("/home/a/100%.txt")]);
    }

    #[test]
    fn overlapping_offers_never_cross_attribute_delayed_pipe_reads() {
        let mut drag = DragTracker::<MockOffer>::default();
        let surface = SurfaceId::new(1);
        let offer_a = MockOffer::default();
        let offer_b = MockOffer::default();

        // Offer A enters and starts reading asynchronously (has not finished yet).
        assert!(drag.entered(surface, 10, offer_a.clone(), pt(10.0, 20.0)).is_none());
        let (key_a, _) = drag.to_read().expect("offer A read key");
        assert!(drag.to_read().is_none(), "duplicate read must not start for offer A");

        // Before offer A's pipe finishes, offer B enters (same or different offer_id).
        assert!(drag.entered(surface, 11, offer_b.clone(), pt(50.0, 60.0)).is_none());
        assert_eq!(offer_a.destroyed.load(Ordering::Relaxed), 1, "superseded offer A destroyed");
        let (key_b, _) = drag.to_read().expect("offer B read key");
        assert_ne!(key_a, key_b);

        // User drops offer B before offer B's pipe finishes, while stale offer A's read completes!
        assert!(drag.dropped().is_none());
        assert!(
            drag.read_finished(key_a, vec![PathBuf::from("/stale/from-a.txt")])
                .is_none(),
            "delayed read from offer A must be discarded and never attributed to offer B"
        );
        assert_eq!(offer_b.finished.load(Ordering::Relaxed), 0);

        // Offer B's own read completes and produces the drop with offer B's paths.
        let event = drag
            .read_finished(key_b, vec![PathBuf::from("/fresh/from-b.txt")])
            .expect("offer B drop delivered");
        assert_eq!(
            event,
            (
                surface,
                DragEvent::Dropped {
                    paths: vec![PathBuf::from("/fresh/from-b.txt")],
                    position: pt(50.0, 60.0),
                }
            )
        );
        assert_eq!(offer_b.finished.load(Ordering::Relaxed), 1);
        assert_eq!(offer_b.destroyed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn source_cancellation_destroys_offer_and_discards_late_read_even_with_reused_offer_id() {
        let mut drag = DragTracker::<MockOffer>::default();
        let surface = SurfaceId::new(2);
        let offer_1 = MockOffer::default();
        let offer_2 = MockOffer::default();

        // Offer 1 enters, starts reading, then source cancels (leave) before read completes.
        assert!(drag.entered(surface, 42, offer_1.clone(), pt(5.0, 5.0)).is_none());
        let (key_1, _) = drag.to_read().expect("offer 1 key");
        assert!(drag.left().is_none(), "unannounced offer emits no Left");
        assert_eq!(offer_1.destroyed.load(Ordering::Relaxed), 1, "cancelled offer destroyed");
        assert!(
            drag.read_finished(key_1, vec![PathBuf::from("/cancelled.txt")])
                .is_none(),
            "late read after source cancellation must be ignored"
        );

        // Compositor reuses protocol ID 42 for a new offer 2: generation distinguishes them.
        assert!(drag.entered(surface, 42, offer_2.clone(), pt(30.0, 40.0)).is_none());
        let (key_2, _) = drag.to_read().expect("offer 2 key");
        assert_ne!(key_1.generation, key_2.generation);
        assert!(
            drag.read_finished(key_1, vec![PathBuf::from("/ghost.txt")])
                .is_none(),
            "stale generation with same protocol ID must be rejected"
        );
        let entered = drag
            .read_finished(key_2, vec![PathBuf::from("/real.txt")])
            .expect("offer 2 entered");
        assert_eq!(
            entered,
            (
                surface,
                DragEvent::Entered {
                    paths: vec![PathBuf::from("/real.txt")],
                    position: pt(30.0, 40.0),
                }
            )
        );
        // Announced offer emits Left on source cancellation.
        assert_eq!(drag.left(), Some((surface, DragEvent::Left)));
        assert_eq!(offer_2.destroyed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn drop_then_leave_preserves_session_until_late_pipe_read_finishes() {
        let mut drag = DragTracker::<MockOffer>::default();
        let surface = SurfaceId::new(3);
        let offer = MockOffer::default();

        assert!(drag.entered(surface, 99, offer.clone(), pt(12.0, 34.0)).is_none());
        let (key, _) = drag.to_read().expect("start async pipe read");

        // User drops before pipe read completes, then compositor sends wl_data_device.leave.
        assert!(drag.dropped().is_none(), "drop waits for async pipe read");
        assert!(
            drag.left().is_none(),
            "leave after drop must keep session alive without emitting Left"
        );
        assert_eq!(offer.destroyed.load(Ordering::Relaxed), 0);

        // Pipe read finishes late and delivers Dropped, finishing and destroying the offer.
        let event = drag
            .read_finished(key, vec![PathBuf::from("/late/drop.png")])
            .expect("late read completes dropped offer");
        assert_eq!(
            event,
            (
                surface,
                DragEvent::Dropped {
                    paths: vec![PathBuf::from("/late/drop.png")],
                    position: pt(12.0, 34.0),
                }
            )
        );
        assert_eq!(offer.finished.load(Ordering::Relaxed), 1);
        assert_eq!(offer.destroyed.load(Ordering::Relaxed), 1);
    }
}
