use std::{
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};

use host_services::system_files::{Conflict, ConflictResolution, Progress, Volume};

use super::Outcome;

pub(super) type Wake = Arc<dyn Fn() + Send + Sync>;
pub(super) type Task = Box<dyn FnOnce() + Send>;

const MAILBOX: usize = 512;

pub(crate) enum Event {
    Done(u64, Outcome),
    Progress(u64, Progress),
    Conflict(u64, Conflict),
    Volumes(u64, Vec<Volume>),
}

/// Cancellation and a single conflict reply slot, shared only with the I/O worker.
#[derive(Default)]
pub(super) struct Control {
    cancelled: AtomicBool,
    gate: Mutex<Gate>,
    notify: Condvar,
}

#[derive(Default)]
enum Gate {
    #[default]
    Running,
    Waiting,
    Answered(ConflictResolution),
}

impl Control {
    pub(super) fn flag(&self) -> &AtomicBool {
        &self.cancelled
    }
    pub(super) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub(super) fn waiting(&self) -> bool {
        matches!(
            *self.gate.lock().expect("system Files gate poisoned"),
            Gate::Waiting
        )
    }

    pub(super) fn cancel(&self) {
        let mut gate = self.gate.lock().expect("system Files gate poisoned");
        self.cancelled.store(true, Ordering::Release);
        // Clear the pending reply before releasing the owner-side callbacks.
        *gate = Gate::Running;
        self.notify.notify_all();
    }

    pub(super) fn stop_or_timeout(&self, timeout: Duration) -> bool {
        let guard = self.gate.lock().expect("system Files gate poisoned");
        drop(
            self.notify
                .wait_timeout_while(guard, timeout, |_| !self.cancelled())
                .expect("system Files gate poisoned"),
        );
        self.cancelled()
    }

    pub(super) fn resolve(&self, decision: ConflictResolution) {
        let mut gate = self.gate.lock().expect("system Files gate poisoned");
        if matches!(*gate, Gate::Waiting) {
            *gate = Gate::Answered(decision);
            self.notify.notify_one();
        }
    }

    pub(super) fn ask(
        &self,
        conflict: Conflict,
        id: u64,
        tx: &SyncSender<Event>,
        wake: &Wake,
    ) -> ConflictResolution {
        let mut gate = self.gate.lock().expect("system Files gate poisoned");
        if self.cancelled() || tx.try_send(Event::Conflict(id, conflict)).is_err() {
            return ConflictResolution::CANCEL;
        }
        *gate = Gate::Waiting;
        wake();
        let mut gate = self
            .notify
            .wait_while(gate, |state| matches!(state, Gate::Waiting))
            .expect("system Files gate poisoned");
        match std::mem::take(&mut *gate) {
            Gate::Answered(decision) => decision,
            _ => ConflictResolution::CANCEL,
        }
    }
}

pub(super) fn mailbox() -> (SyncSender<Event>, Receiver<Event>) {
    mpsc::sync_channel(MAILBOX)
}

/// Dedicated bounded metadata/cache lane; a paused transfer cannot exhaust it.
pub(super) struct IoPool(SyncSender<Task>);

impl IoPool {
    pub(super) fn new() -> io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Task>(128);
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..4 {
            let rx = rx.clone();
            thread::Builder::new()
                .name(format!("zgui-system-io-{i}"))
                .spawn(move || {
                    loop {
                        let task = rx.lock().expect("system Files task queue poisoned").recv();
                        let Ok(task) = task else {
                            return;
                        };
                        task();
                    }
                })?;
        }
        Ok(Self(tx))
    }

    pub(super) fn submit(&self, task: Task) -> io::Result<()> {
        self.0
            .try_send(task)
            .map_err(|_| io::Error::other("system files I/O queue is full"))
    }
}
