use super::*;
use zgui::{
    reactive::{RwSignal, prelude::{Get, Set}},
    runtime::{App, Runtime},
    view::{A11yBinding, AnyView, IntoView, NodeRef, ShowProps, View},
    vocab::{A11y, Role},
};
use zgui_a11y::{A11yBuilder, World, to_a11y};
use zgui_dom::{Document, EverythingMatters, NodeIndex, NodeKey};
use zgui_layout::tree::store::LayoutStore;
use zgui_platform_headless::Harness;

/// Provider-generated deltas over real document edits, without invented TreeUpdate fixtures.
struct Provider {
    document: Document,
    layout: LayoutStore,
    builder: A11yBuilder,
    root: NodeIndex,
}

impl Provider {
    fn new() -> Self { Self::with_id(0) }

    fn with_id(id:u16) -> Self {
        let document = Document::with_id(zgui_dom::DocumentId::new(id).unwrap());
        let root = document.edit(&EverythingMatters, |edit| {
            let root = edit.create_element("root".into());
            edit.insert_before(document.document_index(), root, None);
            root
        }).unwrap();
        let layout = LayoutStore::new(document.store().document());
        Self { document, layout, builder: A11yBuilder::new(), root }
    }

    fn button(&self, parent: NodeIndex, name: &str) -> (NodeIndex, NodeIndex) {
        self.document.edit(&EverythingMatters, |edit| {
            let button = edit.create_element("control".into());
            edit.insert_before(parent, button, None);
            edit.set_semantics(button, Some(A11y::new(Role::Button).into()));
            let text = edit.create_text(name);
            edit.insert_before(button, text, None);
            (button, text)
        }).unwrap()
    }

    fn id(&self, node: NodeIndex) -> LocalNodeId { to_a11y(self.document.store().key_of(node)) }

    fn update(&mut self, focus: Option<NodeKey>) -> TreeUpdate {
        self.builder.collect(&mut self.document);
        self.builder.build(&World {
            document: &self.document, layout: &self.layout,
            placements: &zgui::scene::Placements::EMPTY,
            scale: 1.0, focus, scroll: None,
        })
    }

    fn focus_update(&self, focus: Option<NodeKey>) -> TreeUpdate {
        self.builder.focus_update(&World {
            document: &self.document, layout: &self.layout,
            placements: &zgui::scene::Placements::EMPTY,
            scale: 1.0, focus, scroll: None,
        })
    }
}

fn apply(model: &mut SemanticModel, updates: Vec<TreeUpdate>, host_focused: bool) {
    model.consume_updates(model.updates_consumed + updates.len() as u64, updates, host_focused).unwrap();
}

fn retained(model: &SemanticModel, id: LocalNodeId) -> Option<Node<'_>> {
    model.tree.as_ref()?.state().node_by_tree_local_id(id, TreeId::ROOT)
}

#[test]
fn ordered_deltas_keep_unchanged_nodes_and_retire_an_entire_removed_subtree() {
    let mut provider = Provider::new();
    let (branch, changing_text) = provider.button(provider.root, "Before");
    let (nested, _) = provider.button(branch, "Nested");
    let (stable, _) = provider.button(provider.root, "Stable");
    let branch_id = provider.id(branch);
    let nested_id = provider.id(nested);
    let stable_id = provider.id(stable);
    let mut model = SemanticModel::default();
    apply(&mut model, vec![provider.update(None)], true);
    let initial_count = model.snapshot().nodes.len();

    provider.document.edit(&EverythingMatters, |edit| edit.set_text(changing_text, "After")).unwrap();
    let delta = provider.update(None);
    assert!(delta.tree.is_none());
    assert!(!delta.nodes.iter().any(|(id, _)| *id == stable_id));
    apply(&mut model, vec![delta], true);
    assert_eq!(retained(&model, branch_id).unwrap().label().as_deref(), Some("After"));
    assert_eq!(retained(&model, stable_id).unwrap().label().as_deref(), Some("Stable"));
    assert_eq!(model.snapshot().nodes.len(), initial_count);

    provider.document.edit(&EverythingMatters, |edit| edit.remove(branch)).unwrap();
    apply(&mut model, vec![provider.update(None)], true);
    assert!(retained(&model, branch_id).is_none());
    assert!(retained(&model, nested_id).is_none());
    assert!(retained(&model, stable_id).is_some());
    assert_eq!(model.snapshot().nodes.len(), initial_count - 2);
}

