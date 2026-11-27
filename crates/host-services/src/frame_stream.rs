//! Validated frame submissions and their bounded producer credits.

use crate::ServiceError;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, VecDeque},
    fs::File,
    io::{self, Write},
    os::{
        fd::{BorrowedFd, OwnedFd},
        unix::{fs::FileExt, net::UnixStream},
    },
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

const MAX_STREAMS: usize = 8;
pub(super) const MAX_PLANES: usize = 4;
pub(super) const MAX_HEADER: usize = 4096;
const MAX_ID: u64 = (1 << 53) - 1;
const MAX_CREDITS: usize = 128;

/// Device-validated DRM modifiers, grouped by wire pixel format.
pub type Modifiers = BTreeMap<&'static str, Vec<String>>;

/// A budget shared by every opted-in connection in one session.
#[derive(Clone, Debug, Default)]
pub struct FrameBudget(Arc<Usage>);

#[derive(Debug, Default)]
struct Usage {
    streams: AtomicUsize,
    reserved: AtomicUsize,
    fds: AtomicUsize,
}

impl FrameBudget {
    /// Active streams across the session's frame sockets.
    pub fn streams(&self) -> usize {
        self.0.streams.load(Ordering::Relaxed)
    }
    /// Received descriptors still owned by pending or displayed frames.
    pub fn fds(&self) -> usize {
        self.0.fds.load(Ordering::Relaxed)
    }

    fn admit(&self) -> Result<Permit, ServiceError> {
        self.0
            .reserved
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < MAX_STREAMS).then_some(n + 1)
            })
            .map_err(|_| protocol("session frame stream limit reached"))?;
        self.0.streams.fetch_add(1, Ordering::Relaxed);
        Ok(Permit {
            budget: self.clone(),
            active: true,
        })
    }
}

#[derive(Debug)]
struct Permit {
    budget: FrameBudget,
    active: bool,
}

impl Permit {
    fn retire(&mut self) {
        if std::mem::take(&mut self.active) {
            self.budget.0.streams.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.retire();
        self.budget.0.reserved.fetch_sub(1, Ordering::Relaxed);
    }
}

/// How the pixel bytes are arranged.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PixelFormat {
    /// One straight-alpha RGBA plane.
    Rgba8,
    /// One straight-alpha BGRA plane.
    Bgra8,
    /// A luma plane and an interleaved, 2×2 subsampled chroma plane.
    Nv12,
}

/// The transfer/matrix contract for a frame. NV12 uses studio range.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ColorSpace {
    /// sRGB RGB samples.
    Srgb,
    /// BT.601 YCbCr samples.
    Bt601,
    /// BT.709 YCbCr samples.
    Bt709,
}

/// Which kernel-backed storage a submission names.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// CPU-readable regular shared memory.
    Memfd,
    /// Linux DMA-BUF memory.
    Dmabuf,
}

/// Where the first byte and subsequent rows of one plane live.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Plane {
    /// Index into the ancillary SCM_RIGHTS list.
    pub fd_index: usize,
    /// Byte offset from the start of the descriptor.
    pub offset: u64,
    /// Bytes between consecutive rows.
    pub stride: u64,
}

/// Metadata for one complete wire submit.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FrameHeader {
    /// Producer's socket-local stream identifier.
    pub stream_id: u64,
    /// Producer's stream-local credit identifier.
    pub frame_id: u64,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Storage kind.
    pub transport: Transport,
    /// Sample order and plane count.
    pub pixel_format: PixelFormat,
    /// Optional transfer or matrix; defaults to sRGB for RGB and BT.601 for NV12.
    pub color_space: Option<ColorSpace>,
    /// DRM sample format for imported frames.
    pub drm_fourcc: Option<u32>,
    /// Lossless decimal or `0x` hexadecimal DRM modifier.
    pub drm_modifier: Option<String>,
    /// Plane layout in format order.
    pub planes: Vec<Plane>,
}

