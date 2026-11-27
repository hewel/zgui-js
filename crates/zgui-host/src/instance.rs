//! Process-lifetime activation ingress. It acknowledges admission, not JavaScript execution.
use crate::{
    control_cli::{self, BoundSocket, SocketElection},
    invocation::{Activation, ActivationInfo, MAX_ARG_BYTES},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::Shutdown,
    os::{fd::AsRawFd, unix::net::{UnixListener, UnixStream}},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use zgui::platform::{WakeReason, Waker};

const TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PENDING: usize = 64;
// Worst-case JSON escaping of argv and a Linux pathname, plus one Linux
// environment string (tokens reject control characters, so at most 2x escaping).
const MAX_FRAME_BYTES: usize = 6 * (MAX_ARG_BYTES + 4096) + 2 * 128 * 1024 + 512;
const HELLO_PREFIX: &str = "zgui-activation-v1:hello:";
// App IDs are limited to 128 UTF-8 bytes by instance socket admission.
const MAX_HELLO_BYTES: usize = HELLO_PREFIX.len() + 128 + 1;
const ACK: &[u8] = b"zgui-activation-v1:ok\n";

#[derive(Default)]
struct State {
    pending: VecDeque<Activation>,
    waker: Option<Arc<dyn Waker>>,
    closed: bool,
}

#[derive(Default)]
pub(crate) struct Inbox(Mutex<State>);

impl Inbox {
    fn admit(&self, activation: Activation) -> bool {
        let wake = {
            let mut state = self.0.lock().expect("activation inbox poisoned");
            if state.closed || state.pending.len() >= MAX_PENDING { return false; }
            state.pending.push_back(activation);
            state.waker.clone()
        };
        if let Some(wake) = wake { wake.wake(WakeReason::AppWork); }
        true
    }

    pub(crate) fn len(&self) -> usize { self.0.lock().expect("activation inbox poisoned").pending.len() }
    pub(crate) fn pop(&self) -> Option<Activation> { self.0.lock().expect("activation inbox poisoned").pending.pop_front() }

    fn set_waker(&self, waker: Arc<dyn Waker>) {
        let has_pending = {
            let mut state = self.0.lock().expect("activation inbox poisoned");
            state.waker = Some(Arc::clone(&waker));
            !state.pending.is_empty()
        };
        if has_pending { waker.wake(WakeReason::AppWork); }
    }
}

pub(crate) struct Primary {
    inbox: Arc<Inbox>,
    stop: UnixStream,
    worker: Option<JoinHandle<io::Result<()>>>,
    socket: BoundSocket,
    // Keep the kernel listener alive until the published pathname is removed,
    // even if the serving thread exits unexpectedly.
    _listener: Arc<UnixListener>,
}

impl Primary {
    fn start(app_id: String, listener: UnixListener, socket: BoundSocket) -> io::Result<Self> {
        listener.set_nonblocking(true)?;
        let (stop, stopped) = UnixStream::pair()?;
        let inbox = Arc::new(Inbox::default());
        let worker_inbox = Arc::clone(&inbox);
        let listener = Arc::new(listener);
        let worker_listener = Arc::clone(&listener);
        let worker = thread::Builder::new().name("zgui-instance".into())
            .spawn(move || serve(&app_id, &worker_listener, &stopped, &worker_inbox))?;
        Ok(Self { inbox, stop, worker: Some(worker), socket, _listener: listener })
    }

    pub(crate) fn inbox(&self) -> Arc<Inbox> { Arc::clone(&self.inbox) }
    pub(crate) fn set_waker(&self, waker: Arc<dyn Waker>) { self.inbox.set_waker(waker); }

    fn cancel(&self) {
        let mut state = self.inbox.0.lock().expect("activation inbox poisoned");
        state.closed = true;
        state.waker = None;
        let _ = self.stop.shutdown(Shutdown::Write);
    }

    pub(crate) fn close(mut self) -> io::Result<()> {
        self.cancel();
        let unlinked = self.socket.unlink();
        let worker = self.worker.take().expect("activation listener worker");
        let shutdown = worker.join().map_err(|_| io::Error::other("activation listener panicked"));
        unlinked.and(shutdown?)
    }
}

impl Drop for Primary {
    fn drop(&mut self) {
        self.cancel();
        if let Err(error) = self.socket.unlink() {
            let _ = writeln!(io::stderr().lock(), "instance socket: {error}");
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Frame<A, S> {
    app_id: S,
    args: A,
    cwd: Option<S>,
    activation_token: Option<S>,
}

/// Return `None` only after the existing primary acknowledges this invocation.
pub(crate) fn coordinate(app_id: &str, activation: &Activation) -> io::Result<Option<Primary>> {
    let path = control_cli::instance_socket_path(app_id)?;
    match BoundSocket::elect(&path)? {
        SocketElection::Live(mut peer) => {
            peer.set_nonblocking(false)?;
            forward(&mut peer, app_id, activation)?;
            Ok(None)
        }
        SocketElection::Bound(listener, socket) => Primary::start(app_id.to_owned(), listener, socket).map(Some),
    }
}


fn serve(app_id: &str, listener: &UnixListener, stopped: &UnixStream, inbox: &Inbox) -> io::Result<()> {
    let hello = hello(app_id);
    let mut poll = [
        libc::pollfd { fd: stopped.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 },
    ];
    loop {
        // SAFETY: both descriptors are owned for this call and poll has two initialized entries.
        if unsafe { libc::poll(poll.as_mut_ptr(), 2, -1) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted { continue; }
            return Err(error);
        }
        if poll[0].revents != 0 { return Ok(()); }
        let (mut stream, _) = match listener.accept() {
            Ok(peer) => peer,
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => continue,
            Err(error) => return Err(error),
        };
        let deadline = Instant::now() + TIMEOUT;
        if write_until(&mut stream, hello.as_bytes(), deadline).is_err() {
            continue;
        }
        let accepted = receive(&mut stream, app_id, deadline).is_ok_and(|activation| inbox.admit(activation));
        let _ = write_until(&mut stream, if accepted { ACK } else { b"zgui-activation-v1:no\n" }, deadline);
    }
}

fn receive(socket: &mut UnixStream, app_id: &str, deadline: Instant) -> io::Result<Activation> {
    let mut size = [0_u8; 4];
    read_until(socket, &mut size, deadline)?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > MAX_FRAME_BYTES { return Err(invalid_frame()); }
    let mut bytes = vec![0; size];
    read_until(socket, &mut bytes, deadline)?;
    if bytes.first() != Some(&b'{') { return Err(invalid_frame()); }
    let frame: Frame<Vec<String>, String> = serde_json::from_slice(&bytes).map_err(|_| invalid_frame())?;
    let info = ActivationInfo { args: frame.args, cwd: frame.cwd };
    if frame.app_id != app_id || info.validate().is_err()
        || frame.activation_token.as_deref().is_some_and(|token| !crate::invocation::valid_token(token))
    {
        return Err(invalid_frame());
    }
    Ok(Activation { info, token: frame.activation_token })
}

fn forward(socket: &mut UnixStream, app_id: &str, activation: &Activation) -> io::Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    let expected = hello(app_id);
    let mut received = [0; MAX_HELLO_BYTES];
    let received = received.get_mut(..expected.len()).ok_or_else(invalid_frame)?;
    read_until(socket, received, deadline)?;
    if received != expected.as_bytes() {
        return Err(invalid_frame());
    }

    let frame = Frame {
        app_id,
        args: &activation.info.args,
        cwd: activation.info.cwd.as_deref(),
        activation_token: activation.token.as_deref(),
    };
    let bytes = serde_json::to_vec(&frame)?;
    let size = u32::try_from(bytes.len()).ok().filter(|_| bytes.len() <= MAX_FRAME_BYTES).ok_or_else(invalid_frame)?;
    write_until(socket, &size.to_be_bytes(), deadline)?;
    write_until(socket, &bytes, deadline)?;
    let mut ack = [0; ACK.len()];
    read_until(socket, &mut ack, deadline)?;
    if ack == ACK { Ok(()) } else { Err(io::Error::other("primary rejected activation")) }
}

fn hello(app_id: &str) -> String {
    format!("{HELLO_PREFIX}{app_id}\n")
}

fn invalid_frame() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "invalid activation frame") }

fn remaining(deadline: Instant) -> io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() { Err(io::Error::new(io::ErrorKind::TimedOut, "activation socket deadline exceeded")) }
    else { Ok(remaining) }
}

fn read_until(socket: &mut UnixStream, mut data: &mut [u8], deadline: Instant) -> io::Result<()> {
    while !data.is_empty() {
        socket.set_read_timeout(Some(remaining(deadline)?))?;
        match socket.read(data) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => data = &mut data[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn write_until(socket: &mut UnixStream, mut data: &[u8], deadline: Instant) -> io::Result<()> {
    while !data.is_empty() {
        socket.set_write_timeout(Some(remaining(deadline)?))?;
        match socket.write(data) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => data = &data[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
