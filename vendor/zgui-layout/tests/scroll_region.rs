//! Scroll regions: the `overflow: auto` fixpoint and the gutter a locked container keeps.

mod support;

use std::path::PathBuf;

use support::{Element, Fixture, Frame, lay_out, measurer, relayout};
use zgui_layout::LayoutStore;
use zgui_layout::scroll_region::gutter;
use zgui_testkit_scene::dump::{TreeDump, golden};
use zgui_testkit_scene::text::Writer;

/// A rendering of a layout, as the shared tree-dump seam wants it.
struct Rendering(String);

impl TreeDump for Rendering {
    fn dump(&self, writer: &mut Writer) {
        for line in self.0.lines() {
            writer.line(line);
        }
    }
}

/// Where the goldens live.
fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(format!("{name}.txt"))
}

/// Every fragment's rectangle, in painting order, as text.
fn fragment_rects(store: &LayoutStore, frame: &Frame) -> String {
    let placements = zgui_scene::Placements::of(&frame.spatial);
    let mut out = String::new();
    let root = store.root().expect("a root");
    for key in zgui_layout::fragment::stacking::paint_order(store, root) {
        for frag in store.fragments_of_box(key) {
            let fragment = store.fragment(*frag).expect("live");
            let border = fragment.on_device(fragment.border_box, &placements);
            out.push_str(&format!(
                "{:?} at=({}, {}) size=({} x {})\n",
                fragment.kind,
                border.origin.x.0,
                border.origin.y.0,
                border.size.width.0,
                border.size.height.0,
            ));
        }
    }
    out
}

/// A page whose content overflows the viewport, so its root really does scroll.
fn page(overflow: &str) -> Fixture {
    let rows: Vec<Element> = (0..30)
        .map(|_| Element::new("row").text("alpha bravo delta gamma kappa sigma omega"))
        .collect();
    Fixture::new(
        Element::new("root").children(rows),
        &format!(
            "root {{ display: block; height: 300px; overflow: {overflow} }}
             row {{ display: block }}"
        ),
    )
}

#[test]
fn an_auto_container_reserves_a_gutter_exactly_when_its_content_overflows() {
    // `auto` has no answer until the box has been laid out, so it enters layout reserving nothing
    // and is revised. A container whose content fits must not reserve anything, or every panel in
    // a document loses a scrollbar's width for nothing.
    let fits = Fixture::new(
        Element::new("root").children(vec![Element::new("row")]),
        "root { display: block; height: 300px; overflow: auto }
         row { display: block; height: 20px }",
    );
    let mut store = fits.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 300.0);
    let root = store.root().expect("a root");
    assert_eq!(store.auto_scroll(root), (false, false));
    assert_eq!(
        store
            .layout_of(root)
            .expect("laid out")
            .scrollbar_size
            .width
            .0,
        0.0
    );

    let overflows = page("auto");
    let mut store = overflows.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 300.0);
    let root = store.root().expect("a root");
    assert_eq!(
        store.auto_scroll(root),
        (false, true),
        "the block axis overflowed and only that one"
    );
    assert!(
        store
            .layout_of(root)
            .expect("laid out")
            .scrollbar_size
            .width
            .0
            > 0.0,
        "a vertical scrollbar takes width off the content"
    );
}

#[test]
fn scroll_lock_gutter() {
    // A modal stops the page behind it scrolling. Doing that with `overflow: hidden` alone takes
    // the scrollbar's width back, every line re-wraps wider, and the page jumps sideways behind
    // the modal. Locking the container keeps the gutter, so the fragment rectangles are the same
    // bytes with and without the lock — which is the whole claim, and it is checked as bytes.
    let scrolling = page("auto");
    let mut store = scrolling.box_tree();
    let mut content = measurer();
    let mut frame = lay_out(&mut store, &mut content, 400.0, 300.0);
    let before = fragment_rects(&store, &frame);
    golden::assert_tree(
        &golden_path("scroll-lock-gutter"),
        &Rendering(before.clone()),
    );

    // The same document with the page locked and the style changed to stop it scrolling. The
    // gutter the page was reserving is what the lock holds, and it is read from the layout that
    // reserved it rather than from the one about to be computed.
    let root = store.root().expect("a root");
    let axes = gutter::reserved(&store, root);
    assert_eq!(axes, (false, true), "the page was scrolling vertically");

    let locked = page("hidden");
    let mut locked_store = locked.box_tree();
    let locked_root = locked_store.root().expect("a root");
    gutter::lock_axes(&mut locked_store, locked_root, axes);
    assert!(gutter::is_locked(&locked_store, locked_root));
    let mut content = measurer();
    relayout(&mut frame, &mut locked_store, &mut content, 400.0, 300.0);

    assert_eq!(
        fragment_rects(&locked_store, &frame),
        before,
        "locking the page must not move a single fragment"
    );
}