impl FrameHeader {
    /// Decodes the lossless wire modifier.
    pub fn modifier(&self) -> Option<u64> {
        let value = self.drm_modifier.as_deref()?;
        match value.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16).ok(),
            None => value.parse().ok(),
        }
    }

    fn shape(&self, index: usize) -> (u32, u32) {
        match self.pixel_format {
            PixelFormat::Rgba8 | PixelFormat::Bgra8 => (self.width * 4, self.height),
            PixelFormat::Nv12 if index == 0 => (self.width, self.height),
            PixelFormat::Nv12 => (self.width.div_ceil(2) * 2, self.height.div_ceil(2)),
        }
    }

    fn validate(&self, files: &[File], modifiers: Option<&Modifiers>) -> Result<(), ServiceError> {
        if self.stream_id > MAX_ID
            || self.frame_id > MAX_ID
            || !(1..=8192).contains(&self.width)
            || !(1..=8192).contains(&self.height)
            || self.planes.len()
                != if self.pixel_format == PixelFormat::Nv12 {
                    2
                } else {
                    1
                }
            || files.is_empty()
            || files.len() > MAX_PLANES
        {
            return Err(protocol(
                "invalid frame identity, dimensions, or plane count",
            ));
        }
        let (name, fourcc) = match self.pixel_format {
            PixelFormat::Rgba8 => ("rgba8", u32::from_le_bytes(*b"AB24")),
            PixelFormat::Bgra8 => ("bgra8", u32::from_le_bytes(*b"AR24")),
            PixelFormat::Nv12 => ("nv12", u32::from_le_bytes(*b"NV12")),
        };
        if matches!(
            (self.pixel_format, self.color_space),
            (PixelFormat::Nv12, Some(ColorSpace::Srgb))
                | (
                    PixelFormat::Rgba8 | PixelFormat::Bgra8,
                    Some(ColorSpace::Bt601 | ColorSpace::Bt709)
                )
        ) {
            return Err(protocol("colorSpace does not match pixelFormat"));
        }
        if self.transport == Transport::Dmabuf {
            let modifier = self
                .modifier()
                .ok_or_else(|| protocol("DMA-BUF requires a valid DRM modifier"))?;
            if self.drm_fourcc != Some(fourcc)
                || !modifiers
                    .and_then(|m| m.get(name))
                    .is_some_and(|values| values.iter().any(|s| s.parse() == Ok(modifier)))
            {
                return Err(protocol(
                    "unsupported DMA-BUF format or modifier; negotiate memfd",
                ));
            }
        }
        let mut used = 0u8;
        for (index, plane) in self.planes.iter().enumerate() {
            let file = files
                .get(plane.fd_index)
                .ok_or_else(|| protocol("plane fdIndex is out of range"))?;
            used |= 1 << plane.fd_index;
            let (row_bytes, height) = self.shape(index);
            let end = plane
                .stride
                .checked_mul(u64::from(height))
                .and_then(|span| plane.offset.checked_add(span));
            if plane.stride < u64::from(row_bytes) || end.is_none_or(|end| end > i64::MAX as u64) {
                return Err(protocol("invalid frame plane stride or byte range"));
            }
            if self.transport == Transport::Memfd {
                let metadata = file
                    .metadata()
                    .map_err(|_| protocol("cannot stat memfd plane"))?;
                if !metadata.is_file() || end.is_none_or(|end| end > metadata.len()) {
                    return Err(protocol("memfd plane exceeds regular file size"));
                }
            }
        }
        if used != (1 << files.len()) - 1 {
            return Err(protocol("unreferenced frame descriptor"));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct Inbound {
    header: FrameHeader,
    files: Vec<File>,
    budget: FrameBudget,
    software: Option<Vec<u8>>,
    ready: bool,
}

impl Inbound {
    pub(super) fn new(
        bytes: &[u8],
        fds: Vec<OwnedFd>,
        endpoint: &Endpoint,
    ) -> Result<Self, ServiceError> {
        let header: FrameHeader =
            serde_json::from_slice(bytes).map_err(|_| protocol("invalid frame header JSON"))?;
        let files: Vec<_> = fds.into_iter().map(File::from).collect();
        header.validate(&files, endpoint.modifiers.as_ref())?;
        endpoint
            .budget
            .0
            .fds
            .fetch_add(files.len(), Ordering::Relaxed);
        Ok(Self {
            header,
            files,
            budget: endpoint.budget.clone(),
            software: None,
            ready: false,
        })
    }

    fn close_fds(&mut self) {
        let count = self.files.len();
        self.files.clear();
        self.budget.0.fds.fetch_sub(count, Ordering::Relaxed);
    }
}

impl Drop for Inbound {
    fn drop(&mut self) {
        self.close_fds();
    }
}

/// A claimed frame and its ancillary descriptors.
#[derive(Debug)]
pub struct Submission {
    inbound: Inbound,
    credit: FrameCredit,
}

impl Submission {
    /// Validated frame description.
    pub fn header(&self) -> &FrameHeader {
        &self.inbound.header
    }

    /// Borrows the validated ancillary descriptor at `index`.
    pub fn fd(&self, index: usize) -> Option<BorrowedFd<'_>> {
        use std::os::fd::AsFd;
        Some(self.inbound.files.get(index)?.as_fd())
    }

    /// Takes software pixels prepared by the socket worker, closing their backing memfd.
    pub fn take_rgba(&mut self) -> Option<Vec<u8>> {
        let pixels = self.inbound.software.take()?;
        self.inbound.close_fds();
        Some(pixels)
    }

    /// Holds the producer's storage credit independently of the local file descriptors.
    /// Keep it until all GPU reads finish; the last holder returns the credit.
    pub fn credit(&self) -> FrameCredit {
        self.credit.clone()
    }
}

impl Inbound {
    fn read_rgba(&self) -> io::Result<Vec<u8>> {
        let inbound = self;
        let header = &inbound.header;
        if header.transport != Transport::Memfd {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a memfd submission",
            ));
        }
        let width = header.width as usize;
        let height = header.height as usize;
        let mut pixels = vec![0; width * height * 4];
        let row = |plane: Plane, index: usize, bytes: &mut [u8]| {
            inbound.files[plane.fd_index]
                .read_exact_at(bytes, plane.offset + index as u64 * plane.stride)
        };
        match header.pixel_format {
            PixelFormat::Rgba8 | PixelFormat::Bgra8 => {
                let plane = header.planes[0];
                if plane.stride == (width * 4) as u64 {
                    inbound.files[plane.fd_index].read_exact_at(&mut pixels, plane.offset)?;
                } else {
                    for (y, scanline) in pixels.chunks_exact_mut(width * 4).enumerate() {
                        row(plane, y, scanline)?;
                    }
                }
                if header.pixel_format == PixelFormat::Bgra8 {
                    for pixel in pixels.as_chunks_mut::<4>().0 {
                        pixel.swap(0, 2);
                    }
                }
            }
            PixelFormat::Nv12 => {
                let mut luma = vec![0; width];
                let mut chroma = vec![0; width.div_ceil(2) * 2];
                for (y, scanline) in pixels.chunks_exact_mut(width * 4).enumerate() {
                    row(header.planes[0], y, &mut luma)?;
                    if y % 2 == 0 {
                        row(header.planes[1], y / 2, &mut chroma)?;
                    }
                    for (x, pixel) in scanline.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let uv = x / 2 * 2;
                        *pixel = nv12_rgb(
                            luma[x],
                            chroma[uv],
                            chroma[uv + 1],
                            header.color_space.unwrap_or(ColorSpace::Bt601),
                        );
                    }
                }
            }
        }
        Ok(pixels)
    }
}

