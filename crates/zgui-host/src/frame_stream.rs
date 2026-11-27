//! Session-scoped opaque frame sources and the per-window external texture bindings.

use host_services::frame_stream::{
    ColorSpace, FrameBudget, FrameStream, Modifiers, PixelFormat, Submission, Transport, nv12_rgb,
};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::{
        Arc, Weak as SyncWeak,
        atomic::{AtomicBool, Ordering},
    },
};
use zgui::{
    geom::{CssPx, Device, Size},
    runtime::embed::{EmbedHost, EmbedMaintenanceCx, EmbedSyncCx, EmbedSyncReport},
    scene::ExternalTextureId,
};
use zgui_dom::{
    NodeIndex, NodeKey,
    host::{Intrinsic, ReplacedId},
};
use zgui_render_wgpu::{
    Dmabuf, DmabufFormat, DmabufPlane, FrameTexture, Gpu, ImportedFrame, WgpuRenderer,
};

const MAX_STREAMS: usize = 8;
const PREFIX: &str = "zgui-frame://";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SyntheticFormat {
    Rgba8,
    Bgra8,
    Nv12,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) enum Pattern {
    Solid(String),
    Quadrants([String; 4]),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SyntheticFrame {
    pub stream_id: u64,
    pub frame_id: u64,
    pub width: u32,
    pub height: u32,
    pub pixel_format: SyntheticFormat,
    pub pattern: Pattern,
}

impl SyntheticFrame {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.stream_id > crate::assets::MAX_SAFE_ID
            || self.frame_id > crate::assets::MAX_SAFE_ID
            || !(1..=8192).contains(&self.width)
            || !(1..=8192).contains(&self.height)
            || self.width * self.height > 1024 * 1024
        {
            return Err("invalid fixture frame identity or extent (maximum 1 Mi pixels)".into());
        }
        let colors: &[String] = match &self.pattern {
            Pattern::Solid(color) => std::slice::from_ref(color),
            Pattern::Quadrants(colors) => colors,
        };
        for color in colors {
            rgb(color).ok_or_else(|| format!("expected a #RRGGBB frame color, got {color:?}"))?;
        }
        Ok(())
    }

    fn pixels(self) -> Vec<u8> {
        let color = |text: &str| {
            let [r, g, b, _] = rgb(text).expect("validated fixture color");
            match self.pixel_format {
                SyntheticFormat::Rgba8 | SyntheticFormat::Bgra8 => [r, g, b, 255],
                SyntheticFormat::Nv12 => {
                    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
                    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
                    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                    nv12_rgb(y as u8, u as u8, v as u8, ColorSpace::Bt601)
                }
            }
        };
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        match self.pattern {
            Pattern::Solid(solid) => {
                let solid = color(&solid);
                for _ in 0..self.width * self.height {
                    out.extend_from_slice(&solid);
                }
            }
            Pattern::Quadrants(quadrants) => {
                let colors = quadrants.map(|s| color(&s));
                for y in 0..self.height {
                    for x in 0..self.width {
                        let shift = u32::from(matches!(self.pixel_format, SyntheticFormat::Nv12));
                        let col = usize::from((x >> shift) >= ((self.width / 2) >> shift));
                        let row = usize::from((y >> shift) >= ((self.height / 2) >> shift));
                        out.extend_from_slice(&colors[row * 2 + col]);
                    }
                }
            }
        }
        out
    }
}

fn rgb(color: &str) -> Option<[u8; 4]> {
    let hex = color.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    let n = u32::from_str_radix(hex, 16).ok()?;
    let [_, r, g, b] = n.to_be_bytes();
    Some([r, g, b, 255])
}

enum Pixels {
    Rgba {
        bytes: Vec<u8>,
        _credit: Option<Submission>,
    },
    Imported(ImportedFrame),
}

struct Frame {
    revision: u64,
    size: Size<u32, Device>,
    pixels: Pixels,
}

struct Stream {
    socket: u64,
    stream_id: u64,
    frame: Option<Frame>,
    live: Option<Arc<FrameStream>>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(stream) = &self.live {
            stream.close();
        }
    }
}

