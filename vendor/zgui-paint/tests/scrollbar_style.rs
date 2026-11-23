//! What a scroll container's style says about its bars: the scheme and the colours.

mod support;

use support::{Element, Harness};
use zgui_paint::lower::scrollbar::ScrollbarScheme;

/// The lowered paint style of a named element.
fn style_of(harness: &Harness, name: &str) -> zgui_paint::lower::PaintStyle {
    let key = harness.box_of(name);
    zgui_paint::lower::lower(&harness.store().node(key).style, 1.0)
}

/// A root holding a scroller and a plain box.
fn tree() -> Element {
    Element::new("root").children(vec![Element::new("column"), Element::new("mark")])
}

/// The sheet every case shares, with `extra` after it.
fn sheet(extra: &str) -> String {
    format!(
        "root {{ display: block; width: 200px; height: 200px }}
         column {{ display: block; height: 50px; overflow-y: auto }}
         mark {{ display: block; height: 20px }}
         {extra}"
    )
}

#[test]
fn an_inherited_dark_scheme_reaches_a_scroller_and_no_other_box() {
    let harness = Harness::new(tree(), &sheet("root { color-scheme: dark }"));
    assert_eq!(
        style_of(&harness, "column").scrollbar.scheme,
        ScrollbarScheme::Dark
    );
    assert_eq!(
        style_of(&harness, "mark").scrollbar,
        zgui_paint::lower::ScrollbarStyle::default(),
        "a box that does not scroll reads nothing for its bars"
    );
}

#[test]
fn a_scheme_naming_both_or_none_follows_the_window() {
    let harness = Harness::new(tree(), &sheet("root { color-scheme: light dark }"));
    assert_eq!(
        style_of(&harness, "column").scrollbar.scheme,
        ScrollbarScheme::Window
    );
    let harness = Harness::new(tree(), &sheet(""));
    let style = style_of(&harness, "column").scrollbar;
    assert_eq!(style.scheme, ScrollbarScheme::Window);
    assert!(style.colors.is_none(), "nothing set, nothing allocated");
}

#[test]
fn the_custom_properties_set_the_thumb_and_the_track() {
    let harness = Harness::new(
        tree(),
        &sheet(
            ":root { --zgui-scrollbar-thumb: rgb(0, 255, 0) }
             column { --zgui-scrollbar-track: rgb(0, 0, 255) }",
        ),
    );
    let colors = style_of(&harness, "column")
        .scrollbar
        .colors
        .expect("the colours were read");
    assert_eq!(
        colors.thumb.map(|c| c.to_premultiplied_srgb()),
        Some([0.0, 1.0, 0.0, 1.0])
    );
    assert_eq!(
        colors.track.map(|c| c.to_premultiplied_srgb()),
        Some([0.0, 0.0, 1.0, 1.0])
    );
    assert!(style_of(&harness, "mark").scrollbar.colors.is_none());
}

#[test]
fn overlay_scrollbar_paints_after_scrolled_children_and_respects_sideband_visual() {
    use zgui_dom::NodeKey;
    use zgui_layout::axis::Axis;
    use zgui_layout::scroll_region::ScrollbarMode;
    use zgui_paint::{OverlayBarVisual, OverlayScrollbarSource};
    use zgui_scene::Paint;

    struct FixedVisual(OverlayBarVisual);
    impl OverlayScrollbarSource for FixedVisual {
        fn visual(&self, _container: NodeKey, _axis: Axis) -> Option<OverlayBarVisual> {
            Some(self.0)
        }
    }

    let mut harness = Harness::new(
        Element::new("root").children(vec![Element::new("scroller").children(vec![
            Element::new("row"),
        ])]),
        "root { --zgui-scrollbar-mode: overlay; display: block; width: 200px; height: 200px }
         scroller {
             display: block;
             width: 200px;
             height: 100px;
             overflow-y: auto;
             background: rgb(255, 255, 255);
             --zgui-scrollbar-thumb: rgb(255, 0, 0);
         }
         row { display: block; width: 200px; height: 300px; background: rgb(0, 0, 255) }",
    );
    assert_eq!(
        style_of(&harness, "scroller").scrollbar.mode,
        ScrollbarMode::Overlay
    );

    // 1. Resting overlay bar: container bg -> child row -> red 4px thumb capsule strictly LAST.
    harness.paint_with_overlay_scrollbars(&FixedVisual(OverlayBarVisual::RESTING));
    {
        let scene = harness.scene();
        assert_eq!(
            scene.primitives.quads.len(),
            3,
            "expected container bg, child row, and overlay thumb (no re-painted container bg on scrollbar strip)",
        );
        let child_quad = &scene.primitives.quads[1];
        let thumb_quad = &scene.primitives.quads[2];
        assert_eq!(support::unpoisoned(child_quad.bounds), [0.0, 0.0, 200.0, 300.0]);
        assert_eq!(support::unpoisoned(thumb_quad.bounds)[0], 192.0);
        assert_eq!(thumb_quad.bounds[2], 4.0);
        let Some(Paint::Solid(thumb_color)) =
            scene.paints.get(thumb_quad.fill.id().expect("solid"))
        else {
            panic!("expected solid thumb paint");
        };
        assert_eq!(thumb_color.to_premultiplied_srgb(), [1.0, 0.0, 0.0, 1.0]);
    }

    // 2. Hidden overlay bar (`alpha == 0.0`): emits ONLY container bg + child row (0 scrollbar quads),
    // while replaying the container and child row from cache on the same scene.
    harness.paint_with_overlay_scrollbars(&FixedVisual(OverlayBarVisual::HIDDEN));
    assert_eq!(
        harness.scene().primitives.quads.len(),
        2,
        "hidden overlay scrollbar must emit zero quads",
    );
}