/// A share of one producer credit. The last share returns the credit.
#[derive(Clone, Debug)]
pub struct FrameCredit {
    _claim: Arc<Claim>,
}

#[derive(Debug)]
struct Claim {
    stream: Arc<FrameStream>,
    frame_id: u64,
}

impl Drop for Claim {
    fn drop(&mut self) {
        {
            let mut slot = self.stream.slot.lock().expect("frame stream lock poisoned");
            slot.in_use -= 1;
            if slot.closed && slot.in_use == 0 && !slot.preparing {
                slot.permit.take();
            }
        }
        if let Some(endpoint) = self.stream.endpoint.upgrade() {
            endpoint.release(self.stream.id, self.frame_id);
            endpoint.claims.fetch_sub(1, Ordering::Release);
            endpoint.signal();
            (endpoint.wake)();
        }
    }
}

/// Converts one studio-range NV12 sample to straight-alpha RGBA8.
pub fn nv12_rgb(y: u8, u: u8, v: u8, color_space: ColorSpace) -> [u8; 4] {
    let y = 298 * (i32::from(y) - 16);
    let u = i32::from(u) - 128;
    let v = i32::from(v) - 128;
    let (rv, gu, gv, bu) = match color_space {
        ColorSpace::Bt709 => (459, 55, 136, 541),
        ColorSpace::Bt601 | ColorSpace::Srgb => (409, 100, 208, 516),
    };
    let quantize = |n: i32| ((n + 128) >> 8).clamp(0, 255) as u8;
    [
        quantize(y + rv * v),
        quantize(y - gu * u - gv * v),
        quantize(y + bu * u),
        255,
    ]
}

