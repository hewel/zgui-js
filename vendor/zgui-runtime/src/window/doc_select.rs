//! Opt-in read-only cross-block `<text>` document selection.
//!
//! Kept deliberately separate from [`crate::editing`] (which owns per-field undo history, IME
//! composition, and secure bullet projection). Selection state is anchored on the [`Window`] as
//! directional `(source_text_node, byte_offset, affinity)` endpoints scoped to one opt-in
//! container element (`data-zgui-document-selection="true"`), mapped through each inline
//! formatting context's existing [`zgui_text::TextMap`] and painted through [`crate::caret::Plan`].

use zgui_color::Color;
use zgui_css::values::ui::UserSelectValue;
use zgui_dom::side::BoxKey;
use zgui_dom::{Document, NodeIndex, NodeKey};
use zgui_edit::Affinity;
use zgui_geom::{Device, DevicePx, Point, Rect};
use zgui_interned::AttrName;
use zgui_layout::node::kind::BoxKind;
use zgui_layout::{FragmentKind, LayoutStore};
use zgui_platform::SurfaceEvent;
use zgui_text::SourcePos;
use zgui_view::DocumentSelectionSnapshot;
use zgui_vocab::{Key, KeyState, Modifiers, NamedKey, PointerAction, PointerButton, PointerEvent, Role};

use crate::caret::{Located, Plan, plan_for_document_bands};
use crate::window::Window;

/// DOM attribute marking an explicit opt-in read-only document selection container.
pub const DOCUMENT_SELECTION_ATTR: &str = "data-zgui-document-selection";

/// One directional endpoint inside a read-only document selection container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentEndpoint {
    /// The DOM text node (`NodeKind::Text`) holding the source characters.
    pub node: NodeKey,
    /// UTF-8 byte offset within `node`'s source text (always on a UTF-8 char boundary).
    pub offset: usize,
    /// Visual side at an ambiguous line-wrap or bidi boundary.
    pub affinity: Affinity,
}

/// Active read-only cross-block document selection owned by a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentSelection {
    /// Opt-in container element scoping this selection (never crosses mount regions).
    pub container: NodeKey,
    /// Starting endpoint of the gesture or programmatic range.
    pub anchor: DocumentEndpoint,
    /// Moving endpoint of the gesture or programmatic range (may precede `anchor`).
    pub focus: DocumentEndpoint,
}

/// One laid-out, selectable DOM text run inside a document selection container, in document order.
#[derive(Clone, Copy, Debug)]
struct SelectableRun {
    /// DOM text node key.
    node: NodeKey,
    /// Owning inline formatting context box carrying the shaped paragraph and `TextMap`.
    ifc_box: BoxKey,
    /// Index of this run inside `InlineResolution::sources`.
    run_index: usize,
    /// Source UTF-8 byte length of `node`.
    byte_len: usize,
    /// Nearest enclosing `role="row"` ancestor within the container, for `\t` cell separation.
    table_row: Option<NodeKey>,
}

/// Returns whether `key` is explicitly marked as a read-only document selection container.
pub(crate) fn is_document_selection_container(document: &Document, key: NodeKey) -> bool {
    let store = document.store();
    let Some(attrs) = store.columns().attrs.get(key).and_then(|slot| slot.as_deref()) else {
        return false;
    };
    matches!(
        attrs.get(AttrName::new(DOCUMENT_SELECTION_ATTR)).map(AsRef::as_ref),
        Some("true" | "")
    )
}

/// Clamps `offset` to a valid UTF-8 character boundary within `text`.
fn clamp_char_boundary(text: &str, offset: usize) -> usize {
    let mut clamped = offset.min(text.len());
    while clamped > 0 && !text.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped
}

