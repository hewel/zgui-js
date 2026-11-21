use serde_json::json;
use zgui::{reactive::prelude::GetUntracked, runtime::Window, view::ViewHost};

use crate::inspector::{bounded_text, scroll_metrics};
pub(crate) use crate::inspector::native_state;
use crate::session::SessionRegion;
use super::semantics::SemanticModel;
use crate::preview::evidence::{Bounds, CaptureIdentity, NodeEvidence, NodeState, TreeEvidence};

pub struct Snapshot {
    pub tree: TreeEvidence,
    pub native_nodes: Vec<ui_bridge::registry::SnapshotNode>,
    pub metadata_revision: u64,
}

/// Only native data is read. Neither bridge getters nor application formatters run here.
pub fn snapshot(window: &Window, region: &SessionRegion, generation: u64, ordinal: u64, semantics: &SemanticModel, identity: &CaptureIdentity) -> Snapshot {
    let owned = region.bridge.snapshot(4096);
    let document = window.document().borrow();
    let focused = region.region.host.focused().get_untracked();
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
        let rectangle = |divisor: f64| bounds.map(|bounds| Bounds {
            x: f64::from(bounds.origin.x.0) / divisor,
            y: f64::from(bounds.origin.y.0) / divisor,
            width: f64::from(bounds.size.width.0) / divisor,
            height: f64::from(bounds.size.height.0) / divisor,
        });
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
        nodes.push(NodeEvidence {
            token:format!("node-{}",node.token), parent:node.parent.map(|token|format!("node-{token}")),
            surface:region.name.clone(), generation, kind:node.kind.into(),
            ownership:"application".into(), text, text_truncated:truncated,
            test_id:test_id.map(str::to_owned), test_id_truncated, role:semantic.as_ref().map(|value|value.role.clone()),
            name:semantic.as_ref().and_then(|value|value.name.clone()),
            source:serde_json::Value::Null,
            value:semantic.as_ref().and_then(|value|value.value.clone()),
            name_truncated:semantic.as_ref().is_some_and(|value|value.name_truncated),
            value_truncated:semantic.as_ref().is_some_and(|value|value.value_truncated),
            state:NodeState { enabled:state.map(|value|value.0), visible:state.map(|value|value.1), focused:focused == Some(node.native) },
            bounds_logical:rectangle(scale), bounds_device:rectangle(1.0),
            bounds_kind:"native-window-border-box".into(),
            scroll_metrics:scroll_metrics(window, node.native, scale),
            pointer_reachability_verified:false,
            extra:serde_json::Map::new(),
        });
    }
    let mut extra = serde_json::Map::new();
    extra.insert("layoutVersion".into(), json!(ordinal));
    extra.insert("roots".into(), json!(["node-0"]));
    extra.insert("truncation".into(), json!({"nodes":owned.truncated,"text":text_truncated,"metadata":metadata_truncated,
        "maxNodes":4096,"maxTextBytes":131072,"maxMetadataBytes":131072,"maxSemanticTextBytes":131072}));
    extra.insert("semanticProjection".into(), serde_json::to_value(semantics.snapshot()).expect("semantic snapshot serializes"));
    let tree = TreeEvidence {
        schema_version:1, build_id:identity.build_id.clone(), surface:region.name.clone(), generation,
        frame_ordinal:ordinal, metadata_revision:owned.metadata_revision, presentation_sequence:identity.presentation_sequence,
        resource_revision:identity.resource_revision, surface_configuration:identity.surface_configuration,
        style_revision: identity.style_revision,
        nodes, extra,
    };
    Snapshot { tree, native_nodes:owned.nodes, metadata_revision:owned.metadata_revision }
}
