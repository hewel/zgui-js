//! Dev-only, host-owned, strictly read-only Desktop Runtime Inspector & Profiler.
//!
//! Shares the native-only bounded tree/style snapshot primitives with preview
//! inspection, pairs them with `RuntimeTelemetry`, `ResourceCounts`, and
//! `CompletedFrame` phase timings, and enforces:
//! - zero app JS getter/eval execution,
//! - mandatory redaction of secure/password fields,
//! - mandatory exclusion of `SessionLockSurface` content,
//! - explicit labeling of historical/incoherent frames,
//! - zero overhead when disabled (no `FrameProbe`, no ring buffer, no socket).

use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use serde_json::{Value, json};
use zgui::{
    platform::{SurfaceId, WakeReason, Waker},
    reactive::prelude::GetUntracked,
    runtime::{FrameProbe, Runtime, Window},
    view::{AttrName, ClassName, ElementName, NodeId, ViewHost},
    vocab::{Key, KeyEvent, KeyState, Modifiers, NamedKey, PointerAction, PointerButton, PointerEvent, Role},
};

use crate::{
    preview::evidence::{Bounds, ScrollMetrics},
    session::{ResourceCounts, RuntimeTelemetry, Session, SessionRegion},
    session_host::MountRegion,
};

const MAX_RING_SAMPLES: usize = 256;
const MAX_DEV_NODES: usize = 4096;
const MAX_DEV_TEXT_BYTES: usize = 128 * 1024;
const MAX_DEV_NODE_TEXT_BYTES: usize = 4096;
const MAX_DEV_VISITS: usize = 100_000;
const MAX_REQUEST_LINE_BYTES: usize = 4 * 1024;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
pub(crate) const PICKER_SHEET_NAME: &str = "host-dev-picker";
const PICKER_ATTR_MARKER: &str = "data-zgui-picker-overlay";

const PICKER_SHEET: &str = r#"
.zgui-picker-overlay {
    position: fixed;
    left: 0;
    top: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
    z-index: 2147483646;
}
.zgui-picker-margin {
    position: fixed;
    pointer-events: none;
    background: rgba(249, 115, 22, 0.22);
    border: 1px solid rgba(249, 115, 22, 0.7);
}
.zgui-picker-padding {
    position: fixed;
    pointer-events: none;
    background: rgba(34, 197, 94, 0.24);
    border: 1px solid rgba(34, 197, 94, 0.75);
}
.zgui-picker-content {
    position: fixed;
    pointer-events: none;
    background: rgba(59, 130, 246, 0.26);
    border: 2px solid #3b82f6;
}
.zgui-picker-hud {
    position: fixed;
    pointer-events: none;
    padding: 4px 8px;
    border-radius: 6px;
    background: rgba(15, 23, 42, 0.94);
    border: 1px solid #3b82f6;
    color: #f8fafc;
    font-family: monospace;
    font-size: 11px;
}
"#;

/// Visibility means a positive native box with computed `visibility: visible`,
/// not pointer reachability. Shared between preview inspection and dev inspector.
pub(crate) fn native_state(window: &Window, native: NodeId) -> Option<(bool, bool)> {
    let key = zgui_view_dom::id::to_document(native)?;
    let document = window.document().borrow();
    let mut next = Some(document.store().index_of(key)?);
    let mut enabled = true;
    while let Some(index) = next {
        let core = document.store().core(index);
        enabled &= !core.ui_state().contains(zgui::vocab::UiState::DISABLED);
        next = core.parent();
    }
    let layout = window.layout().borrow();
    let visible = layout.boxes_of(key).iter().any(|box_| {
        layout.get(*box_).is_some_and(|node| {
            node.style.get_inherited_box().visibility
                == zgui_css::values::size::VisibilityValue::Visible
        }) && layout.fragments_of_box(*box_).iter().any(|fragment| {
            layout.fragment(*fragment).is_some_and(|fragment| {
                fragment.border_box.size.width.0 > 0.0 && fragment.border_box.size.height.0 > 0.0
            })
        })
    });
    Some((enabled, visible))
}

/// Read scroll metrics for a native scroll container without running JS.
pub(crate) fn scroll_metrics(window: &Window, native: NodeId, scale: f64) -> Option<ScrollMetrics> {
    let key = zgui_view_dom::id::to_document(native)?;
    let region = {
        let layout = window.layout().borrow();
        zgui_layout::scroll_region::region_of_element(&layout, key)?
    };
    let position = ViewHost::scroll_position(window.host().as_ref(), native);
    let epsilon = f64::from(zgui_layout::scroll_region::bar::EPSILON);
    let viewport_width = f64::from(region.scrollport.size.width.0);
    let viewport_height = f64::from(region.scrollport.size.height.0);
    let content_width = f64::from(region.content.width.0);
    let content_height = f64::from(region.content.height.0);
    Some(ScrollMetrics {
        offset_x: f64::from(position.offset.x.0) / scale,
        offset_y: f64::from(position.offset.y.0) / scale,
        viewport_width: viewport_width / scale,
        viewport_height: viewport_height / scale,
        content_width: content_width / scale,
        content_height: content_height / scale,
        overflow_x: region.scrolls_x && content_width > viewport_width + epsilon,
        overflow_y: region.scrolls_y && content_height > viewport_height + epsilon,
        scrolls_x: region.scrolls_x,
        scrolls_y: region.scrolls_y,
    })
}

/// Bounded native text walk across a DOM subtree.
pub(crate) fn bounded_text(
    document: &zgui_dom::Document,
    root: zgui_dom::NodeIndex,
    bytes: usize,
    visits: &mut usize,
) -> (String, bool) {
    let mut text = String::new();
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if *visits == 0 {
            return (text, true);
        }
        *visits -= 1;
        if let Some(own) = zgui_dom::text::node::text_of(document.store(), index) {
            let keep = own.floor_char_boundary(bytes.saturating_sub(text.len()).min(own.len()));
            text.push_str(&own[..keep]);
            if keep < own.len() {
                return (text, true);
            }
        }
        let core = document.store().core(index);
        if index != root
            && let Some(next) = core.next_sibling()
        {
            pending.push(next);
        }
        if let Some(child) = core.first_child() {
            pending.push(child);
        }
    }
    (text, false)
}

/// Whether `native` or any of its ancestors is a secure / password text control.
pub(crate) fn is_secure_node(window: &Window, native: NodeId) -> bool {
    if window
        .dom()
        .semantics(native)
        .is_some_and(|sem| sem.role == Role::PasswordInput)
    {
        return true;
    }
    let Some(key) = zgui_view_dom::id::to_document(native) else {
        return false;
    };
    let document = window.document().borrow();
    let mut next = document.store().index_of(key);
    while let Some(index) = next {
        let record = document.store().core(index);
        if zgui_dom::Node::new(record).attr("secure").is_some() {
            return true;
        }
        next = record.parent();
    }
    false
}

