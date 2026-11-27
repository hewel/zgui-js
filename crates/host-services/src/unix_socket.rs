//! Bounded Unix stream clients which exchange only owned data with their caller.

use crate::{
    ServiceError,
    frame_stream::{self, FrameBudget, FrameStream, Modifiers, protocol},
    worker::{Publisher, UNWATCHED, Worker, pollfd},
};
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, recvmsg};
use std::{
    collections::VecDeque,
    fmt, fs,
    io::{self, IoSlice, IoSliceMut, Write},
    mem,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileTypeExt, OpenOptionsExt},
            net::UnixStream,
        },
    },
    path::{Path, PathBuf},
    ptr,
    sync::{Arc, Mutex},
};

/// Maximum data bytes in an incoming or outgoing frame, excluding framing.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SENDS: usize = 32;
const TEXT_KIND: u8 = 0x00;
const IMAGE_KIND: u8 = 0x01;
const STREAM_KIND: u8 = 0x02;

/// Delimits messages on a Unix stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// UTF-8 text terminated by LF, which is not part of the delivered text.
    Line,
    /// A big-endian `u32` byte count including a kind byte (text, image or opted-in frame submit).
    Length,
}

impl Framing {
    /// Checks whether a text message can be sent with this framing.
    ///
    /// # Errors
    ///
    /// Returns `TooLarge` for an oversized message and `InvalidMessage` for an
    /// embedded LF in line framing.
    pub fn validate_send(self, message: &str) -> Result<(), ServiceError> {
        if message.len() > MAX_MESSAGE_BYTES {
            return Err(too_large());
        }
        if self == Self::Line && message.contains('\n') {
            return Err(ServiceError::new(
                "InvalidMessage",
                "line messages cannot contain LF",
            ));
        }
        Ok(())
    }
}

/// One ordered notification from the peer.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A complete UTF-8 message, excluding the wire delimiter.
    Message(String),
    /// A length frame tagged as image data; image decoding occurs separately.
    Image(Arc<[u8]>),
    /// The first pending submission opened this stream ID.
    FrameStream(u64),
    /// The final notification, with an error for abnormal closure.
    Close(Option<ServiceError>),
}

/// A bounded Unix connection owned by one caller.
///
/// Dropping requests cancellation and detaches the worker without delivering a
/// cancellation event. Use [`Socket::close`] to wait for the endpoint to close.
pub struct Socket {
    worker: Worker<Event>,
    outgoing: Arc<Mutex<Outgoing>>,
    framing: Framing,
    frames: Option<Arc<frame_stream::Endpoint>>,
}

impl fmt::Debug for Socket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Socket")
            .field("framing", &self.framing)
            .field("active", &self.is_active())
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Outgoing {
    queue: VecDeque<String>,
    bytes: usize,
    closed: bool,
}

impl Socket {
    /// Starts a nonblocking connection to an exact, canonical Unix socket path.
    ///
    /// The caller must authorize the exact path first. On Linux the worker
    /// validates and pins the socket inode through procfs before connecting.
    /// `wake` runs on the worker after an event has been published and must not block.
    ///
    /// # Errors
    ///
    /// An invalid path shape or worker setup failure returns an I/O error.
    /// Filesystem, connection and protocol failures arrive as [`Event::Close`].
    pub fn connect(
        path: PathBuf,
        framing: Framing,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        Self::start(path, framing, None, wake)
    }

    /// Starts a length-framed connection which accepts SCM_RIGHTS frame submissions.
    ///
    /// Pass the same budget to all frame sockets in the session. `None` advertises memfd
    /// only; otherwise `modifiers` lists the renderer's supported imports.
    ///
    /// # Errors
    /// Returns the same setup errors as [`Self::connect`].
    pub fn connect_frames(
        path: PathBuf,
        budget: FrameBudget,
        modifiers: Option<Modifiers>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        let frames = Arc::new(frame_stream::Endpoint::new(budget, modifiers, wake.clone()));
        Self::start(path, Framing::Length, Some(frames), wake)
    }

