//! Session-scoped Drag-and-Drop coordinator for hit-tested internal reorder DnD
//! and capability-gated external file drops.
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

use rquickjs::{
    Array, Ctx, Exception, Function, Object, Persistent, TypedArray, Value,
    function::{Func, Opt},
    object::Property,
};
use ui_bridge::dnd::DndEventKind;
use zgui::{
    geom::{Css, CssPx, Point},
    platform::{DragEvent, OutboundDragPayload, SurfaceId},
    runtime::Window,
    view::NodeId,
    vocab::{Key, KeyEvent, KeyState, NamedKey, PointerAction},
};

use crate::session::{Result as SessionResult, Session};

const MAX_OUTBOUND_DRAG_ITEMS: usize = 16;
const MAX_OUTBOUND_DRAG_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DragEffect {
    Copy,
    Move,
    None,
}

impl DragEffect {
    fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
            Self::None => "none",
        }
    }

    fn parse(raw: Option<&str>, default: Self) -> Self {
        match raw {
            Some("copy") => Self::Copy,
            Some("move") => Self::Move,
            Some("none") => Self::None,
            _ => default,
        }
    }
}

struct InternalDrag {
    id: u64,
    surface: SurfaceId,
    source_node: NodeId,
    types: Vec<String>,
    payload: Persistent<Value<'static>>,
    on_end: Option<Persistent<Function<'static>>>,
    hovered: Vec<NodeId>,
    accepted: Option<(NodeId, DragEffect)>,
}

struct ExternalDrag {
    id: u64,
    surface: SurfaceId,
    types: Vec<String>,
    file_count: usize,
    hovered: Vec<NodeId>,
    accepted: Option<(NodeId, DragEffect)>,
}

struct OutboundDrag {
    id: u64,
    surface: SurfaceId,
    source_node: NodeId,
    on_end: Option<Persistent<Function<'static>>>,
}

#[derive(Default)]
pub(crate) struct DndState {
    next_id: Cell<u64>,
    internal: RefCell<Option<InternalDrag>>,
    external: RefCell<Option<ExternalDrag>>,
    outbound: RefCell<Option<OutboundDrag>>,
}

impl DndState {
    pub(crate) fn new() -> Self {
        Self {
            next_id: Cell::new(1),
            internal: RefCell::new(None),
            external: RefCell::new(None),
            outbound: RefCell::new(None),
        }
    }

    fn alloc_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id.saturating_add(1));
        id
    }

    /// Number of active drag operations owned by JS (used for teardown leak detection).
    pub(crate) fn active_count(&self) -> usize {
        usize::from(self.internal.borrow().is_some()) + usize::from(self.outbound.borrow().is_some())
    }

    pub(crate) fn has_active_internal(&self) -> bool {
        self.internal.borrow().is_some()
    }

    /// Releases all persistent JS roots before dropping the QuickJS runtime.
    pub(crate) fn clear(&self) {
        self.internal.borrow_mut().take();
        self.external.borrow_mut().take();
        self.outbound.borrow_mut().take();
    }
}

fn data_prop<T>(value: T) -> Property<T> {
    Property::from(value).writable().enumerable().configurable()
}

/// Derives safe MIME type strings from dropped paths without exposing raw paths or filenames.
fn infer_external_mime_types(paths: &[std::path::PathBuf]) -> Vec<String> {
    let mut types = Vec::new();
    if !paths.is_empty() {
        types.push("text/uri-list".to_owned());
    }
    for path in paths {
        let mime = mime_for_path(path);
        if !types.iter().any(|existing| existing == mime) {
            types.push(mime.to_owned());
        }
    }
    types
}

fn mime_for_path(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("txt" | "log") => "text/plain",
        Some("md" | "markdown") => "text/markdown",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("js" | "mjs" | "ts" | "tsx") => "text/javascript",
        _ => "application/octet-stream",
    }
}

fn types_to_js<'js>(ctx: &Ctx<'js>, types: &[String]) -> rquickjs::Result<Array<'js>> {
    let arr = Array::new(ctx.clone())?;
    for (idx, item) in types.iter().enumerate() {
        arr.set(idx, item.as_str())?;
    }
    Ok(arr)
}