#[test]
fn unlocking_gives_the_gutter_back() {
    let scrolling = page("auto");
    let mut store = scrolling.box_tree();
    let mut content = measurer();
    let frame = lay_out(&mut store, &mut content, 400.0, 300.0);
    let root = store.root().expect("a root");
    let scrolling_width = fragment_rects(&store, &frame);

    gutter::lock(&mut store, root);
    gutter::unlock(&mut store, root);
    assert!(!gutter::is_locked(&store, root));
    let frame = lay_out(&mut store, &mut content, 400.0, 300.0);
    assert_eq!(fragment_rects(&store, &frame), scrolling_width);
}

#[test]
fn a_padded_bordered_scroller_puts_its_bar_against_the_border() {
    let rows: Vec<Element> = (0..30)
        .map(|_| Element::new("row").text("alpha bravo delta"))
        .collect();
    let fixture = Fixture::new(
        Element::new("root").children(vec![Element::new("column").children(rows)]),
        "root { display: block }
         column { display: block; height: 200px; overflow-y: auto; padding: 0 16px;
                  border: 1px solid red }
         row { display: block }",
    );
    let mut store = fixture.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 300.0);

    let column = fixture
        .document
        .store()
        .core(fixture.root)
        .first_child()
        .expect("the scroller");
    let list = store.boxes_of(fixture.document.store().key_of(column))[0];
    let frags = store.fragments_of_box(list).to_vec();
    let fragment = |kind| {
        frags
            .iter()
            .filter_map(|frag| store.fragment(*frag))
            .find(|fragment| fragment.kind == kind)
            .cloned()
            .expect("the fragment")
    };
    let own = fragment(zgui_layout::FragmentKind::Box);
    let part = |part| {
        fragment(zgui_layout::FragmentKind::Scrollbar {
            axis: zgui_layout::Axis::Vertical,
            part,
        })
        .border_box
    };
    let track = part(zgui_layout::fragment::ScrollbarPart::Track);
    let thumb = part(zgui_layout::fragment::ScrollbarPart::Thumb);

    assert_eq!(
        track.right().0,
        own.border_box.right().0 - 1.0,
        "the track touches the border"
    );
    assert_eq!(track.top(), own.padding_box().top());
    assert_eq!(track.bottom(), own.padding_box().bottom());
    assert!(
        (track.left().0 - (own.content_box().right().0 + 16.0)).abs() < 1e-3,
        "the right padding stays between the content and the bar"
    );
    assert_eq!(thumb.left(), track.left());
    assert_eq!(thumb.top(), track.top());
}

#[test]
fn a_box_is_found_in_its_scrollers_content_space_from_layout_alone() {
    let rows: Vec<Element> = (0..30)
        .map(|_| Element::new("row").classes(&["r"]))
        .collect();
    let fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new("column").children(vec![Element::new("box").children(rows)]),
        ]),
        "root { display: block }
         column { display: block; height: 200px; overflow-y: auto; padding: 10px;
                  border: 2px solid red }
         box { display: block; margin-top: 7px }
         row { display: block; height: 20px }",
    );
    let mut store = fixture.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 300.0);

    let document = fixture.document.store();
    let column = document
        .core(fixture.root)
        .first_child()
        .expect("the scroller");
    let inner = document.core(column).first_child().expect("the wrapper");
    let mut row = document.core(inner).first_child().expect("a row");
    for _ in 0..5 {
        row = document.core(row).next_sibling().expect("a later row");
    }
    let container = store.boxes_of(document.key_of(column))[0];
    let target = store.boxes_of(document.key_of(row))[0];

    let (rect, port) = zgui_layout::scroll_region::into_view::target_in(&store, container, target)
        .expect("the row is below the scroller");
    assert_eq!(
        rect.origin.y.0,
        7.0 + 5.0 * 20.0,
        "the wrapper's margin and five rows"
    );
    assert_eq!(
        rect.origin.x.0, 0.0,
        "measured from the content box, inside the padding"
    );
    assert_eq!(rect.size.height.0, 20.0);
    assert_eq!(
        port.height.0,
        200.0 - 2.0 * 10.0 - 2.0 * 2.0,
        "the scrollport is the content box of a border-box sized scroller"
    );
    assert!(
        zgui_layout::scroll_region::into_view::target_in(&store, target, container).is_none(),
        "the scroller is not below the row"
    );
}