    fn start(
        path: PathBuf,
        framing: Framing,
        frames: Option<Arc<frame_stream::Endpoint>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected an absolute Unix socket path without NUL",
            ));
        }
        let outgoing = Arc::new(Mutex::new(Outgoing::default()));
        let worker_outgoing = Arc::clone(&outgoing);
        let worker_frames = frames.clone();
        let worker = Worker::start(
            "zgui-unix-socket",
            MAX_MESSAGE_BYTES,
            wake,
            move |publisher| {
                let result = connect_and_serve(
                    &path,
                    framing,
                    &worker_outgoing,
                    worker_frames.as_ref(),
                    publisher,
                );
                if let Some(frames) = worker_frames {
                    frames.close();
                }
                {
                    let mut outgoing = worker_outgoing
                        .lock()
                        .expect("outgoing lock is not held during I/O");
                    *outgoing = Outgoing {
                        closed: true,
                        ..Outgoing::default()
                    };
                }
                Some(Event::Close(result.err()))
            },
        )?;
        if let Some(frames) = &frames {
            let _ = frames.signal.set(worker.signal_socket()?);
        }
        Ok(Self {
            worker,
            outgoing,
            framing,
            frames,
        })
    }

    /// Gets a stream opened by the peer. Closed sockets cannot supply new frames.
    pub fn frame_stream(&self, id: u64) -> Option<Arc<FrameStream>> {
        self.frames.as_ref()?.stream(id)
    }

    /// Queues one message without waiting for the socket to become writable.
    ///
    /// # Errors
    ///
    /// In addition to framing errors, returns `Closed` for a closed worker or
    /// `Busy` if the bounded queue cannot accept the message.
    pub fn send(&self, message: &str) -> Result<(), ServiceError> {
        self.framing.validate_send(message)?;
        let mut outgoing = self
            .outgoing
            .lock()
            .expect("outgoing lock is not held during I/O");
        if outgoing.closed || !self.worker.active() {
            return Err(ServiceError::new("Closed", "Unix socket is closed"));
        }
        if outgoing.queue.len() == MAX_SENDS || message.len() > MAX_MESSAGE_BYTES - outgoing.bytes {
            return Err(ServiceError::new("Busy", "Unix socket send queue is full"));
        }
        outgoing.bytes += message.len();
        outgoing.queue.push_back(message.to_owned());
        drop(outgoing);
        self.worker.signal();
        Ok(())
    }

    /// Cancels the connection and blocks until its worker has closed the endpoint.
    ///
    /// Queued sends are not flushed, and no cancellation event is delivered.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the worker panicked.
    pub fn close(self) -> io::Result<()> {
        self.worker.close()
    }

    /// Stops callback delivery without joining. Frame sockets flush held producer credits
    /// before the endpoint closes, subject to a bounded write grace after GPU use ends.
    pub fn retire(self) {
        if let Some(frames) = self.frames {
            frames.retire();
            self.worker.detach();
        }
    }

    /// Returns the next event if one is ready, without waiting.
    pub fn try_recv(&self) -> Option<Event> {
        self.worker.take()
    }

    /// Returns whether the worker is connecting or still owns the endpoint.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.worker.active()
    }
}

fn connect_and_serve(
    path: &Path,
    framing: Framing,
    outgoing: &Mutex<Outgoing>,
    frames: Option<&Arc<frame_stream::Endpoint>>,
    publisher: &Publisher<Event>,
) -> Result<(), ServiceError> {
    if publisher.stopped() {
        return Ok(());
    }
    let inode = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
        .open(path)?;
    // Validate the inode's exact path, then connect without resolving it again.
    let endpoint = PathBuf::from(format!("/proc/self/fd/{}", inode.as_raw_fd()));
    if !inode.metadata()?.file_type().is_socket()
        || fs::read_link(&endpoint)?.as_os_str() != path.as_os_str()
    {
        return Err(ServiceError::new(
            "InvalidPath",
            "endpoint must be an exact canonical path to a Unix socket",
        ));
    }
    if publisher.stopped() {
        return Ok(());
    }
    let (stream, pending) = nonblocking_connect(&endpoint)?;
    drop(inode);
    if pending {
        let mut fds = [UNWATCHED, pollfd(&stream, libc::POLLOUT)];
        while !publisher.stopped() {
            publisher.poll(&mut fds, -1)?;
            if fds[1].revents != 0 {
                if let Some(error) = stream.take_error()? {
                    return Err(error.into());
                }
                break;
            }
        }
    }
    serve(&stream, framing, outgoing, frames, publisher)
}