pub(crate) struct Streams {
    generation: u64,
    next: Cell<u64>,
    current: RefCell<BTreeMap<u64, Stream>>,
    gpu: RefCell<SyncWeak<Gpu>>,
    pub(crate) budget: FrameBudget,
    pub(crate) changed: Arc<AtomicBool>,
}

impl Streams {
    pub(crate) fn new(generation: u64) -> Self {
        Self {
            generation,
            next: Cell::new(1),
            current: RefCell::default(),
            gpu: RefCell::default(),
            budget: FrameBudget::default(),
            changed: Arc::default(),
        }
    }

    pub(crate) fn set_gpu(&self, renderer: &dyn zgui::render::Renderer) {
        let gpu = renderer
            .as_any()
            .and_then(|any| any.downcast_ref::<WgpuRenderer>())
            .map(WgpuRenderer::gpu);
        if let Some(gpu) = gpu {
            self.gpu.replace(Arc::downgrade(gpu));
        }
    }

    pub(crate) fn modifiers(&self) -> Option<Modifiers> {
        let gpu = self.gpu.borrow().upgrade()?;
        Some(
            [
                ("rgba8", DmabufFormat::Rgba8),
                ("bgra8", DmabufFormat::Bgra8),
                ("nv12", DmabufFormat::Nv12Bt601),
            ]
            .into_iter()
            .filter_map(|(format, drm)| {
                let modifiers = gpu.dmabuf_modifiers(drm);
                (!modifiers.is_empty()).then(|| {
                    (
                        format,
                        modifiers.into_iter().map(|m| m.to_string()).collect(),
                    )
                })
            })
            .collect(),
        )
    }

    pub(crate) fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn open_live(&self, socket: u64, stream_id: u64, live: Arc<FrameStream>) -> u64 {
        let token = self.allocate();
        self.current.borrow_mut().insert(
            token,
            Stream {
                socket,
                stream_id,
                frame: None,
                live: Some(live),
            },
        );
        token
    }

    fn allocate(&self) -> u64 {
        let token = self.next.get();
        self.next.set(token + 1);
        token
    }

    pub(crate) fn submit(&self, socket: u64, frame: SyntheticFrame) -> Result<Option<u64>, ()> {
        let mut current = self.current.borrow_mut();
        let size = Size::new(frame.width, frame.height);
        if let Some(stream) = current
            .values_mut()
            .find(|s| s.socket == socket && s.stream_id == frame.stream_id)
        {
            let revision = stream.frame.as_ref().map_or(1, |f| f.revision + 1);
            stream.frame = Some(Frame {
                revision,
                size,
                pixels: Pixels::Rgba {
                    bytes: frame.pixels(),
                    _credit: None,
                },
            });
            return Ok(None);
        }
        if current.len() >= MAX_STREAMS {
            return Err(());
        }
        let token = self.allocate();
        let stream_id = frame.stream_id;
        current.insert(
            token,
            Stream {
                socket,
                stream_id,
                live: None,
                frame: Some(Frame {
                    revision: 1,
                    size,
                    pixels: Pixels::Rgba {
                        bytes: frame.pixels(),
                        _credit: None,
                    },
                }),
            },
        );
        Ok(Some(token))
    }

    pub(crate) fn src(&self, token: u64) -> String {
        format!("{PREFIX}{}/{token}", self.generation)
    }

    fn token(&self, src: &str) -> Option<u64> {
        let (generation, token) = src.strip_prefix(PREFIX)?.split_once('/')?;
        (generation.parse::<u64>().ok()? == self.generation).then_some(())?;
        token.parse().ok()
    }

    pub(crate) fn size(&self, token: u64) -> Size<u32, Device> {
        self.current
            .borrow()
            .get(&token)
            .and_then(|stream| {
                stream
                    .live
                    .as_ref()
                    .and_then(|live| live.pending_size())
                    .map(|(w, h)| Size::new(w, h))
                    .or_else(|| stream.frame.as_ref().map(|frame| frame.size))
            })
            .unwrap_or(Size::new(0, 0))
    }