#[test]
fn subpixel_fractional_scale_rounding_does_not_trigger_auto_scrollbar() {
    let fitted = Fixture::new(
        Element::new("root").children(vec![Element::new("inner")]),
        "root { display: block; width: 460px; height: 60px; overflow: auto }
         inner { display: block; width: 460.25px; height: 60px }",
    );
    let mut store = fitted.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 460.0, 60.0);
    let root = store.root().expect("a root");
    assert_eq!(
        store.auto_scroll(root),
        (false, false),
        "a 0.25px sub-pixel rounding discrepancy must not reserve a spurious scrollbar gutter",
    );

    let genuinely_wider = Fixture::new(
        Element::new("root").children(vec![Element::new("inner")]),
        "root { display: block; width: 460px; height: 60px; overflow: auto }
         inner { display: block; width: 462px; height: 60px }",
    );
    let mut store = genuinely_wider.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 460.0, 60.0);
    let root = store.root().expect("a root");
    assert_eq!(
        store.auto_scroll(root),
        (true, false),
        "a genuine multi-pixel overflow must still trigger overflow: auto",
    );
}

#[test]
fn constrained_replaced_element_does_not_leak_natural_size_into_scrollable_overflow() {
    use zgui_layout::NaturalSize;
    use zgui_layout::node::kind::FormattingContext;
    use zgui_layout::scroll_region::region_of;
    use zgui_layout::style::DeviceStyle;
    use zgui_layout::tree::LayoutTree;

    // Live repro: a 1678x1500 avatar rendered inside a 36x36 <image> inside a 523px column
    // inside a 360x620 scroll container at 1.75x scale must not leak its natural size into
    // scrollable overflow or trigger a phantom scrollbar.
    let fixture = Fixture::with_natural_size(
        Element::new("root").children(vec![Element::new("col").children(vec![
            Element::new("row").children(vec![Element::new("avatar").image(1678.0, 1500.0)]),
            Element::new("body"),
        ])]),
        "root { display: block; width: 360px; height: 620px; overflow: auto }
         col { display: flex; flex-direction: column }
         row { display: flex; flex-direction: row; align-items: center }
         avatar { display: inline-block; width: 36px; height: 36px; flex-shrink: 0 }
         body { display: block; height: 480px }",
        (1678.0, 1500.0),
    );
    let mut store = fixture.box_tree();
    assert!(
        store
            .keys()
            .into_iter()
            .any(|key| store.node(key).fc == FormattingContext::Replaced),
        "avatar must be laid out as a replaced box",
    );
    let mut content = NaturalSize;
    let scale = 1.75;
    let device = DeviceStyle {
        scale,
        ..DeviceStyle::default()
    };
    assert!(
        LayoutTree::new(&mut store, &mut content, device)
            .layout_viewport(360.0 * scale, 620.0 * scale)
    );
    let root = store.root().expect("a root");
    let region = region_of(&store, root).expect("a scroll region");
    assert_eq!(
        store.auto_scroll(root),
        (false, false),
        "a 36x36 replaced element must not trigger a phantom scrollbar from its 1678x1500 natural size",
    );
    assert!(
        region.content.height.0 <= 520.0 * scale,
        "scroll content height ({}) must reflect laid-out content (~516px * {scale}), not natural image height",
        region.content.height.0,
    );
    assert!(
        region.content.width.0 <= 360.0 * scale,
        "scroll content width ({}) must not leak natural image width",
        region.content.width.0,
    );

    // Aspect-ratio constrained replaced element (max-width: 100%; height: auto) must also bound
    // scrollable overflow to its constrained box.
    let aspect = Fixture::with_natural_size(
        Element::new("root").children(vec![Element::new("photo").image(1678.0, 1500.0)]),
        "root { display: block; width: 320px; height: 400px; overflow: auto }
         photo { display: block; max-width: 100%; height: auto }",
        (1678.0, 1500.0),
    );
    let mut store = aspect.box_tree();
    let mut content = NaturalSize;
    assert!(
        LayoutTree::new(&mut store, &mut content, DeviceStyle::default())
            .layout_viewport(320.0, 400.0)
    );
    let root = store.root().expect("a root");
    assert_eq!(
        store.auto_scroll(root),
        (false, false),
        "max-width: 100% replaced image fitting in 320x400 must not trigger scrollbars",
    );

    // Unconstrained replaced element (width: auto; height: auto) must still size to its intrinsic
    // natural dimensions and trigger overflow: auto when larger than the scrollport.
    let natural = Fixture::with_natural_size(
        Element::new("root").children(vec![Element::new("photo").image(1678.0, 1500.0)]),
        "root { display: block; width: 360px; height: 620px; overflow: auto }
         photo { display: block }",
        (1678.0, 1500.0),
    );
    let mut store = natural.box_tree();
    let mut content = NaturalSize;
    assert!(
        LayoutTree::new(&mut store, &mut content, DeviceStyle::default())
            .layout_viewport(360.0, 620.0)
    );
    let root = store.root().expect("a root");
    let region = region_of(&store, root).expect("a scroll region");
    assert_eq!(
        store.auto_scroll(root),
        (true, true),
        "an unconstrained 1678x1500 replaced image must still size to its intrinsic dimensions and scroll",
    );
    assert!((region.content.width.0 - 1678.0).abs() < 1.0);
    assert!((region.content.height.0 - 1500.0).abs() < 1.0);
}