#[derive(Clone, Copy)]
struct BoxGeometry {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl BoxGeometry {
    fn from_window(window: Option<&Window>, node: NodeId) -> Self {
        match window.and_then(|w| w.node_box_css(node)) {
            Some((x, y, width, height)) => Self { x, y, width, height },
            None => Self {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_drag_event<'js>(
    ctx: &Ctx<'js>,
    drag_id: u64,
    source: &'static str,
    types: &[String],
    file_count: usize,
    position: Point<CssPx, Css>,
    geom: BoxGeometry,
    default_effect: DragEffect,
    accepted: Rc<Cell<Option<DragEffect>>>,
    stopped: Rc<Cell<bool>>,
) -> rquickjs::Result<Object<'js>> {
    let ev = Object::new(ctx.clone())?;
    ev.prop("dragId", data_prop(drag_id as f64))?;
    ev.prop("source", data_prop(source))?;
    ev.prop("types", data_prop(types_to_js(ctx, types)?))?;
    ev.prop("fileCount", data_prop(file_count as f64))?;
    ev.prop("clientX", data_prop(f64::from(position.x.0)))?;
    ev.prop("clientY", data_prop(f64::from(position.y.0)))?;
    ev.prop("localX", data_prop(f64::from(position.x.0 - geom.x)))?;
    ev.prop("localY", data_prop(f64::from(position.y.0 - geom.y)))?;
    ev.prop("targetWidth", data_prop(f64::from(geom.width)))?;
    ev.prop("targetHeight", data_prop(f64::from(geom.height)))?;
    let acc_cell = accepted;
    ev.prop(
        "accept",
        data_prop(Func::from(move |effect: Opt<String>| {
            let chosen = DragEffect::parse(effect.0.as_deref(), default_effect);
            if chosen == DragEffect::None {
                acc_cell.set(None);
            } else {
                acc_cell.set(Some(chosen));
            }
        })),
    )?;
    let stop_cell = stopped;
    ev.prop(
        "stopPropagation",
        data_prop(Func::from(move || {
            stop_cell.set(true);
        })),
    )?;
    Ok(ev)
}

impl Session {
    /// Installs `globalThis.__zguiDnd` for internal hit-tested drag initiation.
    pub(crate) fn install_dnd<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        let dnd_obj = Object::new(ctx.clone())?;
        let weak = Rc::downgrade(self);
        dnd_obj.prop(
            "startInternalDrag",
            data_prop(Func::from(
                move |ctx: Ctx<'js>,
                      source_node: rquickjs::Class<'js, ui_bridge::NativeNode>,
                      options: Object<'js>| {
                    let session = weak
                        .upgrade()
                        .ok_or_else(|| Exception::throw_reference(&ctx, "session is disposed"))?;
                    session.start_internal_drag(&ctx, source_node, options)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        dnd_obj.prop(
            "startOutboundDrag",
            data_prop(Func::from(
                move |ctx: Ctx<'js>,
                      source_node: rquickjs::Class<'js, ui_bridge::NativeNode>,
                      options: Object<'js>| {
                    let session = weak
                        .upgrade()
                        .ok_or_else(|| Exception::throw_reference(&ctx, "session is disposed"))?;
                    session.start_outbound_drag(&ctx, source_node, options)
                },
            )),
        )?;
        ctx.globals().set("__zguiDnd", dnd_obj)?;
        Ok(())
    }

    /// Cancels any active platform outbound drag during forced session retirement.
    pub(crate) fn cancel_outbound_drag_on_platform(&self) {
        if let Some(drag) = self.dnd.outbound.borrow_mut().take()
            && let Some(surface) = self.platform_surface(drag.surface)
        {
            surface.cancel_outbound_drag();
        }
    }

    fn start_outbound_drag<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        source_node: rquickjs::Class<'js, ui_bridge::NativeNode>,
        options: Object<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        if self.shutdown_started.get() {
            return Err(Exception::throw_type(ctx, "drag cannot start during session shutdown"));
        }
        if self.dnd.internal.borrow().is_some() || self.dnd.outbound.borrow().is_some() {
            let err = crate::work::service_error(
                ctx,
                "E_DRAG_ACTIVE",
                "another drag operation is already active",
            )?;
            return Err(ctx.throw(err.into_value()));
        }
        let (surface_id, native_node) = self.resolve_node_and_surface(ctx, &source_node)?;
        if self
            .bridge_for_surface(surface_id)
            .is_some_and(|bridge| bridge.is_native_node_effectively_inert(native_node))
        {
            return Err(Exception::throw_type(
                ctx,
                "cannot start outbound drag from an inert node",
            ));
        }
        let items_arr: Array = options.get("items")?;
        if items_arr.is_empty() || items_arr.len() > MAX_OUTBOUND_DRAG_ITEMS {
            return Err(Exception::throw_range(
                ctx,
                "outbound drag items must contain between 1 and 16 MIME entries",
            ));
        }
        let mut items = Vec::with_capacity(items_arr.len());
        let mut total_bytes = 0usize;
        for entry in items_arr.iter::<Object>() {
            let entry = entry?;
            let mime: String = entry.get("mime")?;
            if mime.trim().is_empty() || mime.chars().any(char::is_control) {
                return Err(Exception::throw_type(ctx, "outbound drag MIME type must be non-empty without control characters"));
            }
            let data_val: Value = entry.get("data")?;
            let bytes = if let Some(s) = data_val.as_string() {
                s.to_string()?.into_bytes()
            } else if let Ok(ta) = TypedArray::<u8>::from_value(data_val) {
                let raw = ta
                    .as_raw()
                    .ok_or_else(|| Exception::throw_type(ctx, "outbound drag Uint8Array is detached"))?;
                // SAFETY: `ta` keeps the JS backing buffer alive during this copy.
                unsafe { raw.as_ref().to_vec() }
            } else {
                return Err(Exception::throw_type(ctx, "outbound drag item.data must be a string or Uint8Array"));
            };
            total_bytes = total_bytes.saturating_add(bytes.len());
            if total_bytes > MAX_OUTBOUND_DRAG_BYTES {
                let err = crate::work::service_error(
                    ctx,
                    "E_LIMIT",
                    "outbound drag payload exceeds 4 MiB limit",
                )?;
                return Err(ctx.throw(err.into_value()));
            }
            items.push((mime, bytes));
        }
        let on_end: Option<Function> = options.get("onEnd")?;
        let Some(native_surface) = self.platform_surface(surface_id) else {
            let err = crate::work::service_error(
                ctx,
                "E_DRAG_NO_LIVE_PRESS",
                "originating platform surface is unavailable for outbound drag",
            )?;
            return Err(ctx.throw(err.into_value()));
        };
        if native_surface
            .begin_outbound_drag(OutboundDragPayload::new(items))
            .is_err()
        {
            let err = crate::work::service_error(
                ctx,
                "E_DRAG_NO_LIVE_PRESS",
                "outbound Copy-only drag requires an active live press on the originating surface",
            )?;
            return Err(ctx.throw(err.into_value()));
        }

        let drag_id = self.dnd.alloc_id();
        let _ = self.bridges.take_active_gesture_grant();
        *self.dnd.outbound.borrow_mut() = Some(OutboundDrag {
            id: drag_id,
            surface: surface_id,
            source_node: native_node,
            on_end: on_end.map(|f| Persistent::save(ctx, f)),
        });

        let handle = Object::new(ctx.clone())?;
        handle.prop("id", data_prop(drag_id as f64))?;
        let weak = Rc::downgrade(self);
        handle.prop(
            "cancel",
            data_prop(Func::from(move |ctx: Ctx<'js>| {
                if let Some(session) = weak.upgrade() {
                    session.cancel_outbound_drag_in_ctx(&ctx, Some(drag_id), "cancel")?;
                }
                Ok::<(), rquickjs::Error>(())
            })),
        )?;
        Ok(handle)
    }

    fn cancel_outbound_drag_in_ctx<'js>(
        &self,
        ctx: &Ctx<'js>,
        expected_id: Option<u64>,
        reason: &'static str,
    ) -> rquickjs::Result<()> {
        let should_take = self
            .dnd
            .outbound
            .borrow()
            .as_ref()
            .is_some_and(|drag| expected_id.is_none_or(|id| drag.id == id));
        if !should_take {
            return Ok(());
        }
        let Some(drag) = self.dnd.outbound.borrow_mut().take() else {
            return Ok(());
        };
        if let Some(surface) = self.platform_surface(drag.surface) {
            surface.cancel_outbound_drag();
        }
        if let Some(on_end) = drag.on_end {
            let res = Object::new(ctx.clone())?;
            res.prop("accepted", data_prop(false))?;
            res.prop("effect", data_prop("none"))?;
            res.prop("reason", data_prop(reason))?;
            on_end.restore(ctx)?.call::<_, ()>((res,))?;
        }
        Ok(())
    }