    pub(crate) fn release(&self, token: u64) {
        self.current.borrow_mut().remove(&token);
    }
    pub(crate) fn close(&self, socket: u64) {
        self.current
            .borrow_mut()
            .retain(|_, stream| stream.socket != socket);
    }
    pub(crate) fn count(&self) -> usize {
        let live = self.budget.streams();
        live + self
            .current
            .borrow()
            .values()
            .filter(|stream| stream.live.is_none())
            .count()
    }
    pub(crate) fn clear(&self) {
        self.current.borrow_mut().clear();
    }
}

#[derive(Default)]
struct Texture {
    slot: Option<FrameTexture>,
    revision: u64,
}

#[derive(Clone, Copy, Default)]
struct Attachment {
    token: u64,
    revision: u64,
    size: Option<Size<u32, Device>>,
}

pub(crate) struct Embed {
    streams: Rc<Streams>,
    nodes: HashMap<NodeKey, Attachment>,
    textures: BTreeMap<u64, Texture>,
    device: SyncWeak<Gpu>,
    scanned: Option<u64>,
}

impl Embed {
    pub(crate) fn new(streams: &Rc<Streams>) -> Self {
        Self {
            streams: streams.clone(),
            nodes: HashMap::new(),
            textures: BTreeMap::new(),
            device: SyncWeak::new(),
            scanned: None,
        }
    }

    fn rescan(&mut self, streams: &Streams, cx: &mut EmbedSyncCx<'_>) {
        let doc = cx.document.borrow();
        let store = doc.store();
        let mut nodes = HashMap::new();
        for slot in 0..store.slot_count() as u32 {
            let index = NodeIndex::new(slot);
            if store.try_core(index).is_none() {
                continue;
            }
            let node = doc.node(index);
            let Some(id) = node.replaced_id() else {
                continue;
            };
            let Some(token) = node.attr_text("src").and_then(|src| streams.token(&src)) else {
                continue;
            };
            let previous = self.nodes.remove(&id.node());
            let mut attachment = previous.unwrap_or_default();
            if attachment.token != token {
                attachment.revision = 0;
            }
            attachment.token = token;
            nodes.insert(id.node(), attachment);
        }
        drop(doc);
        for (node, previous) in self.nodes.drain() {
            if previous.size.is_some() {
                cx.intrinsics.remove(ReplacedId::new(node));
                cx.replaced_content_changed(node);
            }
            cx.content.remove_external(ReplacedId::new(node));
            damage(cx, node);
        }
        self.nodes = nodes;
        self.scanned = Some(cx.revision);
    }
}

impl EmbedHost for Embed {
    fn sync(&mut self, cx: &mut EmbedSyncCx<'_>) -> EmbedSyncReport {
        let streams = self.streams.clone();
        if self.scanned != Some(cx.revision) && (!self.nodes.is_empty() || streams.count() != 0) {
            self.rescan(&streams, cx);
        }
        let Some(renderer) = cx
            .renderer
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<WgpuRenderer>())
        else {
            return EmbedSyncReport::default();
        };
        if self.device.as_ptr() != Arc::as_ptr(renderer.gpu()) {
            self.textures.clear();
            self.device = Arc::downgrade(renderer.gpu());
            streams.set_gpu(renderer);
            self.content_forgotten();
        }

