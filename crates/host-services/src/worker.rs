use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::Shutdown,
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

pub(super) const MAX_EVENTS: usize = 32;
pub(super) const UNWATCHED: libc::pollfd = libc::pollfd {
    fd: -1,
    events: 0,
    revents: 0,
};

pub(super) type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug)]
pub(super) struct Worker<T> {
    shared: Arc<Shared<T>>,
    signal: UnixStream,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug)]
struct Shared<T> {
    stop: AtomicBool,
    active: AtomicBool,
    inbox: Mutex<Inbox<T>>,
    byte_limit: usize,
}

#[derive(Debug)]
struct Inbox<T> {
    events: VecDeque<(T, usize)>,
    bytes: usize,
    backpressured: bool,
}

pub(super) struct Publisher<T> {
    shared: Arc<Shared<T>>,
    signal: UnixStream,
    wake: Wake,
}

impl<T: Send + 'static> Worker<T> {
    pub(super) fn start(
        name: &str,
        byte_limit: usize,
        wake: Wake,
        run: impl FnOnce(&Publisher<T>) -> Option<T> + Send + 'static,
    ) -> io::Result<Self> {
        let (signal, peer) = UnixStream::pair()?;
        signal.set_nonblocking(true)?;
        peer.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            active: AtomicBool::new(true),
            inbox: Mutex::new(Inbox {
                events: VecDeque::new(),
                bytes: 0,
                backpressured: false,
            }),
            byte_limit,
        });
        let publisher = Publisher {
            shared: Arc::clone(&shared),
            signal: peer,
            wake,
        };
        let handle = thread::Builder::new().name(name.into()).spawn(move || {
            let terminal = run(&publisher);
            publisher.inactive();
            if let Some(event) = terminal.filter(|_| !publisher.stopped()) {
                // The terminal event has one reserved slot, ordered after all data.
                let mut inbox = publisher
                    .shared
                    .inbox
                    .lock()
                    .expect("inbox lock is not held across callbacks");
                inbox.events.push_back((event, 0));
                drop(inbox);
                (publisher.wake)();
            }
        })?;
        Ok(Self {
            shared,
            signal,
            handle: Some(handle),
        })
    }
}

impl<T> Worker<T> {
    pub(super) fn active(&self) -> bool {
        self.shared.active.load(Ordering::Acquire)
    }

    pub(super) fn take(&self) -> Option<T> {
        let mut inbox = self
            .shared
            .inbox
            .lock()
            .expect("inbox lock is not held across callbacks");
        let (event, bytes) = inbox.events.pop_front()?;
        inbox.bytes -= bytes;
        if inbox.backpressured {
            drop(inbox);
            self.signal();
        }
        Some(event)
    }

    pub(super) fn signal_socket(&self) -> io::Result<UnixStream> {
        self.signal.try_clone()
    }

    pub(super) fn signal(&self) {
        // A full notification socket is already readable. A broken peer means
        // the worker has exited and does not need to be notified.
        let mut signal = &self.signal;
        while signal
            .write(&[0])
            .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
        {}
    }

    fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
        let _ = self.signal.shutdown(Shutdown::Write);
    }

    /// Hands shutdown responsibility to the worker's protocol.
    pub(super) fn detach(mut self) {
        self.handle.take();
    }

    pub(super) fn close(mut self) -> io::Result<()> {
        self.stop();
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| io::Error::other("service worker panicked"))?;
        }
        Ok(())
    }
}

impl<T> Drop for Worker<T> {
    fn drop(&mut self) {
        if self.handle.is_some() {
            self.stop();
        }
    }
}

impl<T> Publisher<T> {
    pub(super) fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }

    pub(super) fn inactive(&self) {
        self.shared.active.store(false, Ordering::Release);
    }

    pub(super) fn offer(&self, event: T, bytes: usize) -> Result<(), (T, usize)> {
        let mut inbox = self
            .shared
            .inbox
            .lock()
            .expect("inbox lock is not held across callbacks");
        let full = inbox.events.len() >= MAX_EVENTS || bytes > self.shared.byte_limit - inbox.bytes;
        inbox.backpressured = full;
        if full {
            return Err((event, bytes));
        }
        inbox.bytes += bytes;
        inbox.events.push_back((event, bytes));
        drop(inbox);
        (self.wake)();
        Ok(())
    }

    /// The first slot is the worker's notification socket; other slots are call-specific.
    pub(super) fn poll(
        &self,
        descriptors: &mut [libc::pollfd],
        timeout: libc::c_int,
    ) -> io::Result<()> {
        let Some(signal) = descriptors.first_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "worker poll requires a notification slot",
            ));
        };
        *signal = pollfd(&self.signal, libc::POLLIN);
        let count = libc::nfds_t::try_from(descriptors.len()).map_err(io::Error::other)?;
        loop {
            // SAFETY: the array is a live writable buffer of `count` pollfd structures.
            if unsafe { libc::poll(descriptors.as_mut_ptr(), count, timeout) } >= 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted || self.stopped() {
                return Err(error);
            }
        }
        if descriptors[0].revents != 0 {
            // One bounded read is enough: remaining notices keep the socket readable.
            let _ = (&self.signal).read(&mut [0; 1024]);
        }
        Ok(())
    }
}

impl<T> Drop for Publisher<T> {
    fn drop(&mut self) {
        self.inactive();
    }
}

pub(super) fn pollfd(fd: &impl AsRawFd, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd: fd.as_raw_fd(),
        events,
        revents: 0,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{UNWATCHED, Wake, Worker};
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
            mpsc::{self, Receiver},
        },
        time::{Duration, Instant},
    };

    pub(crate) const DEADLINE: Duration = Duration::from_secs(5);

    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir()
                .canonicalize()
                .expect("resolve the trusted system temp directory");
            let path = path.join(format!(
                "zgui-ipc-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("create isolated OS service fixture directory");
            Self(path)
        }

        pub(crate) fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn waiter() -> (Wake, Receiver<()>) {
        let (wake, inbox) = mpsc::channel();
        (
            Arc::new(move || {
                let _ = wake.send(());
            }),
            inbox,
        )
    }

    pub(crate) fn observe<T>(inbox: &Receiver<()>, mut receive: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(event) = receive() {
                return event;
            }
            inbox
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("worker publishes an event and then wakes before the deadline");
        }
    }

    #[test]
    fn dropping_worker_wakes_it_without_waiting_for_cleanup() {
        let (release, cleanup) = mpsc::channel::<()>();
        let (stopped, stopping) = mpsc::channel();
        let worker = Worker::<()>::start("test-cancellation", 1, Arc::new(|| {}), move |owner| {
            while !owner.stopped() {
                owner
                    .poll(&mut [UNWATCHED], -1)
                    .expect("wait for cancellation");
            }
            stopped
                .send(())
                .expect("report cancellation before cleanup");
            let _ = cleanup.recv();
            None
        })
        .expect("start a worker with gated cleanup");
        let (returned, dropped) = mpsc::channel();
        let caller = std::thread::spawn(move || {
            drop(worker);
            returned.send(()).expect("report that drop returned");
        });
        let noticed_stop = stopping.recv_timeout(DEADLINE);
        let returned_before_cleanup = dropped.recv_timeout(DEADLINE);
        drop(release);
        caller
            .join()
            .expect("join the thread that dropped the worker");
        noticed_stop.expect("drop must wake the worker");
        returned_before_cleanup.expect("drop must return before cleanup finishes");
    }
}
