//! Bounded, line-framed request/reply transport for a trusted local socket.

use crate::unix_socket::nonblocking_connect;
use crate::worker::{Publisher, UNWATCHED, Worker, pollfd};
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// Maximum bytes in one UTF-8 request frame, excluding its LF.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Maximum arguments in a single control request.
pub const MAX_ARGS: usize = 64;
/// Maximum UTF-8 bytes in one reply's unescaped success or error payload.
pub const MAX_REPLY_BYTES: usize = 64 * 1024;
// JSON can escape each byte as six bytes (`\u0000`); reserve room for its envelope.
const MAX_REPLY_FRAME_BYTES: usize = 6 * MAX_REPLY_BYTES + 64;
const MAX_CLIENTS: usize = 8;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const HANDLER_TIMEOUT: Duration = Duration::from_secs(5);
const EXCHANGE_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

type Reply = Result<String, String>;
type Decoder = fn(&str) -> Result<Vec<String>, String>;
type Encoder = fn(Result<&str, &str>) -> io::Result<Vec<u8>>;

/// An argv request and its one-shot reply slot.
#[derive(Debug)]
pub struct Inbound {
    /// Argument boundaries exactly as sent by the requester.
    pub args: Vec<String>,
    reply: mpsc::SyncSender<Reply>,
    deadline: Instant,
}

impl Inbound {
    /// Returns whether the reply deadline passed before the host took this request.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Completes this request without blocking the owner thread.
    ///
    /// An oversized payload becomes a bounded error. Dropping without replying
    /// reports cancellation to the peer if it is still connected.
    pub fn reply(self, reply: Result<String, String>) {
        let len = match &reply {
            Ok(text) | Err(text) => text.len(),
        };
        let reply = if len <= MAX_REPLY_BYTES {
            reply
        } else {
            Err("control reply exceeds byte limit".to_owned())
        };
        let _ = self.reply.try_send(reply);
    }
}

/// Owner-side inbox for requests accepted on a securely bound Unix listener.
#[derive(Debug)]
pub struct Listener(Worker<Inbound>);

impl Listener {
    /// Serve one request and reply per connection on an existing listener.
    ///
    /// The codec handles bytes between line delimiters; it must not emit an LF.
    /// `wake` runs after an inbound request is published. The codec and wake
    /// must return promptly; no socket I/O runs on the owner thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket cannot be made nonblocking or the worker
    /// thread cannot be started.
    pub fn start(
        listener: UnixListener,
        decode: Decoder,
        encode: Encoder,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> io::Result<Self> {
        listener.set_nonblocking(true)?;
        Worker::start(
            "zgui-control",
            MAX_CLIENTS * MAX_REQUEST_BYTES,
            Arc::new(wake),
            move |publisher| {
                serve(listener, decode, encode, publisher);
                None
            },
        )
        .map(Self)
    }

    /// Takes the next queued request without blocking.
    pub fn try_recv(&self) -> Option<Inbound> {
        while let Some(request) = self.0.take() {
            if !request.is_expired() {
                return Some(request);
            }
        }
        None
    }

    /// Cancels the listener and waits for the worker to close its sockets.
    ///
    /// During cancellation, unanswered requests receive a best-effort
    /// rejection. Use this only outside Bridge turns; dropping the listener
    /// instead requests cancellation without waiting for the thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the listener worker panicked.
    pub fn close(self) -> io::Result<()> {
        self.0.close()
    }
}

fn serve(listener: UnixListener, decode: Decoder, encode: Encoder, publisher: &Publisher<Inbound>) {
    let mut clients = Vec::with_capacity(MAX_CLIENTS);
    'serving: while !publisher.stopped() {
        while !publisher.stopped() && clients.len() < MAX_CLIENTS {
            let socket = match listener.accept() {
                Ok((socket, _)) => socket,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    break;
                }
                Err(_) => break 'serving,
            };
            if socket.set_nonblocking(true).is_err() {
                break;
            }
            clients.push(Client {
                socket,
                phase: Phase::Reading(Vec::new()),
                deadline: Instant::now() + IO_TIMEOUT,
            });
        }
        clients.retain_mut(|client| {
            publisher.stopped() || client.advance(publisher, decode, encode).unwrap_or(false)
        });
        if publisher.stopped() {
            break;
        }

