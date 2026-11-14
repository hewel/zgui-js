//! Bounded background text-asset reads using only owned, thread-safe data.
//!
//! Submission never waits for a worker or completion consumer. Dropping the last
//! executor handle closes admission; detached workers drain admitted requests and
//! exit without joining filesystem operations on the dropping thread. A full
//! completion channel can block a worker, so consumers must drain or disconnect
//! their inbox during shutdown.
//!
//! Asset roots are trusted package directories. Every requested component is
//! checked for symlinks, but portable `std` filesystem APIs cannot make those
//! checks atomic with opening a file: package directories must not be modified
//! concurrently by an untrusted writer.

#[cfg(unix)]
pub mod niri;

use std::{
    error::Error,
    fmt,
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
};

/// An owned read operation. No owner-thread runtime or UI handles belong here.
pub struct Request {
    pub generation: u64,
    pub request_id: u64,
    pub root: PathBuf,
    pub path: String,
    pub max_bytes: usize,
    pub cancelled: Arc<AtomicBool>,
    pub reply: SyncSender<Completion>,
    /// Called on the worker only after the completion has entered `reply`.
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

/// A result whose lifetime is independent of the executor and originating session.
#[derive(Debug)]
pub struct Completion {
    pub generation: u64,
    pub request_id: u64,
    pub result: Result<String, ServiceError>,
}

/// A stable machine-readable code and an owned, human-readable explanation.
///
/// Codes are `Io`, `InvalidPath`, `TooLarge`, `InvalidUtf8`, and `Cancelled`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceError {
    pub code: &'static str,
    pub message: String,
}

impl ServiceError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    fn cancelled() -> Self {
        Self::new("Cancelled", "asset read was cancelled")
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl Error for ServiceError {}

impl From<io::Error> for ServiceError {
    fn from(error: io::Error) -> Self {
        Self::new("Io", error.to_string())
    }
}

/// Admission failed without waiting. The rejected request is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    QueueFull,
    Stopped,
}

impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::QueueFull => "service request queue is full",
            Self::Stopped => "service executor has stopped",
        })
    }
}

impl Error for SubmitError {}

/// A clonable admission handle for a fixed-size worker pool.
#[derive(Clone)]
pub struct Executor {
    sender: SyncSender<Request>,
}

impl Executor {
    /// Creates `workers` detached threads and a bounded request queue.
    ///
    /// Zero workers is invalid. A zero-capacity queue is a rendezvous channel:
    /// submission succeeds only when a worker is already waiting to receive.
    /// If spawning fails, admission closes and already spawned workers exit;
    /// construction never joins them.
    pub fn new(workers: usize, queue_capacity: usize) -> io::Result<Self> {
        if workers == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "at least one worker is required"));
        }
        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        for index in 0..workers {
            let receiver = receiver.clone();
            thread::Builder::new()
                .name(format!("host-services-{index}"))
                .spawn(move || worker(receiver))?;
        }
        Ok(Self { sender })
    }

    /// Attempts admission without waiting for filesystem or completion work.
    pub fn submit(&self, request: Request) -> Result<(), SubmitError> {
        self.sender.try_send(request).map_err(|error| match error {
            TrySendError::Full(_) => SubmitError::QueueFull,
            TrySendError::Disconnected(_) => SubmitError::Stopped,
        })
    }
}

fn worker(receiver: Arc<Mutex<Receiver<Request>>>) {
    loop {
        // std's receiver is not Sync. Hold this lock only while receiving, never
        // across file reads, completion backpressure, or the wake callback.
        let request = {
            let Ok(receiver) = receiver.lock() else { return };
            match receiver.recv() {
                Ok(request) => request,
                Err(_) => return,
            }
        };
        let result = if request.cancelled.load(Ordering::Acquire) {
            Err(ServiceError::cancelled())
        } else {
            let result = read_text_asset(&request.root, &request.path, request.max_bytes);
            if request.cancelled.load(Ordering::Acquire) {
                Err(ServiceError::cancelled())
            } else {
                result
            }
        };
        let completion = Completion {
            generation: request.generation,
            request_id: request.request_id,
            result,
        };
        if request.reply.send(completion).is_ok() {
            (request.wake)();
        }
    }
}

fn read_text_asset(root: &Path, path: &str, max_bytes: usize) -> Result<String, ServiceError> {
    if path.is_empty()
        || path.contains(['\\', '\0'])
        || path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || !Path::new(path).components().all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(ServiceError::new("InvalidPath", "asset path must be a canonical relative path"));
    }

    let mut target = root.canonicalize()?;
    if !target.is_dir() {
        return Err(ServiceError::new("InvalidPath", "asset root must be a directory"));
    }
    let mut parts = path.split('/').peekable();
    while let Some(part) = parts.next() {
        target.push(part);
        let metadata = fs::symlink_metadata(&target)?;
        if metadata.file_type().is_symlink() {
            return Err(ServiceError::new("InvalidPath", "asset path must not contain symlinks"));
        }
        if parts.peek().is_some() && !metadata.is_dir() {
            return Err(ServiceError::new("InvalidPath", "asset parent must be a directory"));
        }
        if parts.peek().is_none() && !metadata.is_file() {
            return Err(ServiceError::new("InvalidPath", "asset must be a regular file"));
        }
    }

    let file = File::open(target)?;
    if !file.metadata()?.is_file() {
        return Err(ServiceError::new("InvalidPath", "asset must be a regular file"));
    }
    read_text(file, max_bytes)
}

fn read_text(reader: impl Read, max_bytes: usize) -> Result<String, ServiceError> {
    // Do not trust metadata length: a file may grow or report an inaccurate size.
    let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    reader.take(limit).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(ServiceError::new("TooLarge", "asset exceeds the text read byte limit"));
    }
    String::from_utf8(bytes)
        .map_err(|error| ServiceError::new("InvalidUtf8", error.utf8_error().to_string()))
}

#[cfg(test)]
mod tests;