/// Checks whether `descendant` is still attached beneath `ancestor` (or equals `ancestor`).
fn is_descendant_of(document: &Document, descendant: NodeKey, ancestor: NodeKey) -> bool {
    let store = document.store();
    let Some(mut index) = store.index_of(descendant) else {
        return false;
    };
    loop {
        if store.key_of(index) == ancestor {
            return true;
        }
        let Some(parent) = store.core(index).parent() else {
            return false;
        };
        index = parent;
    }
}

/// Resolves the `user-select` color used for read-only selection bands on an inline context box.
fn selection_band_color(layout: &LayoutStore, ifc_box: BoxKey) -> Color {
    let style = &layout.node(ifc_box).style;
    zgui_css::values::color::to_color(zgui_css::values::color::current(style))
}

/// Finds the enclosing inline formatting context box (`InlineResolution`) and run index for a
/// live DOM text node.
fn locate_text_run(layout: &LayoutStore, text_node: NodeKey) -> Option<(BoxKey, usize)> {
    for &box_key in layout.boxes_of(text_node) {
        let box_node = layout.get(box_key)?;
        if box_node.kind != BoxKind::TextRun || box_node.pseudo.is_some() {
            continue;
        }
        let mut cur = box_key;
        loop {
            if let Some(resolution) = layout.inline_resolution(cur)
                && let Some(run_index) = resolution.sources.iter().position(|&src| src == box_key)
            {
                return Some((cur, run_index));
            }
            cur = layout.get(cur)?.parent?;
        }
    }
    None
}

/// Collects every selectable, laid-out DOM text run inside `container` in document preorder.
fn collect_selectable_runs(
    document: &Document,
    layout: &LayoutStore,
    container: NodeKey,
    root_selectable: bool,
) -> Vec<SelectableRun> {
    let store = document.store();
    let Some(root_index) = store.index_of(container) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_recursive(
        document,
        layout,
        container,
        root_index,
        root_selectable,
        None,
        &mut out,
    );
    out
}

fn collect_recursive(
    document: &Document,
    layout: &LayoutStore,
    container: NodeKey,
    index: NodeIndex,
    inherited_selectable: bool,
    table_row: Option<NodeKey>,
    out: &mut Vec<SelectableRun>,
) {
    let store = document.store();
    let key = store.key_of(index);
    let core = store.core(index);

    if core.kind() == zgui_dom::NodeKind::Text {
        if !inherited_selectable {
            return;
        }
        let Some(text) = zgui_dom::text::text_of(store, index) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if let Some((ifc_box, run_index)) = locate_text_run(layout, key) {
            out.push(SelectableRun {
                node: key,
                ifc_box,
                run_index,
                byte_len: text.len(),
                table_row,
            });
        }
        return;
    }

    if !core.kind().is_element() {
        return;
    }

    if key != container {
        // Never select into editable controls (fields, editors, password inputs) or nested
        // independent document-selection containers.
        if crate::editing::Editors::is_selectable(document, key)
            || is_document_selection_container(document, key)
        {
            return;
        }
    }

    let selectable = match document
        .node(index)
        .primary_style()
        .map(|style| style.get_ui().user_select)
    {
        Some(UserSelectValue::None) => false,
        Some(UserSelectValue::Auto) | None => inherited_selectable,
        Some(_) => true,
    };

    let row = if store
        .columns()
        .semantics
        .get(key)
        .and_then(|slot| slot.as_deref())
        .map(|sem| sem.role)
        == Some(Role::Row)
    {
        Some(key)
    } else {
        table_row
    };

    let mut child = core.first_child();
    while let Some(child_index) = child {
        collect_recursive(document, layout, container, child_index, selectable, row, out);
        child = store.core(child_index).next_sibling();
    }
}