        let mut polling = [UNWATCHED; MAX_CLIENTS + 2];
        if clients.len() < MAX_CLIENTS {
            polling[1] = pollfd(&listener, libc::POLLIN);
        }
        let now = Instant::now();
        let mut timeout = None;
        for (client, entry) in clients.iter().zip(&mut polling[2..]) {
            let mut remaining = client.deadline.saturating_duration_since(now);
            *entry = match &client.phase {
                Phase::Reading(_) => pollfd(&client.socket, libc::POLLIN),
                Phase::Waiting(_) => {
                    remaining = remaining.min(POLL_INTERVAL);
                    UNWATCHED
                }
                Phase::Writing { .. } => pollfd(&client.socket, libc::POLLOUT),
            };
            timeout = Some(timeout.map_or(remaining, |other: Duration| other.min(remaining)));
        }
        let millis = timeout.map_or(-1, |duration| {
            i32::try_from(duration.as_millis().saturating_add(1)).unwrap_or(i32::MAX)
        });
        if publisher.poll(&mut polling, millis).is_err() {
            break;
        }
    }
    drop(listener);
    if publisher.stopped() {
        for client in clients {
            let _ = client.finish_retired(encode);
        }
    }
}

struct Client {
    socket: UnixStream,
    deadline: Instant,
    phase: Phase,
}

enum Phase {
    Reading(Vec<u8>),
    Waiting(mpsc::Receiver<Reply>),
    Writing { frame: Vec<u8>, sent: usize },
}

impl Client {
    fn advance(
        &mut self,
        publisher: &Publisher<Inbound>,
        decode: Decoder,
        encode: Encoder,
    ) -> io::Result<bool> {
        let now = Instant::now();
        let expired = now >= self.deadline;
        let reply = match &mut self.phase {
            Phase::Reading(_) if expired => Err("control request timed out".to_owned()),
            Phase::Reading(frame) => match read_frame(&mut self.socket, frame, MAX_REQUEST_BYTES) {
                Ok(false) => return Ok(true),
                Ok(true) => {
                    let args = match str::from_utf8(frame) {
                        Ok(text) => decode(text),
                        Err(_) => Err("control request is not UTF-8".to_owned()),
                    };
                    match args {
                        Ok(args) if args.len() <= MAX_ARGS => {
                            let (reply, waiting) = mpsc::sync_channel(1);
                            let deadline = now + HANDLER_TIMEOUT;
                            if publisher
                                .offer(
                                    Inbound {
                                        args,
                                        reply,
                                        deadline,
                                    },
                                    frame.len(),
                                )
                                .is_ok()
                            {
                                self.phase = Phase::Waiting(waiting);
                                self.deadline = deadline;
                                return Ok(true);
                            }
                            Err("control request queue is full".to_owned())
                        }
                        Ok(_) => Err("control request has too many arguments".to_owned()),
                        Err(error) => Err(error),
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    Err(error.to_string())
                }
                Err(error) => return Err(error),
            },
            Phase::Waiting(_) if expired => Err("control reply timed out".to_owned()),
            Phase::Waiting(waiting) => match waiting.try_recv() {
                Ok(reply) => reply,
                Err(mpsc::TryRecvError::Empty) => return Ok(true),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err("control request cancelled".to_owned())
                }
            },
            Phase::Writing { .. } if expired => return Ok(false),
            Phase::Writing { frame, sent } => {
                return write_available(&mut self.socket, frame, sent).map(|done| !done);
            }
        };

        let reply = reply.as_ref().map(String::as_str).map_err(String::as_str);
        let frame = encode_reply(encode, reply)?;
        let mut sent = 0;
        if write_available(&mut self.socket, &frame, &mut sent)? {
            return Ok(false);
        }
        self.phase = Phase::Writing { frame, sent };
        self.deadline = now + IO_TIMEOUT;
        Ok(true)
    }

    fn finish_retired(mut self, encode: Encoder) -> io::Result<()> {
        let (frame, mut sent) = match self.phase {
            Phase::Reading(_) => return Ok(()),
            Phase::Waiting(waiting) => {
                let reply = waiting.try_recv();
                let reply = match &reply {
                    Ok(reply) => reply.as_ref().map(String::as_str).map_err(String::as_str),
                    Err(_) => Err("control listener retired"),
                };
                (encode_reply(encode, reply)?, 0)
            }
            Phase::Writing { frame, sent } => (frame, sent),
        };
        // Do not wait for incomplete input or for a peer to make room to write.
        write_available(&mut self.socket, &frame, &mut sent)?;
        Ok(())
    }
}

