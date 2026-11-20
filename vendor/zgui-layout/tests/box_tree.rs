//! The invariants the box tree has to hold whatever document it was built from.

mod support;

use support::{Element, Fixture};
use zgui_layout::tree::store::LayoutStore;
use zgui_layout::{BoxKey, BoxKind};

/// Every box below `root`, in layout order.
fn walk(store: &LayoutStore) -> Vec<BoxKey> {
    let mut out = Vec::new();
    let Some(root) = store.root() else {
        return out;
    };
    let mut stack = vec![root];
    while let Some(key) = stack.pop() {
        // An out-of-flow box is reached twice — once through the paint list of the box it was
        // written inside, once through the layout list of the ancestor that positions it — and
        // visiting it twice would make every invariant below count its children twice.
        if out.contains(&key) {
            continue;
        }
        out.push(key);
        stack.extend(store.node(key).children.iter().copied());
        for &child in &store.node(key).paint_children {
            if !store.node(key).children.contains(&child) {
                stack.push(child);
            }
        }
    }
    out
}

/// Asserts every structural invariant over one built tree.
fn assert_invariants(store: &LayoutStore) {
    let root = store.root().expect("a root");
    let boxes = walk(store);
    assert!(!boxes.is_empty());

    for &key in &boxes {
        let node = store.node(key);
        if key == root {
            assert_eq!(node.parent, None, "the root has no parent");
        } else {
            let parent = node.parent.expect("every box but the root has a parent");
            let parent_node = store.node(parent);
            // The *layout* list, not either list: a box is laid out by exactly one container, and
            // that container is what its parent has to name. Accepting the paint list too would let
            // a box swept into an anonymous wrapper keep pointing at the block that no longer lays
            // it out, and every walk that goes up from a box would leave the wrapper out.
            assert!(
                parent_node.children.contains(&key),
                "a box's parent is not the container that lays it out"
            );
            assert_eq!(
                node.parent_fc, parent_node.fc,
                "a box records a different formatting context from the one its parent runs"
            );
        }

        if matches!(
            node.kind,
            BoxKind::AnonymousBlock | BoxKind::AnonymousInlineRoot
        ) {
            assert!(
                !node.children.is_empty(),
                "an anonymous wrapper with nothing to wrap should not exist"
            );
        }

        // A box whose element flattened away leaves no orphan: every box a child list names is
        // live, so nothing points at a slot that was dropped.
        for &child in node.children.iter().chain(node.paint_children.iter()) {
            assert!(store.contains(child), "a child list names a dead box");
        }
    }

    // The two orders hold the same boxes for every container that did not move anything out of
    // flow, and a box that *was* moved appears in exactly one layout list.
    let mut layout_parents: Vec<BoxKey> = Vec::new();
    for &key in &boxes {
        for &child in &store.node(key).children {
            assert!(
                !layout_parents.contains(&child),
                "a box is a layout child of two containers"
            );
            layout_parents.push(child);
        }
    }
}

#[test]
fn a_plain_tree_holds_every_invariant() {
    let fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new("a").text("one"),
            Element::new("b").children(vec![Element::new("c").text("two")]),
        ]),
        "root { display: block; width: 200px }
         a { display: block }
         b { display: flex }
         c { display: block }",
    );
    assert_invariants(&fixture.box_tree());
}

#[test]
fn display_contents_leaves_no_orphan() {
    // The element generates no box and its children take its place, so nothing may point at a box
    // that was never made and no child may be lost on the way up.
    let fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new("wrapper").children(vec![
                Element::new("a").text("one"),
                Element::new("b").text("two"),
            ]),
            Element::new("c").text("three"),
        ]),
        "root { display: block; width: 200px }
         wrapper { display: contents }
         a { display: block }
         b { display: block }
         c { display: block }",
    );
    let store = fixture.box_tree();
    assert_invariants(&store);

    // The three block-level children are siblings of one another, with no box for the wrapper.
    let root = store.root().expect("a root");
    assert_eq!(
        store.node(root).children.len(),
        3,
        "the flattened element's children became the root's own"
    );
    assert!(
        walk(&store).iter().all(
            |&key| store.node(key).kind != BoxKind::Element || store.node(key).source.is_some()
        ),
        "every element box names an element"
    );
}

#[test]
fn a_flattened_element_with_no_children_generates_nothing_at_all() {
    let fixture = Fixture::new(
        Element::new("root").children(vec![Element::new("wrapper"), Element::new("a").text("one")]),
        "root { display: block; width: 200px }
         wrapper { display: contents }
         a { display: block }",
    );
    let store = fixture.box_tree();
    assert_invariants(&store);
    assert_eq!(store.node(store.root().expect("a root")).children.len(), 1);
}

#[test]
fn a_hidden_element_generates_no_box_and_neither_does_its_subtree() {
    let fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new("gone").children(vec![Element::new("a").text("one")]),
            Element::new("b").text("two"),
        ]),
        "root { display: block; width: 200px }
         gone { display: none }
         a { display: block }
         b { display: block }",
    );
    let store = fixture.box_tree();
    assert_invariants(&store);
    assert_eq!(store.node(store.root().expect("a root")).children.len(), 1);
}