/// An active socket-local stream with space for only the displayed and latest pending frame.
#[derive(Debug)]
pub struct FrameStream {
    id: u64,
    slot: Mutex<Slot>,
    endpoint: Weak<Endpoint>,
}

#[derive(Debug)]
struct Slot {
    pending: Option<Inbound>,
    in_use: usize,
    closed: bool,
    prepare_requested: bool,
    preparing: bool,
    permit: Option<Permit>,
    size: (u32, u32),
}

impl FrameStream {
    /// Claims the latest frame. Whilst two claims are outstanding the worker releases
    /// any new submission rather than retaining a third. Dropping a claim returns its credit.
    pub fn take_pending(self: &Arc<Self>) -> Option<Submission> {
        self.claim(false)
    }

    /// Takes a prepared frame, or asks the socket worker to prepare it. A subsequent wake
    /// signals that software pixels or a completed DMA-BUF are ready to take.
    pub fn take_presentable(self: &Arc<Self>) -> Option<Submission> {
        self.claim(true)
    }

    fn claim(self: &Arc<Self>, ready: bool) -> Option<Submission> {
        let mut slot = self.slot.lock().expect("frame stream lock poisoned");
        if ready && slot.pending.as_ref().is_some_and(|frame| !frame.ready) {
            if !slot.prepare_requested {
                slot.prepare_requested = true;
                if let Some(endpoint) = self.endpoint.upgrade() {
                    endpoint.signal();
                }
            }
            return None;
        }
        let inbound = slot.pending.take()?;
        slot.prepare_requested = false;
        slot.in_use += 1;
        if let Some(endpoint) = self.endpoint.upgrade() {
            endpoint.claims.fetch_add(1, Ordering::Relaxed);
        }
        let credit = FrameCredit {
            _claim: Arc::new(Claim {
                stream: self.clone(),
                frame_id: inbound.header.frame_id,
            }),
        };
        Some(Submission { inbound, credit })
    }

    /// The last accepted pixel extent, even while the worker is preparing it.
    pub fn pending_size(&self) -> Option<(u32, u32)> {
        let slot = self.slot.lock().expect("frame stream lock poisoned");
        (!slot.closed).then_some(slot.size)
    }

    // Only the socket worker calls this, so no submit can overtake the frame while reading.
    fn prepare(&self) -> Result<bool, ServiceError> {
        let mut slot = self.slot.lock().expect("frame stream lock poisoned");
        if !slot.prepare_requested {
            return Ok(false);
        }
        let Some(mut frame) = slot.pending.take() else {
            return Ok(false);
        };
        slot.preparing = true;
        drop(slot);
        let result = prepare_frame(&mut frame);
        let ready = frame.ready;
        let mut slot = self.slot.lock().expect("frame stream lock poisoned");
        slot.preparing = false;
        if slot.closed || result.is_err() {
            let frame_id = frame.header.frame_id;
            drop(frame);
            if slot.closed && slot.in_use == 0 {
                slot.permit.take();
            }
            drop(slot);
            self.release(frame_id);
            return result.map(|()| false);
        }
        slot.pending = Some(frame);
        slot.prepare_requested = !ready;
        drop(slot);
        if ready && let Some(endpoint) = self.endpoint.upgrade() {
            (endpoint.wake)();
        }
        Ok(!ready)
    }