/// Ordered slice of `(run_index_in_runs, start_byte, end_byte)` for a non-empty selection.
fn ordered_endpoints(
    runs: &[SelectableRun],
    anchor: DocumentEndpoint,
    focus: DocumentEndpoint,
) -> Option<((usize, usize), (usize, usize))> {
    let a_idx = runs.iter().position(|run| run.node == anchor.node)?;
    let f_idx = runs.iter().position(|run| run.node == focus.node)?;
    let a_off = anchor.offset.min(runs[a_idx].byte_len);
    let f_off = focus.offset.min(runs[f_idx].byte_len);
    if (a_idx, a_off) <= (f_idx, f_off) {
        Some(((a_idx, a_off), (f_idx, f_off)))
    } else {
        Some(((f_idx, f_off), (a_idx, a_off)))
    }
}

/// Extracts plain text in document order from `runs` between `(lo, hi)`.
fn extract_text(
    document: &Document,
    runs: &[SelectableRun],
    lo: (usize, usize),
    hi: (usize, usize),
) -> String {
    if lo == hi {
        return String::new();
    }
    let store = document.store();
    let mut out = String::new();
    let mut prev_run: Option<&SelectableRun> = None;

    for (idx, run) in runs.iter().enumerate().take(hi.0 + 1).skip(lo.0) {
        let Some(node_idx) = store.index_of(run.node) else {
            continue;
        };
        let Some(text) = zgui_dom::text::text_of(store, node_idx) else {
            continue;
        };
        let start = if idx == lo.0 {
            clamp_char_boundary(text, lo.1)
        } else {
            0
        };
        let end = if idx == hi.0 {
            clamp_char_boundary(text, hi.1)
        } else {
            text.len()
        };
        if start >= end {
            continue;
        }
        if let Some(prev) = prev_run
            && prev.ifc_box != run.ifc_box
        {
            if prev.table_row.is_some() && prev.table_row == run.table_row {
                out.push('\t');
            } else {
                out.push('\n');
            }
        }
        out.push_str(&text[start..end]);
        prev_run = Some(run);
    }
    out
}

/// Computes the device-pixel bounding box of an inline formatting context box.
fn ifc_device_bounds(
    layout: &LayoutStore,
    placements: &zgui_scene::Placements,
    ifc_box: BoxKey,
) -> Option<Rect<DevicePx, Device>> {
    let mut union: Option<Rect<DevicePx, Device>> = None;
    for &frag_id in layout.fragments_of_box(ifc_box) {
        let Some(fragment) = layout.fragment(frag_id) else {
            continue;
        };
        if !matches!(fragment.kind, FragmentKind::Line { .. } | FragmentKind::Box) {
            continue;
        }
        let rect = if fragment.kind == FragmentKind::Box {
            fragment.content_box()
        } else {
            fragment.border_box
        };
        if rect.is_empty() {
            continue;
        }
        let placed =
            zgui_layout::fragment::transform::placed::onto_device(rect, fragment.transform, placements);
        union = Some(match union {
            Some(existing) => existing.union(placed),
            None => placed,
        });
    }
    union
}

fn rect_min_x(rect: Rect<DevicePx, Device>) -> f32 {
    rect.origin.x.0
}
fn rect_max_x(rect: Rect<DevicePx, Device>) -> f32 {
    rect.origin.x.0 + rect.size.width.0
}
fn rect_min_y(rect: Rect<DevicePx, Device>) -> f32 {
    rect.origin.y.0
}
fn rect_max_y(rect: Rect<DevicePx, Device>) -> f32 {
    rect.origin.y.0 + rect.size.height.0
}

/// Squared distance from `point` to the closest point inside `rect` (0 when inside).
fn rect_distance_sq(rect: Rect<DevicePx, Device>, point: Point<DevicePx, Device>) -> f32 {
    let dx = if point.x.0 < rect_min_x(rect) {
        rect_min_x(rect) - point.x.0
    } else if point.x.0 > rect_max_x(rect) {
        point.x.0 - rect_max_x(rect)
    } else {
        0.0
    };
    let dy = if point.y.0 < rect_min_y(rect) {
        rect_min_y(rect) - point.y.0
    } else if point.y.0 > rect_max_y(rect) {
        point.y.0 - rect_max_y(rect)
    } else {
        0.0
    };
    dx * dx + dy * dy
}