#[test]
fn overlay_scrollbar_reserves_zero_gutter_and_inherits_from_root() {
    use zgui_layout::scroll_region::ScrollbarMode;

    // Non-overflowing vs overflowing overlay containers keep the exact same content width (zero
    // gutter), whereas a classic descendant override reserves the classic gutter.
    let fits = Fixture::new(
        Element::new("root").children(vec![Element::new("scroller").children(vec![
            Element::new("child"),
        ])]),
        "root { --zgui-scrollbar-mode: overlay; display: block; width: 320px; height: 240px }
         scroller { display: block; width: 300px; height: 120px; overflow: auto }
         child { display: block; height: 40px }",
    );
    let mut fits_store = fits.box_tree();
    let mut content = measurer();
    lay_out(&mut fits_store, &mut content, 320.0, 240.0);
    let fits_root = fits_store.root().expect("root");
    let fits_scroller = fits_store.node(fits_root).children[0];
    let fits_child = fits_store.node(fits_scroller).children[0];
    assert_eq!(fits_store.scrollbar_mode(fits_scroller), ScrollbarMode::Overlay);
    let fits_width = fits_store
        .layout_of(fits_child)
        .expect("child layout")
        .size
        .width
        .0;

    let overflows = Fixture::new(
        Element::new("root").children(vec![
            Element::new("scroller").children(vec![Element::new("child")]),
            Element::new("classic").children(vec![Element::new("classic_child")]),
            Element::new("invalid").children(vec![Element::new("invalid_child")]),
        ]),
        "root { --zgui-scrollbar-mode: overlay; display: block; width: 320px; height: 600px }
         scroller { display: block; width: 300px; height: 120px; overflow: auto }
         child { display: block; height: 400px }
         classic { --zgui-scrollbar-mode: classic; display: block; width: 300px; height: 120px; overflow: auto }
         classic_child { display: block; height: 400px }
         invalid { --zgui-scrollbar-mode: bogus; display: block; width: 300px; height: 120px; overflow: auto }
         invalid_child { display: block; height: 400px }",
    );
    let mut store = overflows.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 320.0, 600.0);
    let root = store.root().expect("root");
    let scroller = store.node(root).children[0];
    let child = store.node(scroller).children[0];
    let classic = store.node(root).children[1];
    let classic_child = store.node(classic).children[0];
    let invalid = store.node(root).children[2];

    assert_eq!(store.scrollbar_mode(scroller), ScrollbarMode::Overlay);
    assert_eq!(store.scrollbar_mode(classic), ScrollbarMode::Classic);
    assert_eq!(store.scrollbar_mode(invalid), ScrollbarMode::Classic);

    let scroller_layout = store.layout_of(scroller).expect("scroller layout");
    assert_eq!(scroller_layout.scrollbar_size.width.0, 0.0);
    assert_eq!(scroller_layout.scrollbar_size.height.0, 0.0);
    assert_eq!(
        store.layout_of(child).expect("child layout").size.width.0,
        fits_width,
        "overlay scrollbar must not narrow content width when overflow turns on",
    );
    assert!(
        store
            .layout_of(classic_child)
            .expect("classic child layout")
            .size
            .width
            .0
            < fits_width,
        "local --zgui-scrollbar-mode: classic override must still reserve a gutter",
    );
}