    /// Stops taking frames from this stream and immediately retires its pending submission.
    pub fn close(&self) {
        let mut slot = self.slot.lock().expect("frame stream lock poisoned");
        slot.closed = true;
        if let Some(permit) = &mut slot.permit {
            permit.retire();
        }
        let frame_id = slot.pending.take().map(|frame| frame.header.frame_id);
        if slot.in_use == 0 && !slot.preparing {
            slot.permit.take();
        }
        drop(slot);
        if let Some(frame_id) = frame_id {
            self.release(frame_id);
        }
    }

    fn retire(&self, frame: Inbound) {
        let frame_id = frame.header.frame_id;
        drop(frame);
        self.release(frame_id);
    }

    fn release(&self, frame_id: u64) {
        if let Some(endpoint) = self.endpoint.upgrade() {
            endpoint.release(self.id, frame_id);
        }
    }
}

fn prepare_frame(frame: &mut Inbound) -> Result<(), ServiceError> {
    if frame.header.transport == Transport::Memfd {
        frame.software = Some(
            frame
                .read_rgba()
                .map_err(|error| protocol(&format!("cannot read memfd plane: {error}")))?,
        );
        frame.ready = true;
        return Ok(());
    }

    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let mut descriptors = std::array::from_fn::<_, MAX_PLANES, _>(|i| {
        PollFd::new(frame.files.get(i).unwrap_or(&frame.files[0]), PollFlags::IN)
    });
    let active = &mut descriptors[..frame.files.len()];
    let zero = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // DMA-BUF poll(POLLIN) waits for implicit write fences without reading any pixels.
    poll(active, Some(&zero))
        .map_err(|error| protocol(&format!("cannot poll DMA-BUF: {error}")))?;
    if active.iter().any(|fd| {
        fd.revents()
            .intersects(PollFlags::ERR | PollFlags::NVAL | PollFlags::HUP)
    }) {
        return Err(protocol("DMA-BUF fence failed"));
    }
    frame.ready = active.iter().all(|fd| fd.revents().contains(PollFlags::IN));
    Ok(())
}

pub(super) struct Endpoint {
    budget: FrameBudget,
    modifiers: Option<Modifiers>,
    streams: Mutex<BTreeMap<u64, Arc<FrameStream>>>,
    credits: Mutex<Credits>,
    wake: Arc<dyn Fn() + Send + Sync>,
    claims: AtomicUsize,
    retiring: std::sync::atomic::AtomicBool,
    pub(super) signal: OnceLock<UnixStream>,
}

#[derive(Default)]
struct Credits {
    queue: VecDeque<(u64, u64)>,
    closed: bool,
    overflowed: bool,
}

