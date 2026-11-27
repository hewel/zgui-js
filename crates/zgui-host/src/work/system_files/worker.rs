use std::{
    collections::VecDeque,
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle, Thread},
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
    Waiting(Option<PreviewWait>),
    Answered(ConflictResolution, u64),
}

struct PreviewWait {
    tasks: Arc<PreviewTasks>,
    worker: Thread,
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
            Gate::Waiting(_)
        )
    }

    pub(super) fn cancel(&self) {
        let mut gate = self.gate.lock().expect("system Files gate poisoned");
        self.cancelled.store(true, Ordering::Release);
        // Clear the pending reply before releasing the owner-side callbacks.
        if let Gate::Waiting(Some(preview)) = &*gate {
            preview.worker.unpark();
        }
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
        if let Gate::Waiting(preview) = &*gate {
            let through = preview.as_ref().map_or(0, |preview| {
                // Reads queued before the answer always see the paused state,
                // regardless of how quickly the worker receives the answer.
                let through = preview.tasks.latest();
                preview.worker.unpark();
                through
            });
            *gate = Gate::Answered(decision, through);
            self.notify.notify_one();
        }
    }

    pub(super) fn ask(
        &self,
        conflict: Conflict,
        id: u64,
        tx: &SyncSender<Event>,
        wake: &Wake,
        preview: Option<&Arc<PreviewTasks>>,
    ) -> ConflictResolution {
        let mut gate = self.gate.lock().expect("system Files gate poisoned");
        if self.cancelled() || tx.try_send(Event::Conflict(id, conflict)).is_err() {
            return ConflictResolution::CANCEL;
        }
        *gate = Gate::Waiting(preview.map(|tasks| PreviewWait {
            tasks: tasks.clone(),
            worker: thread::current(),
        }));
        wake();
        if let Some(tasks) = preview {
            // Conflict handlers may await metadata reads, but later mutation
            // jobs must not run while this job still owns the conflict.
            while matches!(*gate, Gate::Waiting(_)) {
                let read = tasks.read(u64::MAX);
                drop(gate);
                if let Some(read) = read {
                    read();
                } else {
                    thread::park();
                }
                gate = self.gate.lock().expect("system Files gate poisoned");
            }
        } else {
            gate = self
                .notify
                .wait_while(gate, |state| matches!(state, Gate::Waiting(_)))
                .expect("system Files gate poisoned");
        }
        let answer = std::mem::take(&mut *gate);
        drop(gate);
        match answer {
            Gate::Answered(decision, through) => {
                if let Some(tasks) = preview {
                    while !self.cancelled() {
                        let Some(read) = tasks.read(through) else {
                            break;
                        };
                        read();
                    }
                }
                decision
            }
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

/// One ordered fixture lane shared by metadata and mutation jobs.
pub(super) struct PreviewWorker {
    pub(super) tasks: Arc<PreviewTasks>,
    worker: JoinHandle<()>,
}

#[derive(Default)]
pub(super) struct PreviewTasks(Mutex<PreviewQueue>);

#[derive(Default)]
struct PreviewQueue {
    pending: VecDeque<PreviewTask>,
    jobs: usize,
    stopped: bool,
}

struct PreviewTask {
    id: u64,
    job: bool,
    run: Task,
}

impl PreviewTasks {
    fn latest(&self) -> u64 {
        self.0
            .lock()
            .expect("fixture task queue poisoned")
            .pending
            .back()
            .map_or(0, |task| task.id)
    }

    fn read(&self, through: u64) -> Option<Task> {
        let mut queue = self.0.lock().expect("fixture task queue poisoned");
        let index = queue
            .pending
            .iter()
            .position(|task| !task.job && task.id <= through)?;
        queue.pending.remove(index).map(|task| task.run)
    }
}

impl PreviewWorker {
    pub(super) fn new() -> io::Result<Self> {
        let tasks = Arc::new(PreviewTasks::default());
        let work = tasks.clone();
        let worker = thread::Builder::new()
            .name("zgui-system-fixture".into())
            .spawn(move || {
                loop {
                    let task = {
                        let mut queue = work.0.lock().expect("fixture task queue poisoned");
                        if queue.stopped {
                            return;
                        }
                        queue.pending.pop_front()
                    };
                    if let Some(task) = task {
                        (task.run)();
                        if task.job {
                            work.0.lock().expect("fixture task queue poisoned").jobs -= 1;
                        }
                    } else {
                        thread::park();
                    }
                }
            })?;
        Ok(Self { tasks, worker })
    }

    pub(super) fn submit(&self, id: u64, job: bool, run: Task) -> io::Result<()> {
        let mut queue = self.tasks.0.lock().expect("fixture task queue poisoned");
        if queue.stopped || queue.pending.len() >= 128 {
            return Err(io::Error::other(
                "system files fixture queue is unavailable",
            ));
        }
        queue.pending.push_back(PreviewTask { id, job, run });
        queue.jobs += usize::from(job);
        self.worker.thread().unpark();
        Ok(())
    }

    pub(super) fn jobs(&self) -> usize {
        self.tasks
            .0
            .lock()
            .expect("fixture task queue poisoned")
            .jobs
    }

    pub(super) fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }

    pub(super) fn stop(&self) {
        let mut queue = self.tasks.0.lock().expect("fixture task queue poisoned");
        queue.stopped = true;
        queue.jobs -= queue.pending.iter().filter(|task| task.job).count();
        queue.pending.clear();
        self.worker.thread().unpark();
    }
}