#[test]
fn an_out_of_flow_box_is_a_layout_child_of_the_ancestor_that_positions_it() {
    // Its paint entry stays where it was written, because painting order and accessible geometry
    // follow the document rather than the containing block.
    let fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new("holder").children(vec![Element::new("floating").text("x")]),
        ]),
        "root { display: block; width: 200px; position: relative }
         holder { display: block }
         floating { display: block; position: absolute; top: 0; left: 0 }",
    );
    let store = fixture.box_tree();
    assert_invariants(&store);

    let root = store.root().expect("a root");
    let holder = store.node(root).children[0];
    let floating = *store
        .node(holder)
        .paint_children
        .iter()
        .find(|&&child| store.node(child).source.is_some())
        .expect("the out-of-flow box is painted where it was written");
    assert!(
        store.node(root).children.contains(&floating),
        "the out-of-flow box is laid out by the positioned ancestor"
    );
    assert!(
        !store.node(holder).children.contains(&floating),
        "and not by the box it was written inside"
    );
}

#[test]
fn an_out_of_flow_box_with_no_positioned_ancestor_is_a_layout_child_of_the_root() {
    // The root's box is the initial containing block. Laid out by the box it was written inside, a
    // fixed box with `height: 100%` takes the height of that box rather than of the window.
    let fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new("holder").children(vec![Element::new("floating").text("x")]),
        ]),
        "root { display: block; width: 200px }
         holder { display: block }
         floating { display: block; position: fixed; top: 0; left: 0 }",
    );
    let store = fixture.box_tree();
    assert_invariants(&store);

    let root = store.root().expect("a root");
    let holder = store.node(root).children[0];
    let floating = *store
        .node(holder)
        .paint_children
        .iter()
        .find(|&&child| store.node(child).source.is_some())
        .expect("the out-of-flow box is painted where it was written");
    assert!(
        store.node(root).children.contains(&floating),
        "the out-of-flow box is laid out by the root"
    );
    assert!(
        !store.node(holder).children.contains(&floating),
        "and not by the box it was written inside"
    );
}

#[test]
fn a_root_that_is_itself_out_of_flow_stays_the_root() {
    let fixture = Fixture::new(
        Element::new("root").children(vec![Element::new("a").text("one")]),
        "root { display: block; width: 200px; position: fixed }
         a { display: block }",
    );
    let store = fixture.box_tree();
    assert_invariants(&store);
    let root = store.root().expect("a root");
    assert_eq!(store.node(root).parent, None, "nothing contains the root");
}

#[test]
fn a_wrapped_inline_run_belongs_to_the_wrapper_and_not_to_the_block_it_was_written_in() {
    // The wrapper is what lays the run out and what establishes the formatting context it takes
    // part in. A run still pointing at the block above the wrapper would make every walk upwards
    // skip a box that has a position of its own, and would report the run as taking part in block
    // layout when it takes part in inline layout.
    let fixture = Fixture::new(
        Element::new("root").children(vec![Element::new("a").text("one")]),
        "root { display: block; width: 200px }
         a { display: block }",
    );
    let store = fixture.box_tree();
    assert_invariants(&store);

    let block = store.node(store.root().expect("a root")).children[0];
    let wrapper = store.node(block).children[0];
    assert_eq!(store.node(wrapper).kind, BoxKind::AnonymousInlineRoot);
    let run = store.node(wrapper).children[0];
    assert_eq!(store.node(run).kind, BoxKind::TextRun);
    assert_eq!(
        store.node(run).parent,
        Some(wrapper),
        "the run's parent is the wrapper that lays it out"
    );
    assert_eq!(
        store.node(run).parent_fc,
        zgui_layout::FormattingContext::Inline,
        "the run takes part in the wrapper's inline formatting context"
    );
    assert!(
        store.node(block).paint_children.contains(&run),
        "and it is still painted where it was written"
    );
}

#[test]
fn order_permutes_the_layout_list_and_leaves_the_paint_list_alone() {
    let fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new("a").text("one"),
            Element::new("b").text("two"),
        ]),
        "root { display: flex; width: 200px }
         a { order: 2 }
         b { order: 1 }",
    );
    let store = fixture.box_tree();
    assert_invariants(&store);
    let root = store.root().expect("a root");
    let node = store.node(root);
    assert_eq!(node.children.len(), 2);
    assert_eq!(node.paint_children.len(), 2);
    assert_ne!(
        node.children, node.paint_children,
        "`order` moved nothing, so the fixture tests nothing"
    );
    assert_eq!(node.children[0], node.paint_children[1]);
}

