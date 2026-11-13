use super::*;
use std::{
    io::Cursor,
    sync::{atomic::AtomicU64, mpsc::RecvTimeoutError},
    time::Duration,
};

const DEADLINE: Duration = Duration::from_secs(5);

struct Assets(PathBuf);

impl Assets {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "zgui-host-services-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        fs::write(self.0.join(name), bytes).unwrap();
    }
}

impl Drop for Assets {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn request(assets: &Assets, path: &str, max_bytes: usize) -> (Request, Receiver<Completion>, Receiver<()>) {
    let (reply, receiver) = mpsc::sync_channel(1);
    let (wake_sender, wake_receiver) = mpsc::channel();
    let request = Request {
        generation: 7,
        request_id: 42,
        root: assets.0.clone(),
        path: path.into(),
        max_bytes,
        cancelled: Arc::new(AtomicBool::new(false)),
        reply,
        wake: Arc::new(move || { wake_sender.send(()).unwrap(); }),
    };
    (request, receiver, wake_receiver)
}

fn read(executor: &Executor, assets: &Assets, path: &str, max_bytes: usize) -> Result<String, ServiceError> {
    let (request, receiver, wake) = request(assets, path, max_bytes);
    executor.submit(request).unwrap();
    let completion = receiver.recv_timeout(DEADLINE).unwrap();
    wake.recv_timeout(DEADLINE).unwrap();
    assert_eq!(completion.generation, 7);
    assert_eq!(completion.request_id, 42);
    completion.result
}

#[test]
fn completion_is_owned_and_wakes_after_delivery() {
    fn assert_send<T: Send>() {}
    assert_send::<Request>();
    assert_send::<Completion>();
    assert_send::<Executor>();

    let assets = Assets::new();
    fs::create_dir(assets.0.join("nested")).unwrap();
    assets.write("nested/message.txt", "hello 🌍".as_bytes());
    let executor = Executor::new(2, 2).unwrap();
    let (mut request, receiver, wake) = request(&assets, "nested/message.txt", 64);
    request.generation = 91;
    request.request_id = 1234;
    executor.clone().submit(request).unwrap();
    drop(executor);
    wake.recv_timeout(DEADLINE).unwrap();
    // Waking guarantees the owned message is already available without waiting.
    let completion = receiver.try_recv().unwrap();
    drop(assets);
    assert_eq!(completion.generation, 91);
    assert_eq!(completion.request_id, 1234);
    assert_eq!(completion.result.unwrap(), "hello 🌍");
}

#[test]
fn noncanonical_paths_and_nonregular_files_are_rejected() {
    let assets = Assets::new();
    let executor = Executor::new(1, 1).unwrap();
    for path in ["", ".", "..", "../outside", "a/../b", "a/./b", "a//b", "/outside", "a/", "a\\b", "a\0b"] {
        assert_eq!(read(&executor, &assets, path, 32).unwrap_err().code, "InvalidPath", "{path:?}");
    }
    fs::create_dir(assets.0.join("directory")).unwrap();
    assets.write("file", b"text");
    assert_eq!(read(&executor, &assets, "directory", 32).unwrap_err().code, "InvalidPath");
    assert_eq!(read(&executor, &assets, "file/child", 32).unwrap_err().code, "InvalidPath");
    assert_eq!(read(&executor, &assets, "missing", 32).unwrap_err().code, "Io");
}

#[cfg(unix)]
#[test]
fn file_parent_and_dangling_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;

    let assets = Assets::new();
    let outside = Assets::new();
    outside.write("secret", b"outside");
    symlink(outside.0.join("secret"), assets.0.join("linked-file")).unwrap();
    symlink(&outside.0, assets.0.join("linked-parent")).unwrap();
    symlink("missing", assets.0.join("dangling")).unwrap();
    let executor = Executor::new(1, 1).unwrap();
    for path in ["linked-file", "linked-parent/secret", "dangling"] {
        assert_eq!(read(&executor, &assets, path, 64).unwrap_err().code, "InvalidPath");
    }
}

#[test]
fn size_limits_and_utf8_are_enforced_in_bytes() {
    let assets = Assets::new();
    assets.write("empty", b"");
    assets.write("text", "é".as_bytes());
    assets.write("invalid", &[0xff, 0xfe]);
    let executor = Executor::new(1, 1).unwrap();
    assert_eq!(read(&executor, &assets, "empty", 0).unwrap(), "");
    assert_eq!(read(&executor, &assets, "text", 2).unwrap(), "é");
    assert_eq!(read(&executor, &assets, "text", 1).unwrap_err().code, "TooLarge");
    assert_eq!(read(&executor, &assets, "text", 0).unwrap_err().code, "TooLarge");
    assert_eq!(read(&executor, &assets, "invalid", 2).unwrap_err().code, "InvalidUtf8");
    assert_eq!(read(&executor, &assets, "invalid", 1).unwrap_err().code, "TooLarge");
}

