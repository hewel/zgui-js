//! Read-only reconstruction of the native provider's ordered AccessKit updates.
//!
//! A delta is never evidence of a complete tree. The consumer owns replacement, root changes,
//! focus and unreachable-node retirement; this module owns log continuity and bounded evidence.

use std::fmt;

use accesskit::{NodeId as LocalNodeId, TreeId, TreeUpdate};
use accesskit_consumer::{TreeChangeHandler, Node, Tree};
use serde::Serialize;
use zgui::{platform::{Surface, SurfaceId}, runtime::Window, view::NodeId};
use zgui_platform_headless::OffscreenSurface;

use crate::preview::Error;

const MAX_NODES: usize = 4096;
const MAX_FIELD_BYTES: usize = 4096;
const MAX_TEXT_BYTES: usize = 128 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticNode {
    pub role: String,
    pub name: Option<String>,
    /// The node's own AccessKit disabled flag, not disabled-ancestor or input-reachability proof.
    pub enabled: Option<bool>,
    /// Inherited semantic hidden state, including native no-box projection. This does not prove
    /// CSS visibility, clipping, occlusion or pointer reachability.
    pub hidden: Option<bool>,
    /// Native text-input value, not the accessible name or a bridge/application getter.
    /// Password inputs retain the provider's masked value, never the secret editing buffer.
    pub value: Option<String>,
    pub focused: Option<bool>,
    pub name_truncated: bool,
    pub value_truncated: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSnapshot {
    pub available: bool,
    pub baseline_lost: bool,
    pub updates_consumed: u64,
    pub root: Option<String>,
    pub focus: Option<String>,
    pub nodes: Vec<SemanticRecord>,
    pub truncation: SemanticTruncation,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticRecord {
    /// This identifier belongs to the accessibility domain, not to bridge token space.
    pub id: String,
    pub parent: Option<String>,
    #[serde(flatten)]
    pub semantics: SemanticNode,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTruncation {
    pub nodes: bool,
    pub text: bool,
    pub max_nodes: usize,
    pub max_field_bytes: usize,
    pub max_text_bytes: usize,
}

#[derive(Default)]
pub struct SemanticModel {
    tree: Option<Tree>,
    surface: Option<SurfaceId>,
    updates_consumed: u64,
    baseline_lost: bool,
}

impl SemanticModel {
    /// Drain only on the UI thread, after native frame completion. No native work is requested.
    ///
    /// The headless log and its counter are separate reads, safe here because the same owner
    /// thread publishes them synchronously. Another log consumer or retention loss invalidates
    /// the baseline permanently; a fresh model and a real full provider update are then required.
    pub fn consume(&mut self, surface: &OffscreenSurface, window: &Window) -> Result<(), Error> {
        if surface.id() != window.surface().id()
            || self.surface.is_some_and(|id| id != surface.id())
        {
            self.tree = None;
            self.baseline_lost = true;
            return Err(Error::new("E_HOST_INVARIANT", 9, "semantic probe does not belong to the current native surface"));
        }
        self.surface = Some(surface.id());
        self.consume_updates(surface.a11y_updates(), surface.take_a11y_log(), window.is_surface_focused())
    }

    fn consume_updates(&mut self, published: u64, updates: Vec<TreeUpdate>, host_focused: bool) -> Result<(), Error> {
        let expected = self.updates_consumed.checked_add(updates.len() as u64);
        if self.baseline_lost || expected != Some(published) {
            return self.lose_baseline("native accessibility update log lost continuity");
        }
        for update in updates {
            if let Some(tree) = &mut self.tree {
                tree.update_and_process_changes(update, &mut IgnoreChanges);
            } else {
                if update.tree.is_none() || update.tree_id != TreeId::ROOT {
                    return self.lose_baseline("initial native accessibility update has no complete root tree");
                }
                self.tree = Some(Tree::new(update, host_focused));
            }
            self.updates_consumed += 1;
        }
        if let Some(tree) = &mut self.tree
            && tree.state().is_host_focused() != host_focused
        {
            tree.update_host_focus_state_and_process_changes(host_focused, &mut IgnoreChanges);
        }
        Ok(())
    }

    fn lose_baseline(&mut self, message: &str) -> Result<(), Error> {
        self.tree = None;
        self.baseline_lost = true;
        Err(Error::new("E_SEMANTIC_BASELINE_LOST", 9, message))
    }

    /// Project only a currently live node of the window whose log this model consumed.
    /// Oversized exact-match values stay unknown, never a truncated selector match.
    pub fn project(&self, window: &Window, native: NodeId) -> Option<SemanticNode> {
        if self.baseline_lost || self.surface != Some(window.surface().id()) {
            return None;
        }
        let id = LocalNodeId(window.semantic_node_id(native)?);
        let node = self.tree.as_ref()?.state().node_by_tree_local_id(id, TreeId::ROOT)?;
        let mut budget = MAX_TEXT_BYTES;
        let mut projected = project_node(node, &mut budget);
        if projected.name_truncated { projected.name = None; }
        if projected.value_truncated { projected.value = None; }

        // The provider prefers a declared semantic value over editing text. Such a declaration
        // is valid accessibility evidence but cannot establish the native editor's actual value.
        let document = window.document().borrow();
        let key = zgui_view_dom::id::to_document(native)?;
        if document.store().columns().semantics.get(key)
            .and_then(|slot| slot.as_deref())
            .is_some_and(|semantics| semantics.value.is_some())
        {
            projected.value = None;
        }
        Some(projected)
    }

    /// Flat preorder evidence of the full retained semantic tree, not merely the latest delta.
    /// Values here are the native accessibility values (which may be explicitly declared), not
    /// independently verified editor values. Prefixes are retained with explicit truncation.
    pub fn snapshot(&self) -> SemanticSnapshot {
        let mut snapshot = SemanticSnapshot {
            available: self.tree.is_some() && !self.baseline_lost,
            baseline_lost: self.baseline_lost,
            updates_consumed: self.updates_consumed,
            root: None,
            focus: None,
            nodes: Vec::new(),
            truncation: SemanticTruncation {
                nodes: false, text: false, max_nodes: MAX_NODES,
                max_field_bytes: MAX_FIELD_BYTES, max_text_bytes: MAX_TEXT_BYTES,
            },
        };
        let Some(tree) = &self.tree else { return snapshot; };
        let state = tree.state();
        snapshot.root = Some(semantic_id(state.root()));
        snapshot.focus = state.focus().map(semantic_id);
        let mut budget = MAX_TEXT_BYTES;
        let mut next = Some(state.root());
        while let Some(node) = next {
            if snapshot.nodes.len() == MAX_NODES {
                snapshot.truncation.nodes = true;
                break;
            }
            let semantics = project_node(node, &mut budget);
            snapshot.truncation.text |= semantics.name_truncated || semantics.value_truncated;
            snapshot.nodes.push(SemanticRecord {
                id: semantic_id(node),
                parent: node.parent().map(semantic_id),
                semantics,
            });
            next = next_preorder(node);
        }
        snapshot
    }
}

fn semantic_id(node: Node<'_>) -> String {
    format!("a11y-{}", u128::from(node.id()))
}

// Follow parent/sibling links instead of allocating an unbounded pending-child list.
fn next_preorder(mut node: Node<'_>) -> Option<Node<'_>> {
    if let Some(child) = node.children().next() { return Some(child); }
    loop {
        if let Some(sibling) = node.following_siblings().next() { return Some(sibling); }
        node = node.parent()?;
    }
}

fn project_node(node: Node<'_>, budget: &mut usize) -> SemanticNode {
    let mut role = format!("{:?}", node.role());
    role[..1].make_ascii_lowercase();
    let mut name = BoundedString::new((*budget).min(MAX_FIELD_BYTES));
    let has_name = node.write_label(&mut name);
    *budget -= name.text.len();
    let mut value = BoundedString::new((*budget).min(MAX_FIELD_BYTES));
    let has_value = if node.is_text_input() {
        node.write_value(&mut value)
    } else {
        Ok(false)
    };
    *budget -= value.text.len();
    SemanticNode {
        role,
        name: (has_name.unwrap_or(name.truncated)).then_some(name.text),
        enabled: Some(!node.is_disabled()),
        hidden: Some(node.is_hidden()),
        value: (has_value.unwrap_or(value.truncated)).then_some(value.text),
        focused: Some(node.is_focused()),
        name_truncated: name.truncated,
        value_truncated: value.truncated,
    }
}

struct BoundedString {
    text: String,
    limit: usize,
    truncated: bool,
}

impl BoundedString {
    fn new(limit: usize) -> Self { Self { text: String::new(), limit, truncated: false } }
}

impl fmt::Write for BoundedString {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let keep = text.floor_char_boundary((self.limit - self.text.len()).min(text.len()));
        self.text.push_str(&text[..keep]);
        if keep < text.len() {
            self.truncated = true;
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

// The consumer's change processing also retires unreachable state. No callbacks may mutate UI.
struct IgnoreChanges;
impl TreeChangeHandler for IgnoreChanges {
    fn node_added(&mut self, _: &Node) {}
    fn node_updated(&mut self, _: &Node, _: &Node) {}
    fn focus_moved(&mut self, _: Option<&Node>, _: Option<&Node>) {}
    fn node_removed(&mut self, _: &Node) {}
}

#[cfg(test)]
mod tests;