#[test]
fn provider_root_replacement_discards_the_previous_document_tree() {
    let mut old = Provider::new();
    let (old_button, _) = old.button(old.root, "Old");
    let old_id = old.id(old_button);
    let old_root = old.id(old.document.document_index());
    let mut current = Provider::with_id(1);
    let (new_button, _) = current.button(current.root, "New");
    let new_id = current.id(new_button);
    let mut model = SemanticModel::default();
    apply(&mut model, vec![old.update(None), current.update(None)], true);
    assert!(retained(&model, old_root).is_none());
    assert!(retained(&model, old_id).is_none());
    assert_eq!(retained(&model, new_id).unwrap().label().as_deref(), Some("New"));
    assert_eq!(model.snapshot().nodes.len(), 3);
    assert_eq!(model.snapshot().root, Some(semantic_id(model.tree.as_ref().unwrap().state().root())));
}

#[test]
fn focus_only_updates_and_host_focus_changes_preserve_the_tree() {
    let mut provider = Provider::new();
    let (first, _) = provider.button(provider.root, "First");
    let (second, _) = provider.button(provider.root, "Second");
    let first_id = provider.id(first);
    let second_id = provider.id(second);
    let first_key = provider.document.store().key_of(first);
    let second_key = provider.document.store().key_of(second);
    let mut model = SemanticModel::default();
    apply(&mut model, vec![provider.update(Some(first_key))], true);
    assert!(retained(&model, first_id).unwrap().is_focused());
    let count = model.snapshot().nodes.len();

    let delta = provider.focus_update(Some(second_key));
    assert!(delta.nodes.is_empty());
    apply(&mut model, vec![delta], true);
    assert!(!retained(&model, first_id).unwrap().is_focused());
    assert!(retained(&model, second_id).unwrap().is_focused());
    let second_focus = model.snapshot().focus;
    apply(&mut model, Vec::new(), false);
    assert!(model.snapshot().focus.is_none());
    assert!(!retained(&model, second_id).unwrap().is_focused());
    apply(&mut model, Vec::new(), true);
    assert_eq!(model.snapshot().focus, second_focus);
    assert_eq!(model.snapshot().nodes.len(), count);

    provider.document.edit(&EverythingMatters, |edit| edit.remove(second)).unwrap();
    apply(&mut model, vec![provider.update(Some(second_key))], true);
    assert!(retained(&model, second_id).is_none());
    assert_eq!(model.snapshot().focus, model.snapshot().root);
}

#[test]
fn names_follow_provider_label_relations_and_label_retirement() {
    let mut provider = Provider::new();
    let (label, field, text) = provider.document.edit(&EverythingMatters, |edit| {
        let label = edit.create_element("label".into());
        edit.insert_before(provider.root, label, None);
        edit.set_semantics(label, Some(A11y::new(Role::Label).into()));
        let text = edit.create_text("Account name");
        edit.insert_before(label, text, None);
        let field = edit.create_element("field".into());
        edit.insert_before(provider.root, field, None);
        let label_id = to_a11y(provider.document.store().key_of(label));
        edit.set_semantics(field, Some(A11y::new(Role::TextInput).labelled_by(label_id).into()));
        (label, field, text)
    }).unwrap();
    let field_id = provider.id(field);
    let mut model = SemanticModel::default();
    apply(&mut model, vec![provider.update(None)], true);
    assert_eq!(retained(&model, field_id).unwrap().label().as_deref(), Some("Account name"));
    provider.document.edit(&EverythingMatters, |edit| edit.set_text(text, "Updated account")).unwrap();
    apply(&mut model, vec![provider.update(None)], true);
    assert_eq!(retained(&model, field_id).unwrap().label().as_deref(), Some("Updated account"));
    provider.document.edit(&EverythingMatters, |edit| edit.remove(label)).unwrap();
    apply(&mut model, vec![provider.update(None)], true);
    assert_eq!(retained(&model, field_id).unwrap().label(), None);
}

#[test]
fn replacement_nodes_clear_old_disabled_state_and_explicit_names() {
    let mut provider = Provider::new();
    let (button, _) = provider.button(provider.root, "Native content");
    let id = provider.id(button);
    provider.document.edit(&EverythingMatters, |edit| {
        edit.set_semantics(button, Some(A11y::new(Role::Button).label("Explicit").disabled(true).into()));
    }).unwrap();
    let mut model = SemanticModel::default();
    apply(&mut model, vec![provider.update(None)], true);
    let mut budget = MAX_TEXT_BYTES;
    let initial = project_node(retained(&model, id).unwrap(), &mut budget);
    assert_eq!(initial.enabled, Some(false));
    assert_eq!(initial.name.as_deref(), Some("Explicit"));
    provider.document.edit(&EverythingMatters, |edit| {
        edit.set_semantics(button, Some(A11y::new(Role::Button).into()));
    }).unwrap();
    apply(&mut model, vec![provider.update(None)], true);
    let mut budget = MAX_TEXT_BYTES;
    let replaced = project_node(retained(&model, id).unwrap(), &mut budget);
    assert_eq!(replaced.enabled, Some(true));
    assert_eq!(replaced.name.as_deref(), Some("Native content"));
}