impl Window {
    /// Returns the active read-only document selection on this window, if any.
    pub fn document_selection(&self) -> Option<DocumentSelection> {
        self.doc_selection
    }

    /// Returns the plain text currently selected by the active read-only document selection.
    pub fn document_selection_text(&self) -> Option<String> {
        let selection = self.doc_selection?;
        let root_selectable = self.selectable_at(selection.container);
        let document = self.document.borrow();
        let layout = self.layout.borrow();
        let runs = collect_selectable_runs(&document, &layout, selection.container, root_selectable);
        let (lo, hi) = ordered_endpoints(&runs, selection.anchor, selection.focus)?;
        if lo == hi {
            return None;
        }
        Some(extract_text(&document, &runs, lo, hi))
    }

    /// Clears the active read-only document selection and notifies the host record.
    pub(crate) fn clear_document_selection_state(&mut self) {
        self.doc_selection = None;
        self.doc_dragging = None;
        self.host.write_document_selection(None);
    }

    /// Publishes the current read-only document selection snapshot to [`RuntimeHost`].
    pub(crate) fn sync_document_selection_host(&self) {
        let Some(selection) = self.doc_selection else {
            self.host.write_document_selection(None);
            return;
        };
        let root_selectable = self.selectable_at(selection.container);
        let document = self.document.borrow();
        let layout = self.layout.borrow();
        let runs = collect_selectable_runs(&document, &layout, selection.container, root_selectable);
        let Some((lo, hi)) = ordered_endpoints(&runs, selection.anchor, selection.focus) else {
            self.host.write_document_selection(None);
            return;
        };
        let text = extract_text(&document, &runs, lo, hi);
        self.host
            .write_document_selection(Some(DocumentSelectionSnapshot {
                container: zgui_view_dom::id::to_view(selection.container),
                anchor_node: zgui_view_dom::id::to_view(selection.anchor.node),
                anchor_offset: selection.anchor.offset,
                focus_node: zgui_view_dom::id::to_view(selection.focus.node),
                focus_offset: selection.focus.offset,
                text,
                is_collapsed: lo == hi,
            }));
    }

    /// Validates the active read-only document selection against the live DOM and layout, clearing
    /// it safely if its container or either endpoint detached or became non-selectable.
    pub(crate) fn validate_document_selection(&mut self) {
        let Some(mut selection) = self.doc_selection else {
            return;
        };
        let root_selectable = self.selectable_at(selection.container);
        let valid = {
            let document = self.document.borrow();
            let layout = self.layout.borrow();
            if !is_document_selection_container(&document, selection.container)
                || !is_descendant_of(&document, selection.anchor.node, selection.container)
                || !is_descendant_of(&document, selection.focus.node, selection.container)
            {
                false
            } else {
                let runs = collect_selectable_runs(
                    &document,
                    &layout,
                    selection.container,
                    root_selectable,
                );
                let store = document.store();
                let anchor_ok = runs.iter().any(|r| r.node == selection.anchor.node);
                let focus_ok = runs.iter().any(|r| r.node == selection.focus.node);
                if anchor_ok && focus_ok {
                    if let Some(idx) = store.index_of(selection.anchor.node)
                        && let Some(text) = zgui_dom::text::text_of(store, idx)
                    {
                        selection.anchor.offset =
                            clamp_char_boundary(text, selection.anchor.offset);
                    }
                    if let Some(idx) = store.index_of(selection.focus.node)
                        && let Some(text) = zgui_dom::text::text_of(store, idx)
                    {
                        selection.focus.offset = clamp_char_boundary(text, selection.focus.offset);
                    }
                    true
                } else {
                    false
                }
            }
        };
        if valid {
            self.doc_selection = Some(selection);
            self.sync_document_selection_host();
        } else {
            self.clear_document_selection_state();
        }
    }