        let mut current = streams.current.borrow_mut();
        self.textures.retain(|token, texture| {
            let keep =
                current.contains_key(token) && self.nodes.values().any(|n| n.token == *token);
            if !keep && let Some(slot) = texture.slot.take() {
                renderer.remove_frame(slot);
            }
            keep
        });
        for (&token, stream) in current.iter_mut() {
            if !self.nodes.values().any(|n| n.token == token) {
                continue;
            }
            if let Some(claim) = stream.live.as_ref().and_then(FrameStream::take_presentable) {
                let revision = stream.frame.as_ref().map_or(1, |frame| frame.revision + 1);
                if let Some(frame) = prepared(renderer, claim, revision) {
                    stream.frame = Some(frame);
                }
            }
            let Some(frame) = &stream.frame else {
                continue;
            };
            let texture = self.textures.entry(token).or_default();
            if texture.revision == frame.revision {
                continue;
            }
            let id = ExternalTextureId(token);
            let attached = match &frame.pixels {
                Pixels::Rgba { bytes, .. } => {
                    renderer.write_frame(id, &mut texture.slot, frame.size, bytes)
                }
                Pixels::Imported(frame) => {
                    if let Some(previous) = texture.slot.take() {
                        renderer.remove_frame(previous);
                    }
                    texture.slot = renderer.attach_imported_frame(id, frame);
                    texture.slot.is_some()
                }
            };
            if attached {
                texture.revision = frame.revision;
            }
        }
        for (&node, attachment) in &mut self.nodes {
            let id = ReplacedId::new(node);
            let texture = self.textures.get(&attachment.token);
            let (revision, size) = texture.map_or((0, None), |texture| {
                (
                    texture.revision,
                    texture.slot.as_ref().map(FrameTexture::size),
                )
            });
            if attachment.size != size {
                if let Some(size) = size {
                    cx.intrinsics.set(
                        id,
                        Intrinsic {
                            size: Some(Size::new(
                                CssPx(size.width as f32),
                                CssPx(size.height as f32),
                            )),
                            ratio: Some(size.width as f32 / size.height as f32),
                            baseline: None,
                        },
                    );
                } else {
                    cx.intrinsics.remove(id);
                }
                cx.replaced_content_changed(node);
                attachment.size = size;
            }
            if attachment.revision != revision {
                if let Some(size) = size {
                    cx.content
                        .set_external_frame(id, ExternalTextureId(attachment.token), size);
                } else {
                    cx.content.remove_external(id);
                }
                damage(cx, node);
                attachment.revision = revision;
            }
        }
        EmbedSyncReport::default()
    }

    fn content_forgotten(&mut self) {
        for node in self.nodes.values_mut() {
            node.revision = 0;
        }
    }

    fn detach(&mut self, cx: &mut EmbedMaintenanceCx<'_>) {
        if let Some(renderer) = cx
            .renderer
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<WgpuRenderer>())
            && self.device.as_ptr() == Arc::as_ptr(renderer.gpu())
        {
            for (_, texture) in std::mem::take(&mut self.textures) {
                if let Some(slot) = texture.slot {
                    renderer.remove_frame(slot);
                }
            }
        }
        for (node, _) in self.nodes.drain() {
            let id = ReplacedId::new(node);
            cx.content.remove_external(id);
            cx.intrinsics.remove(id);
        }
    }
}

fn prepared(renderer: &WgpuRenderer, mut claim: Submission, revision: u64) -> Option<Frame> {
    let header = claim.header();
    let size = Size::new(header.width, header.height);
    let pixels = match header.transport {
        Transport::Memfd => Pixels::Rgba {
            bytes: claim.take_rgba()?,
            _credit: Some(claim),
        },
        Transport::Dmabuf => {
            let format = match header.pixel_format {
                PixelFormat::Rgba8 => DmabufFormat::Rgba8,
                PixelFormat::Bgra8 => DmabufFormat::Bgra8,
                PixelFormat::Nv12 if header.color_space == Some(ColorSpace::Bt709) => {
                    DmabufFormat::Nv12Bt709
                }
                PixelFormat::Nv12 => DmabufFormat::Nv12Bt601,
            };
            let mut planes = [DmabufPlane {
                fd: claim.fd(0)?,
                offset: 0,
                stride: 0,
            }; 2];
            for (out, plane) in planes.iter_mut().zip(&header.planes) {
                *out = DmabufPlane {
                    fd: claim.fd(plane.fd_index)?,
                    offset: plane.offset,
                    stride: plane.stride,
                };
            }
            let credit = claim.credit();
            let result = renderer.import_dmabuf(
                Dmabuf {
                    size,
                    format,
                    modifier: header.modifier()?,
                    planes: &planes[..header.planes.len()],
                },
                move || drop(credit),
            );
            match result {
                Ok(frame) => Pixels::Imported(frame),
                Err(error) => {
                    eprintln!("frame-import: {error}; producer can use memfd for this layout");
                    return None;
                }
            }
        }
    };
    Some(Frame {
        revision,
        size,
        pixels,
    })
}

fn damage(cx: &mut EmbedSyncCx<'_>, node: NodeKey) {
    let placed =
        zgui_layout::fragment::index::device_ink_of(&cx.layout.borrow(), node, cx.placements);
    if !placed.is_empty() {
        cx.damage
            .absorb(zgui_layout::fragment::diff::pixels(placed));
    }
}