/// Bounded native text extraction with mandatory secure/password redaction for
/// the dev inspector (never copies raw secret text from secure subtrees).
fn bounded_redacted_text(
    window: &Window,
    document: &zgui_dom::Document,
    native: NodeId,
    root: zgui_dom::NodeIndex,
    bytes: usize,
    visits: &mut usize,
) -> (String, bool, bool) {
    if is_secure_node(window, native) {
        let (raw, truncated) = bounded_text(document, root, 1, visits);
        let redacted = if raw.is_empty() {
            String::new()
        } else {
            "[REDACTED]".to_owned()
        };
        return (redacted, truncated, true);
    }
    // Walk descendants, redacting any nested secure subtrees without exposing raw bytes.
    let mut text = String::new();
    let mut pending = vec![root];
    let mut redacted_any = false;
    while let Some(index) = pending.pop() {
        if *visits == 0 {
            return (text, true, redacted_any);
        }
        *visits -= 1;
        let record = document.store().core(index);
        if zgui_dom::Node::new(record).attr("secure").is_some() {
            redacted_any = true;
            let (raw, _) = bounded_text(document, index, 1, visits);
            if !raw.is_empty() && text.len() + "[REDACTED]".len() <= bytes {
                text.push_str("[REDACTED]");
            }
            let core = document.store().core(index);
            if index != root
                && let Some(next) = core.next_sibling()
            {
                pending.push(next);
            }
            continue;
        }
        if let Some(own) = zgui_dom::text::node::text_of(document.store(), index) {
            let keep = own.floor_char_boundary(bytes.saturating_sub(text.len()).min(own.len()));
            text.push_str(&own[..keep]);
            if keep < own.len() {
                return (text, true, redacted_any);
            }
        }
        let core = document.store().core(index);
        if index != root
            && let Some(next) = core.next_sibling()
        {
            pending.push(next);
        }
        if let Some(child) = core.first_child() {
            pending.push(child);
        }
    }
    (text, false, redacted_any)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameSample {
    pub surface_id: u64,
    pub ordinal: u64,
    pub outcome: String,
    pub document_revision: u64,
    pub style_revision: u64,
    pub layout_revision: Option<u64>,
    pub coherent: bool,
    pub style_us: u64,
    pub layout_us: u64,
    pub paint_us: u64,
    pub total_us: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckpointSample {
    pub generation: u64,
    pub queue_depth: usize,
    pub js_entry_max_us: u64,
    pub js_heap_bytes: u64,
}

#[derive(Default)]
struct ProfilerRing {
    frames: VecDeque<FrameSample>,
    checkpoints: VecDeque<CheckpointSample>,
}

impl ProfilerRing {
    fn push_frame(&mut self, sample: FrameSample) {
        if self.frames.len() >= MAX_RING_SAMPLES {
            self.frames.pop_front();
        }
        self.frames.push_back(sample);
    }

    fn push_checkpoint(&mut self, sample: CheckpointSample) {
        if self.checkpoints.len() >= MAX_RING_SAMPLES {
            self.checkpoints.pop_front();
        }
        self.checkpoints.push_back(sample);
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoxEdges {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoxModel {
    pub border_box: Bounds,
    pub content_box: Bounds,
    pub padding: BoxEdges,
    pub border: BoxEdges,
    pub margin: BoxEdges,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComputedStyleSummary {
    pub display: String,
    pub position: String,
    pub visibility: String,
    pub opacity: f32,
    pub font_size: f32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickerTarget {
    pub surface: String,
    pub token: String,
    pub kind: String,
    pub test_id: Option<String>,
    pub classes: Vec<String>,
    pub source: Option<String>,
    pub box_model: BoxModel,
}

struct PickerOverlay {
    region: MountRegion,
    tree: Option<NodeId>,
}

impl PickerOverlay {
    fn show(region: MountRegion, target: &PickerTarget) -> Self {
        let dom = &region.dom;
        let tree = dom.create_element(ElementName::new("box"));
        dom.set_classes(tree, &[ClassName::new("zgui-picker-overlay")]);
        dom.set_attribute(tree, AttrName::new(PICKER_ATTR_MARKER), Some("1"));
        dom.set_attribute(tree, AttrName::new("data-testid"), Some("zgui-picker-overlay"));

        let bb = &target.box_model.border_box;
        let m = &target.box_model.margin;
        let cb = &target.box_model.content_box;

        // Margin box
        if m.top > 0.0 || m.right > 0.0 || m.bottom > 0.0 || m.left > 0.0 {
            let margin_el = dom.create_element(ElementName::new("box"));
            dom.set_classes(margin_el, &[ClassName::new("zgui-picker-margin")]);
            dom.set_attribute(
                margin_el,
                AttrName::new("style"),
                Some(&format!(
                    "left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px;",
                    (bb.x - m.left).max(0.0),
                    (bb.y - m.top).max(0.0),
                    (bb.width + m.left + m.right).max(1.0),
                    (bb.height + m.top + m.bottom).max(1.0),
                )),
            );
            dom.insert(tree, margin_el, None);
        }

        // Padding / border box
        let pad_el = dom.create_element(ElementName::new("box"));
        dom.set_classes(pad_el, &[ClassName::new("zgui-picker-padding")]);
        dom.set_attribute(
            pad_el,
            AttrName::new("style"),
            Some(&format!(
                "left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px;",
                bb.x.max(0.0),
                bb.y.max(0.0),
                bb.width.max(1.0),
                bb.height.max(1.0),
            )),
        );
        dom.insert(tree, pad_el, None);

        // Content box
        let content_el = dom.create_element(ElementName::new("box"));
        dom.set_classes(content_el, &[ClassName::new("zgui-picker-content")]);
        dom.set_attribute(
            content_el,
            AttrName::new("style"),
            Some(&format!(
                "left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px;",
                cb.x.max(0.0),
                cb.y.max(0.0),
                cb.width.max(1.0),
                cb.height.max(1.0),
            )),
        );
        dom.insert(tree, content_el, None);

        // Concise HUD label (never reads raw text content, preventing secret leaks).
        let class_tag = if target.classes.is_empty() {
            String::new()
        } else {
            format!(".{}", target.classes.iter().take(2).cloned().collect::<Vec<_>>().join("."))
        };
        let source_tag = target
            .source
            .as_deref()
            .map(|src| format!(" | {src}"))
            .unwrap_or_default();
        let hud_text = format!(
            "<{}{}> {:.0}x{:.0}{}",
            target.kind, class_tag, bb.width, bb.height, source_tag
        );
        let hud_el = dom.create_element(ElementName::new("text"));
        dom.set_classes(hud_el, &[ClassName::new("zgui-picker-hud")]);
        dom.set_attribute(hud_el, AttrName::new("data-testid"), Some("zgui-picker-hud"));
        let hud_y = if bb.y >= 28.0 { bb.y - 26.0 } else { bb.y + bb.height + 4.0 };
        dom.set_attribute(
            hud_el,
            AttrName::new("style"),
            Some(&format!("left:{:.1}px;top:{:.1}px;", bb.x.max(4.0), hud_y.max(4.0))),
        );
        dom.insert(hud_el, dom.create_text(&hud_text), None);
        dom.insert(tree, hud_el, None);

        region.host.install_stylesheet(PICKER_SHEET_NAME, PICKER_SHEET);
        let container = dom.parent(region.root).unwrap_or(region.root);
        dom.insert(container, tree, None);
        Self { region, tree: Some(tree) }
    }

    fn clear(&mut self) {
        let Some(tree) = self.tree.take() else {
            return;
        };
        self.region.dom.detach(tree);
        self.region.host.remove_stylesheet(PICKER_SHEET_NAME);
    }
}

impl Drop for PickerOverlay {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Default)]
struct PickerState {
    active: bool,
    hovered: Option<PickerTarget>,
    selected: Option<PickerTarget>,
    overlay: Option<PickerOverlay>,
}

/// Dev-only inspector state & bounded profiler ring buffer. Installed only when
/// explicitly enabled via `--dev-inspect`; absent (`None`) in non-dev/release runs.
pub(crate) struct DevInspector {
    ring: RefCell<ProfilerRing>,
    picker: RefCell<PickerState>,
}

impl DevInspector {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            ring: RefCell::new(ProfilerRing::default()),
            picker: RefCell::new(PickerState::default()),
        })
    }

    pub(crate) fn clear_overlay(&self) {
        let mut picker = self.picker.borrow_mut();
        if let Some(mut overlay) = picker.overlay.take() {
            overlay.clear();
        }
        picker.hovered = None;
    }

    pub(crate) fn picker_snapshot(&self, session_locked: bool) -> Value {
        if session_locked {
            self.clear_overlay();
            return json!({
                "active": false,
                "forbidden": true,
                "reason": "session-lock-surface-forbidden",
                "hovered": null,
                "selected": null,
            });
        }
        let picker = self.picker.borrow();
        json!({
            "active": picker.active,
            "hovered": picker.hovered,
            "selected": picker.selected,
        })
    }

    pub(crate) fn set_picker_active(
        &self,
        runtime: &Runtime,
        session: Option<&Session>,
        active: bool,
    ) -> Result<Value, &'static str> {
        let session_locked = session.is_some_and(|s| s.shell.lock_is_retained());
        if session_locked {
            let mut picker = self.picker.borrow_mut();
            picker.active = false;
            picker.hovered = None;
            picker.selected = None;
            if let Some(mut overlay) = picker.overlay.take() {
                overlay.clear();
            }
            return Err("session-lock-surface-forbidden");
        }
        {
            let mut picker = self.picker.borrow_mut();
            picker.active = active;
            if !active {
                picker.hovered = None;
                if let Some(mut overlay) = picker.overlay.take() {
                    overlay.clear();
                }
            }
        }
        for window in runtime.windows() {
            window.request_frame();
        }
        Ok(self.picker_snapshot(false))
    }

    pub(crate) fn handle_pick_command(
        &self,
        runtime: &Runtime,
        session: Option<&Session>,
        action: &str,
        surface_filter: Option<&str>,
        token: Option<&str>,
    ) -> Value {
        let session_locked = session.is_some_and(|s| s.shell.lock_is_retained());
        if session_locked {
            self.clear_overlay();
            return json!({
                "error": "session-lock-surface-forbidden",
                "picker": self.picker_snapshot(true),
            });
        }
        match action {
            "enable" => match self.set_picker_active(runtime, session, true) {
                Ok(picker) => json!({ "ok": true, "picker": picker }),
                Err(reason) => json!({ "error": reason }),
            },
            "disable" => match self.set_picker_active(runtime, session, false) {
                Ok(picker) => json!({ "ok": true, "picker": picker }),
                Err(reason) => json!({ "error": reason }),
            },
            "toggle" => {
                let next = !self.picker.borrow().active;
                match self.set_picker_active(runtime, session, next) {
                    Ok(picker) => json!({ "ok": true, "picker": picker }),
                    Err(reason) => json!({ "error": reason }),
                }
            }
            "status" => json!({ "ok": true, "picker": self.picker_snapshot(false) }),
            "select" => {
                let Some(token) = token else {
                    return json!({ "error": "pick select requires token" });
                };
                if let Some((region, target)) =
                    find_target_by_token(runtime, session, surface_filter, token)
                {
                    let mut picker = self.picker.borrow_mut();
                    if let Some(mut old) = picker.overlay.take() {
                        old.clear();
                    }
                    picker.overlay = Some(PickerOverlay::show(region.region.clone(), &target));
                    picker.selected = Some(target);
                    picker.active = false;
                    drop(picker);
                    if let Some(win) = runtime
                        .windows()
                        .iter()
                        .find(|w| w.document_id() == region.region.document)
                    {
                        win.request_frame();
                    }
                    json!({ "ok": true, "picker": self.picker_snapshot(false) })
                } else {
                    json!({ "error": "node token not found or layout incoherent", "picker": self.picker_snapshot(false) })
                }
            }
            _ => json!({ "error": "unsupported pick action" }),
        }
    }

    /// Handle Ctrl+Shift+I (toggle picker) and Escape (cancel picker).
    pub(crate) fn handle_key_event(
        &self,
        runtime: &Runtime,
        session: Option<&Session>,
        state: KeyState,
        key_ev: &KeyEvent,
        modifiers: Modifiers,
    ) -> bool {
        if state != KeyState::Pressed {
            return false;
        }
        let ctrl_shift = modifiers.contains(Modifiers::CONTROL) && modifiers.contains(Modifiers::SHIFT);
        if ctrl_shift && matches!(&key_ev.key, Key::Character(ch) if ch.eq_ignore_ascii_case("i")) {
            let next = !self.picker.borrow().active;
            let _ = self.set_picker_active(runtime, session, next);
            return true;
        }
        if self.picker.borrow().active && matches!(&key_ev.key, Key::Named(NamedKey::Escape)) {
            let _ = self.set_picker_active(runtime, session, false);
            return true;
        }
        false
    }

    /// Pure native hover hit-testing + click selection while visual picker is active.
    pub(crate) fn handle_pointer_event(
        &self,
        runtime: &Runtime,
        session: Option<&Session>,
        surface: SurfaceId,
        action: PointerAction,
        ptr: &PointerEvent,
    ) -> bool {
        if !self.picker.borrow().active {
            return false;
        }
        let Some(session) = session else {
            return false;
        };
        if session.shell.lock_is_retained() {
            let _ = self.set_picker_active(runtime, Some(session), false);
            return false;
        }
        let Some((surface_name, region)) = session
            .mount_regions()
            .into_iter()
            .find(|(_, r)| r.surface == surface)
        else {
            return false;
        };
        if region.is_lock_surface() {
            let _ = self.set_picker_active(runtime, Some(session), false);
            return false;
        }
        let Some(window) = runtime
            .windows()
            .iter()
            .find(|w| w.document_id() == region.region.document)
        else {
            return false;
        };
        let x = f64::from(ptr.position.x.0);
        let y = f64::from(ptr.position.y.0);
        let hit = hit_test_region(&surface_name, &region, window, x, y);
        match action {
            PointerAction::Moved => {
                let mut picker = self.picker.borrow_mut();
                let same_target = match (&picker.hovered, &hit) {
                    (Some(prev), Some(next)) => {
                        prev.surface == next.surface
                            && prev.token == next.token
                            && prev.box_model.border_box == next.box_model.border_box
                    }
                    (None, None) => true,
                    _ => false,
                };
                if !same_target {
                    if let Some(mut old) = picker.overlay.take() {
                        old.clear();
                    }
                    if let Some(target) = hit {
                        picker.overlay = Some(PickerOverlay::show(region.region.clone(), &target));
                        picker.hovered = Some(target);
                    } else {
                        picker.hovered = None;
                    }
                    window.request_frame();
                }
                true
            }
            PointerAction::Pressed if ptr.button == Some(PointerButton::Primary) => {
                let mut picker = self.picker.borrow_mut();
                let chosen = hit.or_else(|| picker.hovered.clone());
                if let Some(mut old) = picker.overlay.take() {
                    old.clear();
                }
                if let Some(target) = chosen {
                    picker.overlay = Some(PickerOverlay::show(region.region.clone(), &target));
                    picker.selected = Some(target);
                }
                picker.hovered = None;
                picker.active = false;
                window.request_frame();
                true
            }
            _ => true,
        }
    }

    pub(crate) fn probe(self: &Rc<Self>) -> Rc<dyn FrameProbe> {
        Rc::new(DevFrameProbe {
            inspector: Rc::clone(self),
        })
    }

    pub(crate) fn record_checkpoint(
        &self,
        generation: u64,
        queue_depth: usize,
        telemetry: Option<RuntimeTelemetry>,
    ) {
        let (js_entry_max_us, js_heap_bytes) = telemetry.map_or((0, 0), |t| {
            (t.max_entry_duration.as_micros() as u64, t.js_heap_bytes)
        });
        self.ring.borrow_mut().push_checkpoint(CheckpointSample {
            generation,
            queue_depth,
            js_entry_max_us,
            js_heap_bytes,
        });
    }

    fn record_frame(&self, window: &Window) {
        let Some(completed) = window.completed_frame() else {
            return;
        };
        let surface_id = format!("{:?}", window.surface().id());
        // Extract numeric surface id if formatted as SurfaceId(n) or fallback to ordinal hash.
        let numeric_surface = surface_id
            .trim_start_matches("SurfaceId(")
            .trim_end_matches(')')
            .parse::<u64>()
            .unwrap_or(0);
        self.ring.borrow_mut().push_frame(FrameSample {
            surface_id: numeric_surface,
            ordinal: completed.ordinal,
            outcome: format!("{:?}", completed.outcome),
            document_revision: completed.document_revision,
            style_revision: completed.style_revision,
            layout_revision: completed.layout_revision,
            coherent: completed.layout_document_coherent,
            style_us: completed.phases.style_us,
            layout_us: completed.phases.layout_us,
            paint_us: completed.phases.paint_us,
            total_us: completed.phases.total_us,
        });
    }

    pub(crate) fn profiler_snapshot(&self) -> Value {
        let ring = self.ring.borrow();
        json!({
            "maxSamples": MAX_RING_SAMPLES,
            "frameCount": ring.frames.len(),
            "checkpointCount": ring.checkpoints.len(),
            "frames": ring.frames,
            "checkpoints": ring.checkpoints,
        })
    }

    /// Build a strictly read-only, native-only inspection payload.
    pub(crate) fn capture_snapshot(
        &self,
        runtime: &Runtime,
        session: Option<&Session>,
        include_tree: bool,
        include_profile: bool,
        surface_filter: Option<&str>,
    ) -> Value {
        let session_locked = session.is_some_and(|s| s.shell.lock_is_retained());
        let generation = session.map_or(0, |s| s.generation);
        let resources: ResourceCounts = session.map_or_else(ResourceCounts::default, Session::counts);
        let telemetry = session.and_then(Session::live_telemetry);

        let mut surfaces = Vec::new();
        if let Some(session) = session {
            for (name, region) in session.mount_regions() {
                if let Some(filter) = surface_filter
                    && filter != name
                {
                    continue;
                }
                let window = runtime
                    .windows()
                    .iter()
                    .find(|w| w.document_id() == region.region.document);
                surfaces.push(snapshot_surface(
                    &name,
                    &region,
                    window,
                    generation,
                    session_locked,
                    include_tree,
                ));
            }
        }

        let mut response = json!({
            "schemaVersion": 1,
            "devOnly": true,
            "readOnly": true,
            "generation": generation,
            "sessionLocked": session_locked,
            "resources": resources,
            "telemetry": telemetry.map(|t| json!({
                "maxEntryDurationUs": t.max_entry_duration.as_micros() as u64,
                "jsHeapBytes": t.js_heap_bytes,
                "jsHeapLiveBytes": t.js_heap_live_bytes,
                "jsStackLimitBytes": t.js_stack_limit_bytes,
            })),
            "picker": self.picker_snapshot(session_locked),
            "surfaces": surfaces,
        });
        if include_profile {
            response["profiler"] = self.profiler_snapshot();
        }
        response
    }
}

struct DevFrameProbe {
    inspector: Rc<DevInspector>,
}

impl FrameProbe for DevFrameProbe {
    fn frame_ended(&self, window: &Window) {
        self.inspector.record_frame(window);
    }

    fn describe(&self) -> &str {
        "zgui-js dev inspector frame probe"
    }
}

fn node_box_and_style(
    window: &Window,
    native: NodeId,
    scale: f64,
    bounds_logical: Option<&Bounds>,
) -> (Option<BoxModel>, Option<ComputedStyleSummary>) {
    let Some(key) = zgui_view_dom::id::to_document(native) else {
        return (None, None);
    };
    let layout = window.layout().borrow();
    let Some(&box_key) = layout.boxes_of(key).first() else {
        return (None, None);
    };
    let Some(layout_node) = layout.get(box_key) else {
        return (None, None);
    };
    let style = &layout_node.style;
    let computed = ComputedStyleSummary {
        display: format!("{:?}", style.get_box().display),
        position: format!("{:?}", style.get_box().position),
        visibility: format!("{:?}", style.get_inherited_box().visibility),
        opacity: style.get_effects().opacity,
        font_size: style.get_font().font_size.computed_size().px(),
    };
    let box_model = bounds_logical.map(|bb| {
        let (padding, border) = layout
            .fragments_of_box(box_key)
            .first()
            .and_then(|frag_key| layout.fragment(*frag_key))
            .map_or((BoxEdges::default(), BoxEdges::default()), |frag| {
                (
                    BoxEdges {
                        top: f64::from(frag.padding.top.0) / scale,
                        right: f64::from(frag.padding.right.0) / scale,
                        bottom: f64::from(frag.padding.bottom.0) / scale,
                        left: f64::from(frag.padding.left.0) / scale,
                    },
                    BoxEdges {
                        top: f64::from(frag.border.top.0) / scale,
                        right: f64::from(frag.border.right.0) / scale,
                        bottom: f64::from(frag.border.bottom.0) / scale,
                        left: f64::from(frag.border.left.0) / scale,
                    },
                )
            });
        let margin_px = |value: &zgui_css::values::size::MarginValue| -> f64 {
            match value {
                zgui_css::values::size::MarginValue::LengthPercentage(inner)
                | zgui_css::values::size::MarginValue::AnchorContainingCalcFunction(inner) => {
                    inner.to_length().map_or(0.0, |l| f64::from(l.px()))
                }
                _ => 0.0,
            }
        };
        let m = style.get_margin();
        let margin = BoxEdges {
            top: margin_px(&m.margin_top),
            right: margin_px(&m.margin_right),
            bottom: margin_px(&m.margin_bottom),
            left: margin_px(&m.margin_left),
        };
        let content_box = Bounds {
            x: bb.x + border.left + padding.left,
            y: bb.y + border.top + padding.top,
            width: (bb.width - border.left - border.right - padding.left - padding.right).max(0.0),
            height: (bb.height - border.top - border.bottom - padding.top - padding.bottom).max(0.0),
        };
        BoxModel {
            border_box: bb.clone(),
            content_box,
            padding,
            border,
            margin,
        }
    });
    (box_model, Some(computed))
}

fn node_classes_and_source(
    document: &zgui_dom::Document,
    index: zgui_dom::NodeIndex,
) -> (Vec<String>, Option<String>) {
    let classes = document
        .store()
        .classes_of(index)
        .iter()
        .map(|atom| atom.0.to_string())
        .collect();
    let source = zgui_dom::Node::new(document.store().core(index))
        .attr("data-zgui-source")
        .map(|s| s.as_str().to_owned());
    (classes, source)
}

fn hit_test_region(
    surface_name: &str,
    region: &SessionRegion,
    window: &Window,
    x: f64,
    y: f64,
) -> Option<PickerTarget> {
    if !window
        .completed_frame()
        .is_some_and(|frame| frame.layout_document_coherent)
    {
        return None;
    }
    let owned = region.bridge.snapshot(MAX_DEV_NODES);
    let document = window.document().borrow();
    let scale = window.surface().scale_factor();
    let mut best: Option<(f64, usize, PickerTarget)> = None;
    for (order, node) in owned.nodes.iter().enumerate() {
        let Some(bounds) = ViewHost::window_box(window.host().as_ref(), node.native).map(|b| Bounds {
            x: f64::from(b.origin.x.0) / scale,
            y: f64::from(b.origin.y.0) / scale,
            width: f64::from(b.size.width.0) / scale,
            height: f64::from(b.size.height.0) / scale,
        }) else {
            continue;
        };
        if bounds.width <= 0.0
            || bounds.height <= 0.0
            || x < bounds.x
            || y < bounds.y
            || x > bounds.x + bounds.width
            || y > bounds.y + bounds.height
        {
            continue;
        }
        let index = zgui_view_dom::id::resolve(&document, node.native);
        let (classes, source) = node_classes_and_source(&document, index);
        let (Some(box_model), _) = node_box_and_style(window, node.native, scale, Some(&bounds)) else {
            continue;
        };
        let area = bounds.width * bounds.height;
        let better = best
            .as_ref()
            .is_none_or(|(best_area, best_order, _)| area < *best_area || (area == *best_area && order > *best_order));
        if better {
            best = Some((
                area,
                order,
                PickerTarget {
                    surface: surface_name.to_owned(),
                    token: format!("node-{}", node.token),
                    kind: node.kind.to_string(),
                    test_id: node.test_id.clone(),
                    classes,
                    source,
                    box_model,
                },
            ));
        }
    }
    best.map(|(_, _, target)| target)
}

fn find_target_by_token(
    runtime: &Runtime,
    session: Option<&Session>,
    surface_filter: Option<&str>,
    token: &str,
) -> Option<(Rc<SessionRegion>, PickerTarget)> {
    let session = session?;
    for (name, region) in session.mount_regions() {
        if region.is_lock_surface() {
            continue;
        }
        if let Some(filter) = surface_filter
            && filter != name
        {
            continue;
        }
        let Some(window) = runtime
            .windows()
            .iter()
            .find(|w| w.document_id() == region.region.document)
        else {
            continue;
        };
        if !window
            .completed_frame()
            .is_some_and(|frame| frame.layout_document_coherent)
        {
            continue;
        }
        let owned = region.bridge.snapshot(MAX_DEV_NODES);
        let document = window.document().borrow();
        let scale = window.surface().scale_factor();
        for node in &owned.nodes {
            if format!("node-{}", node.token) != token {
                continue;
            }
            let bounds = ViewHost::window_box(window.host().as_ref(), node.native).map(|b| Bounds {
                x: f64::from(b.origin.x.0) / scale,
                y: f64::from(b.origin.y.0) / scale,
                width: f64::from(b.size.width.0) / scale,
                height: f64::from(b.size.height.0) / scale,
            })?;
            let index = zgui_view_dom::id::resolve(&document, node.native);
            let (classes, source) = node_classes_and_source(&document, index);
            let (Some(box_model), _) = node_box_and_style(window, node.native, scale, Some(&bounds))
            else {
                continue;
            };
            return Some((
                region,
                PickerTarget {
                    surface: name,
                    token: token.to_owned(),
                    kind: node.kind.to_string(),
                    test_id: node.test_id.clone(),
                    classes,
                    source,
                    box_model,
                },
            ));
        }
    }
    None
}

fn snapshot_surface(
    name: &str,
    region: &SessionRegion,
    window: Option<&Window>,
    generation: u64,
    session_locked: bool,
    include_tree: bool,
) -> Value {
    // Strictly forbid capture of SessionLockSurface content or any surface while session-locked.
    if region.is_lock_surface() || session_locked {
        return json!({
            "surface": name,
            "generation": generation,
            "excluded": true,
            "reason": "session-lock-surface-forbidden",
            "nodes": [],
        });
    }
    let Some(window) = window else {
        return json!({
            "surface": name,
            "generation": generation,
            "coherenceStatus": "no-window",
            "coherent": false,
            "nodes": [],
        });
    };

    let live_doc_rev = window.dom().revision();
    let live_style_rev = window.style_revision();
    let completed = window.completed_frame();

    let (coherence_status, coherent, frame_meta) = match completed {
        None => (
            "no-completed-frame",
            false,
            json!({
                "frameOrdinal": null,
                "frameDocumentRevision": null,
                "liveDocumentRevision": live_doc_rev,
                "consumedStyleRevision": null,
                "liveStyleRevision": live_style_rev,
                "consumedLayoutRevision": null,
            }),
        ),
        Some(frame) => {
            let is_coherent = frame.layout_document_coherent
                && frame.document_revision == live_doc_rev
                && frame.style_revision == live_style_rev;
            let status = if is_coherent {
                "coherent"
            } else {
                "historical-incoherent"
            };
            (
                status,
                is_coherent,
                json!({
                    "frameOrdinal": frame.ordinal,
                    "outcome": format!("{:?}", frame.outcome),
                    "frameDocumentRevision": frame.document_revision,
                    "liveDocumentRevision": live_doc_rev,
                    "consumedStyleRevision": frame.style_revision,
                    "liveStyleRevision": live_style_rev,
                    "consumedLayoutRevision": frame.layout_revision,
                    "phases": {
                        "styleUs": frame.phases.style_us,
                        "layoutUs": frame.phases.layout_us,
                        "paintUs": frame.phases.paint_us,
                        "totalUs": frame.phases.total_us,
                    },
                }),
            )
        }
    };

    let mut payload = json!({
        "surface": name,
        "surfaceId": format!("{:?}", region.surface),
        "generation": generation,
        "excluded": false,
        "coherent": coherent,
        "coherenceStatus": coherence_status,
        "frame": frame_meta,
    });

    if !include_tree {
        return payload;
    }

    let owned = region.bridge.snapshot(MAX_DEV_NODES);
    let document = window.document().borrow();
    let focused = region.region.host.focused().get_untracked();
    let scale = window.surface().scale_factor();
    let mut text_budget = MAX_DEV_TEXT_BYTES;
    let mut visit_budget = MAX_DEV_VISITS;
    let mut text_truncated = false;
    let mut nodes = Vec::with_capacity(owned.nodes.len());

    for node in &owned.nodes {
        let index = zgui_view_dom::id::resolve(&document, node.native);
        let (text, truncated, redacted) = bounded_redacted_text(
            window,
            &document,
            node.native,
            index,
            text_budget.min(MAX_DEV_NODE_TEXT_BYTES),
            &mut visit_budget,
        );
        text_budget = text_budget.saturating_sub(text.len());
        text_truncated |= truncated;

        // Only attach frame layout bounds when the completed frame is coherent with live DOM;
        // never silently join live tree nodes to stale/incoherent layout geometry.
        let bounds_logical = if coherent {
            ViewHost::window_box(window.host().as_ref(), node.native).map(|bounds| Bounds {
                x: f64::from(bounds.origin.x.0) / scale,
                y: f64::from(bounds.origin.y.0) / scale,
                width: f64::from(bounds.size.width.0) / scale,
                height: f64::from(bounds.size.height.0) / scale,
            })
        } else {
            None
        };
        let state = native_state(window, node.native);
        let (classes, source) = node_classes_and_source(&document, index);
        let (box_model, computed_style) =
            node_box_and_style(window, node.native, scale, bounds_logical.as_ref());
        let role = window
            .dom()
            .semantics(node.native)
            .map(|sem| format!("{:?}", sem.role));

        nodes.push(json!({
            "token": format!("node-{}", node.token),
            "parent": node.parent.map(|t| format!("node-{t}")),
            "kind": node.kind,
            "testId": node.test_id,
            "classes": classes,
            "source": source,
            "role": role,
            "text": text,
            "textTruncated": truncated,
            "redacted": redacted,
            "state": {
                "enabled": state.map(|v| v.0),
                "visible": state.map(|v| v.1),
                "focused": focused == Some(node.native),
            },
            "boundsLogical": bounds_logical,
            "boxModel": box_model,
            "computedStyle": computed_style,
            "scrollMetrics": if coherent { scroll_metrics(window, node.native, scale) } else { None },
        }));
    }

    payload["metadataRevision"] = json!(owned.metadata_revision);
    payload["truncation"] = json!({
        "nodes": owned.truncated,
        "text": text_truncated,
        "maxNodes": MAX_DEV_NODES,
        "maxTextBytes": MAX_DEV_TEXT_BYTES,
    });
    payload["nodes"] = Value::Array(nodes);
    payload
}

pub(crate) enum InspectAction {
    Snapshot { include_tree: bool, include_profile: bool },
    Pick { action: String, token: Option<String> },
}

/// Request queued from a dev inspector socket client to the UI thread.
pub(crate) struct InspectRequest {
    pub action: InspectAction,
    pub surface: Option<String>,
    reply: mpsc::SyncSender<String>,
}

impl InspectRequest {
    pub(crate) fn reply(self, payload: &Value) {
        let mut encoded = serde_json::to_string(payload)
            .unwrap_or_else(|_| "{\"error\":\"failed to serialize inspector snapshot\"}".into());
        if encoded.len() > MAX_RESPONSE_BYTES {
            encoded = json!({
                "error": "inspector snapshot exceeded response byte budget",
                "maxBytes": MAX_RESPONSE_BYTES,
            })
            .to_string();
        }
        let _ = self.reply.send(encoded);
    }
}

/// Host-owned, strictly read-only Unix socket server for `--dev-inspect`.
pub(crate) struct DevInspectorServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    rx: mpsc::Receiver<InspectRequest>,
    thread: Option<thread::JoinHandle<()>>,
}

impl DevInspectorServer {
    pub(crate) fn start(path: PathBuf, waker: Arc<dyn Waker>) -> std::io::Result<Self> {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let (tx, rx) = mpsc::sync_channel::<InspectRequest>(8);

        let handle = thread::Builder::new()
            .name("zgui-dev-inspector".into())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            handle_client(stream, &tx, &waker);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(15));
                        }
                        Err(_) => break,
                    }
                }
            })?;

        Ok(Self {
            path,
            stop,
            rx,
            thread: Some(handle),
        })
    }

    pub(crate) fn try_recv(&self) -> Option<InspectRequest> {
        self.rx.try_recv().ok()
    }

    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for DevInspectorServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle_client(mut stream: UnixStream, tx: &mpsc::SyncSender<InspectRequest>, waker: &Arc<dyn Waker>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut line = String::new();
    let Ok(cloned) = stream.try_clone() else { return; };
    let mut reader = BufReader::new(cloned);
    let Ok(read_bytes) = reader.read_line(&mut line) else { return; };
    if read_bytes == 0 || read_bytes > MAX_REQUEST_LINE_BYTES {
        let _ = writeln!(stream, "{{\"error\":\"invalid or oversized inspector request\"}}");
        return;
    }
    let parsed: Result<Value, _> = serde_json::from_str(line.trim());
    let Ok(req) = parsed else {
        let _ = writeln!(stream, "{{\"error\":\"inspector request must be JSON\"}}");
        return;
    };
    let op = req.get("op").and_then(Value::as_str).unwrap_or("snapshot");
    let action = match op {
        "snapshot" => InspectAction::Snapshot { include_tree: true, include_profile: true },
        "tree" => InspectAction::Snapshot { include_tree: true, include_profile: false },
        "profile" => InspectAction::Snapshot { include_tree: false, include_profile: true },
        "pick" => {
            let pick_action = req
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("toggle")
                .to_owned();
            if !matches!(
                pick_action.as_str(),
                "enable" | "disable" | "toggle" | "status" | "select"
            ) {
                let _ = writeln!(stream, "{{\"error\":\"unsupported pick action\"}}");
                return;
            }
            let token = req.get("token").and_then(Value::as_str).map(str::to_owned);
            InspectAction::Pick { action: pick_action, token }
        }
        _ => {
            let _ = writeln!(
                stream,
                "{{\"error\":\"read-only dev inspector: unsupported operation\"}}"
            );
            return;
        }
    };
    let surface = req
        .get("surface")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    if tx
        .try_send(InspectRequest {
            action,
            surface,
            reply: reply_tx,
        })
        .is_err()
    {
        let _ = writeln!(stream, "{{\"error\":\"inspector queue full\"}}");
        return;
    }
    waker.wake(WakeReason::AppWork);
    match reply_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(payload) => {
            let _ = writeln!(stream, "{payload}");
        }
        Err(_) => {
            let _ = writeln!(stream, "{{\"error\":\"inspector snapshot timed out\"}}");
        }
    }
}

