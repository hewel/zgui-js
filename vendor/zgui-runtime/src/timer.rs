//! Scheduled callbacks, and the deadline they give the loop.
//!
//! There is otherwise no way to ask for something to happen at a future time, and a great many
//! ordinary controls need one: a tooltip's open delay, a toast's auto-dismiss, a carousel's
//! autoplay, a search field's debounce. None of them can be written with CSS, because the content
//! they animate is portalled out of the trigger's subtree, and none of them may bring their own
//! timer thread, because a single-threaded loop that already computes when to wake up gets this
//! for nothing.
//!
//! Three properties follow, and each is load-bearing.
//!
//! * **The clock is the platform's**, never the system's, so a seven-hundred-millisecond delay is
//!   exercisable in a microsecond and nothing is wall-clock flaky.
//! * **Entries fire at the start of a frame, before its reactive work**, so what a callback writes
//!   settles in the same frame it fired in: one wake, one frame.
//! * **A pending entry marks no invalidation on any node.** It is a deadline the loop owes, not
//!   work the document owes, so a callback that writes nothing costs a frame that skips every
//!   stage from the restyle onwards.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::rc::Rc;
use web_time::{Duration, Instant};

use zgui_view::DocumentId;
use zgui_view::host::{Repeat, TimerId};

/// One scheduled callback.
struct Entry {
    /// When it should run.
    deadline: Instant,
    /// Which callback this is.
    id: TimerId,
    /// The window it belongs to, so a deadline reached wakes the right surface.
    document: DocumentId,
    /// Whether it repeats.
    repeat: Repeat,
    /// How long it was scheduled for, which is also how long each repeat waits.
    period: Duration,
    /// What to run.
    callback: Rc<dyn Fn()>,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        (self.deadline, self.id) == (other.deadline, other.id)
    }
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        // By deadline, then by identity, so two entries due at the same instant fire in the order
        // they were registered rather than in whatever order the heap happens to hold them.
        self.deadline
            .cmp(&other.deadline)
            .then_with(|| self.id.cmp(&other.id))
    }
}

/// The scheduled callbacks of every window, ordered by when they are due.
///
/// One heap rather than one per window, because the loop parks on a single deadline and each entry
/// carries the window it belongs to — which is what lets a reached deadline ask exactly the right
/// surface to draw.
#[derive(Default)]
pub struct Timers {
    /// The entries, earliest first.
    entries: BinaryHeap<Reverse<Entry>>,
    /// The next identity, never reused.
    next: u64,
}

impl Timers {
    /// A heap with nothing scheduled.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many entries are scheduled and not cancelled.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is scheduled.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Schedules `callback` to run `after` from `now`, in the window `document` names.
    pub fn schedule(
        &mut self,
        document: DocumentId,
        now: Instant,
        after: Duration,
        repeat: Repeat,
        callback: Rc<dyn Fn()>,
    ) -> TimerId {
        self.schedule_at(document, now + after, after, repeat, callback)
    }

    pub(crate) fn schedule_at(
        &mut self,
        document: DocumentId,
        deadline: Instant,
        after: Duration,
        repeat: Repeat,
        callback: Rc<dyn Fn()>,
    ) -> TimerId {
        self.next += 1;
        let id = TimerId::new(self.next);
        self.entries.push(Reverse(Entry {
            deadline,
            id,
            document,
            repeat,
            period: after,
            callback,
        }));
        id
    }

    /// Cancels a scheduled callback.
    ///
    /// Cancelling one that already fired, or one that was already cancelled, does nothing.
    pub fn cancel(&mut self, id: TimerId) {
        // Reclaim the closure now, not when its original deadline arrives.
        // Otherwise schedule/cancel loops bypass callers' live-timer limits.
        self.entries.retain(|entry| entry.0.id != id);
    }

    /// When the earliest live entry is due, and which window it belongs to.
    pub fn peek(&mut self) -> Option<(Instant, DocumentId)> {
        self.entries.peek().map(|entry| (entry.0.deadline, entry.0.document))
    }

    /// The earliest deadline for one window, if it has one.
    pub fn peek_for(&self, document: DocumentId, now: Instant) -> Option<Instant> {
        let _ = now;
        self.entries
            .iter()
            .filter(|entry| entry.0.document == document)
            .map(|entry| entry.0.deadline)
            .min()
    }