    /// Finds the nearest opt-in document selection container at or above `target`, stopping if an
    /// editable control is encountered first.
    fn document_selection_container_at(&self, target: Option<NodeKey>) -> Option<NodeKey> {
        let document = self.document.borrow();
        let store = document.store();
        let mut index = store.index_of(target?)?;
        loop {
            let key = store.key_of(index);
            if crate::editing::Editors::is_selectable(&document, key) {
                return None;
            }
            if is_document_selection_container(&document, key) {
                return Some(key);
            }
            index = store.core(index).parent()?;
        }
    }

    /// Projects a surface point in device pixels to the nearest selectable [`DocumentEndpoint`]
    /// inside `container`.
    fn hit_document_container(
        &self,
        container: NodeKey,
        point: Point<DevicePx, Device>,
    ) -> Option<DocumentEndpoint> {
        let root_selectable = self.selectable_at(container);
        let document = self.document.borrow();
        let layout = self.layout.borrow();
        let runs = collect_selectable_runs(&document, &layout, container, root_selectable);
        if runs.is_empty() {
            return None;
        }

        // Deduplicate IFC boxes in document order.
        let mut ifcs: Vec<BoxKey> = Vec::new();
        for run in &runs {
            if ifcs.last().copied() != Some(run.ifc_box) {
                ifcs.push(run.ifc_box);
            }
        }

        let placements = self.host.placements();
        let mut candidates: Vec<(BoxKey, Rect<DevicePx, Device>)> = Vec::with_capacity(ifcs.len());
        for ifc_box in ifcs {
            if let Some(bounds) = ifc_device_bounds(&layout, &placements, ifc_box) {
                candidates.push((ifc_box, bounds));
            }
        }
        if candidates.is_empty() {
            return None;
        }

        // Choose the best IFC: prefer direct hit, then same-row horizontal match, then vertically
        // closest block (for gaps or dragging past top/bottom).
        let (chosen_ifc, chosen_bounds) = candidates
            .iter()
            .copied()
            .find(|(_, rect)| rect.contains(point))
            .or_else(|| {
                candidates
                    .iter()
                    .copied()
                    .filter(|(_, rect)| {
                        point.y.0 >= rect_min_y(*rect) && point.y.0 <= rect_max_y(*rect)
                    })
                    .min_by(|(_, a), (_, b)| {
                        rect_distance_sq(*a, point).total_cmp(&rect_distance_sq(*b, point))
                    })
            })
            .or_else(|| {
                candidates.iter().copied().min_by(|(_, a), (_, b)| {
                    let dy_a = if point.y.0 < rect_min_y(*a) {
                        rect_min_y(*a) - point.y.0
                    } else if point.y.0 > rect_max_y(*a) {
                        point.y.0 - rect_max_y(*a)
                    } else {
                        0.0
                    };
                    let dy_b = if point.y.0 < rect_min_y(*b) {
                        rect_min_y(*b) - point.y.0
                    } else if point.y.0 > rect_max_y(*b) {
                        point.y.0 - rect_max_y(*b)
                    } else {
                        0.0
                    };
                    dy_a.total_cmp(&dy_b)
                        .then_with(|| rect_distance_sq(*a, point).total_cmp(&rect_distance_sq(*b, point)))
                })
            })?;

        // Clamp the query point slightly inside vertical bounds when dragging in a gap or outside
        // the top/bottom edge so Parley hits the appropriate line cleanly.
        let clamped_point = if chosen_bounds.contains(point) {
            point
        } else {
            let y = point.y.0.clamp(
                rect_min_y(chosen_bounds) + 0.5,
                (rect_max_y(chosen_bounds) - 0.5).max(rect_min_y(chosen_bounds)),
            );
            let x = if point.y.0 < rect_min_y(chosen_bounds) && chosen_ifc == candidates[0].0 {
                rect_min_x(chosen_bounds)
            } else if point.y.0 > rect_max_y(chosen_bounds)
                && chosen_ifc == candidates[candidates.len() - 1].0
            {
                rect_max_x(chosen_bounds)
            } else {
                point.x.0
            };
            Point::new(DevicePx(x), DevicePx(y))
        };

        let located = Located::of_box(&layout, &self.text, chosen_ifc)?;
        let resolution = layout.inline_resolution(chosen_ifc)?;
        let (source_pos, affinity) = located.hit_source(clamped_point, &self.scene.spatial)?;

        // Map `source_pos.run` back to a selectable run in `chosen_ifc`. If the hit landed on a
        // non-selectable inline span (`user-select: none`) or pseudo run inside the same IFC, snap
        // to the nearest selectable run in this IFC.
        let ifc_runs: Vec<&SelectableRun> = runs.iter().filter(|r| r.ifc_box == chosen_ifc).collect();
        if ifc_runs.is_empty() {
            return None;
        }
        let (run, raw_offset) = if let Some(exact) = ifc_runs.iter().find(|r| r.run_index == source_pos.run) {
            (*exact, source_pos.offset)
        } else if let Some(after) = ifc_runs.iter().find(|r| r.run_index > source_pos.run) {
            (*after, 0)
        } else {
            let last = *ifc_runs.last()?;
            (last, last.byte_len)
        };

        let _ = resolution;
        let store = document.store();
        let node_idx = store.index_of(run.node)?;
        let text = zgui_dom::text::text_of(store, node_idx)?;
        let offset = clamp_char_boundary(text, raw_offset);
        Some(DocumentEndpoint {
            node: run.node,
            offset,
            affinity,
        })
    }