fn encode_reply(encode: Encoder, reply: Result<&str, &str>) -> io::Result<Vec<u8>> {
    let reply = match reply {
        Err(text) if text.len() > MAX_REPLY_BYTES => Err("control reply exceeds byte limit"),
        bounded => bounded,
    };
    let mut frame = encode(reply)?;
    validate_frame(&frame, MAX_REPLY_FRAME_BYTES)?;
    frame.push(b'\n');
    Ok(frame)
}

/// Connects without waiting indefinitely on a Unix listener's backlog.
///
/// # Errors
///
/// Returns an error when the path or connection is invalid, the backlog is full,
/// or the connection deadline expires.
pub fn connect(path: &Path) -> io::Result<UnixStream> {
    let (socket, pending) = nonblocking_connect(path)?;
    if pending {
        let deadline = Instant::now() + IO_TIMEOUT;
        loop {
            if let Some(error) = socket.take_error()? {
                return Err(error);
            }
            match socket.peer_addr() {
                Ok(_) => break,
                Err(error) if error.kind() == io::ErrorKind::NotConnected => pause_until(deadline)?,
                Err(error) => return Err(error),
            }
        }
    }
    Ok(socket)
}

/// Sends a single UTF-8 request frame and reads one reply frame, each without LF.
///
/// # Errors
///
/// Returns an error for an invalid request frame, a transport I/O failure,
/// an oversized or truncated response, or a deadline expiring.
pub fn exchange(mut socket: UnixStream, frame: &[u8]) -> io::Result<Vec<u8>> {
    validate_frame(frame, MAX_REQUEST_BYTES)?;
    if str::from_utf8(frame).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control request is not UTF-8",
        ));
    }
    socket.set_nonblocking(true)?;
    let deadline = Instant::now() + IO_TIMEOUT;
    for part in [frame, b"\n"] {
        let mut sent = 0;
        while !write_available(&mut socket, part, &mut sent)? {
            pause_until(deadline)?;
        }
    }

    let deadline = Instant::now() + EXCHANGE_REPLY_TIMEOUT;
    let mut reply = Vec::new();
    while !read_frame(&mut socket, &mut reply, MAX_REPLY_FRAME_BYTES)? {
        pause_until(deadline)?;
    }
    Ok(reply)
}

fn pause_until(deadline: Instant) -> io::Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "control socket deadline exceeded",
        ));
    }
    thread::sleep(remaining.min(POLL_INTERVAL));
    Ok(())
}

fn validate_frame(frame: &[u8], max: usize) -> io::Result<()> {
    if frame.len() > max {
        return Err(frame_too_large());
    }
    if frame.contains(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control frame contains an LF",
        ));
    }
    Ok(())
}

fn frame_too_large() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "control frame exceeds byte limit",
    )
}

fn read_frame(reader: &mut UnixStream, frame: &mut Vec<u8>, max: usize) -> io::Result<bool> {
    let mut chunk = [0; 4096];
    loop {
        let count = match reader.read(&mut chunk) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "control frame is missing LF",
                ));
            }
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let lf = chunk[..count].iter().position(|&byte| byte == b'\n');
        let data = &chunk[..lf.unwrap_or(count)];
        let len = frame.len() + data.len();
        if len > max {
            return Err(frame_too_large());
        }
        if len > frame.capacity() {
            let capacity = frame.capacity().saturating_mul(2).max(len).min(max);
            frame.reserve_exact(capacity - frame.len());
        }
        frame.extend_from_slice(data);
        if lf.is_some() {
            return Ok(true);
        }
    }
}

