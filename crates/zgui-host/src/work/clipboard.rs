//! Session-owned clipboard requests routed through the platform `Clipboard` contract.
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

use rquickjs::{
    Ctx, Exception, Function, IntoJs, Object, Persistent, TypedArray, Value,
    function::{Func, Opt},
};
use zgui::platform::{
    Clipboard, ClipboardData, ClipboardError, ClipboardFormat, ClipboardKind, ClipboardSerial,
    ClipboardWriteOptions,
};

use super::{
    ImageLease, MAX_DIRECT_BYTES_PER_SESSION, MAX_DIRECT_IMAGE_BYTES, MAX_DIRECT_IMAGES, Mode,
    Work, data_property, owner, service_error,
};

const MAX_CLIPBOARD_REQUESTS: usize = 64;
const MAX_CLIPBOARD_TEXT_BYTES: usize = 8 * 1024 * 1024;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipboardRequestKind {
    ReadText,
    ReadImage,
    Write,
}

struct PendingClipboard {
    kind: ClipboardRequestKind,
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
}

enum ClipboardOp {
    Read {
        id: u64,
        format: ClipboardFormat,
    },
    Write {
        id: u64,
        data: ClipboardData,
        options: ClipboardWriteOptions,
    },
}

pub(crate) enum ClipboardResult {
    Void,
    Text(Option<String>),
    Image(Option<Arc<[u8]>>),
    Err { code: &'static str, message: String },
}

pub(crate) struct ClipboardCompletion {
    id: u64,
    result: ClipboardResult,
}

pub(super) struct ClipboardState {
    allow_read: bool,
    pending: RefCell<HashMap<u64, PendingClipboard>>,
    ops: RefCell<VecDeque<ClipboardOp>>,
    completions: RefCell<VecDeque<ClipboardCompletion>>,
    in_flight_reads: RefCell<HashMap<ClipboardSerial, u64>>,
}

impl ClipboardState {
    pub(super) fn new(allow_read: bool) -> Self {
        Self {
            allow_read,
            pending: RefCell::new(HashMap::new()),
            ops: RefCell::new(VecDeque::new()),
            completions: RefCell::new(VecDeque::new()),
            in_flight_reads: RefCell::new(HashMap::new()),
        }
    }

    pub(super) fn pending_count(&self) -> usize {
        self.pending.borrow().len()
    }

    pub(super) fn has_ops(&self) -> bool {
        !self.ops.borrow().is_empty()
    }

    pub(super) fn has_completions(&self) -> bool {
        !self.completions.borrow().is_empty()
    }

    pub(super) fn next_completion(&self) -> Option<ClipboardCompletion> {
        self.completions.borrow_mut().pop_front()
    }

