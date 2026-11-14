use serde_json::{Value, json};
use zgui::{reactive::prelude::GetUntracked, runtime::Window, view::ViewHost};

use crate::session::Session;
use super::semantics::SemanticModel;

pub struct Snapshot {
    pub tree: Value,
    pub native_nodes: Vec<ui_bridge::registry::SnapshotNode>,
    pub metadata_revision: u64,
}

/// Visibility means a positive native box with computed visibility:visible, not pointer reachability.
pub fn native_state(window: &Window, native: zgui::view::NodeId) -> Option<(bool, bool)> {
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
        layout.get(*box_).is_some_and(|node| node.style.get_inherited_box().visibility == zgui_css::values::size::VisibilityValue::Visible)
            && layout.fragments_of_box(*box_).iter().any(|fragment| layout.fragment(*fragment)
                .is_some_and(|fragment| fragment.border_box.size.width.0 > 0.0 && fragment.border_box.size.height.0 > 0.0))
    });
    Some((enabled, visible))
}

/// Only native data is read. Neither bridge getters nor application formatters run here.
pub fn snapshot(window: &Window, session: &Session, ordinal: u64, semantics: &SemanticModel) -> Snapshot {
    let owned = session.bridge.snapshot(4096);
    let document = window.document().borrow();
    let focused = session.region.host.focused().get_untracked();
    let scale = window.surface().scale_factor();
    let mut text_budget = 128 * 1024;
    let mut visit_budget = 100_000;
    let mut text_truncated = false;
    let mut nodes = Vec::with_capacity(owned.nodes.len());
    let mut metadata_budget = 128 * 1024;
    let mut semantic_budget = 128 * 1024;
    let mut metadata_truncated = false;
    for node in &owned.nodes {
        let index = zgui_view_dom::id::resolve(&document, node.native);
        let (text, truncated) = bounded_text(&document, index, text_budget.min(4096), &mut visit_budget);
        text_budget -= text.len();
        text_truncated |= truncated;
        let bounds = ViewHost::window_box(window.host().as_ref(), node.native);
        let rectangle = |divisor: f64| bounds.map(|bounds| json!({
            "x": f64::from(bounds.origin.x.0) / divisor,
            "y": f64::from(bounds.origin.y.0) / divisor,
            "width": f64::from(bounds.size.width.0) / divisor,
            "height": f64::from(bounds.size.height.0) / divisor,
        }));
        let mut semantic = semantics.project(window, node.native);
        if let Some(semantic) = &mut semantic {
            for (value,truncated) in [(&mut semantic.name,&mut semantic.name_truncated),(&mut semantic.value,&mut semantic.value_truncated)] {
                if let Some(text) = value {
                    if text.len()>semantic_budget { *value=None; *truncated=true; }
                    else { semantic_budget-=text.len(); }
                }
            }
        }
        let test_id = node.test_id.as_deref().filter(|value| {
            if value.len()>metadata_budget { return false; }
            metadata_budget-=value.len();
            true
        });
        let test_id_truncated = node.test_id.is_some() && test_id.is_none();
        metadata_truncated |= test_id_truncated;
        let state = native_state(window, node.native);
        nodes.push(json!({
            "token":format!("node-{}",node.token), "parent":node.parent.map(|token|format!("node-{token}")),
            "surface":"main", "generation":session.generation, "kind":node.kind,
            "ownership":"application", "text":text, "textTruncated":truncated,
            "testId":test_id, "testIdTruncated":test_id_truncated, "role":semantic.as_ref().map(|value|&value.role),
            "name":semantic.as_ref().and_then(|value|value.name.as_ref()), "source":null,
            "value":semantic.as_ref().and_then(|value|value.value.as_ref()),
            "nameTruncated":semantic.as_ref().is_some_and(|value|value.name_truncated),
            "valueTruncated":semantic.as_ref().is_some_and(|value|value.value_truncated),
            "state":{"enabled":state.map(|value|value.0),"visible":state.map(|value|value.1),"focused":focused == Some(node.native)},
            "boundsLogical":rectangle(scale), "boundsDevice":rectangle(1.0),
            "boundsKind":"native-window-border-box", "pointerReachabilityVerified":false
        }));
    }
    let tree = json!({
        "schemaVersion":1,"surface":"main","generation":session.generation,
        "layoutVersion":ordinal,"frameOrdinal":ordinal,"metadataRevision":owned.metadata_revision,"roots":["node-0"],"nodes":nodes,
        "truncation":{"nodes":owned.truncated,"text":text_truncated,"metadata":metadata_truncated,"maxNodes":4096,"maxTextBytes":131072,"maxMetadataBytes":131072,"maxSemanticTextBytes":131072},
        "semanticProjection":semantics.snapshot()
    });
    Snapshot { tree, native_nodes:owned.nodes, metadata_revision:owned.metadata_revision }
}

fn bounded_text(document: &zgui_dom::Document, root: zgui_dom::NodeIndex, bytes: usize, visits: &mut usize) -> (String, bool) {
    let mut text = String::new();
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if *visits == 0 { return (text, true); }
        *visits -= 1;
        if let Some(own) = zgui_dom::text::node::text_of(document.store(), index) {
            let keep = own.floor_char_boundary(bytes.saturating_sub(text.len()).min(own.len()));
            text.push_str(&own[..keep]);
            if keep < own.len() { return (text, true); }
        }
        let core = document.store().core(index);
        if index != root && let Some(next) = core.next_sibling() { pending.push(next); }
        if let Some(child) = core.first_child() { pending.push(child); }
    }
    (text, false)
}