#[test]
fn text_input_values_follow_native_document_edits_without_replacing_the_name() {
    let mut provider = Provider::new();
    let (field, text) = provider.document.edit(&EverythingMatters, |edit| {
        let field = edit.create_element("field".into());
        edit.insert_before(provider.root, field, None);
        edit.set_semantics(field, Some(A11y::new(Role::TextInput).label("Account").into()));
        let text = edit.create_text("Initial");
        edit.insert_before(field, text, None);
        (field, text)
    }).unwrap();
    let id = provider.id(field);
    let mut model = SemanticModel::default();
    apply(&mut model, vec![provider.update(None)], true);
    let mut budget = MAX_TEXT_BYTES;
    let initial = project_node(retained(&model, id).unwrap(), &mut budget);
    assert_eq!(initial.name.as_deref(), Some("Account"));
    assert_eq!(initial.value.as_deref(), Some("Initial"));
    provider.document.edit(&EverythingMatters, |edit| edit.set_text(text, "Typed")).unwrap();
    apply(&mut model, vec![provider.update(None)], true);
    let mut budget = MAX_TEXT_BYTES;
    let edited = project_node(retained(&model, id).unwrap(), &mut budget);
    assert_eq!(edited.name.as_deref(), Some("Account"));
    assert_eq!(edited.value.as_deref(), Some("Typed"));
}

#[test]
fn snapshot_text_budget_is_shared_across_nodes_without_losing_topology() {
    let mut provider = Provider::new();
    let name = "x".repeat(MAX_FIELD_BYTES);
    for _ in 0..(MAX_TEXT_BYTES / MAX_FIELD_BYTES + 2) {
        provider.button(provider.root, &name);
    }
    let mut model = SemanticModel::default();
    apply(&mut model, vec![provider.update(None)], true);
    let snapshot = model.snapshot();
    assert!(!snapshot.truncation.nodes);
    assert!(snapshot.truncation.text);
    assert_eq!(snapshot.nodes.len(), MAX_TEXT_BYTES / MAX_FIELD_BYTES + 4);
    let bytes: usize = snapshot.nodes.iter().filter_map(|record| record.semantics.name.as_ref()).map(String::len).sum();
    assert_eq!(bytes, MAX_TEXT_BYTES);
    assert!(snapshot.nodes.last().unwrap().semantics.name_truncated);
}

#[test]
fn missing_baseline_and_a_lost_update_fail_closed_without_stale_evidence() {
    let mut provider = Provider::new();
    provider.button(provider.root, "Kept");
    let baseline = provider.update(None);
    let mut missing = SemanticModel::default();
    let error = missing.consume_updates(2, vec![baseline.clone()], true).unwrap_err();
    assert_eq!(error.code, "E_SEMANTIC_BASELINE_LOST");
    assert!(!missing.snapshot().available);
    assert!(missing.snapshot().baseline_lost);

    let mut no_root = SemanticModel::default();
    let delta = provider.focus_update(None);
    assert_eq!(no_root.consume_updates(1, vec![delta], true).unwrap_err().code, "E_SEMANTIC_BASELINE_LOST");

    let mut model = SemanticModel::default();
    apply(&mut model, vec![baseline], true);
    assert!(model.snapshot().available);
    let error = model.consume_updates(2, Vec::new(), true).unwrap_err();
    assert_eq!(error.code, "E_SEMANTIC_BASELINE_LOST");
    assert!(model.snapshot().nodes.is_empty());
    // Even a later full update is not silently accepted after an unexplained sequence gap.
    provider.builder.forget();
    let full = provider.update(None);
    assert!(model.consume_updates(2, vec![full], true).is_err());
}

#[test]
fn semantic_evidence_bounds_unicode_text_and_node_counts_explicitly() {
    let mut provider = Provider::new();
    provider.button(provider.root, &"界".repeat(MAX_FIELD_BYTES));
    for index in 0..MAX_NODES {
        provider.button(provider.root, &format!("Button {index}"));
    }
    let mut model = SemanticModel::default();
    apply(&mut model, vec![provider.update(None)], true);
    let snapshot = model.snapshot();
    assert_eq!(snapshot.nodes.len(), MAX_NODES);
    assert!(snapshot.truncation.nodes);
    assert!(snapshot.truncation.text);
    let long = &snapshot.nodes[2].semantics;
    assert!(long.name_truncated);
    assert!(long.name.as_ref().unwrap().len() <= MAX_FIELD_BYTES);
    let text_bytes: usize = snapshot.nodes.iter().map(|record| {
        record.semantics.name.as_ref().map_or(0, String::len)
            + record.semantics.value.as_ref().map_or(0, String::len)
    }).sum();
    assert!(text_bytes <= MAX_TEXT_BYTES);
    let encoded = serde_json::to_vec(&snapshot).unwrap();
    assert!(encoded.len() < 2 * 1024 * 1024);
    // Serialization truncation must not mutate the retained consumer state.
    assert!(model.tree.as_ref().unwrap().state().root().children().next().unwrap().children().count() > MAX_NODES);
}