    pub(super) fn clear(&self) {
        self.ops.borrow_mut().clear();
        self.completions.borrow_mut().clear();
        self.in_flight_reads.borrow_mut().clear();
        self.pending.borrow_mut().clear();
    }
}

impl Work {
    pub(super) fn install_clipboard<'js>(
        self: &Rc<Self>,
        services: &Object<'js>,
    ) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        services.prop(
            "readClipboardText",
            data_property(Func::from(move |ctx: Ctx<'js>, extra: Opt<Value<'js>>| {
                if extra.0.is_some() {
                    return Err(Exception::throw_type(
                        &ctx,
                        "readClipboardText takes no arguments",
                    ));
                }
                owner(&weak, &ctx)?.start_clipboard_read(&ctx, ClipboardRequestKind::ReadText)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "readClipboardImage",
            data_property(Func::from(move |ctx: Ctx<'js>, extra: Opt<Value<'js>>| {
                if extra.0.is_some() {
                    return Err(Exception::throw_type(
                        &ctx,
                        "readClipboardImage takes no arguments",
                    ));
                }
                owner(&weak, &ctx)?.start_clipboard_read(&ctx, ClipboardRequestKind::ReadImage)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "writeClipboardText",
            data_property(Func::from(
                move |ctx: Ctx<'js>,
                      text: Opt<Value<'js>>,
                      options: Opt<Value<'js>>,
                      extra: Opt<Value<'js>>| {
                    if extra.0.is_some() {
                        return Err(Exception::throw_type(
                            &ctx,
                            "writeClipboardText accepts at most two arguments",
                        ));
                    }
                    let text = text.0.ok_or_else(|| {
                        Exception::throw_type(&ctx, "writeClipboardText requires a text string")
                    })?;
                    owner(&weak, &ctx)?.start_clipboard_write_text(&ctx, text, options.0)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "writeClipboardImage",
            data_property(Func::from(
                move |ctx: Ctx<'js>,
                      source: Opt<Value<'js>>,
                      options: Opt<Value<'js>>,
                      extra: Opt<Value<'js>>| {
                    if extra.0.is_some() {
                        return Err(Exception::throw_type(
                            &ctx,
                            "writeClipboardImage accepts at most two arguments",
                        ));
                    }
                    let source = source.0.ok_or_else(|| {
                        Exception::throw_type(
                            &ctx,
                            "writeClipboardImage requires a Uint8Array or ImageHandle",
                        )
                    })?;
                    owner(&weak, &ctx)?.start_clipboard_write_image(&ctx, source, options.0)
                },
            )),
        )
    }

    fn require_clipboard_read(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        self.require_creation(ctx)?;
        if self.clipboard.allow_read || self.in_user_gesture() {
            return Ok(());
        }
        let error = service_error(
            ctx,
            "E_CAPABILITY",
            "clipboard read requires capabilities.clipboard.read or an active user gesture",
        )?;
        Err(ctx.throw(error.into_value()))
    }

    fn enqueue_clipboard_request<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        kind: ClipboardRequestKind,
        op: impl FnOnce(u64) -> ClipboardOp,
    ) -> rquickjs::Result<Object<'js>> {
        if self.clipboard.pending.borrow().len() >= MAX_CLIPBOARD_REQUESTS {
            return Err(Exception::throw_range(
                ctx,
                "concurrent clipboard request limit reached",
            ));
        }
        let id = self.id(ctx)?;
        let (promise, resolve, reject) = ctx.promise()?;
        let object = Object::new(ctx.clone())?;
        object.prop("promise", data_property(promise))?;
        let weak = Rc::downgrade(self);
        object.prop(
            "cancel",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                owner(&weak, &ctx)?.cancel_clipboard_request(&ctx, id)
            })),
        )?;
        self.clipboard.pending.borrow_mut().insert(
            id,
            PendingClipboard {
                kind,
                resolve: Persistent::save(ctx, resolve),
                reject: Persistent::save(ctx, reject),
            },
        );
        self.clipboard.ops.borrow_mut().push_back(op(id));
        (self.wake)();
        Ok(object)
    }

    fn start_clipboard_read<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        kind: ClipboardRequestKind,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_clipboard_read(ctx)?;
        let format = match kind {
            ClipboardRequestKind::ReadText => ClipboardFormat::Text,
            ClipboardRequestKind::ReadImage => ClipboardFormat::Image,
            ClipboardRequestKind::Write => unreachable!(),
        };
        self.enqueue_clipboard_request(ctx, kind, |id| ClipboardOp::Read { id, format })
    }

