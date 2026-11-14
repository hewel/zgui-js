use serde_json::{Value, json};
use zgui::{reactive::prelude::GetUntracked, runtime::Window, view::ViewHost};

use crate::session::Session;

/// Only native data is read. Neither bridge getters nor application formatters run here.
pub fn snapshot(window: &Window, session: &Session, ordinal: u64) -> Value {
    let owned = session.bridge.snapshot(4096);
    let document = window.document().borrow();
    let focused = session.region.host.focused().get_untracked();
    let scale = window.surface().scale_factor();
    let mut text_budget = 128 * 1024;
    let mut visit_budget = 100_000;
    let mut text_truncated = false;
    let mut nodes = Vec::with_capacity(owned.nodes.len());
    for node in owned.nodes {
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
        nodes.push(json!({
            "token":format!("node-{}",node.token), "parent":node.parent.map(|token|format!("node-{token}")),
            "surface":"main", "generation":session.generation, "kind":node.kind,
            "ownership":"application", "text":text, "textTruncated":truncated,
            "testId":null, "role":null, "name":null, "source":null,
            "state":{"enabled":null,"visible":null,"focused":focused == Some(node.native)},
            "boundsLogical":rectangle(scale), "boundsDevice":rectangle(1.0),
            "boundsKind":"native-window-border-box", "pointerReachabilityVerified":false
        }));
    }
    json!({
        "schemaVersion":1,"surface":"main","generation":session.generation,
        "layoutVersion":ordinal,"frameOrdinal":ordinal,"roots":["node-0"],"nodes":nodes,
        "truncation":{"nodes":owned.truncated,"text":text_truncated,"maxNodes":4096,"maxTextBytes":131072},
        "semanticProjection":"unavailable"
    })
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
