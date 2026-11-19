//! Application deadlines that do not depend on a window or frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zgui_platform::{Clock, PlatformCx, WakeReason, Waker};
use zgui_view::DocumentId;
use zgui_view::host::{Repeat, TimerId};

use crate::timer::Timers;

/// Application-scoped one-shot timers, independent of the lifetime of native surfaces.
#[derive(Clone, Default)]
pub struct AppTimers {
    inner: Rc<RefCell<AppTimerState>>,
}

#[derive(Default)]
struct AppTimerState {
    timers: Timers,
    platform: Option<(Arc<dyn Clock>, Arc<dyn Waker>)>,
}

impl core::fmt::Debug for AppTimers {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppTimers").finish_non_exhaustive()
    }
}

impl AppTimers {
    /// Schedules `callback` on the platform clock, returning no ID if there is no active clock or
    /// the delay exceeds its range.
    pub fn schedule(&self, after: Duration, callback: Rc<dyn Fn()>) -> Option<TimerId> {
        let (id, waker) = {
            let mut inner = self.inner.borrow_mut();
            let earliest = inner.timers.peek().map(|(deadline, _)| deadline);
            let (clock, waker) = inner.platform.as_ref()?;
            let deadline = clock.now().checked_add(after)?;
            let waker = earliest
                .is_none_or(|earliest| deadline < earliest)
                .then(|| Arc::clone(waker));
            // Every application entry belongs to the same scope, not to a window's timer heap.
            let id = inner.timers.schedule_at(
                DocumentId::FIRST,
                deadline,
                after,
                Repeat::Once,
                callback,
            );
            (id, waker)
        };
        if let Some(waker) = waker {
            waker.wake(WakeReason::AppWork);
        }
        Some(id)
    }

    /// Removes a pending application timer. Already-dispatched timers are unaffected.
    pub fn cancel(&self, id: TimerId) {
        self.inner.borrow_mut().timers.cancel(id);
    }

    pub(crate) fn attach(&self, cx: &dyn PlatformCx) {
        self.inner.borrow_mut().platform = Some((cx.clock(), cx.waker()));
    }

    pub(crate) fn detach(&self) {
        let timers = {
            let mut inner = self.inner.borrow_mut();
            inner.platform = None;
            std::mem::take(&mut inner.timers)
        };
        drop(timers);
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.inner.borrow_mut().timers.peek().map(|(deadline, _)| deadline)
    }

    pub(crate) fn due(&self, now: Instant) -> Vec<Rc<dyn Fn()>> {
        self.inner.borrow_mut().timers.due(DocumentId::FIRST, now)
    }
}