    fn start_clipboard_write_text<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        value: Value<'js>,
        options: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        let string = value
            .into_string()
            .ok_or_else(|| Exception::throw_type(ctx, "clipboard text must be a string"))?
            .to_cstring()?;
        if string.len() > MAX_CLIPBOARD_TEXT_BYTES {
            return Err(Exception::throw_range(
                ctx,
                "clipboard text exceeds 8 MiB limit",
            ));
        }
        // SAFETY: QuickJS owns `string`'s `len` initialized bytes for the CString's lifetime.
        let raw = unsafe { std::slice::from_raw_parts(string.as_ptr().cast::<u8>(), string.len()) };
        let text = std::str::from_utf8(raw)
            .map_err(|_| Exception::throw_type(ctx, "clipboard text contains invalid UTF-8"))?
            .to_owned();
        let write_options = parse_write_options(ctx, options)?;
        self.enqueue_clipboard_request(ctx, ClipboardRequestKind::Write, |id| ClipboardOp::Write {
            id,
            data: ClipboardData::Text(text.into()),
            options: write_options,
        })
    }

    fn start_clipboard_write_image<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        source: Value<'js>,
        options: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        let write_options = parse_write_options(ctx, options)?;
        let bytes: Arc<[u8]> = if let Ok(view) = TypedArray::<u8>::from_value(source.clone()) {
            let raw = view.as_raw().ok_or_else(|| {
                Exception::throw_type(ctx, "clipboard image bytes view is detached or out of bounds")
            })?;
            let len = raw.len();
            if len == 0 || len > MAX_DIRECT_IMAGE_BYTES as usize {
                return Err(Exception::throw_range(
                    ctx,
                    "clipboard image bytes must be between 1 byte and 8 MiB",
                ));
            }
            // SAFETY: `view` keeps the backing ArrayBuffer alive while copied into `Arc<[u8]>`.
            Arc::from(unsafe { raw.as_ref() })
        } else if let Some(handle) = source.into_object() {
            let src_val: Value = handle.get("src")?;
            let src = src_val
                .into_string()
                .ok_or_else(|| {
                    Exception::throw_type(
                        ctx,
                        "writeClipboardImage requires a Uint8Array or live ImageHandle",
                    )
                })?
                .to_string()?;
            self.image_blobs.lookup(&src).ok_or_else(|| {
                let err = service_error(
                    ctx,
                    "InvalidState",
                    "clipboard image handle is released or does not belong to this session",
                );
                match err {
                    Ok(obj) => ctx.throw(obj.into_value()),
                    Err(e) => e,
                }
            })?
        } else {
            return Err(Exception::throw_type(
                ctx,
                "writeClipboardImage requires a Uint8Array or ImageHandle",
            ));
        };
        if !bytes.starts_with(&PNG_SIGNATURE) {
            let error = service_error(
                ctx,
                "UnsupportedFormat",
                "clipboard image bytes must be PNG-encoded",
            )?;
            return Err(ctx.throw(error.into_value()));
        }
        self.enqueue_clipboard_request(ctx, ClipboardRequestKind::Write, |id| ClipboardOp::Write {
            id,
            data: ClipboardData::Image {
                media_type: "image/png".into(),
                bytes: bytes.as_ref().into(),
            },
            options: write_options,
        })
    }

    fn cancel_clipboard_request<'js>(&self, ctx: &Ctx<'js>, id: u64) -> rquickjs::Result<()> {
        let Some(pending) = self.clipboard.pending.borrow_mut().remove(&id) else {
            return Ok(());
        };
        self.clipboard
            .ops
            .borrow_mut()
            .retain(|op| match op {
                ClipboardOp::Read { id: other, .. } | ClipboardOp::Write { id: other, .. } => {
                    *other != id
                }
            });
        self.clipboard
            .completions
            .borrow_mut()
            .retain(|completion| completion.id != id);
        self.clipboard
            .in_flight_reads
            .borrow_mut()
            .retain(|_, other| *other != id);
        pending
            .reject
            .restore(ctx)?
            .call::<_, ()>((service_error(ctx, "Cancelled", "clipboard request cancelled")?,))
    }

    pub(crate) fn next_clipboard_completion(&self) -> Option<ClipboardCompletion> {
        if self.mode.get() != Mode::Mounted {
            return None;
        }
        self.clipboard.next_completion()
    }

    /// Dispatches queued clipboard requests against the active platform backend.
    pub(crate) fn drain_clipboard(&self, clipboard: &dyn Clipboard) -> bool {
        let mut produced_completion = false;
        while let Some(op) = self.clipboard.ops.borrow_mut().pop_front() {
            match op {
                ClipboardOp::Write { id, data, options } => {
                    if !self.clipboard.pending.borrow().contains_key(&id) {
                        continue;
                    }
                    let result = match clipboard.write(ClipboardKind::Standard, data, options) {
                        Ok(()) => ClipboardResult::Void,
                        Err(error) => map_clipboard_error(error),
                    };
                    self.clipboard
                        .completions
                        .borrow_mut()
                        .push_back(ClipboardCompletion { id, result });
                    produced_completion = true;
                }
                ClipboardOp::Read { id, format } => {
                    if !self.clipboard.pending.borrow().contains_key(&id) {
                        continue;
                    }
                    let serial = clipboard.read(ClipboardKind::Standard, format);
                    self.clipboard
                        .in_flight_reads
                        .borrow_mut()
                        .insert(serial, id);
                }
            }
        }
        if produced_completion {
            (self.wake)();
        }
        produced_completion
    }

    pub(crate) fn owns_clipboard_read(&self, serial: ClipboardSerial) -> bool {
        self.clipboard.in_flight_reads.borrow().contains_key(&serial)
    }

    pub(crate) fn finish_clipboard_read(
        &self,
        serial: ClipboardSerial,
        result: Result<ClipboardData, ClipboardError>,
    ) {
        let Some(id) = self.clipboard.in_flight_reads.borrow_mut().remove(&serial) else {
            return;
        };
        let kind = match self.clipboard.pending.borrow().get(&id) {
            Some(pending) => pending.kind,
            None => return,
        };
        let mapped = match (kind, result) {
            (ClipboardRequestKind::ReadText, Ok(ClipboardData::Text(text))) => {
                ClipboardResult::Text(Some(text.as_str().to_owned()))
            }
            (ClipboardRequestKind::ReadImage, Ok(ClipboardData::Image { bytes, .. })) => {
                ClipboardResult::Image(Some(Arc::from(bytes.as_ref())))
            }
            (
                ClipboardRequestKind::ReadText,
                Err(ClipboardError::Empty(_) | ClipboardError::UnavailableFormat(_)),
            ) => ClipboardResult::Text(None),
            (
                ClipboardRequestKind::ReadImage,
                Err(ClipboardError::Empty(_) | ClipboardError::UnavailableFormat(_)),
            ) => ClipboardResult::Image(None),
            (_, Err(error)) => map_clipboard_error(error),
            (_, Ok(_)) => ClipboardResult::Err {
                code: "UnsupportedFormat",
                message: "clipboard returned an unexpected format".into(),
            },
        };
        self.clipboard
            .completions
            .borrow_mut()
            .push_back(ClipboardCompletion { id, result: mapped });
    }

    pub(crate) fn settle_clipboard<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        completion: ClipboardCompletion,
    ) -> rquickjs::Result<()> {
        if self.mode.get() != Mode::Mounted {
            return Ok(());
        }
        let Some(pending) = self.clipboard.pending.borrow_mut().remove(&completion.id) else {
            return Ok(());
        };
        match completion.result {
            ClipboardResult::Void => {
                pending
                    .resolve
                    .restore(ctx)?
                    .call::<_, ()>((Value::new_undefined(ctx.clone()),))
            }
            ClipboardResult::Text(Some(text)) => {
                pending.resolve.restore(ctx)?.call::<_, ()>((text,))
            }
            ClipboardResult::Text(None) | ClipboardResult::Image(None) => {
                pending
                    .resolve
                    .restore(ctx)?
                    .call::<_, ()>((Value::new_null(ctx.clone()),))
            }
            ClipboardResult::Image(Some(bytes)) => {
                let len = match u32::try_from(bytes.len()) {
                    Ok(len) if (1..=MAX_DIRECT_IMAGE_BYTES).contains(&len) => len,
                    _ => {
                        let error = service_error(
                            ctx,
                            "TooLarge",
                            "clipboard image exceeds 8 MiB limit",
                        )?;
                        return pending.reject.restore(ctx)?.call::<_, ()>((error,));
                    }
                };
                let budget = self.direct_images.get();
                if budget.handles >= MAX_DIRECT_IMAGES
                    || len > MAX_DIRECT_BYTES_PER_SESSION - budget.bytes
                {
                    let error = service_error(
                        ctx,
                        "TooLarge",
                        "session direct image budget exceeded",
                    )?;
                    return pending.reject.restore(ctx)?.call::<_, ()>((error,));
                }
                let handle = self.image_handle(ctx, bytes, ImageLease::Direct(len))?;
                pending
                    .resolve
                    .restore(ctx)?
                    .call::<_, ()>((handle.into_js(ctx)?,))
            }
            ClipboardResult::Err { code, message } => {
                let error = service_error(ctx, code, &message)?;
                pending.reject.restore(ctx)?.call::<_, ()>((error,))
            }
        }
    }
}