#[test]
fn empty_and_whitespace_text_nodes_generate_no_flex_or_grid_items() {
    use support::{lay_out, measurer};
    use zgui_layout::boxtree::patch::{Retext, retext};

    let mut fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new("icon"),
            Element::new("icon"),
            Element::new("icon"),
            Element::new("icon"),
            Element::new("icon"),
        ]),
        "root { display: flex; flex-direction: row; gap: 8px; width: 232px; height: 40px }
         icon { display: block; width: 40px; height: 40px; flex-shrink: 0 }",
    );
    let root_dom = fixture.root;
    // Insert two empty anchor text nodes (as Solid <For each={[]}> / <Show when={false}> do)
    // and one collapsible whitespace-only text node directly inside the flex row.
    fixture.edit_and_restyle(|edit| {
        let a = edit.create_text("");
        let b = edit.create_text("");
        let ws = edit.create_text("  \n\t ");
        edit.insert_before(root_dom, a, None);
        edit.insert_before(root_dom, b, None);
        edit.insert_before(root_dom, ws, None);
    });

    let mut store = fixture.box_tree();
    assert_invariants(&store);
    let root = store.root().expect("a root");
    assert_eq!(
        store.node(root).children.len(),
        5,
        "empty and whitespace-only text nodes must not become flex items or eat gap",
    );
    let mut content = measurer();
    lay_out(&mut store, &mut content, 232.0, 40.0);
    let last = *store.node(root).children.last().expect("fifth icon");
    let last_layout = store.layout_of(last).expect("laid out");
    assert_eq!(
        last_layout.origin.x.0, 192.0,
        "5 * 40px + 4 * 8px = 232px with zero extra gaps from empty text anchors",
    );

    // Emptying a non-empty direct flex text item via retext forces a rebuild so its box disappears.
    let mut dynamic = Fixture::new(
        Element::new("root").children(vec![Element::new("icon")]).text("live"),
        "root { display: flex; gap: 8px; width: 200px }
         icon { display: block; width: 40px; height: 40px }",
    );
    let mut dynamic_store = dynamic.box_tree();
    let dyn_root = dynamic_store.root().expect("a root");
    assert_eq!(dynamic_store.node(dyn_root).children.len(), 2);
    let text_node = {
        let store_ref = dynamic.document.store();
        let mut cursor = store_ref.core(dynamic.root).first_child();
        let mut found = None;
        while let Some(child) = cursor {
            if store_ref.core(child).kind() == zgui_dom::NodeKind::Text {
                found = Some(child);
                break;
            }
            cursor = store_ref.core(child).next_sibling();
        }
        found.expect("text node")
    };
    dynamic.edit_and_restyle(|edit| edit.set_text(text_node, ""));
    let root_index = dynamic.document.root_index().expect("root");
    assert_eq!(
        retext(&mut dynamic_store, &dynamic.document, root_index),
        Retext::Rebuild,
        "emptying a direct flex text item must rebuild to drop the anonymous item",
    );
    let rebuilt = dynamic.box_tree();
    assert_eq!(
        rebuilt.node(rebuilt.root().expect("root")).children.len(),
        1,
    );
}

#[test]
fn empty_flex_field_and_editor_preserve_their_text_run_for_caret_placement() {
    use support::{lay_out, measurer};
    use zgui_layout::boxtree::patch::{Retext, retext};

    for tag in ["field", "editor"] {
        let mut fixture = Fixture::new(
            Element::new("root").children(vec![Element::new(tag)]),
            "root { display: block; width: 240px; height: 48px }
             field, editor { display: flex; align-items: center; width: 200px; height: 36px }",
        );
        let control = {
            let store_ref = fixture.document.store();
            store_ref.core(fixture.root).first_child().expect("control element")
        };
        let text_node = {
            let mut created = None;
            fixture.edit_and_restyle(|edit| {
                let text = edit.create_text("");
                edit.insert_before(control, text, None);
                created = Some(text);
            });
            created.expect("empty text child")
        };

        let mut store = fixture.box_tree();
        assert_invariants(&store);
        let root = store.root().expect("a root");
        let control_box = store.node(root).children[0];
        assert_eq!(
            store.node(control_box).children.len(),
            1,
            "empty flex <{tag}> must keep its anonymous text item so caret placement finds an IFC",
        );

        let mut content = measurer();
        lay_out(&mut store, &mut content, 240.0, 48.0);
        let text_box = store.node(control_box).children[0];
        assert!(
            store.inline_resolution(text_box).is_some(),
            "empty flex <{tag}> text box must resolve an inline formatting context",
        );

        // Typing and then clearing back to empty must stay in place without dropping the text box.
        fixture.edit_and_restyle(|edit| edit.set_text(text_node, "hi"));
        let root_index = fixture.document.root_index().expect("root");
        assert!(matches!(retext(&mut store, &fixture.document, root_index), Retext::Patched(_)));
        fixture.edit_and_restyle(|edit| edit.set_text(text_node, ""));
        assert!(
            matches!(retext(&mut store, &fixture.document, root_index), Retext::Patched(_)),
            "clearing a flex <{tag}> back to empty must keep its TextRun box in place",
        );
        assert_eq!(store.node(control_box).children.len(), 1);
    }
}