    /// Handles pointer press/move/release for opt-in read-only document selection.
    pub(crate) fn point_at_document_selection(
        &mut self,
        event: &SurfaceEvent,
        target: Option<NodeKey>,
        modifiers: Modifiers,
    ) -> bool {
        let SurfaceEvent::Pointer {
            action,
            event: pointer,
            ..
        } = event
        else {
            return false;
        };

        match action {
            PointerAction::Pressed => {
                if pointer
                    .button
                    .is_some_and(|button| button != PointerButton::Primary)
                {
                    self.doc_dragging = None;
                    return false;
                }
                let Some(container) = self.document_selection_container_at(target) else {
                    let suppress_clear =
                        target.is_some_and(|hit_node| !self.selectable_at(hit_node));
                    if !suppress_clear
                        && (self.doc_selection.is_some() || self.doc_dragging.is_some())
                    {
                        self.clear_document_selection_state();
                    }
                    return false;
                };
                if let Some(hit_node) = target
                    && !self.selectable_at(hit_node)
                {
                    self.doc_dragging = None;
                    return false;
                }
                let point = self.doc_device_point(pointer);
                let Some(endpoint) = self.hit_document_container(container, point) else {
                    self.clear_document_selection_state();
                    return false;
                };
                let anchor = if modifiers.shift()
                    && let Some(existing) = self.doc_selection
                    && existing.container == container
                {
                    existing.anchor
                } else {
                    endpoint
                };
                self.doc_selection = Some(DocumentSelection {
                    container,
                    anchor,
                    focus: endpoint,
                });
                self.doc_dragging = Some(container);
                self.carets.stop();
                self.sync_document_selection_host();
                true
            }
            PointerAction::Moved => {
                let Some(container) = self.doc_dragging else {
                    return false;
                };
                let Some(mut selection) = self.doc_selection else {
                    self.doc_dragging = None;
                    return false;
                };
                if selection.container != container {
                    self.doc_dragging = None;
                    return false;
                }
                let point = self.doc_device_point(pointer);
                let Some(endpoint) = self.hit_document_container(container, point) else {
                    return false;
                };
                selection.focus = endpoint;
                self.doc_selection = Some(selection);
                self.sync_document_selection_host();
                true
            }
            PointerAction::Released | PointerAction::Cancelled => {
                let was_dragging = self.doc_dragging.take().is_some();
                if let Some(selection) = self.doc_selection
                    && selection.anchor.node == selection.focus.node
                    && selection.anchor.offset == selection.focus.offset
                {
                    self.clear_document_selection_state();
                } else if was_dragging {
                    self.sync_document_selection_host();
                }
                false
            }
            _ => false,
        }
    }