    /// Takes every entry of one window due at or before `now`, in deadline order, re-arming the
    /// repeating ones.
    ///
    /// The callbacks come back rather than being run here, because running one re-enters the code
    /// that schedules them and a heap borrowed across that call is a heap borrowed across
    /// arbitrary work.
    ///
    /// Scoped to `document` because a callback runs inside the frame that took it, under that
    /// window's reactive scope and against that window's document. One heap serves every window,
    /// so a drain that ignored which window an entry belongs to would run one window's callbacks
    /// inside whichever window happened to frame first. An entry belonging to another window is
    /// left scheduled: its own deadline is what parks the loop, and its own frame is what runs it.
    pub fn due(&mut self, document: DocumentId, now: Instant) -> Vec<Rc<dyn Fn()>> {
        let mut due = Vec::new();
        let mut rearm = Vec::new();
        let mut others = Vec::new();
        while let Some(Reverse(entry)) = self.entries.peek() {
            if entry.deadline > now {
                break;
            }
            let Some(Reverse(entry)) = self.entries.pop() else {
                break;
            };
            if entry.document != document {
                others.push(Reverse(entry));
                continue;
            }
            due.push(Rc::clone(&entry.callback));
            if entry.repeat.is_repeating() {
                // From `now` rather than from the missed deadline: a loop that was blocked for a
                // second must not then fire a sixteen-millisecond interval sixty times to catch up.
                rearm.push(Entry {
                    deadline: now + entry.period,
                    ..entry
                });
            }
        }
        for entry in others {
            self.entries.push(entry);
        }
        for entry in rearm {
            self.entries.push(Reverse(entry));
        }
        due
    }