fn write_available(writer: &mut UnixStream, frame: &[u8], sent: &mut usize) -> io::Result<bool> {
    while *sent < frame.len() {
        match writer.write(&frame[*sent..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "control socket stopped accepting bytes",
                ));
            }
            Ok(count) => *sent += count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_PATH: AtomicUsize = AtomicUsize::new(0);

    struct TestSocket(PathBuf);

    impl TestSocket {
        fn open() -> (Self, Listener, mpsc::Receiver<()>) {
            let name = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("zgui-control-{}-{name}.sock", std::process::id()));
            let socket = UnixListener::bind(&path).expect("bind test control socket");
            let (wake, wakes) = mpsc::channel();
            let listener = Listener::start(socket, decode, encode, move || {
                wake.send(())
                    .expect("test wake receiver stays alive while listening");
            })
            .expect("start test control listener");
            (Self(path), listener, wakes)
        }

        fn connect(&self) -> UnixStream {
            UnixStream::connect(&self.0).expect("connect to test control socket")
        }
    }

    impl Drop for TestSocket {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    // Use a text codec to exercise the transport independently of zgui-host's JSON codec.
    fn decode(frame: &str) -> Result<Vec<String>, String> {
        if frame.is_empty() {
            return Err("empty test frame".to_owned());
        }
        Ok(vec![frame.to_owned()])
    }

    fn encode(reply: Result<&str, &str>) -> io::Result<Vec<u8>> {
        Ok(match reply {
            Ok(text) => format!("ok:{text}").into_bytes(),
            Err(text) => format!("err:{text}").into_bytes(),
        })
    }

    #[test]
    fn complete_request_can_pass_a_stalled_peer_and_reply_once() {
        let (socket, listener, wakes) = TestSocket::open();
        let mut stalled = socket.connect();
        stalled
            .write_all(b"incomplete")
            .expect("write a partial request");
        let complete = socket.connect();
        let client = thread::spawn(|| exchange(complete, "status ☀".as_bytes()));
        wakes
            .recv_timeout(Duration::from_secs(1))
            .expect("complete peer wakes owner despite stalled peer");
        let request = listener
            .try_recv()
            .expect("wake follows the inbound request");
        assert_eq!(
            request.args,
            ["status ☀"],
            "only a complete frame reaches the owner"
        );
        request.reply(Ok("ready".to_owned()));
        assert_eq!(
            client
                .join()
                .expect("join control client")
                .expect("receive framed reply"),
            b"ok:ready",
            "reply consumes exactly one request"
        );
        let started = Instant::now();
        listener.close().expect("join the cancelled listener");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "stalled socket must not delay listener shutdown"
        );
    }

    #[test]
    fn oversized_or_invalid_utf8_frames_never_wake_the_owner() {
        let (socket, listener, wakes) = TestSocket::open();
        for (mut frame, expected) in [
            (
                vec![b'x'; MAX_REQUEST_BYTES + 1],
                b"err:control frame exceeds byte limit".as_slice(),
            ),
            (vec![0xff], b"err:control request is not UTF-8".as_slice()),
        ] {
            frame.push(b'\n');
            let mut client = socket.connect();
            client
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("bound test read");
            client.write_all(&frame).expect("write malformed frame");
            let mut reply = Vec::new();
            read_until_lf(&mut client, &mut reply).expect("read rejection frame");
            assert_eq!(
                reply.strip_suffix(b"\n"),
                Some(expected),
                "transport rejects input before handing it to the owner"
            );
        }
        assert!(wakes.try_recv().is_err(), "no wake for rejected input");
        assert!(
            listener.try_recv().is_none(),
            "no inbound request for rejected input"
        );
    }

    #[test]
    fn dropping_an_inbound_request_cancels_its_reply() {
        let (socket, listener, wakes) = TestSocket::open();
        let connected = socket.connect();
        let client = thread::spawn(|| exchange(connected, b"status"));
        wakes
            .recv_timeout(Duration::from_secs(1))
            .expect("request reaches owner");
        drop(
            listener
                .try_recv()
                .expect("take inbound request without replying"),
        );
        assert_eq!(
            client
                .join()
                .expect("join control client")
                .expect("receive cancellation reply"),
            b"err:control request cancelled",
            "retired handler must not leave the peer waiting for a deadline"
        );
        drop(listener);
    }

    #[test]
    fn oversized_handler_result_becomes_a_bounded_error() {
        let (socket, listener, wakes) = TestSocket::open();
        let connected = socket.connect();
        let client = thread::spawn(|| exchange(connected, b"status"));
        wakes
            .recv_timeout(Duration::from_secs(1))
            .expect("request reaches owner");
        listener
            .try_recv()
            .expect("take inbound request")
            .reply(Ok("x".repeat(MAX_REPLY_BYTES + 1)));
        assert_eq!(
            client
                .join()
                .expect("join control client")
                .expect("receive size error"),
            b"err:control reply exceeds byte limit",
            "oversized handler result never reaches the wire as successful payload",
        );
        drop(listener);
    }

    fn read_until_lf(client: &mut UnixStream, reply: &mut Vec<u8>) -> io::Result<usize> {
        use std::io::BufRead as _;
        io::BufReader::new(client).read_until(b'\n', reply)
    }
}