    fn start_internal_drag<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        source_node: rquickjs::Class<'js, ui_bridge::NativeNode>,
        options: Object<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        if self.shutdown_started.get() {
            return Err(Exception::throw_type(ctx, "drag cannot start during session shutdown"));
        }
        if self.dnd.internal.borrow().is_some() || self.dnd.outbound.borrow().is_some() {
            let err = crate::work::service_error(
                ctx,
                "E_DRAG_ACTIVE",
                "another drag operation is already active",
            )?;
            return Err(ctx.throw(err.into_value()));
        }
        let (surface, native_node) = self.resolve_node_and_surface(ctx, &source_node)?;
        if self
            .bridge_for_surface(surface)
            .is_some_and(|bridge| bridge.is_native_node_effectively_inert(native_node))
        {
            return Err(Exception::throw_type(
                ctx,
                "cannot start internal drag from an inert node",
            ));
        }
        let types_arr: Array = options.get("types")?;
        let mut types = Vec::with_capacity(types_arr.len());
        for item in types_arr.iter::<String>() {
            types.push(item?);
        }
        let payload_val: Value = options.get("payload")?;
        let on_end: Option<Function> = options.get("onEnd")?;
        let drag_id = self.dnd.alloc_id();
        let _ = self.bridges.take_active_gesture_grant();
        *self.dnd.internal.borrow_mut() = Some(InternalDrag {
            id: drag_id,
            surface,
            source_node: native_node,
            types,
            payload: Persistent::save(ctx, payload_val),
            on_end: on_end.map(|f| Persistent::save(ctx, f)),
            hovered: Vec::new(),
            accepted: None,
        });