/// CLI client helper for `zgui-host inspect --socket <path> [--op snapshot|tree|profile|pick] [--surface <name>]`
/// and `zgui-host inspect pick --socket <path> [--off|--status|--select <node-N>]`.
pub(crate) fn run_cli_inspect(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut socket: Option<PathBuf> = None;
    let mut op = "snapshot".to_owned();
    let mut pick_action: Option<String> = None;
    let mut pick_token: Option<String> = None;
    let mut surface: Option<String> = None;
    let mut i = 0;
    if args.first().is_some_and(|arg| arg == "pick") {
        op = "pick".to_owned();
        pick_action = Some("toggle".to_owned());
        i = 1;
    }
    while i < args.len() {
        match args[i].as_str() {
            "--socket" => {
                i += 1;
                socket = Some(PathBuf::from(
                    args.get(i).ok_or("--socket requires a path")?,
                ));
            }
            "--op" => {
                i += 1;
                op = args.get(i).ok_or("--op requires snapshot, tree, profile, or pick")?.clone();
            }
            "--action" => {
                i += 1;
                pick_action = Some(args.get(i).ok_or("--action requires enable, disable, toggle, status, or select")?.clone());
            }
            "--token" | "--select" => {
                i += 1;
                pick_token = Some(args.get(i).ok_or("--token requires a node token (e.g. node-1)")?.clone());
                pick_action.get_or_insert_with(|| "select".to_owned());
            }
            "--on" => {
                pick_action = Some("enable".to_owned());
            }
            "--off" => {
                pick_action = Some("disable".to_owned());
            }
            "--status" => {
                pick_action = Some("status".to_owned());
            }
            "--surface" => {
                i += 1;
                surface = Some(args.get(i).ok_or("--surface requires a surface name")?.clone());
            }
            other => return Err(format!("unknown inspect option: {other}").into()),
        }
        i += 1;
    }
    let path = socket.ok_or("inspect requires --socket <path>")?;
    let mut payload = json!({ "op": op });
    if let Some(surface) = surface {
        payload["surface"] = json!(surface);
    }
    if let Some(action) = pick_action {
        payload["action"] = json!(action);
    }
    if let Some(token) = pick_token {
        payload["token"] = json!(token);
    }
    let response = send_socket_request(&path, &payload)?;
    println!("{response}");
    Ok(())
}