pub(super) fn nonblocking_connect(path: &Path) -> io::Result<(UnixStream, bool)> {
    // SAFETY: sockaddr_un has no fields with invalid all-zero representations.
    let mut address: libc::sockaddr_un = unsafe { mem::zeroed() };
    let path = path.as_os_str().as_bytes();
    if path.len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unix socket path exceeds the platform address limit",
        ));
    }
    address.sun_family = libc::sa_family_t::try_from(libc::AF_UNIX).map_err(io::Error::other)?;
    // SAFETY: the destination has room for these bytes and its zero terminator.
    unsafe {
        ptr::copy_nonoverlapping(
            path.as_ptr(),
            address.sun_path.as_mut_ptr().cast(),
            path.len(),
        )
    };
    let length =
        libc::socklen_t::try_from(mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1)
            .map_err(io::Error::other)?;
    // SAFETY: the socket accepts these type flags and on success returns a fresh fd.
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the freshly created file descriptor is owned solely by this function.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: the initialized Unix address has at least `length` readable bytes.
    let result = unsafe { libc::connect(fd.as_raw_fd(), ptr::from_ref(&address).cast(), length) };
    let pending = if result == 0 {
        false
    } else {
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::EINPROGRESS | libc::EINTR)) {
            return Err(error);
        }
        true
    };
    Ok((UnixStream::from(fd), pending))
}