fn parse_write_options(
    ctx: &Ctx<'_>,
    options: Option<Value<'_>>,
) -> rquickjs::Result<ClipboardWriteOptions> {
    let Some(value) = options else {
        return Ok(ClipboardWriteOptions::default());
    };
    if value.is_undefined() || value.is_null() {
        return Ok(ClipboardWriteOptions::default());
    }
    let object = value
        .into_object()
        .ok_or_else(|| Exception::throw_type(ctx, "clipboard write options must be an object"))?;
    let secret_val: Value = object.get("secret")?;
    let secret = if secret_val.is_undefined() {
        false
    } else if let Some(b) = secret_val.as_bool() {
        b
    } else {
        return Err(Exception::throw_type(
            ctx,
            "clipboard write options.secret must be a boolean",
        ));
    };
    Ok(if secret {
        ClipboardWriteOptions::SECRET
    } else {
        ClipboardWriteOptions::default()
    })
}

fn map_clipboard_error(error: ClipboardError) -> ClipboardResult {
    let code = match &error {
        ClipboardError::Empty(_) => "Empty",
        ClipboardError::UnavailableFormat(_) => "UnavailableFormat",
        ClipboardError::UnsupportedFormat(_) => "UnsupportedFormat",
        ClipboardError::UnsupportedKind(_) => "UnsupportedKind",
        ClipboardError::TimedOut => "TimedOut",
        ClipboardError::Backend(_) => "Backend",
        _ => "Backend",
    };
    ClipboardResult::Err {
        code,
        message: error.to_string(),
    }
}
