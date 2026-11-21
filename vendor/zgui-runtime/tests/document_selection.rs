//! Opt-in cross-block read-only `<text>` document selection (#68).
//!
//! Exercises reverse cross-block drag selection, block/table separators, soft-wrap preservation,
//! `user-select: none` and editable-field exclusions, `Ctrl+C` clipboard output, highlight
//! recomputation across container resize/rewrap, and safe clearing when an endpoint node detaches.

mod support;

use std::time::Duration;

use zgui_geom::{CssPx, DevicePx, Point, Size};
use zgui_platform::{ClipboardData, ClipboardFormat, ClipboardKind, PlatformCx, SurfaceEvent};
use zgui_reactive::RwSignal;
use zgui_reactive::prelude::*;
use zgui_view::{A11yBinding, AnyView, AttrName, BuildCx, ClassName, IntoView, NodeRef, View};
use zgui_vocab::{
    Key, KeyCode, KeyEvent, KeyState, Modifiers, PhysicalKey, PointerAction, PointerEvent, Role,
    Timestamp,
};

const CSS: &str = "
.root { display: flex; flex-direction: column; width: 400px; height: 320px; padding: 12px; gap: 8px; }
.doc { display: flex; flex-direction: column; width: 320px; gap: 8px; color: rgb(0, 102, 204); }
.doc.narrow { width: 96px; }
.h1 { display: block; font-size: 18px; }
.para { display: block; }
.noselect { display: block; user-select: none; }
.row { display: flex; flex-direction: row; gap: 12px; }
.cell { display: block; }
.field { display: block; width: 160px; height: 24px; }
";

const INK: [f32; 3] = [0.0, 102.0 / 255.0, 204.0 / 255.0];
const BAND_ALPHA: f32 = 0.30;

fn deliver(
    harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>,
    event: SurfaceEvent,
) {
    harness.deliver_to_first(event);
    harness.settle(8);
}

fn repaint_all(harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>) {
    deliver(harness, SurfaceEvent::Occluded(true));
    harness.deliver_to_first(SurfaceEvent::Occluded(false));
    harness.advance(Duration::from_millis(20));
    harness.settle(8);
}

fn pointer(
    harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>,
    action: PointerAction,
    x: f32,
    y: f32,
) {
    deliver(
        harness,
        SurfaceEvent::Pointer {
            action,
            event: PointerEvent::mouse(Point::new(CssPx(x), CssPx(y))),
            modifiers: Modifiers::NONE,
            timestamp: Timestamp::ORIGIN,
        },
    );
}

fn copy_shortcut(harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>) {
    let event = KeyEvent {
        key: Key::Character("c".into()),
        key_without_modifiers: Key::Character("c".into()),
        physical: PhysicalKey::Code(KeyCode::KeyC),
        location: zgui_vocab::KeyLocation::Standard,
        repeat: false,
    };
    deliver(
        harness,
        SurfaceEvent::Key {
            state: KeyState::Pressed,
            event,
            modifiers: Modifiers::PRIMARY,
            timestamp: Timestamp::ORIGIN,
        },
    );
}

fn selection_band_quads(
    harness: &zgui_platform_headless::Harness<zgui_runtime::Runtime>,
) -> Vec<[f32; 4]> {
    let window = &harness.app().windows()[0];
    let scene = window.scene();
    scene
        .primitives
        .quads
        .iter()
        .filter_map(|quad| {
            let color = quad
                .fill
                .id()
                .and_then(|id| scene.paints.get(id))
                .and_then(|paint| match paint {
                    zgui_scene::Paint::Solid(color) => Some(*color),
                    _ => None,
                })?;
            let alpha_match = (color.alpha() - BAND_ALPHA).abs() < 0.01;
            let rgb_match = color
                .components()
                .iter()
                .zip(INK)
                .all(|(got, want)| (got - want).abs() < 0.02);
            (alpha_match && rgb_match).then_some(quad.bounds)
        })
        .collect()
}