fn serve(
    stream: &UnixStream,
    framing: Framing,
    outgoing: &Mutex<Outgoing>,
    frames: Option<&Arc<frame_stream::Endpoint>>,
    publisher: &Publisher<Event>,
) -> Result<(), ServiceError> {
    let mut buffer = [0; 8192];
    let (mut start, mut end) = (0, 0);
    let mut decoder = Decoder::new(framing);
    let mut pending_event = None;
    let mut outgoing_frame = frames
        .map(|frames| Frame::new(frames.hello(), framing, STREAM_KIND))
        .transpose()?;
    let mut flush_deadline = None;
    while !publisher.stopped() {
        let retiring = frames.is_some_and(|frames| frames.retiring());
        if retiring {
            decoder.fds.clear();
        }
        let waiting_for_fences = frames
            .filter(|_| !retiring)
            .map(|frames| frames.prepare())
            .transpose()?
            .unwrap_or(false);
        let mut progressed = false;
        if !retiring && let Some((event, bytes)) = pending_event.take() {
            pending_event = publisher.offer(event, bytes).err();
            progressed = pending_event.is_none();
        }
        if !retiring
            && pending_event.is_none()
            && frames.is_none_or(|frames| frames.allow_receive())
        {
            if start == end {
                let limit = frames.map_or(buffer.len(), |_| decoder.recv_limit().min(buffer.len()));
                match receive(stream, &mut buffer[..limit]) {
                    Ok((_, fds))
                        if !fds.is_empty() && (frames.is_none() || decoder.is_partial()) =>
                    {
                        return Err(protocol(
                            "SCM_RIGHTS must accompany the first byte of an opted-in frame",
                        ));
                    }
                    Ok((0, _)) if decoder.is_partial() => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "peer closed in the middle of a frame",
                        )
                        .into());
                    }
                    Ok((0, _)) => return Ok(()),
                    Ok((size, fds)) => {
                        start = 0;
                        end = size;
                        if !fds.is_empty() {
                            decoder.fds = fds;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => progressed = true,
                    Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                        return Err(protocol("truncated Unix socket ancillary data"));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            if start != end {
                let (consumed, event) = decoder.read(&buffer[start..end], frames)?;
                start += consumed;
                pending_event =
                    event.and_then(|(event, bytes)| publisher.offer(event, bytes).err());
                progressed = true;
            }
        }
        if outgoing_frame.is_none()
            && let Some(frames) = frames
            && let Some(message) = frames.next_release()?
        {
            outgoing_frame = Some(Frame::new(message, framing, STREAM_KIND)?);
        }
        if !retiring && outgoing_frame.is_none() {
            let mut outgoing = outgoing
                .lock()
                .expect("outgoing lock is not held during I/O");
            if let Some(message) = outgoing.queue.pop_front() {
                outgoing.bytes -= message.len();
                drop(outgoing);
                outgoing_frame = Some(Frame::new(message, framing, TEXT_KIND)?);
            }
        }
        if let Some(frame) = outgoing_frame.as_mut() {
            match frame.write(stream, framing) {
                Ok(true) => {
                    outgoing_frame = None;
                    progressed = true;
                }
                Ok(false) => progressed = true,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => progressed = true,
                Err(error) => return Err(error.into()),
            }
        }
        if let Some(frames) = frames
            && retiring
            && frames.claims_done()
        {
            if outgoing_frame.is_none() && frames.credits_sent() {
                return Ok(());
            }
            let now = std::time::Instant::now();
            let deadline =
                flush_deadline.get_or_insert_with(|| now + std::time::Duration::from_millis(250));
            if now >= *deadline {
                return Ok(());
            }
        }
        if !progressed {
            let mut interest = 0;
            if !retiring
                && pending_event.is_none()
                && frames.is_none_or(|frames| frames.allow_receive())
            {
                interest |= libc::POLLIN;
            }
            if outgoing_frame.is_some() {
                interest |= libc::POLLOUT;
            }
            let data = if interest == 0 {
                UNWATCHED
            } else {
                pollfd(stream, interest)
            };
            publisher.poll(
                &mut [UNWATCHED, data],
                if flush_deadline.is_some() {
                    250
                } else if waiting_for_fences {
                    5
                } else {
                    -1
                },
            )?;
        }
    }
    Ok(())
}

fn receive(stream: &UnixStream, buffer: &mut [u8]) -> io::Result<(usize, Vec<OwnedFd>)> {
    let mut control =
        [mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(frame_stream::MAX_PLANES))];
    let mut control = RecvAncillaryBuffer::new(&mut control);
    let msg = recvmsg(
        stream,
        &mut [IoSliceMut::new(buffer)],
        &mut control,
        RecvFlags::CMSG_CLOEXEC,
    )?;
    // Rustix owns the ancillary buffer: even on truncation, its drain/drop closes every
    // installed descriptor. Linux closes rights that did not fit in the control buffer.
    if msg.flags.contains(ReturnFlags::CTRUNC) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut rights = Vec::new();
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(fds) = message {
            rights.extend(fds);
        }
    }
    Ok((msg.bytes, rights))
}

struct Frame {
    message: String,
    header: [u8; 5],
    offset: usize,
}

impl Frame {
    fn new(message: String, framing: Framing, kind: u8) -> Result<Self, ServiceError> {
        let mut header = [0; 5];
        if framing == Framing::Length {
            let length = u32::try_from(message.len() + 1).map_err(|_| too_large())?;
            header[..4].copy_from_slice(&length.to_be_bytes());
            header[4] = kind;
        }
        Ok(Self {
            message,
            header,
            offset: 0,
        })
    }

    fn write(&mut self, mut stream: &UnixStream, framing: Framing) -> io::Result<bool> {
        let (first, second): (&[u8], &[u8]) = match framing {
            Framing::Line => (self.message.as_bytes(), b"\n"),
            Framing::Length => (&self.header, self.message.as_bytes()),
        };
        let written = if self.offset < first.len() {
            stream.write_vectored(&[IoSlice::new(&first[self.offset..]), IoSlice::new(second)])?
        } else {
            stream.write(&second[self.offset - first.len()..])?
        };
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write Unix socket frame",
            ));
        }
        self.offset += written;
        Ok(self.offset == first.len() + second.len())
    }
}