fn native_app<V>(view: V) -> Harness<Runtime>
where V: FnMut(&mut zgui::view::BuildCx<'_>) -> Box<dyn zgui::view::Anchor> + 'static {
    let app = App::new().with_size(400.0, 300.0)
        .with_stylesheet("root { display: block; width: 400px; height: 300px } control { display: block; width: 100px; height: 24px }")
        .with_renderer(Box::new(|_, target| {
            let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
            zgui::render::Renderer::configure(&mut renderer, target);
            Ok(Box::new(renderer))
        }))
        .into_handler(view).unwrap();
    let mut harness = Harness::new(app);
    harness.settle(8);
    harness
}

#[test]
fn live_native_mapping_rejects_foreign_retired_and_replaced_handles() {
    let shown = RwSignal::new_local(true);
    let button = NodeRef::new();
    let mut harness = native_app(move |cx| Box::new(zgui::elements::column()
        .child(ShowProps::builder().when(move || shown.get()).children(move || {
            AnyView::new(zgui::elements::control().node_ref(button)
                .a11y(A11yBinding::new(Role::Button).label("Native button")))
        }).build().render()).into_view().build(cx)));
    let surface = harness.platform().offscreens()[0].clone();
    let window = &harness.app().windows()[0];
    let native = button.get_untracked().unwrap();
    let key = zgui_view_dom::id::to_document(native).unwrap();
    assert_eq!(window.semantic_node_id(native), Some(to_a11y(key).0));
    let foreign = Provider::with_id((window.document_id().get()+1)%4096);
    let foreign_handle = zgui_view_dom::id::to_view(foreign.document.store().key_of(foreign.root));
    assert!(window.semantic_node_id(foreign_handle).is_none());
    let mut model = SemanticModel::default();
    model.consume(&surface, window).unwrap();
    let semantic = model.project(window, native).unwrap();
    assert_eq!(semantic.role, "button");
    assert_eq!(semantic.name.as_deref(), Some("Native button"));
    assert!(model.project(window, foreign_handle).is_none());

    shown.set(false);
    harness.settle(8);
    let window = &harness.app().windows()[0];
    assert!(window.semantic_node_id(native).is_none());
    assert!(model.project(window, native).is_none());
    model.consume(&surface, window).unwrap();
    shown.set(true);
    harness.settle(8);
    let window = &harness.app().windows()[0];
    let replacement = button.get_untracked().unwrap();
    assert_ne!(replacement, native);
    model.consume(&surface, window).unwrap();
    assert!(window.semantic_node_id(native).is_none());
    assert!(model.project(window, native).is_none());
    assert_eq!(model.project(window, replacement).unwrap().name.as_deref(), Some("Native button"));

    // A second consumer draining the actual headless channel is detected by its cumulative count.
    shown.set(false);
    harness.settle(8);
    assert!(!surface.take_a11y_log().is_empty());
    assert_eq!(model.consume(&surface, &harness.app().windows()[0]).unwrap_err().code, "E_SEMANTIC_BASELINE_LOST");
    assert!(!model.snapshot().available);
}

#[test]
fn exact_native_projection_does_not_match_truncated_names_or_declared_editor_values() {
    let oversized = "é".repeat(MAX_FIELD_BYTES);
    let mut harness = native_app(move |cx| Box::new(zgui::elements::column()
        .child(zgui::elements::control().class("long")
            .a11y(A11yBinding::new(Role::Button).label(oversized.clone())))
        .child(zgui::elements::control().class("declared")
            .a11y(A11yBinding::new(Role::TextInput).value("authored, not editor text")))
        .into_view().build(cx)));
    harness.settle(8);
    let window = &harness.app().windows()[0];
    let surface = harness.platform().offscreens()[0].clone();
    let mut model = SemanticModel::default();
    model.consume(&surface, window).unwrap();
    let long = model.project(window, window.nodes_with_class("long")[0]).unwrap();
    assert!(long.name_truncated);
    assert_eq!(long.name, None);
    let declared = model.project(window, window.nodes_with_class("declared")[0]).unwrap();
    assert_eq!(declared.value, None);
}