#[test]
fn reverse_cross_block_drag_respects_separators_exclusions_reflow_and_detach() {
    let heading_ref = NodeRef::new();
    let cell_b_ref = NodeRef::new();
    let narrow_sig = RwSignal::new(false);
    let show_heading = RwSignal::new(true);

    let mut harness = support::app_with_text(CSS, move |cx: &mut BuildCx<'_>| {
        let doc = zgui_elements::column()
            .class("doc")
            .class_toggle(ClassName::new("narrow"), move || narrow_sig.get())
            .attribute(AttrName::new("data-zgui-document-selection"), "true")
            .child(zgui_view::Show::new(
                move || show_heading.get(),
                move || {
                    AnyView::new(
                        zgui_elements::text()
                            .class("h1")
                            .node_ref(heading_ref)
                            .child("Alpha Heading"),
                    )
                },
            ))
            .child(
                zgui_elements::text()
                    .class("para")
                    .child("One two three four five six seven"),
            )
            .child(zgui_elements::text().class("noselect").child("SKIP_ME"))
            .child(zgui_elements::field().class("field").child("SECRET_INPUT"))
            .child(
                zgui_elements::row()
                    .class("row")
                    .a11y(A11yBinding::new(Role::Row))
                    .child(zgui_elements::text().class("cell").child("CellA"))
                    .child(
                        zgui_elements::text()
                            .class("cell")
                            .node_ref(cell_b_ref)
                            .child("CellB"),
                    ),
            );
        Box::new(
            zgui_elements::column()
                .class("root")
                .child(doc)
                .into_view()
                .build(cx),
        )
    });
    harness.settle(8);

    // Measure heading and second table cell positions in CSS pixels.
    let (hx, hy, _hw, hh) = harness.app().windows()[0]
        .node_box_css(heading_ref.get_untracked().expect("heading"))
        .expect("heading box");
    let (bx, by, bw, bh) = harness.app().windows()[0]
        .node_box_css(cell_b_ref.get_untracked().expect("cell_b"))
        .expect("cell_b box");

    // Reverse drag from right end of CellB up to left start of Alpha Heading.
    let start_x = bx + bw + 4.0;
    let start_y = by + bh * 0.5;
    let end_x = hx - 2.0;
    let end_y = hy + hh * 0.5;

    pointer(&mut harness, PointerAction::Pressed, start_x, start_y);
    pointer(
        &mut harness,
        PointerAction::Moved,
        start_x,
        (start_y + end_y) * 0.5,
    );
    pointer(&mut harness, PointerAction::Moved, end_x, end_y);
    let drag_bands = selection_band_quads(&harness);
    assert_eq!(
        drag_bands.len(),
        4,
        "expected 4 highlight bands across heading, paragraph, and 2 cells immediately after drag"
    );
    pointer(&mut harness, PointerAction::Released, end_x, end_y);

    // Verify copied text is in document order, skips `user-select: none` and `<field>`, joins
    // table cells with `\t`, separates blocks with `\n`, and never inserts `\n` for soft wraps.
    let selected = harness.app().windows()[0]
        .document_selection_text()
        .expect("selection must be active");
    assert_eq!(
        selected,
        "Alpha Heading\nOne two three four five six seven\nCellA\tCellB",
        "unexpected cross-block selection text"
    );

    copy_shortcut(&mut harness);
    let copied = harness
        .platform()
        .clipboard()
        .read_blocking(ClipboardKind::Standard, ClipboardFormat::Text)
        .expect("Ctrl+C must reach clipboard");
    assert_eq!(
        copied,
        ClipboardData::Text("Alpha Heading\nOne two three four five six seven\nCellA\tCellB".into()),
        "Ctrl+C must write document-order selection to clipboard"
    );

    // Force a full repaint and verify selection highlight quads exist across blocks.
    repaint_all(&mut harness);
    let wide_bands = selection_band_quads(&harness);
    assert!(
        wide_bands.len() >= 4,
        "expected >= 4 selection band quads across heading, paragraph, and 2 cells, got {}",
        wide_bands.len()
    );

    // Narrow the container so the paragraph soft-wraps onto multiple lines: selection text must
    // still have NO `\n` inserted for soft wraps, and highlight band count must increase!
    narrow_sig.set(true);
    let _ = zgui_reactive::flush();
    harness.settle(8);
    deliver(
        &mut harness,
        SurfaceEvent::Resized(Size::new(DevicePx(400.0), DevicePx(320.0))),
    );
    repaint_all(&mut harness);

    let narrow_text = harness.app().windows()[0]
        .document_selection_text()
        .expect("selection survives reflow");
    assert_eq!(
        narrow_text,
        "Alpha Heading\nOne two three four five six seven\nCellA\tCellB",
        "soft wrapping after container resize must not insert \\n"
    );
    let narrow_bands = selection_band_quads(&harness);
    assert!(
        narrow_bands.len() > wide_bands.len(),
        "narrowing container should produce more highlight bands across wrapped lines ({} vs {})",
        narrow_bands.len(),
        wide_bands.len()
    );

    // Detach the heading (an endpoint node) -> selection must clear safely with no stale highlights.
    show_heading.set(false);
    let _ = zgui_reactive::flush();
    harness.settle(8);
    assert!(
        harness.app().windows()[0].document_selection().is_none(),
        "detaching an endpoint node must safely clear document selection"
    );
    repaint_all(&mut harness);
    assert!(
        selection_band_quads(&harness).is_empty(),
        "clearing selection after detach must remove all painted highlight quads"
    );
}