type Decoded = (usize, Option<(Event, usize)>);

struct Decoder {
    frame: Vec<u8>,
    length: Option<LengthHeader>,
    fds: Vec<OwnedFd>,
}

#[derive(Default)]
struct LengthHeader {
    prefix: [u8; 4],
    filled: usize,
    size: usize,
    kind: Option<FrameKind>,
}

#[derive(Clone, Copy)]
enum FrameKind {
    Text,
    Image,
    Stream,
}

impl Decoder {
    fn new(framing: Framing) -> Self {
        Self {
            frame: Vec::new(),
            length: (framing == Framing::Length).then(LengthHeader::default),
            fds: Vec::new(),
        }
    }

    fn is_partial(&self) -> bool {
        !self.frame.is_empty()
            || self
                .length
                .as_ref()
                .is_some_and(|header| header.filled != 0)
    }

    // A stream recvmsg reports an ancillary barrier but not the byte offset. Reading the
    // first byte separately, and never reading across the boundary, makes the pairing exact.
    fn recv_limit(&self) -> usize {
        let header = self
            .length
            .as_ref()
            .expect("frame streams use length framing");
        match (header.filled, header.kind) {
            (0, _) => 1,
            (1..4, _) => 4 - header.filled,
            (_, None) => 1,
            (_, Some(_)) => header.size - self.frame.len(),
        }
    }

    fn read(
        &mut self,
        input: &[u8],
        endpoint: Option<&Arc<frame_stream::Endpoint>>,
    ) -> Result<Decoded, ServiceError> {
        if let Some(header) = &mut self.length {
            let mut consumed = 0;
            if header.filled < header.prefix.len() {
                consumed = input.len().min(header.prefix.len() - header.filled);
                header.prefix[header.filled..header.filled + consumed]
                    .copy_from_slice(&input[..consumed]);
                header.filled += consumed;
                if header.filled != header.prefix.len() {
                    return Ok((consumed, None));
                }
                let wire_size =
                    usize::try_from(u32::from_be_bytes(header.prefix)).map_err(|_| too_large())?;
                header.size = wire_size.checked_sub(1).ok_or_else(|| {
                    ServiceError::new("InvalidMessage", "Unix socket frame has no kind byte")
                })?;
                if header.size > MAX_MESSAGE_BYTES {
                    return Err(too_large());
                }
            }
            let kind = match header.kind {
                Some(kind) => kind,
                None => {
                    let Some(byte) = input.get(consumed) else {
                        return Ok((consumed, None));
                    };
                    consumed += 1;
                    let kind = match *byte {
                        TEXT_KIND => FrameKind::Text,
                        IMAGE_KIND => FrameKind::Image,
                        STREAM_KIND if endpoint.is_some() => FrameKind::Stream,
                        STREAM_KIND => {
                            return Err(protocol("frame streams were not enabled on this socket"));
                        }
                        _ => {
                            return Err(ServiceError::new(
                                "InvalidMessage",
                                "Unix socket frame has an unknown kind byte",
                            ));
                        }
                    };
                    if matches!(kind, FrameKind::Stream) {
                        if self.fds.is_empty() || header.size > frame_stream::MAX_HEADER {
                            return Err(protocol(
                                "frame submit requires descriptors and bounded header data",
                            ));
                        }
                    } else if !self.fds.is_empty() {
                        return Err(protocol("SCM_RIGHTS sent with a non-frame message"));
                    }
                    header.kind = Some(kind);
                    kind
                }
            };
            let data = (input.len() - consumed).min(header.size - self.frame.len());
            self.frame
                .extend_from_slice(&input[consumed..consumed + data]);
            consumed += data;
            if self.frame.len() != header.size {
                return Ok((consumed, None));
            }
            let size = header.size;
            header.filled = 0;
            header.kind = None;
            let payload = mem::take(&mut self.frame);
            let event = match kind {
                FrameKind::Text => Event::Message(String::from_utf8(payload).map_err(|_| {
                    ServiceError::new("InvalidUtf8", "Unix socket text frame is not valid UTF-8")
                })?),
                FrameKind::Image => Event::Image(Arc::from(payload)),
                FrameKind::Stream => {
                    let endpoint = endpoint.expect("stream kind requires an opted-in connection");
                    let frame =
                        frame_stream::Inbound::new(&payload, mem::take(&mut self.fds), endpoint)?;
                    return Ok((
                        consumed,
                        endpoint.offer(frame)?.map(|id| (Event::FrameStream(id), 0)),
                    ));
                }
            };
            Ok((consumed, Some((event, size))))
        } else {
            let newline = input.iter().position(|byte| *byte == b'\n');
            let bytes = newline.unwrap_or(input.len());
            if bytes > MAX_MESSAGE_BYTES - self.frame.len() {
                return Err(too_large());
            }
            self.frame.extend_from_slice(&input[..bytes]);
            if newline.is_none() {
                return Ok((bytes, None));
            }
            let payload = mem::take(&mut self.frame);
            let size = payload.len();
            let message = String::from_utf8(payload).map_err(|_| {
                ServiceError::new("InvalidUtf8", "Unix socket line is not valid UTF-8")
            })?;
            Ok((bytes + 1, Some((Event::Message(message), size))))
        }
    }
}