    /// Handles keyboard commands (`Ctrl/Cmd+C`, `Ctrl/Cmd+A`, `Escape`) for an active read-only
    /// document selection when no focused editable control consumed the key.
    pub(crate) fn handle_document_selection_key(&mut self, event: &SurfaceEvent) -> bool {
        let SurfaceEvent::Key {
            state: KeyState::Pressed,
            event: key,
            modifiers,
            ..
        } = event
        else {
            return false;
        };
        let Some(selection) = self.doc_selection else {
            return false;
        };

        if matches!(key.key, Key::Named(NamedKey::Escape)) && modifiers.is_empty() {
            self.clear_document_selection_state();
            return true;
        }

        let primary_mod = (modifiers.control() || modifiers.meta()) && !modifiers.alt();
        if !primary_mod {
            return false;
        }

        let ch = match &key.key_without_modifiers {
            Key::Character(s) => s.as_str(),
            _ => match &key.key {
                Key::Character(s) => s.as_str(),
                _ => return false,
            },
        };

        if ch.eq_ignore_ascii_case("c") {
            if let Some(text) = self.document_selection_text()
                && !text.is_empty()
            {
                self.clipboard.push(text);
                return true;
            }
            return false;
        }

        if ch.eq_ignore_ascii_case("a") {
            return self.select_all_in_document_container(selection.container);
        }

        false
    }

    /// Programmatically sets the read-only document selection inside `container`.
    pub(crate) fn set_document_selection_endpoints(
        &mut self,
        container: NodeKey,
        anchor: (NodeKey, usize),
        focus: (NodeKey, usize),
    ) {
        let root_selectable = self.selectable_at(container);
        let validated = {
            let document = self.document.borrow();
            let layout = self.layout.borrow();
            if !is_document_selection_container(&document, container)
                || !is_descendant_of(&document, anchor.0, container)
                || !is_descendant_of(&document, focus.0, container)
            {
                None
            } else {
                let runs = collect_selectable_runs(&document, &layout, container, root_selectable);
                if runs.iter().any(|r| r.node == anchor.0) && runs.iter().any(|r| r.node == focus.0)
                {
                    let store = document.store();
                    let a_text = store
                        .index_of(anchor.0)
                        .and_then(|idx| zgui_dom::text::text_of(store, idx))
                        .unwrap_or_default();
                    let f_text = store
                        .index_of(focus.0)
                        .and_then(|idx| zgui_dom::text::text_of(store, idx))
                        .unwrap_or_default();
                    Some(DocumentSelection {
                        container,
                        anchor: DocumentEndpoint {
                            node: anchor.0,
                            offset: clamp_char_boundary(a_text, anchor.1),
                            affinity: Affinity::Downstream,
                        },
                        focus: DocumentEndpoint {
                            node: focus.0,
                            offset: clamp_char_boundary(f_text, focus.1),
                            affinity: Affinity::Upstream,
                        },
                    })
                } else {
                    None
                }
            }
        };
        if let Some(selection) = validated {
            self.doc_selection = Some(selection);
            self.doc_dragging = None;
            self.carets.stop();
            self.sync_document_selection_host();
        } else {
            self.clear_document_selection_state();
        }
    }

