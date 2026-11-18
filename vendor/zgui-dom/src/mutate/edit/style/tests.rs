//! What an element's own declarations do to the block and to the engine's hint.

use style::invalidation::element::restyle_hints::RestyleHint;
use zgui_interned::{CustomPropertyName, ElementName};

use crate::arena::document::Document;
use crate::id::node_key::NodeIndex;
use crate::mutate::filter::EverythingMatters;
use crate::node::kind::NodeKind;

use super::InlineStyle;
/// A document with one element, styled once so a hint has somewhere to land.
fn one() -> (Document, NodeIndex) {
    let mut document = Document::new();
    let root = document.append(
        document.document_index(),
        NodeKind::Element,
        ElementName::new("root"),
    );
    document.node(root).establish_style_data();
    (document, root)
}

/// How many declarations `node` carries of its own.
fn declarations(document: &Document, node: NodeIndex) -> usize {
    let key = document.store().key_of(node);
    let guard = document.store().lock().read();
    document
        .store()
        .columns()
        .inline_style
        .get(key)
        .and_then(Option::as_ref)
        .map_or(0, |block| block.read_with(&guard).len())
}

/// The hint the element is carrying.
fn hint(document: &Document, node: NodeIndex) -> RestyleHint {
    document
        .node(node)
        .borrow_style_data()
        .expect("the element has data")
        .hint
}

#[test]
fn inline_style_text_replaces_the_whole_block_and_asks_for_a_cascade_only() {
    let (document, root) = one();
    document
        .edit(&EverythingMatters, |edit| {
            edit.set_inline_style(root, Some("color: red; display: flex"));
        })
        .expect("not poisoned");
    assert_eq!(declarations(&document, root), 2);
    assert!(hint(&document, root).contains(RestyleHint::RESTYLE_STYLE_ATTRIBUTE));
    assert!(!hint(&document, root).contains(RestyleHint::RESTYLE_SELF));

    document
        .edit(&EverythingMatters, |edit| {
            edit.set_inline_style(root, None);
        })
        .expect("not poisoned");
    assert_eq!(declarations(&document, root), 0);
}

#[test]
fn one_property_is_replaced_without_disturbing_the_others() {
    let (document, root) = one();
    document
        .edit(&EverythingMatters, |edit| {
            assert!(edit.set_style_property(root, "color", Some("red")));
            assert!(edit.set_style_property(root, "display", Some("flex")));
            assert!(edit.set_style_property(root, "color", Some("blue")));
        })
        .expect("not poisoned");
    assert_eq!(declarations(&document, root), 2);

    document
        .edit(&EverythingMatters, |edit| {
            assert!(edit.set_style_property(root, "color", None));
        })
        .expect("not poisoned");
    assert_eq!(declarations(&document, root), 1);
}

/// The whole point of the return value: `vector { fill: red }` has to be reportable, and a
/// property gated to another engine is not a property this build has.
#[test]
fn a_property_this_build_does_not_have_is_refused_rather_than_dropped_silently() {
    let (document, root) = one();
    document
        .edit(&EverythingMatters, |edit| {
            assert!(!edit.set_style_property(root, "fill", Some("red")));
            assert!(!edit.set_style_property(root, "stroke-width", Some("2px")));
            assert!(!edit.set_style_property(root, "colour", Some("red")));
            assert!(!edit.set_style_property(root, "color", Some("not-a-colour")));
        })
        .expect("not poisoned");
    assert_eq!(declarations(&document, root), 0);
}

#[test]
fn a_custom_property_re_cascades_the_element_and_tells_nothing_below() {
    let (document, root) = one();
    document
        .edit(&EverythingMatters, |edit| {
            assert!(edit.set_custom_property(
                root,
                CustomPropertyName::new("zgui-fill"),
                Some("red")
            ));
        })
        .expect("not poisoned");
    assert_eq!(declarations(&document, root), 1);
    assert!(hint(&document, root).contains(RestyleHint::RESTYLE_STYLE_ATTRIBUTE));
    assert!(
        !hint(&document, root).contains(RestyleHint::RECASCADE_DESCENDANTS),
        "the traversal hands the new map down by name; a blanket recascade is what it replaces"
    );
}

#[test]
fn writing_the_same_declaration_twice_asks_the_engine_for_nothing_the_second_time() {
    let (document, root) = one();
    document
        .edit(&EverythingMatters, |edit| {
            edit.set_style_property(root, "color", None);
        })
        .expect("not poisoned");
    assert!(
        hint(&document, root).is_empty(),
        "removing a declaration that was never there is not a change"
    );
}

#[test]
fn inline_transaction_rejects_the_whole_map_before_committing() {
    let (document, root) = one();
    document.edit(&EverythingMatters, |edit| {
        edit.replace_inline_style(root, [InlineStyle::Text("height: 11px; --Accent: 7")]).unwrap();
    }).expect("not poisoned");
    for invalid in [
        "width: 30px; height: NaNpx",
        "color: blue; unknown-property: red",
        "color: blue; width: invalid",
        "background: url('not-supported.png')",
        "--dynamic: NaNpx",
        "--dynamic: Infinitypx",
        "--dynamic: 1e999px",
        "--image: image-set('not-supported.png' 1x)",
    ] {
        document.edit(&EverythingMatters, |edit| {
            assert!(edit.replace_inline_style(root, [InlineStyle::Text(invalid)]).is_err(), "{invalid}");
        }).expect("not poisoned");
        assert_eq!(declarations(&document, root), 2, "{invalid}");
    }
    document.edit(&EverythingMatters, |edit| {
        edit.replace_inline_style(root, [InlineStyle::Text("padding: 1px; padding-left: 8px")]).unwrap();
        edit.replace_inline_style(root, [InlineStyle::Text("padding-left: 8px")]).unwrap();
    }).expect("not poisoned");
    assert_eq!(declarations(&document, root), 1);
    document.edit(&EverythingMatters, |edit| {
        edit.replace_inline_style(root, []).unwrap();
    }).expect("not poisoned");
    assert_eq!(declarations(&document, root), 0);
}

#[test]
fn inline_transaction_isolates_values_and_ownership_blocks() {
    let (document, root) = one();
    document.edit(&EverythingMatters, |edit| {
        edit.replace_inline_style(root, [
            InlineStyle::Text("position: relative !important; /*"),
            InlineStyle::Declaration("width", "10px/*", false),
            InlineStyle::Declaration("height", "20px", false),
            InlineStyle::Declaration("position", "fixed", true),
        ]).unwrap();
    }).expect("not poisoned");
    assert_eq!(declarations(&document, root), 3);
    document.edit(&EverythingMatters, |edit| {
        assert!(edit.replace_inline_style(root, [
            InlineStyle::Declaration("width", "30px", false),
            InlineStyle::Declaration("height", "20px; color:red", false),
        ]).is_err());
    }).expect("not poisoned");
    assert_eq!(declarations(&document, root), 3);
}