fn too_large() -> ServiceError {
    ServiceError::new("TooLarge", "Unix socket message exceeds the byte limit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{
        MAX_EVENTS,
        tests::{DEADLINE, TempDir, observe, waiter},
    };
    use std::{
        io::Read,
        os::unix::{fs::symlink, net::UnixListener},
        thread,
        time::{Duration, Instant},
    };

    fn accept(listener: &UnixListener) -> UnixStream {
        listener
            .set_nonblocking(true)
            .expect("set a deadline-compatible test listener");
        let deadline = Instant::now() + DEADLINE;
        loop {
            match listener.accept() {
                Ok((stream, _)) => return stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "worker must connect to the test listener"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("test listener failed to accept: {error}"),
            }
        }
    }

    #[test]
    fn line_socket_exchanges_text_and_delivers_eof_after_the_messages() {
        let dir = TempDir::new();
        let path = dir.join("peer.sock");
        let listener = UnixListener::bind(&path).expect("bind a local socket for the line peer");
        let (wake, inbox) = waiter();
        let socket = Socket::connect(path, Framing::Line, wake).expect("start line client");
        assert_eq!(
            socket
                .send("one\ntwo")
                .expect_err("LF would inject a second frame")
                .code,
            "InvalidMessage",
            "line send must reject embedded LF"
        );
        socket
            .send("hé")
            .expect("queue non-ASCII line before connection completes");
        let mut peer = accept(&listener);
        peer.set_read_timeout(Some(Duration::from_secs(5)))
            .expect("bound the peer read");
        let mut wire = [0; 4];
        peer.read_exact(&mut wire)
            .expect("read one line from client");
        assert_eq!(
            &wire, b"h\xc3\xa9\n",
            "line framing uses UTF-8 with exactly one LF"
        );
        peer.write_all(b"first\n\nlast\n")
            .expect("send three framed lines");
        drop(peer);
        for expected in ["first", "", "last"] {
            assert_eq!(
                observe(&inbox, || socket.try_recv()),
                Event::Message(expected.into()),
                "line events preserve wire order and empty frames"
            );
        }
        assert_eq!(
            observe(&inbox, || socket.try_recv()),
            Event::Close(None),
            "clean EOF delivers exactly one successful close after text"
        );
        assert!(
            !socket.is_active(),
            "close must only publish after the endpoint has closed"
        );
    }

    #[test]
    fn length_socket_exchanges_tagged_frames_and_rejects_an_oversized_header() {
        let dir = TempDir::new();
        let path = dir.join("peer.sock");
        let listener = UnixListener::bind(&path).expect("bind a local socket for the length peer");
        let (wake, inbox) = waiter();
        let socket = Socket::connect(path, Framing::Length, wake).expect("start length client");
        socket
            .send("é\nlast")
            .expect("length-framed messages can contain LF");
        let mut peer = accept(&listener);
        peer.set_read_timeout(Some(Duration::from_secs(5)))
            .expect("bound the peer read");
        let mut wire = [0; 12];
        peer.read_exact(&mut wire)
            .expect("read one length-prefixed UTF-8 message");
        assert_eq!(
            &wire, b"\0\0\0\x08\0\xc3\xa9\nlast",
            "length framing counts the kind byte and uses a big-endian prefix"
        );
        peer.write_all(b"\0\0")
            .expect("send the first part of the length header");
        peer.write_all(b"\0\x04")
            .expect("send the rest of the length header");
        peer.write_all(b"\0\xc3")
            .expect("send the text kind and first UTF-8 byte");
        peer.write_all(b"\xa9!\0\0\0\x01\0")
            .expect("finish the UTF-8 text and send an empty text frame");
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#;
        let image_size = u32::try_from(svg.len() + 1).expect("SVG fits the wire header");
        peer.write_all(&image_size.to_be_bytes())
            .expect("send the SVG frame size");
        peer.write_all(b"\x01").expect("send the image kind");
        peer.write_all(svg).expect("send UTF-8 SVG bytes");
        let oversize =
            u32::try_from(MAX_MESSAGE_BYTES + 2).expect("socket limit fits the wire header");
        peer.write_all(&oversize.to_be_bytes())
            .expect("send only an oversized length header");
        assert_eq!(
            observe(&inbox, || socket.try_recv()),
            Event::Message("é!".into()),
            "UTF-8 length frames remain textual"
        );
        assert_eq!(
            observe(&inbox, || socket.try_recv()),
            Event::Message(String::new()),
            "the kind alone can frame empty text"
        );
        assert_eq!(
            observe(&inbox, || socket.try_recv()),
            Event::Image(Arc::from(svg.as_slice())),
            "the image kind routes SVG bytes even when they are valid UTF-8"
        );
        let Event::Close(Some(error)) = observe(&inbox, || socket.try_recv()) else {
            panic!("expected an ordered error close for an oversized length header");
        };
        assert_eq!(
            error.code, "TooLarge",
            "an excessive length must be rejected before its body arrives"
        );
    }

    #[test]
    fn length_socket_closes_on_invalid_kind_or_text_without_delivering_an_image() {
        for (wire, code, eof) in [
            (&b"\0\0\0\0"[..], "InvalidMessage", false),
            (&b"\0\0\0\x04\x03"[..], "InvalidMessage", false),
            (&b"\0\0\0\x04\x02"[..], "E_PROTOCOL", false),
            (&b"\0\0\0\x03\0\xffx"[..], "InvalidUtf8", false),
            (&b"\0\0\0\x01"[..], "Io", true),
        ] {
            let dir = TempDir::new();
            let path = dir.join("peer.sock");
            let listener = UnixListener::bind(&path).expect("bind an invalid-frame test peer");
            let (wake, inbox) = waiter();
            let socket = Socket::connect(path, Framing::Length, wake).expect("start length client");
            let mut peer = accept(&listener);
            peer.write_all(wire).expect("send invalid frame");
            if eof {
                peer.shutdown(std::net::Shutdown::Write)
                    .expect("end the stream before sending the kind");
            }
            let Event::Close(Some(error)) = observe(&inbox, || socket.try_recv()) else {
                panic!("bad frame {wire:?} must close without delivering a message or image");
            };
            assert_eq!(error.code, code, "bad frame {wire:?} reports its error");
        }
    }

    #[test]
    fn length_socket_budget_excludes_header_and_kind() {
        let dir = TempDir::new();
        let path = dir.join("peer.sock");
        let listener = UnixListener::bind(&path).expect("bind the byte-limit test peer");
        let (wake, inbox) = waiter();
        let socket = Socket::connect(path, Framing::Length, wake).expect("start length client");
        let mut peer = accept(&listener);
        peer.set_write_timeout(Some(DEADLINE))
            .expect("bound the peer write");
        peer.set_read_timeout(Some(DEADLINE))
            .expect("bound the peer read");
        let mut payload = "x".repeat(MAX_MESSAGE_BYTES + 1);
        assert_eq!(
            socket
                .send(&payload)
                .expect_err("reject text over 8 MiB before queuing")
                .code,
            "TooLarge"
        );
        payload.pop().expect("remove the byte over budget");
        socket.send(&payload).expect("queue exactly 8 MiB of text");
        let mut header = [0; 5];
        peer.read_exact(&mut header)
            .expect("read the largest outgoing frame header");
        assert_eq!(header, [0, 0x80, 0, 1, 0], "header counts data and kind");
        let mut chunk = [0; 8192];
        for expected in payload.as_bytes().chunks(chunk.len()) {
            let actual = &mut chunk[..expected.len()];
            peer.read_exact(actual).expect("read outgoing data chunk");
            assert!(&*actual == expected, "preserve data across partial writes");
        }

        let size = u32::try_from(MAX_MESSAGE_BYTES + 1).expect("socket limit fits the wire header");
        peer.write_all(&size.to_be_bytes())
            .expect("send the largest permitted frame size");
        peer.write_all(b"\0").expect("send the text kind");
        peer.write_all(payload.as_bytes())
            .expect("send exactly 8 MiB of text across multiple reads");
        let Event::Message(message) = observe(&inbox, || socket.try_recv()) else {
            panic!("8 MiB of length-framed text must be delivered");
        };
        assert!(message == payload, "preserve data across partial reads");
    }

    #[test]
    fn exact_socket_path_rejects_symlinks_and_non_socket_files() {
        let dir = TempDir::new();
        let target = dir.join("peer.sock");
        let _listener = UnixListener::bind(&target).expect("bind target of the forbidden alias");
        let alias = dir.join("alias.sock");
        symlink(&target, &alias).expect("create a symlink to the socket");
        let parent_alias = dir.join("parent");
        symlink(
            target.parent().expect("test socket has a parent"),
            &parent_alias,
        )
        .expect("create alias for the socket's parent");
        let file = dir.join("plain-file");
        fs::write(&file, b"not a socket").expect("create a non-socket endpoint");
        for path in [alias, parent_alias.join("peer.sock"), file] {
            let (wake, inbox) = waiter();
            let socket = Socket::connect(path, Framing::Line, wake)
                .expect("dispatch endpoint validation to worker");
            let Event::Close(Some(error)) = observe(&inbox, || socket.try_recv()) else {
                panic!("expected the worker to close a forbidden endpoint");
            };
            assert_eq!(
                error.code, "InvalidPath",
                "only an exact canonical Unix socket can be opened"
            );
        }
    }

    #[test]
    fn explicitly_closing_a_backpressured_socket_waits_for_its_fd() {
        let dir = TempDir::new();
        let path = dir.join("peer.sock");
        let listener = UnixListener::bind(&path).expect("bind peer for the backpressure test");
        let (wake, inbox) = waiter();
        let socket =
            Socket::connect(path, Framing::Line, wake).expect("start backpressured client");
        let mut peer = accept(&listener);
        peer.set_read_timeout(Some(Duration::from_secs(5)))
            .expect("bound peer closure detection");
        peer.write_all(&b"\n".repeat(MAX_EVENTS + 2))
            .expect("put more frames on the wire than the event queue can retain");
        for _ in 0..MAX_EVENTS {
            inbox
                .recv_timeout(DEADLINE)
                .expect("fill the event queue without consuming any events");
        }
        socket.close().expect("join the backpressured worker");
        let result = peer.read(&mut [0; 1]);
        assert!(
            matches!(result, Ok(0))
                || result.is_err_and(|error| error.kind() == io::ErrorKind::ConnectionReset),
            "close waits for the actual endpoint even when nobody drains events"
        );
    }
}