#[test]
fn actual_reads_stop_at_limit_plus_one_without_metadata() {
    let mut input = Cursor::new(b"abcdef");
    assert_eq!(read_text(&mut input, 3).unwrap_err().code, "TooLarge");
    assert_eq!(input.position(), 4);
    let mut empty_limit = Cursor::new(b"abcdef");
    assert_eq!(read_text(&mut empty_limit, 0).unwrap_err().code, "TooLarge");
    assert_eq!(empty_limit.position(), 1);
}

#[test]
fn cancelled_request_does_not_touch_the_filesystem() {
    let assets = Assets::new();
    let executor = Executor::new(1, 1).unwrap();
    let (request, receiver, wake) = request(&assets, "missing", 32);
    request.cancelled.store(true, Ordering::Release);
    executor.submit(request).unwrap();
    let completion = receiver.recv_timeout(DEADLINE).unwrap();
    wake.recv_timeout(DEADLINE).unwrap();
    assert_eq!(completion.result.unwrap_err().code, "Cancelled");
}

#[test]
fn bounded_submission_and_drop_do_not_wait_for_a_worker() {
    let assets = Assets::new();
    assets.write("text", b"ready");
    let executor = Executor::new(1, 1).unwrap();
    let (mut first, first_result, _) = request(&assets, "text", 32);
    let (entered_sender, entered) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let release = Mutex::new(release);
    first.wake = Arc::new(move || {
        entered_sender.send(()).unwrap();
        release.lock().unwrap().recv_timeout(DEADLINE).unwrap();
    });
    executor.submit(first).unwrap();
    entered.recv_timeout(DEADLINE).unwrap();
    let (queued, queued_result, queued_wake) = request(&assets, "text", 32);
    executor.submit(queued).unwrap();
    let (overflow, _, _) = request(&assets, "text", 32);
    assert_eq!(executor.submit(overflow), Err(SubmitError::QueueFull));
    // The sole worker is held by a channel, so a joining Drop would deadlock.
    drop(executor);
    release_sender.send(()).unwrap();
    assert_eq!(first_result.recv_timeout(DEADLINE).unwrap().result.unwrap(), "ready");
    assert_eq!(queued_result.recv_timeout(DEADLINE).unwrap().result.unwrap(), "ready");
    queued_wake.recv_timeout(DEADLINE).unwrap();
}

#[test]
fn blocked_completion_does_not_hold_receiver_lock_or_block_other_workers() {
    let assets = Assets::new();
    assets.write("text", b"ready");
    let executor = Executor::new(2, 2).unwrap();
    let (mut blocked, _, _) = request(&assets, "text", 32);
    let (reply, blocked_result) = mpsc::sync_channel(0);
    blocked.reply = reply;
    let (wake_sender, wake) = mpsc::channel();
    blocked.wake = Arc::new(move || { wake_sender.send(()).unwrap(); });
    executor.submit(blocked).unwrap();
    // Either worker can receive this; both must remain independently useful.
    let (other, other_result, other_wake) = request(&assets, "text", 32);
    executor.submit(other).unwrap();
    assert_eq!(other_result.recv_timeout(DEADLINE).unwrap().result.unwrap(), "ready");
    other_wake.recv_timeout(DEADLINE).unwrap();
    assert_eq!(wake.try_recv(), Err(mpsc::TryRecvError::Empty));
    assert_eq!(blocked_result.recv_timeout(DEADLINE).unwrap().result.unwrap(), "ready");
    wake.recv_timeout(DEADLINE).unwrap();
}

#[test]
fn disconnected_completion_is_not_woken() {
    let assets = Assets::new();
    assets.write("text", b"ready");
    let executor = Executor::new(1, 2).unwrap();
    let (mut abandoned, receiver, _) = request(&assets, "text", 32);
    let (wake_sender, wake) = mpsc::channel();
    abandoned.wake = Arc::new(move || { wake_sender.send(()).unwrap(); });
    drop(receiver);
    executor.submit(abandoned).unwrap();
    assert_eq!(read(&executor, &assets, "text", 32).unwrap(), "ready");
    assert_eq!(wake.recv_timeout(DEADLINE), Err(RecvTimeoutError::Disconnected));
}

#[test]
fn zero_workers_is_invalid_and_disconnected_admission_is_stopped() {
    assert_eq!(Executor::new(0, 1).err().unwrap().kind(), io::ErrorKind::InvalidInput);
    let assets = Assets::new();
    let (sender, receiver) = mpsc::sync_channel(1);
    drop(receiver);
    let executor = Executor { sender };
    let (request, _, _) = request(&assets, "missing", 32);
    assert_eq!(executor.submit(request), Err(SubmitError::Stopped));
}