    /// Forgets every entry belonging to one window, which closing it does.
    pub fn forget(&mut self, document: DocumentId) {
        let kept: Vec<Reverse<Entry>> = core::mem::take(&mut self.entries)
            .into_iter()
            .filter(|entry| entry.0.document != document)
            .collect();
        self.entries = kept.into_iter().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::Timers;
    use std::cell::Cell;
    use std::rc::Rc;
    use web_time::{Duration, Instant};
    use zgui_view::DocumentId;
    use zgui_view::host::Repeat;

    /// A callback that counts how many times it ran, and the counter behind it.
    fn counting() -> (Rc<dyn Fn()>, Rc<Cell<u32>>) {
        let count = Rc::new(Cell::new(0));
        let held = Rc::clone(&count);
        (Rc::new(move || held.set(held.get() + 1)), count)
    }

    #[test]
    fn nothing_scheduled_means_no_deadline_at_all() {
        let mut timers = Timers::new();
        assert!(timers.peek().is_none());
        assert!(timers.is_empty());
        assert!(timers.due(DocumentId::FIRST, Instant::now()).is_empty());
    }

    #[test]
    fn an_entry_fires_once_when_its_deadline_has_passed_and_not_before() {
        let mut timers = Timers::new();
        let now = Instant::now();
        let (callback, count) = counting();
        timers.schedule(
            DocumentId::FIRST,
            now,
            Duration::from_millis(700),
            Repeat::Once,
            callback,
        );

        assert_eq!(
            timers.peek(),
            Some((now + Duration::from_millis(700), DocumentId::FIRST))
        );
        for due in timers.due(DocumentId::FIRST, now + Duration::from_millis(699)) {
            due();
        }
        assert_eq!(count.get(), 0);

        for due in timers.due(DocumentId::FIRST, now + Duration::from_millis(700)) {
            due();
        }
        assert_eq!(count.get(), 1);
        assert!(
            timers.peek().is_none(),
            "a one-shot entry leaves no deadline behind"
        );

        for due in timers.due(DocumentId::FIRST, now + Duration::from_secs(10)) {
            due();
        }
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn a_repeating_entry_re_arms_from_now_rather_than_from_the_deadline_it_missed() {
        let mut timers = Timers::new();
        let now = Instant::now();
        let (callback, count) = counting();
        timers.schedule(
            DocumentId::FIRST,
            now,
            Duration::from_millis(16),
            Repeat::Every,
            callback,
        );

        // The loop was blocked for a whole second. One run, not sixty.
        for due in timers.due(DocumentId::FIRST, now + Duration::from_secs(1)) {
            due();
        }
        assert_eq!(count.get(), 1);
        assert_eq!(
            timers.peek().map(|(at, _)| at),
            Some(now + Duration::from_secs(1) + Duration::from_millis(16))
        );
    }

    #[test]
    fn a_cancelled_entry_never_runs_and_leaves_no_deadline() {
        let mut timers = Timers::new();
        let now = Instant::now();
        let (callback, count) = counting();
        let id = timers.schedule(
            DocumentId::FIRST,
            now,
            Duration::from_millis(700),
            Repeat::Once,
            callback,
        );
        timers.cancel(id);

        assert!(timers.peek().is_none());
        for due in timers.due(DocumentId::FIRST, now + Duration::from_secs(1)) {
            due();
        }
        assert_eq!(count.get(), 0);
        // Cancelling twice is not an error, and neither is cancelling after the fact.
        timers.cancel(id);
    }

    #[test]
    fn entries_due_at_the_same_moment_fire_in_the_order_they_were_registered() {
        let mut timers = Timers::new();
        let now = Instant::now();
        let order = Rc::new(std::cell::RefCell::new(Vec::new()));
        for name in ["first", "second", "third"] {
            let order = Rc::clone(&order);
            timers.schedule(
                DocumentId::FIRST,
                now,
                Duration::from_millis(10),
                Repeat::Once,
                Rc::new(move || order.borrow_mut().push(name)),
            );
        }
        for due in timers.due(DocumentId::FIRST, now + Duration::from_millis(10)) {
            due();
        }
        assert_eq!(*order.borrow(), ["first", "second", "third"]);
    }

    #[test]
    fn a_deadline_belongs_to_the_window_that_scheduled_it() {
        let mut timers = Timers::new();
        let now = Instant::now();
        let second = DocumentId::new(2).expect("in range");
        let (callback, _) = counting();
        timers.schedule(
            second,
            now,
            Duration::from_millis(50),
            Repeat::Once,
            callback,
        );

        assert_eq!(timers.peek().map(|(_, document)| document), Some(second));
        assert!(timers.peek_for(DocumentId::FIRST, now).is_none());
        assert_eq!(
            timers.peek_for(second, now),
            Some(now + Duration::from_millis(50))
        );

        timers.forget(second);
        assert!(timers.peek().is_none());
    }

    #[test]
    fn a_frame_runs_only_its_own_windows_callbacks() {
        let mut timers = Timers::new();
        let now = Instant::now();
        let second = DocumentId::new(2).expect("in range");
        let (first_callback, first_count) = counting();
        let (second_callback, second_count) = counting();
        timers.schedule(
            DocumentId::FIRST,
            now,
            Duration::from_millis(10),
            Repeat::Once,
            first_callback,
        );
        timers.schedule(
            second,
            now,
            Duration::from_millis(10),
            Repeat::Once,
            second_callback,
        );

        // One window's frame, with both entries long overdue.
        for due in timers.due(DocumentId::FIRST, now + Duration::from_secs(1)) {
            due();
        }
        assert_eq!(first_count.get(), 1);
        assert_eq!(
            second_count.get(),
            0,
            "another window's callback must not run inside this window's frame"
        );
        assert_eq!(
            timers.peek_for(second, now),
            Some(now + Duration::from_millis(10)),
            "and it stays scheduled, so its own window still parks on it"
        );

        // The other window's frame, whenever it comes.
        for due in timers.due(second, now + Duration::from_secs(1)) {
            due();
        }
        assert_eq!(second_count.get(), 1);
        assert!(timers.peek().is_none());
    }

    #[test]
    fn cancelling_long_deadlines_releases_callbacks_without_deadline_polling() {
        let mut timers = Timers::new();
        let now = Instant::now();
        let (live_callback, live_count) = counting();
        timers.schedule(DocumentId::FIRST, now, Duration::from_secs(1), Repeat::Once, live_callback);
        for _ in 0..4096 {
            let held = Rc::new(());
            let weak = Rc::downgrade(&held);
            let callback = Rc::new(move || { let _ = &held; });
            let id = timers.schedule(DocumentId::FIRST, now, Duration::from_secs(u32::MAX as u64), Repeat::Once, callback);
            timers.cancel(id);
            timers.cancel(id);
            assert!(weak.upgrade().is_none(), "cancelled callbacks must not retain resources until their deadline");
        }
        assert_eq!(timers.len(), 1);
        for callback in timers.due(DocumentId::FIRST, now + Duration::from_secs(1)) { callback(); }
        assert_eq!(live_count.get(), 1);
        assert!(timers.is_empty());
    }

    #[test]
    fn cancelling_rearmed_repeat_keeps_only_the_already_selected_callback() {
        let mut timers = Timers::new();
        let now = Instant::now();
        let (callback, count) = counting();
        let id = timers.schedule(DocumentId::FIRST, now, Duration::from_secs(1), Repeat::Every, callback);
        let selected = timers.due(DocumentId::FIRST, now + Duration::from_secs(1));
        timers.cancel(id);
        timers.cancel(id);
        for callback in selected { callback(); }
        assert_eq!(count.get(), 1);
        assert!(timers.peek_for(DocumentId::FIRST, now).is_none());
        assert!(timers.due(DocumentId::FIRST, now + Duration::from_secs(10)).is_empty());
    }
}