impl Endpoint {
    pub(super) fn new(
        budget: FrameBudget,
        modifiers: Option<Modifiers>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            budget,
            modifiers,
            streams: Mutex::default(),
            credits: Mutex::default(),
            wake,
            claims: AtomicUsize::new(0),
            retiring: std::sync::atomic::AtomicBool::new(false),
            signal: OnceLock::new(),
        }
    }

    pub(super) fn hello(&self) -> String {
        serde_json::json!({
            "type": "hello", "dmabufSupported": self.modifiers.as_ref().is_some_and(|m| !m.is_empty()),
            "supportedFormats": ["rgba8", "bgra8", "nv12"],
            "supportedModifiers": self.modifiers.as_ref().unwrap_or(&Modifiers::new()),
        }).to_string()
    }

    pub(super) fn stream(&self, id: u64) -> Option<Arc<FrameStream>> {
        self.streams
            .lock()
            .expect("frame endpoints lock poisoned")
            .get(&id)
            .cloned()
    }

    pub(super) fn offer(self: &Arc<Self>, frame: Inbound) -> Result<Option<u64>, ServiceError> {
        let id = frame.header.stream_id;
        let size = (frame.header.width, frame.header.height);
        let mut streams = self.streams.lock().expect("frame endpoints lock poisoned");
        if self.retiring() {
            drop(streams);
            let frame_id = frame.header.frame_id;
            drop(frame);
            self.release(id, frame_id);
            return Ok(None);
        }
        // Reap retired entries before admitting an ID, including reuse of the same wire ID.
        streams.retain(|_, s| !s.slot.lock().expect("frame stream lock poisoned").closed);
        if let Some(stream) = streams.get(&id) {
            let mut slot = stream.slot.lock().expect("frame stream lock poisoned");
            // The handle may have closed since retain(). Do not revive it behind the owner's back.
            let discarded = if slot.closed || slot.in_use == 2 {
                Some(frame)
            } else {
                slot.size = size;
                slot.pending.replace(frame)
            };
            let wake = discarded.is_none();
            drop(slot);
            if let Some(frame) = discarded {
                stream.retire(frame);
            }
            if wake {
                (self.wake)();
            }
            return Ok(None);
        }
        let permit = self.budget.admit()?;
        streams.insert(
            id,
            Arc::new(FrameStream {
                id,
                slot: Mutex::new(Slot {
                    pending: Some(frame),
                    in_use: 0,
                    closed: false,
                    prepare_requested: false,
                    preparing: false,
                    permit: Some(permit),
                    size,
                }),
                endpoint: Arc::downgrade(self),
            }),
        );
        Ok(Some(id))
    }

    pub(super) fn retire(&self) {
        let streams = self.streams.lock().expect("frame endpoints lock poisoned");
        for stream in streams.values() {
            stream.close();
        }
        self.retiring.store(true, Ordering::Release);
        drop(streams);
        self.signal();
    }

    pub(super) fn retiring(&self) -> bool {
        self.retiring.load(Ordering::Acquire)
    }

    pub(super) fn claims_done(&self) -> bool {
        self.claims.load(Ordering::Acquire) == 0
    }

    pub(super) fn credits_sent(&self) -> bool {
        self.credits
            .lock()
            .expect("frame credits lock poisoned")
            .queue
            .is_empty()
    }

    pub(super) fn close(&self) {
        self.credits
            .lock()
            .expect("frame credits lock poisoned")
            .closed = true;
        for (_, stream) in
            std::mem::take(&mut *self.streams.lock().expect("frame endpoints lock poisoned"))
        {
            stream.close();
        }
    }

    fn release(&self, stream: u64, frame: u64) {
        let mut credits = self.credits.lock().expect("frame credits lock poisoned");
        if credits.closed {
            return;
        }
        if credits.queue.len() == MAX_CREDITS {
            credits.overflowed = true;
        } else {
            credits.queue.push_back((stream, frame));
        }
        drop(credits);
        self.signal();
    }

    fn signal(&self) {
        if let Some(mut signal) = self.signal.get() {
            while signal
                .write(&[0])
                .is_err_and(|err| err.kind() == io::ErrorKind::Interrupted)
            {}
        }
    }

    pub(super) fn prepare(&self) -> Result<bool, ServiceError> {
        let mut streams = [const { None }; MAX_STREAMS];
        for (out, stream) in streams.iter_mut().zip(
            self.streams
                .lock()
                .expect("frame endpoints lock poisoned")
                .values(),
        ) {
            *out = Some(stream.clone());
        }
        let mut waiting_for_fences = false;
        for stream in streams.into_iter().flatten() {
            waiting_for_fences |= stream.prepare()?;
        }
        Ok(waiting_for_fences)
    }

    pub(super) fn allow_receive(&self) -> bool {
        self.credits
            .lock()
            .expect("frame credits lock poisoned")
            .queue
            .len()
            < MAX_CREDITS / 2
    }

    pub(super) fn next_release(&self) -> Result<Option<String>, ServiceError> {
        let mut credits = self.credits.lock().expect("frame credits lock poisoned");
        if credits.overflowed {
            return Err(protocol("frame release queue could not drain"));
        }
        Ok(credits.queue.pop_front().map(|(stream, frame)| {
            format!(r#"{{"type":"release","streamId":{stream},"frameId":{frame}}}"#)
        }))
    }
}

pub(super) fn protocol(message: &str) -> ServiceError {
    ServiceError::new("E_PROTOCOL", message)
}