#[test]
fn overlay_scrollbar_fragments_emit_only_on_overflowing_axes_and_avoid_corner() {
    use zgui_layout::axis::Axis;
    use zgui_layout::fragment::{FragmentKind, ScrollbarPart};
    use zgui_layout::scroll_region::bar::live;

    // Non-overflowing `overflow: scroll` in overlay mode must emit zero scrollbar fragments.
    let fits = Fixture::new(
        Element::new("root").children(vec![Element::new("child")]),
        "root { --zgui-scrollbar-mode: overlay; display: block; width: 200px; height: 200px; overflow: scroll }
         child { display: block; width: 100px; height: 100px }",
    );
    let mut fits_store = fits.box_tree();
    let mut content = measurer();
    lay_out(&mut fits_store, &mut content, 200.0, 200.0);
    let fits_root = fits_store.root().expect("root");
    let fits_bars: Vec<_> = fits_store
        .fragments_of_box(fits_root)
        .iter()
        .filter_map(|key| fits_store.fragment(*key))
        .filter(|frag| matches!(frag.kind, FragmentKind::Scrollbar { .. }))
        .collect();
    assert!(
        fits_bars.is_empty(),
        "non-overflowing overlay container must emit no scrollbar fragments even with overflow: scroll",
    );

    // Dual-axis overflowing overlay container emits both axes with 12px logical strips, avoiding
    // the 12x12 bottom-right corner, and `live::travel_of` / `live::thumb_of` match the fragments.
    let dual = Fixture::new(
        Element::new("root").children(vec![Element::new("child")]),
        "root { --zgui-scrollbar-mode: overlay; display: block; width: 200px; height: 160px; overflow: auto }
         child { display: block; width: 500px; height: 480px }",
    );
    let mut store = dual.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 200.0, 160.0);
    let root = store.root().expect("root");

    let mut v_track = None;
    let mut v_thumb = None;
    let mut h_track = None;
    let mut h_thumb = None;
    for key in store.fragments_of_box(root) {
        let frag = store.fragment(*key).expect("live");
        match frag.kind {
            FragmentKind::Scrollbar {
                axis: Axis::Vertical,
                part: ScrollbarPart::Track,
            } => v_track = Some(frag.border_box),
            FragmentKind::Scrollbar {
                axis: Axis::Vertical,
                part: ScrollbarPart::Thumb,
            } => v_thumb = Some(frag.border_box),
            FragmentKind::Scrollbar {
                axis: Axis::Horizontal,
                part: ScrollbarPart::Track,
            } => h_track = Some(frag.border_box),
            FragmentKind::Scrollbar {
                axis: Axis::Horizontal,
                part: ScrollbarPart::Thumb,
            } => h_thumb = Some(frag.border_box),
            _ => {}
        }
    }
    let v_track = v_track.expect("vertical overlay track");
    let v_thumb = v_thumb.expect("vertical overlay thumb");
    let h_track = h_track.expect("horizontal overlay track");
    let h_thumb = h_thumb.expect("horizontal overlay thumb");

    assert!((v_track.size.width.0 - 12.0).abs() < 0.01);
    assert!((v_track.origin.x.0 - 188.0).abs() < 0.01);
    assert!((v_track.size.height.0 - 148.0).abs() < 0.01);

    assert!((h_track.size.height.0 - 12.0).abs() < 0.01);
    assert!((h_track.origin.y.0 - 148.0).abs() < 0.01);
    assert!((h_track.size.width.0 - 188.0).abs() < 0.01);

    // Live travel matches the emitted fragments.
    let root_node = store.node(root).source.expect("root node");
    let v_travel = live::travel_of(&store, root_node, Axis::Vertical).expect("vertical travel");
    let h_travel = live::travel_of(&store, root_node, Axis::Horizontal).expect("horizontal travel");
    assert!((v_travel.start - v_track.origin.y.0).abs() < 0.01);
    assert!((v_travel.length - v_track.size.height.0).abs() < 0.01);
    assert!((v_travel.thumb - v_thumb.size.height.0).abs() < 0.01);
    assert_eq!(live::thumb_of(&store, root, Axis::Vertical), Some(v_thumb));
    assert!((h_travel.start - h_track.origin.x.0).abs() < 0.01);
    assert!((h_travel.length - h_track.size.width.0).abs() < 0.01);
    assert!((h_travel.thumb - h_thumb.size.width.0).abs() < 0.01);
    assert_eq!(live::thumb_of(&store, root, Axis::Horizontal), Some(h_thumb));
}