    /// Selects every selectable text run inside `container`.
    pub(crate) fn select_all_in_document_container(&mut self, container: NodeKey) -> bool {
        let root_selectable = self.selectable_at(container);
        let endpoints = {
            let document = self.document.borrow();
            let layout = self.layout.borrow();
            if !is_document_selection_container(&document, container) {
                None
            } else {
                let runs = collect_selectable_runs(&document, &layout, container, root_selectable);
                if let (Some(first), Some(last)) = (runs.first(), runs.last()) {
                    Some((
                        DocumentEndpoint {
                            node: first.node,
                            offset: 0,
                            affinity: Affinity::Downstream,
                        },
                        DocumentEndpoint {
                            node: last.node,
                            offset: last.byte_len,
                            affinity: Affinity::Upstream,
                        },
                    ))
                } else {
                    None
                }
            }
        };
        let Some((anchor, focus)) = endpoints else {
            return false;
        };
        self.doc_selection = Some(DocumentSelection {
            container,
            anchor,
            focus,
        });
        self.doc_dragging = None;
        self.carets.stop();
        self.sync_document_selection_host();
        true
    }

    /// Builds this frame's highlight plan and device-pixel damage rectangles for the active
    /// read-only document selection.
    pub(crate) fn doc_selection_plan(&self) -> (Plan, Vec<Rect<DevicePx, Device>>) {
        let Some(selection) = self.doc_selection else {
            return (Plan::empty(), Vec::new());
        };
        let root_selectable = self.selectable_at(selection.container);
        let document = self.document.borrow();
        let layout = self.layout.borrow();
        let runs = collect_selectable_runs(&document, &layout, selection.container, root_selectable);
        let Some((lo, hi)) = ordered_endpoints(&runs, selection.anchor, selection.focus) else {
            return (Plan::empty(), Vec::new());
        };
        if lo == hi {
            return (Plan::empty(), Vec::new());
        }

        // Group covered selectable runs by inline formatting context, preserving contiguous slices
        // so `user-select: none` inline spans inside a mixed IFC are skipped in highlight bands.
        let mut located_blocks: Vec<(Located, Vec<zgui_edit::hit::Band>, Color)> = Vec::new();
        let mut idx = lo.0;
        while idx <= hi.0 {
            let ifc_box = runs[idx].ifc_box;
            let Some(located) = Located::of_box(&layout, &self.text, ifc_box) else {
                idx += 1;
                continue;
            };
            let color = selection_band_color(&layout, ifc_box);
            let mut bands = Vec::new();
            while idx <= hi.0 && runs[idx].ifc_box == ifc_box {
                let slice_start_idx = idx;
                let start_run = runs[slice_start_idx].run_index;
                let start_off = if slice_start_idx == lo.0 { lo.1 } else { 0 };
                let mut slice_end_idx = slice_start_idx;
                while slice_end_idx + 1 <= hi.0
                    && runs[slice_end_idx + 1].ifc_box == ifc_box
                    && runs[slice_end_idx + 1].run_index == runs[slice_end_idx].run_index + 1
                {
                    slice_end_idx += 1;
                }
                let end_run = runs[slice_end_idx].run_index;
                let end_off = if slice_end_idx == hi.0 {
                    hi.1
                } else {
                    runs[slice_end_idx].byte_len
                };
                bands.extend(located.bands_for_sources(
                    SourcePos {
                        run: start_run,
                        offset: start_off,
                    },
                    SourcePos {
                        run: end_run,
                        offset: end_off,
                    },
                ));
                idx = slice_end_idx + 1;
            }
            if !bands.is_empty() {
                located_blocks.push((located, bands, color));
            }
        }

        let plan = plan_for_document_bands(
            located_blocks
                .iter()
                .map(|(located, bands, color)| (located, bands.clone(), *color)),
        );
        let drawn = self.absolute_rects_for_plan(&layout, &plan);
        (plan, drawn)
    }

    fn doc_device_point(&self, pointer: &PointerEvent) -> Point<DevicePx, Device> {
        Point::new(
            DevicePx(pointer.position.x.0 * self.scale),
            DevicePx(pointer.position.y.0 * self.scale),
        )
    }
}