pub(crate) fn send_socket_request(path: &Path, req: &Value) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    writeln!(stream, "{req}")?;
    stream.flush()?;
    let mut response = String::new();
    let mut reader = BufReader::new(stream);
    reader.read_line(&mut response)?;
    Ok(response.trim_end().to_owned())
}

#[cfg(test)]
pub(crate) fn query_socket(
    path: &Path,
    op: &str,
    surface: Option<&str>,
) -> std::io::Result<String> {
    let mut req = json!({ "op": op });
    if let Some(surface) = surface {
        req["surface"] = json!(surface);
    }
    send_socket_request(path, &req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        session::{SessionModule, SessionPolicy},
        session_host::{ImageChoice, RecoveryPolicy, Root, RuntimeSurface, SessionHost, SurfaceOpacity},
    };
    use zgui::{
        platform::{AppHandler, PlatformCapabilities, PlatformCx, SurfaceEvent},
        view::AttrName,
    };
    use zgui_platform_headless::Headless;

    #[test]
    fn profiler_ring_buffer_is_strictly_bounded() {
        let inspector = DevInspector::new();
        for i in 0..400u64 {
            inspector.record_checkpoint(1, i as usize, None);
            inspector.ring.borrow_mut().push_frame(FrameSample {
                surface_id: 1,
                ordinal: i + 1,
                outcome: "Presented".into(),
                document_revision: i,
                style_revision: 1,
                layout_revision: Some(i),
                coherent: true,
                style_us: 10,
                layout_us: 20,
                paint_us: 30,
                total_us: 60,
            });
        }
        let snap = inspector.profiler_snapshot();
        assert_eq!(snap["frameCount"], MAX_RING_SAMPLES);
        assert_eq!(snap["checkpointCount"], MAX_RING_SAMPLES);
        assert_eq!(snap["frames"][0]["ordinal"], 400 - MAX_RING_SAMPLES as u64 + 1);
        assert_eq!(snap["checkpoints"][0]["queueDepth"], 400 - MAX_RING_SAMPLES);
    }

    #[test]
    fn inspector_redacts_secure_fields_labels_incoherent_frames_and_excludes_locked_sessions() {
        let source = r#"
            export const hostApiVersion = 1;
            export function mount(root) {
                const pubLabel = __zgui.createElement("label");
                __zgui.insertNode(pubLabel, __zgui.createTextNode("Public Greeting"), null);
                __zgui.setProperty(pubLabel, "testId", "public-text");
                __zgui.insertNode(root, pubLabel, null);

                const secretField = __zgui.createElement("field");
                __zgui.setProperty(secretField, "secure", true);
                __zgui.setProperty(secretField, "testId", "secret-input");
                __zgui.setProperty(secretField, "value", "SuperSecretPassword123!");
                __zgui.insertNode(root, secretField, null);

                return () => {};
            }
        "#;
        let inspector = DevInspector::new();
        let pending = Rc::new(RefCell::new(Vec::new()));
        let roots = pending.clone();
        let app = zgui::app()
            .with_title("inspector-test")
            .with_size(320.0, 200.0)
            .with_renderer(Box::new(|_, target| {
                let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
                zgui::render::Renderer::configure(&mut renderer, target);
                Ok(Box::new(renderer))
            }))
            .with_probe(inspector.probe());
        let mut runtime = app
            .into_handler(move || Root(roots.clone()))
            .expect("handler")
            .into_runtime();
        let platform = Headless::new().with_capabilities(PlatformCapabilities::none());
        runtime.surfaces_available(&platform);

        let policy = SessionPolicy::live(host_services::Executor::new(1, 16).expect("exec"));
        let mut host = SessionHost::new(
            pending,
            SessionModule::inline("inspector-test", source, vec![]),
            policy,
            RecoveryPolicy::Propagate,
            SurfaceOpacity::Opaque,
        );
        let retry: Rc<dyn Fn(zgui::view::DocumentId, u64)> = Rc::new(|_, _| {});
        {
            let mut surface = RuntimeSurface::new(&mut runtime, &platform, ImageChoice::Live);
            host.attach(&mut surface, platform.waker(), retry).expect("attach");
        }
        // Pump one frame so CompletedFrame is populated cleanly.
        let win_surface = runtime.windows()[0].surface().id();
        runtime.surface_event(&platform, win_surface, SurfaceEvent::RedrawRequested);
        platform.virtual_clock().advance(Duration::from_millis(20));
        runtime.surface_event(&platform, win_surface, SurfaceEvent::RedrawRequested);

        let session = host.sessions().next().expect("active session").clone();
        let snap = inspector.capture_snapshot(&runtime, Some(&session), true, true, None);
        let serialized = serde_json::to_string(&snap).expect("serialize");

        // 1. Public text is visible, raw password is NEVER leaked.
        assert!(serialized.contains("Public Greeting"), "expected public text in snapshot: {serialized}");
        assert!(
            !serialized.contains("SuperSecretPassword123!"),
            "secure password leaked into dev inspector snapshot: {serialized}"
        );
        let nodes = snap["surfaces"][0]["nodes"].as_array().expect("nodes array");
        let secret_node = nodes
            .iter()
            .find(|n| n["testId"] == "secret-input")
            .expect("secret-input node present");
        assert_eq!(secret_node["redacted"], true);
        assert_ne!(secret_node["text"], "SuperSecretPassword123!");

        // 2. Frame profiler recorded phase timings cleanly.
        assert!(snap["profiler"]["frameCount"].as_u64().unwrap_or(0) >= 1);

        // 3. Mutate DOM after completed frame -> snapshot explicitly labels historical/incoherent
        //    and refuses to join live tree nodes to stale layout boxes.
        let doc_root = session.primary().expect("primary").region.root;
        session
            .primary()
            .expect("primary")
            .region
            .dom
            .set_attribute(doc_root, AttrName::new("data-mutated"), Some("1"));
        let incoherent = inspector.capture_snapshot(&runtime, Some(&session), true, false, None);
        assert_eq!(incoherent["surfaces"][0]["coherent"], false);
        assert_eq!(incoherent["surfaces"][0]["coherenceStatus"], "historical-incoherent");
        for node in incoherent["surfaces"][0]["nodes"].as_array().unwrap() {
            assert!(node["boundsLogical"].is_null(), "incoherent snapshot must not join stale bounds");
        }

        // 4. Locking the session forbids capturing any surface content.
        let _ = session.update_lock_state(zgui::platform::SessionLockState::Locked);
        let locked = inspector.capture_snapshot(&runtime, Some(&session), true, false, None);
        assert_eq!(locked["sessionLocked"], true);
        assert_eq!(locked["surfaces"][0]["excluded"], true);
        assert_eq!(locked["surfaces"][0]["reason"], "session-lock-surface-forbidden");
        assert!(locked["surfaces"][0]["nodes"].as_array().unwrap().is_empty());
        assert!(!serde_json::to_string(&locked).unwrap().contains("Public Greeting"));

        let mut surface = RuntimeSurface::new(&mut runtime, &platform, ImageChoice::Live);
        host.stop(&mut surface);
    }

    #[test]
    fn readonly_socket_server_serves_snapshots_and_rejects_write_ops() {
        let dir = std::env::temp_dir();
        let sock = dir.join(format!("zgui-dev-inspect-test-{}.sock", std::process::id()));
        let platform = Headless::new();
        let mut server = DevInspectorServer::start(sock.clone(), platform.waker()).expect("start server");

        let sock_for_client = sock.clone();
        let client = thread::spawn(move || {
            let bad = query_socket(&sock_for_client, "eval", None).expect("eval reply");
            let ok = query_socket(&sock_for_client, "profile", None).expect("profile reply");
            (bad, ok)
        });

        // Wait briefly for the valid request to arrive on the host side and answer it.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut answered = false;
        while std::time::Instant::now() < deadline {
            if let Some(req) = server.try_recv() {
                assert!(matches!(
                    req.action,
                    InspectAction::Snapshot { include_tree: false, include_profile: true }
                ));
                req.reply(&json!({ "readOnly": true, "profiler": { "frameCount": 0 } }));
                answered = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(answered, "expected server to receive valid profile request");
        let (bad, ok) = client.join().expect("client finished");
        assert!(bad.contains("unsupported operation"), "write/eval op must be rejected: {bad}");
        assert!(ok.contains("\"readOnly\":true"), "valid read-only op must return JSON: {ok}");
        server.stop();
        assert!(!sock.exists(), "socket file cleaned up on stop");
    }

    #[test]
    fn visual_picker_highlights_node_surfaces_source_and_refuses_lock_surface() {
        let source = r#"
            export const hostApiVersion = 1;
            export function mount(root) {
                const card = __zgui.createElement("box");
                __zgui.setProperty(card, "testId", "inspect-card");
                __zgui.setProperty(card, "class", "demo-card");
                __zgui.setProperty(card, "data-zgui-source", "app/src/main.tsx:12:5");
                __zgui.setProperty(card, "style", { width: "180px", height: "60px", padding: "8px", margin: "4px" });
                __zgui.insertNode(card, __zgui.createTextNode("Card Content"), null);
                __zgui.insertNode(root, card, null);
                return () => {};
            }
        "#;
        let inspector = DevInspector::new();
        let pending = Rc::new(RefCell::new(Vec::new()));
        let roots = pending.clone();
        let app = zgui::app()
            .with_title("picker-test")
            .with_size(320.0, 200.0)
            .with_renderer(Box::new(|_, target| {
                let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
                zgui::render::Renderer::configure(&mut renderer, target);
                Ok(Box::new(renderer))
            }))
            .with_probe(inspector.probe());
        let mut runtime = app
            .into_handler(move || Root(roots.clone()))
            .expect("handler")
            .into_runtime();
        let platform = Headless::new().with_capabilities(PlatformCapabilities::none());
        runtime.surfaces_available(&platform);

        let policy = SessionPolicy::live(host_services::Executor::new(1, 16).expect("exec"));
        let mut host = SessionHost::new(
            pending,
            SessionModule::inline("picker-test", source, vec![]),
            policy,
            RecoveryPolicy::Propagate,
            SurfaceOpacity::Opaque,
        );
        let retry: Rc<dyn Fn(zgui::view::DocumentId, u64)> = Rc::new(|_, _| {});
        {
            let mut surface = RuntimeSurface::new(&mut runtime, &platform, ImageChoice::Live);
            host.attach(&mut surface, platform.waker(), retry).expect("attach");
        }
        let win_surface = runtime.windows()[0].surface().id();
        runtime.surface_event(&platform, win_surface, SurfaceEvent::RedrawRequested);
        platform.virtual_clock().advance(Duration::from_millis(20));
        runtime.surface_event(&platform, win_surface, SurfaceEvent::RedrawRequested);

        let session = host.sessions().next().expect("active session").clone();
        let snap = inspector.capture_snapshot(&runtime, Some(&session), true, false, None);
        let nodes = snap["surfaces"][0]["nodes"].as_array().expect("nodes");
        let card = nodes.iter().find(|n| n["testId"] == "inspect-card").expect("inspect-card");
        assert_eq!(card["source"], "app/src/main.tsx:12:5");
        assert_eq!(card["classes"], json!(["demo-card"]));
        assert!(card["boxModel"]["contentBox"]["width"].as_f64().unwrap_or(0.0) > 100.0);
        assert!(card["computedStyle"]["display"].is_string());

        // Enable picker via command and hover + click the card center.
        let enabled = inspector.handle_pick_command(&runtime, Some(&session), "enable", None, None);
        assert_eq!(enabled["ok"], true);
        assert_eq!(enabled["picker"]["active"], true);

        let bb = &card["boundsLogical"];
        let cx = (bb["x"].as_f64().unwrap() + bb["width"].as_f64().unwrap() / 2.0) as f32;
        let cy = (bb["y"].as_f64().unwrap() + bb["height"].as_f64().unwrap() / 2.0) as f32;
        let ptr = PointerEvent::mouse(zgui::geom::Point::new(zgui::geom::CssPx(cx), zgui::geom::CssPx(cy)))
            .with_button(PointerButton::Primary);
        assert!(inspector.handle_pointer_event(&runtime, Some(&session), win_surface, PointerAction::Moved, &ptr));
        let hover_status = inspector.handle_pick_command(&runtime, Some(&session), "status", None, None);
        assert_eq!(hover_status["picker"]["hovered"]["testId"], "inspect-card");
        assert_eq!(hover_status["picker"]["hovered"]["source"], "app/src/main.tsx:12:5");

        // Primary click locks selection and disables active hover capture.
        assert!(inspector.handle_pointer_event(
            &runtime,
            Some(&session),
            win_surface,
            PointerAction::Pressed,
            &ptr,
        ));
        let selected_status = inspector.handle_pick_command(&runtime, Some(&session), "status", None, None);
        assert_eq!(selected_status["picker"]["active"], false);
        assert_eq!(selected_status["picker"]["selected"]["testId"], "inspect-card");
        assert_eq!(selected_status["picker"]["selected"]["source"], "app/src/main.tsx:12:5");

        // Locking session strictly forbids picker activation and clears overlay.
        let _ = session.update_lock_state(zgui::platform::SessionLockState::Locked);
        let locked_reply = inspector.handle_pick_command(&runtime, Some(&session), "enable", None, None);
        assert_eq!(locked_reply["error"], "session-lock-surface-forbidden");
        assert_eq!(locked_reply["picker"]["forbidden"], true);

        inspector.clear_overlay();
        let mut surface = RuntimeSurface::new(&mut runtime, &platform, ImageChoice::Live);
        host.stop(&mut surface);
    }
}