        let handle = Object::new(ctx.clone())?;
        handle.prop("id", data_prop(drag_id as f64))?;
        let weak = Rc::downgrade(self);
        handle.prop(
            "cancel",
            data_prop(Func::from(move |ctx: Ctx<'js>| {
                if let Some(session) = weak.upgrade() {
                    session.cancel_internal_drag_in_ctx(&ctx, Some(drag_id), "cancel")?;
                }
                Ok::<(), rquickjs::Error>(())
            })),
        )?;
        Ok(handle)
    }

    fn cancel_internal_drag_in_ctx<'js>(
        &self,
        ctx: &Ctx<'js>,
        expected_id: Option<u64>,
        reason: &'static str,
    ) -> rquickjs::Result<()> {
        let should_take = self
            .dnd
            .internal
            .borrow()
            .as_ref()
            .is_some_and(|drag| expected_id.is_none_or(|id| drag.id == id));
        if !should_take {
            return Ok(());
        }
        let Some(drag) = self.dnd.internal.borrow_mut().take() else {
            return Ok(());
        };
        if let Some(bridge) = self.bridge_for_surface(drag.surface) {
            for node in drag.hovered {
                if let Some(cb) = bridge.dnd_callback(node, DndEventKind::DragLeave) {
                    let ev = build_drag_event(
                        ctx,
                        drag.id,
                        "internal",
                        &drag.types,
                        0,
                        Point::new(CssPx(0.0), CssPx(0.0)),
                        BoxGeometry::from_window(None, node),
                        DragEffect::Move,
                        Rc::new(Cell::new(None)),
                        Rc::new(Cell::new(false)),
                    )?;
                    cb.restore(ctx)?.call::<_, ()>((ev,))?;
                }
            }
        }
        if let Some(on_end) = drag.on_end {
            let res = Object::new(ctx.clone())?;
            res.prop("dropped", data_prop(false))?;
            res.prop("effect", data_prop("none"))?;
            res.prop("reason", data_prop(reason))?;
            on_end.restore(ctx)?.call::<_, ()>((res,))?;
        }
        Ok(())
    }

    /// Cancels an active internal drag (e.g. Escape, detach, surface loss) inside a bridge turn.
    pub(crate) fn cancel_internal_drag(&self, surface_filter: Option<SurfaceId>, reason: &'static str) -> SessionResult<()> {
        if self.shutdown_started.get() {
            return Ok(());
        }
        let matches = self
            .dnd
            .internal
            .borrow()
            .as_ref()
            .is_some_and(|d| surface_filter.is_none_or(|s| d.surface == s));
        if !matches {
            return Ok(());
        }
        let Ok(mut engine) = self.engine.try_borrow_mut() else {
            return Ok(());
        };
        let Some(engine) = engine.as_mut() else {
            return Ok(());
        };
        self.bridges
            .run(engine.turn(), |js| {
                js.with_context(|ctx| self.cancel_internal_drag_in_ctx(&ctx, None, reason))
            })
            .map_err(|error| self.diagnostic(error).into())
    }

    /// Cancels any active internal or outbound drag whose source node was detached or became
    /// effectively inert, and prunes inert targets from an active external drag.
    pub(crate) fn reconcile_inert_drags(&self) -> SessionResult<()> {
        if self.shutdown_started.get() {
            return Ok(());
        }
        let internal_cancel = {
            let guard = self.dnd.internal.borrow();
            guard.as_ref().and_then(|drag| {
                let bridge = self.bridge_for_surface(drag.surface);
                let attached = bridge
                    .as_ref()
                    .is_some_and(|b| b.is_native_node_attached(drag.source_node));
                if !attached {
                    return Some("detach");
                }
                let inert = bridge
                    .as_ref()
                    .is_some_and(|b| b.is_native_node_effectively_inert(drag.source_node));
                inert.then_some("inert")
            })
        };
        if let Some(reason) = internal_cancel {
            self.cancel_internal_drag(None, reason)?;
        }

        let outbound_cancel = {
            let guard = self.dnd.outbound.borrow();
            guard.as_ref().and_then(|drag| {
                let bridge = self.bridge_for_surface(drag.surface);
                let attached = bridge
                    .as_ref()
                    .is_some_and(|b| b.is_native_node_attached(drag.source_node));
                if !attached {
                    return Some((drag.id, "detach"));
                }
                let inert = bridge
                    .as_ref()
                    .is_some_and(|b| b.is_native_node_effectively_inert(drag.source_node));
                inert.then_some((drag.id, "inert"))
            })
        };
        if let Some((drag_id, reason)) = outbound_cancel
            && let Ok(mut engine) = self.engine.try_borrow_mut()
            && let Some(engine) = engine.as_mut()
        {
            self.bridges
                .run(engine.turn(), |js| {
                    js.with_context(|ctx| {
                        self.cancel_outbound_drag_in_ctx(&ctx, Some(drag_id), reason)
                    })
                })
                .map_err(|error| self.diagnostic(error))?;
        }

        let external_inert = {
            let guard = self.dnd.external.borrow();
            guard.as_ref().and_then(|ext| {
                let bridge = self.bridge_for_surface(ext.surface)?;
                let inert_nodes: Vec<NodeId> = ext
                    .hovered
                    .iter()
                    .copied()
                    .filter(|node| bridge.is_native_node_effectively_inert(*node))
                    .collect();
                if inert_nodes.is_empty() {
                    None
                } else {
                    Some((ext.surface, ext.id, ext.types.clone(), ext.file_count, inert_nodes))
                }
            })
        };
        if let Some((surface, drag_id, types, file_count, inert_nodes)) = external_inert {
            if let Some(ext) = self.dnd.external.borrow_mut().as_mut()
                && ext.id == drag_id
            {
                ext.hovered.retain(|node| !inert_nodes.contains(node));
                if ext
                    .accepted
                    .is_some_and(|(target, _)| inert_nodes.contains(&target))
                {
                    ext.accepted = None;
                }
            }
            if let Some(bridge) = self.bridge_for_surface(surface)
                && let Ok(mut engine) = self.engine.try_borrow_mut()
                && let Some(engine) = engine.as_mut()
            {
                self.bridges
                    .run(engine.turn(), |js| {
                        js.with_context(|ctx| {
                            for node in inert_nodes {
                                if let Some(cb) = bridge.dnd_callback(node, DndEventKind::DragLeave)
                                {
                                    let ev = build_drag_event(
                                        &ctx,
                                        drag_id,
                                        "external",
                                        &types,
                                        file_count,
                                        Point::new(CssPx(0.0), CssPx(0.0)),
                                        BoxGeometry::from_window(None, node),
                                        DragEffect::Copy,
                                        Rc::new(Cell::new(None)),
                                        Rc::new(Cell::new(false)),
                                    )?;
                                    cb.restore(&ctx)?.call::<_, ()>((ev,))?;
                                }
                            }
                            Ok(())
                        })
                    })
                    .map_err(|error| self.diagnostic(error))?;
            }
        }
        Ok(())
    }

    /// Cancels any internal, external, or outbound drag targeting a surface that was just destroyed.
    pub(crate) fn cancel_drags_for_surface(&self, surface: SurfaceId) -> SessionResult<()> {
        if self
            .dnd
            .external
            .borrow()
            .as_ref()
            .is_some_and(|ext| ext.surface == surface)
        {
            self.dnd.external.borrow_mut().take();
        }
        let has_outbound = self
            .dnd
            .outbound
            .borrow()
            .as_ref()
            .is_some_and(|d| d.surface == surface);
        if has_outbound
            && let Ok(mut engine) = self.engine.try_borrow_mut()
            && let Some(engine) = engine.as_mut()
        {
            self.bridges
                .run(engine.turn(), |js| {
                    js.with_context(|ctx| self.cancel_outbound_drag_in_ctx(&ctx, None, "surface-lost"))
                })
                .map_err(|error| self.diagnostic(error))?;
        }
        self.cancel_internal_drag(Some(surface), "surface-lost")
    }

    /// Handles `Escape` cancellation for an active internal drag.
    /// Returns `true` if Escape cancelled an active drag.
    pub(crate) fn handle_drag_key(&self, state: KeyState, key: &KeyEvent) -> SessionResult<bool> {
        if state != KeyState::Pressed
            || key.key != Key::Named(NamedKey::Escape)
            || !self.dnd.has_active_internal()
        {
            return Ok(false);
        }
        self.cancel_internal_drag(None, "escape")?;
        Ok(true)
    }

    /// Handles pointer motion/release/cancel while an internal drag is active.
    pub(crate) fn handle_internal_drag_pointer(
        &self,
        window: &Window,
        surface: SurfaceId,
        action: PointerAction,
        position: Point<CssPx, Css>,
    ) -> SessionResult<()> {
        if self.shutdown_started.get() || !self.dnd.has_active_internal() {
            return Ok(());
        }
        let (drag_surface, source_node) = {
            let guard = self.dnd.internal.borrow();
            let Some(drag) = guard.as_ref() else {
                return Ok(());
            };
            (drag.surface, drag.source_node)
        };
        // Auto-cancel if the source node was detached or became effectively inert while dragging.
        let bridge_for_source = self.bridge_for_surface(drag_surface);
        let live = bridge_for_source
            .as_ref()
            .is_some_and(|bridge| bridge.is_native_node_attached(source_node));
        if !live {
            return self.cancel_internal_drag(None, "detach");
        }
        if bridge_for_source
            .as_ref()
            .is_some_and(|bridge| bridge.is_native_node_effectively_inert(source_node))
        {
            return self.cancel_internal_drag(None, "inert");
        }
        if action == PointerAction::Cancelled {
            return self.cancel_internal_drag(None, "cancel");
        }
        if !matches!(action, PointerAction::Moved | PointerAction::Released) {
            return Ok(());
        }
        let Some(bridge) = self.bridge_for_surface(surface) else {
            return Ok(());
        };
        let chain = window.chain_at_css(position);
        let Ok(mut engine) = self.engine.try_borrow_mut() else {
            return Ok(());
        };
        let Some(engine) = engine.as_mut() else {
            return Ok(());
        };
        self.bridges
            .run(engine.turn(), |js| {
                js.with_context(|ctx| {
                    let (drag_id, types, prev_hovered) = {
                        let guard = self.dnd.internal.borrow();
                        let Some(drag) = guard.as_ref() else {
                            return Ok(());
                        };
                        (drag.id, drag.types.clone(), drag.hovered.clone())
                    };
                    let accepted = self.dispatch_hover_and_over(
                        &ctx,
                        &bridge,
                        Some(window),
                        drag_id,
                        "internal",
                        &types,
                        0,
                        position,
                        DragEffect::Move,
                        &prev_hovered,
                        &chain,
                    )?;
                    if action == PointerAction::Moved {
                        if let Some(drag) = self.dnd.internal.borrow_mut().as_mut()
                            && drag.id == drag_id
                        {
                            drag.hovered = chain;
                            drag.accepted = accepted;
                        }
                        return Ok(());
                    }
                    // PointerAction::Released -> commit drop or cancel.
                    let Some(drag) = self.dnd.internal.borrow_mut().take() else {
                        return Ok(());
                    };
                    if let Some((target_node, effect)) = accepted {
                        let drop_payload = drag.payload.restore(&ctx)?;
                        self.dispatch_drop(
                            &ctx,
                            &bridge,
                            Some(window),
                            drag.id,
                            "internal",
                            &drag.types,
                            position,
                            effect,
                            target_node,
                            &chain,
                            Some(drop_payload),
                            None,
                        )?;
                        if let Some(on_end) = drag.on_end {
                            let res = Object::new(ctx.clone())?;
                            res.prop("dropped", data_prop(true))?;
                            res.prop("effect", data_prop(effect.as_str()))?;
                            res.prop("reason", data_prop("drop"))?;
                            on_end.restore(&ctx)?.call::<_, ()>((res,))?;
                        }
                    } else {
                        for node in chain {
                            if let Some(cb) = bridge.dnd_callback(node, DndEventKind::DragLeave) {
                                let ev = build_drag_event(
                                    &ctx,
                                    drag.id,
                                    "internal",
                                    &drag.types,
                                    0,
                                    position,
                                    BoxGeometry::from_window(Some(window), node),
                                    DragEffect::Move,
                                    Rc::new(Cell::new(None)),
                                    Rc::new(Cell::new(false)),
                                )?;
                                cb.restore(&ctx)?.call::<_, ()>((ev,))?;
                            }
                        }
                        if let Some(on_end) = drag.on_end {
                            let res = Object::new(ctx.clone())?;
                            res.prop("dropped", data_prop(false))?;
                            res.prop("effect", data_prop("none"))?;
                            res.prop("reason", data_prop("cancel"))?;
                            on_end.restore(&ctx)?.call::<_, ()>((res,))?;
                        }
                    }
                    Ok(())
                })
            })
            .map_err(|error| self.diagnostic(error).into())
    }

    /// Handles external platform drag events (`Entered`, `Moved`, `Left`, `Dropped`)
    /// through the exact `Window::chain_at_css` hit-testing pipeline.
    ///
    /// Hover stages expose MIME types & file count only (never raw paths or handles).
    /// Only an accepted drop mints read-only Wave 1 `FileHandle` capabilities.
    pub(crate) fn handle_external_drag(
        self: &Rc<Self>,
        window: &Window,
        surface: SurfaceId,
        event: &DragEvent,
    ) -> SessionResult<()> {
        if self.shutdown_started.get() {
            return Ok(());
        }
        let Some(bridge) = self.bridge_for_surface(surface) else {
            return Ok(());
        };
        let Ok(mut engine) = self.engine.try_borrow_mut() else {
            return Ok(());
        };
        let Some(engine) = engine.as_mut() else {
            return Ok(());
        };
        self.bridges
            .run(engine.turn(), |js| {
                js.with_context(|ctx| match event {
                    DragEvent::Entered { paths, position } => {
                        let types = infer_external_mime_types(paths);
                        let file_count = paths.len();
                        let drag_id = self.dnd.alloc_id();
                        let chain = window.chain_at_css(*position);
                        let accepted = self.dispatch_hover_and_over(
                            &ctx,
                            &bridge,
                            Some(window),
                            drag_id,
                            "external",
                            &types,
                            file_count,
                            *position,
                            DragEffect::Copy,
                            &[],
                            &chain,
                        )?;
                        *self.dnd.external.borrow_mut() = Some(ExternalDrag {
                            id: drag_id,
                            surface,
                            types,
                            file_count,
                            hovered: chain,
                            accepted,
                        });
                        Ok(())
                    }
                    DragEvent::Moved { position } => {
                        let (drag_id, types, file_count, prev_hovered) = {
                            let guard = self.dnd.external.borrow();
                            let Some(ext) = guard.as_ref().filter(|e| e.surface == surface) else {
                                return Ok(());
                            };
                            (ext.id, ext.types.clone(), ext.file_count, ext.hovered.clone())
                        };
                        let chain = window.chain_at_css(*position);
                        let accepted = self.dispatch_hover_and_over(
                            &ctx,
                            &bridge,
                            Some(window),
                            drag_id,
                            "external",
                            &types,
                            file_count,
                            *position,
                            DragEffect::Copy,
                            &prev_hovered,
                            &chain,
                        )?;
                        if let Some(ext) = self.dnd.external.borrow_mut().as_mut()
                            && ext.id == drag_id
                        {
                            ext.hovered = chain;
                            ext.accepted = accepted;
                        }
                        Ok(())
                    }
                    DragEvent::Left => {
                        let Some(ext) = self.dnd.external.borrow_mut().take() else {
                            return Ok(());
                        };
                        for node in ext.hovered {
                            if let Some(cb) = bridge.dnd_callback(node, DndEventKind::DragLeave) {
                                let ev = build_drag_event(
                                    &ctx,
                                    ext.id,
                                    "external",
                                    &ext.types,
                                    ext.file_count,
                                    Point::new(CssPx(0.0), CssPx(0.0)),
                                    BoxGeometry::from_window(Some(window), node),
                                    DragEffect::Copy,
                                    Rc::new(Cell::new(None)),
                                    Rc::new(Cell::new(false)),
                                )?;
                                cb.restore(&ctx)?.call::<_, ()>((ev,))?;
                            }
                        }
                        Ok(())
                    }
                    DragEvent::Dropped { paths, position } => {
                        let existing = self.dnd.external.borrow_mut().take();
                        let (drag_id, types, prev_hovered) = match existing {
                            Some(ext) if ext.surface == surface => {
                                let types = if ext.types.is_empty() {
                                    infer_external_mime_types(paths)
                                } else {
                                    ext.types
                                };
                                (ext.id, types, ext.hovered)
                            }
                            _ => (self.dnd.alloc_id(), infer_external_mime_types(paths), Vec::new()),
                        };
                        let chain = window.chain_at_css(*position);
                        let accepted = self.dispatch_hover_and_over(
                            &ctx,
                            &bridge,
                            Some(window),
                            drag_id,
                            "external",
                            &types,
                            paths.len(),
                            *position,
                            DragEffect::Copy,
                            &prev_hovered,
                            &chain,
                        )?;
                        if let Some((target_node, effect)) = accepted {
                            // Mint strictly read-only Wave 1 FileHandle capabilities ONLY on accepted drop.
                            let files = self.work.mint_dropped_file_handles(&ctx, paths)?;
                            self.dispatch_drop(
                                &ctx,
                                &bridge,
                                Some(window),
                                drag_id,
                                "external",
                                &types,
                                *position,
                                effect,
                                target_node,
                                &chain,
                                None,
                                Some(files),
                            )?;
                        } else {
                            for node in chain {
                                if let Some(cb) = bridge.dnd_callback(node, DndEventKind::DragLeave) {
                                    let ev = build_drag_event(
                                        &ctx,
                                        drag_id,
                                        "external",
                                        &types,
                                        paths.len(),
                                        *position,
                                        BoxGeometry::from_window(Some(window), node),
                                        DragEffect::Copy,
                                        Rc::new(Cell::new(None)),
                                        Rc::new(Cell::new(false)),
                                    )?;
                                    cb.restore(&ctx)?.call::<_, ()>((ev,))?;
                                }
                            }
                        }
                        Ok(())
                    }
                    DragEvent::OutboundFinished { accepted } => {
                        let Some(drag) = self.dnd.outbound.borrow_mut().take() else {
                            return Ok(());
                        };
                        if let Some(on_end) = drag.on_end {
                            let res = Object::new(ctx.clone())?;
                            res.prop("accepted", data_prop(*accepted))?;
                            res.prop("effect", data_prop(if *accepted { "copy" } else { "none" }))?;
                            res.prop("reason", data_prop(if *accepted { "finished" } else { "cancel" }))?;
                            on_end.restore(&ctx)?.call::<_, ()>((res,))?;
                        }
                        Ok(())
                    }
                    _ => Ok(()),
                })
            })
            .map_err(|error| self.diagnostic(error).into())
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_hover_and_over<'js>(
        &self,
        ctx: &Ctx<'js>,
        bridge: &Rc<ui_bridge::UiBridge>,
        window: Option<&Window>,
        drag_id: u64,
        source: &'static str,
        types: &[String],
        file_count: usize,
        position: Point<CssPx, Css>,
        default_effect: DragEffect,
        prev_hovered: &[NodeId],
        chain: &[NodeId],
    ) -> rquickjs::Result<Option<(NodeId, DragEffect)>> {
        // 1. Fire onDragLeave on nodes no longer under the pointer.
        for &node in prev_hovered {
            if !chain.contains(&node)
                && let Some(cb) = bridge.dnd_callback(node, DndEventKind::DragLeave)
            {
                let ev = build_drag_event(
                    ctx,
                    drag_id,
                    source,
                    types,
                    file_count,
                    position,
                    BoxGeometry::from_window(window, node),
                    default_effect,
                    Rc::new(Cell::new(None)),
                    Rc::new(Cell::new(false)),
                )?;
                cb.restore(ctx)?.call::<_, ()>((ev,))?;
            }
        }
        // 2. Fire onDragEnter on newly entered nodes (root to leaf).
        for &node in chain {
            if !prev_hovered.contains(&node)
                && let Some(cb) = bridge.dnd_callback(node, DndEventKind::DragEnter)
            {
                let ev = build_drag_event(
                    ctx,
                    drag_id,
                    source,
                    types,
                    file_count,
                    position,
                    BoxGeometry::from_window(window, node),
                    default_effect,
                    Rc::new(Cell::new(None)),
                    Rc::new(Cell::new(false)),
                )?;
                cb.restore(ctx)?.call::<_, ()>((ev,))?;
            }
        }
        // 3. Fire synchronous onDragOver bubbling from leaf to root; first acceptor wins.
        let mut accepted: Option<(NodeId, DragEffect)> = None;
        let stopped = Rc::new(Cell::new(false));
        for &node in chain.iter().rev() {
            if stopped.get() {
                break;
            }
            if let Some(cb) = bridge.dnd_callback(node, DndEventKind::DragOver) {
                let acc_cell = Rc::new(Cell::new(None));
                let ev = build_drag_event(
                    ctx,
                    drag_id,
                    source,
                    types,
                    file_count,
                    position,
                    BoxGeometry::from_window(window, node),
                    default_effect,
                    acc_cell.clone(),
                    stopped.clone(),
                )?;
                cb.restore(ctx)?.call::<_, ()>((ev,))?;
                if accepted.is_none()
                    && let Some(effect) = acc_cell.get()
                {
                    accepted = Some((node, effect));
                }
            }
        }
        Ok(accepted)
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_drop<'js>(
        &self,
        ctx: &Ctx<'js>,
        bridge: &Rc<ui_bridge::UiBridge>,
        window: Option<&Window>,
        drag_id: u64,
        source: &'static str,
        types: &[String],
        position: Point<CssPx, Css>,
        effect: DragEffect,
        target_node: NodeId,
        chain: &[NodeId],
        payload: Option<Value<'js>>,
        files: Option<Array<'js>>,
    ) -> rquickjs::Result<()> {
        let stopped = Rc::new(Cell::new(false));
        // Bubble from the accepting target up through its ancestors in `chain`.
        let target_idx = chain.iter().position(|&n| n == target_node);
        let slice = match target_idx {
            Some(idx) => &chain[..=idx],
            None => chain,
        };
        let files_len = files.as_ref().map_or(0, Array::len);
        for &node in slice.iter().rev() {
            if stopped.get() {
                break;
            }
            if let Some(cb) = bridge.dnd_callback(node, DndEventKind::Drop) {
                let geom = BoxGeometry::from_window(window, node);
                let ev = Object::new(ctx.clone())?;
                ev.prop("dragId", data_prop(drag_id as f64))?;
                ev.prop("source", data_prop(source))?;
                ev.prop("types", data_prop(types_to_js(ctx, types)?))?;
                ev.prop("effect", data_prop(effect.as_str()))?;
                ev.prop("fileCount", data_prop(files_len as f64))?;
                ev.prop("clientX", data_prop(f64::from(position.x.0)))?;
                ev.prop("clientY", data_prop(f64::from(position.y.0)))?;
                ev.prop("localX", data_prop(f64::from(position.x.0 - geom.x)))?;
                ev.prop("localY", data_prop(f64::from(position.y.0 - geom.y)))?;
                ev.prop("targetWidth", data_prop(f64::from(geom.width)))?;
                ev.prop("targetHeight", data_prop(f64::from(geom.height)))?;
                if let Some(p) = payload.clone() {
                    ev.prop("payload", data_prop(p))?;
                } else {
                    ev.prop("payload", data_prop(Value::new_undefined(ctx.clone())))?;
                }
                if let Some(f) = files.clone() {
                    ev.prop("files", data_prop(f))?;
                } else {
                    ev.prop("files", data_prop(Array::new(ctx.clone())?))?;
                }
                let stop_cell = stopped.clone();
                ev.prop(
                    "stopPropagation",
                    data_prop(Func::from(move || {
                        stop_cell.set(true);
                    })),
                )?;
                cb.restore(ctx)?.call::<_, ()>((ev,))?;
            }
        }
        Ok(())
    }
}