#[test]
fn scrollbar_hit_order_sorts_strictly_after_children_on_rebuild_and_incremental_diff() {
    use zgui_geom::{Device, DevicePx, Point};
    use zgui_layout::fragment::FragmentKind;
    use zgui_layout::fragment::diff::Everything;

    let mut fixture = Fixture::new(
        Element::new("root").children(vec![Element::new("child")]),
        "root { --zgui-scrollbar-mode: overlay; display: block; width: 200px; height: 100px; overflow: auto }
         child { display: block; width: 200px; height: 60px }
         .tall { height: 300px }",
    );
    let mut store = fixture.box_tree();
    let mut content = measurer();
    let mut frame = lay_out(&mut store, &mut content, 200.0, 100.0);
    let root = store.root().expect("root");
    assert!(
        store
            .fragments_of_box(root)
            .iter()
            .filter_map(|key| store.fragment(*key))
            .all(|frag| !matches!(frag.kind, FragmentKind::Scrollbar { .. })),
        "initially non-overflowing"
    );

    // Toggle child to overflow and run an incremental fragment diff without forcing a full HitIndex
    // rebuild, verifying the newborn overlay scrollbar sorts strictly above the child.
    let child_dom = fixture
        .document
        .store()
        .core(fixture.root)
        .first_child()
        .expect("child");
    fixture.edit_and_restyle(|edit| {
        edit.add_class(child_dom, zgui_interned::ClassName::new("tall"));
    });
    let mut store = fixture.box_tree();
    let mut content = measurer();
    {
        let mut tree = zgui_layout::tree::LayoutTree::new(
            &mut store,
            &mut content,
            zgui_layout::DeviceStyle::default(),
        );
        assert!(tree.layout_root(cephal::Size {
            width: 200.0,
            height: 100.0,
        }));
    }
    let root = store.root().expect("root");
    support::fragments(&mut frame, &mut store, root, &mut Everything);

    let edge = Point::<DevicePx, Device>::new(DevicePx(194.0), DevicePx(20.0));
    let hits = frame.hit.hit(edge, &frame.clips, &frame.spatial);
    assert!(!hits.is_empty(), "edge point must hit scrollbar and child");
    let topmost = store.fragment(hits[0]).expect("topmost fragment");
    assert!(
        matches!(topmost.kind, FragmentKind::Scrollbar { .. }),
        "topmost hit at the edge after incremental diff must be a Scrollbar fragment, got {:?}",
        topmost.kind,
    );

    // A full rebuild must produce the same topmost-scrollbar ordering.
    frame.hit.rebuild(&mut store, 1.0);
    let rebuilt_hits = frame.hit.hit(edge, &frame.clips, &frame.spatial);
    let rebuilt_topmost = store.fragment(rebuilt_hits[0]).expect("rebuilt topmost");
    assert!(
        matches!(rebuilt_topmost.kind, FragmentKind::Scrollbar { .. }),
        "topmost hit at the edge after full HitIndex::rebuild must be a Scrollbar fragment, got {:?}",
        rebuilt_topmost.kind,
    );
}
