//! Official StyleX priority selectors and strict native-profile preflight.

mod support;

use support::{Harness, color};
use zgui_style::preflight_stylesheet;

#[test]
fn official_priority_selectors_match_and_preserve_specificity() {
    let mut h = Harness::new();
    let child = h.append(h.root, "control");
    h.set_classes(h.root, &["theme"]);
    h.set_classes(child, &["x"]);
    let css = r#"
        :root { --ink: rgb(1, 2, 3) }
        .theme.theme:root { --ink: rgb(4, 5, 6) }
        .x:not(#\#):not(#\#) { color: rgb(30, 0, 0) }
        .x:not(#\#) { color: rgb(20, 0, 0) }
        .x.x { background-color: rgb(7, 8, 9); color: rgb(10, 0, 0) }
        .x { background-color: rgb(1, 1, 1) }
        root { color: var(--ink) }
    "#;
    preflight_stylesheet(css).expect("official selectors pass without rewriting");
    assert!(h.add_author(css).is_empty());
    h.frame();
    assert_eq!(color(&h, h.root), (4, 5, 6), "native root is not an html tag");
    assert_eq!(color(&h, child), (30, 0, 0), "ID negation specificity beats later classes");
    assert_eq!(support::background(&h, child), (7, 8, 9), "doubled class specificity");
    h.retire_all();
    h.edit(|edit| edit.set_id(child, Some(zgui_interned::Ident::new("#"))));
    h.frame();
    assert_eq!(color(&h, child), (10, 0, 0), "escaped ID really means #, not a rewritten sentinel");
}

#[test]
fn replacing_a_root_custom_property_restyles_an_inline_reader_and_leaves_no_style_work() {
    let mut h = Harness::new();
    let wrapper = h.append(h.root, "column");
    let child = h.append(wrapper, "box");
    h.edit(|edit| edit.set_inline_style(child, Some("background-color: var(--color-primary)")));
    assert!(h.add_author(":root { --color-primary: #ff0000 }").is_empty());
    h.frame();
    assert_eq!(support::background(&h, child), (255, 0, 0));
    h.retire_all();

    assert!(h.replace(0, ":root { --color-primary: #7dd3fc }").is_empty());
    h.engine.force_author_rules_dirty();
    h.frame();

    assert_eq!(support::background(&h, child), (125, 211, 252));
    assert_eq!(h.frame().passes, 0, "the completed restyle must not leave phantom work");
}

#[test]
fn partial_invalid_sheets_fail_preflight_without_installing_the_valid_subset() {
    let mut h = Harness::new();
    h.add_author("root { color: rgb(1, 2, 3) }");
    let errors = preflight_stylesheet("root { color: red;\n not-a-native-property: 1; width: invalid }").expect_err("parser drops reject the candidate");
    assert!(errors.len() >= 2, "all parser diagnostics survive: {errors:?}");
    assert!(errors.iter().any(|d| d.location.line == 1));
    h.frame();
    assert_eq!(color(&h, h.root), (1, 2, 3), "preflight has no install side effect");
}

#[test]
fn visual_and_typography_grammar_is_checked_at_preflight() {
    preflight_stylesheet(r#"
        .x:not(#\#) {
            box-shadow: 0 2px 4px #0008; text-shadow: 1px 1px black; filter: opacity(.5);
            backdrop-filter: blur(4px); mix-blend-mode: multiply; isolation: isolate;
            font-variation-settings: "wght" 650; font-style: oblique 12deg; font-stretch: 85%;
            white-space: pre-wrap; text-overflow: ellipsis; pointer-events: none; user-select: none;
            translate: 10px 20%; rotate: 15deg; scale: 1.25;
        }
    "#).expect("supported visual declarations pass preflight");
    for declaration in [
        "box-shadow: 1px", "text-shadow: 1px", "filter: unknown()", "backdrop-filter: blur(red)",
        "mix-blend-mode: none", "isolation: none", r#"font-variation-settings: "weight" 650"#,
        "font-style: 12deg", "font-stretch: 2px", "white-space: none", "text-overflow: auto",
        "pointer-events: 1", "user-select: hidden", "translate: 12", "rotate: 15", "scale: 2px",
    ] {
        preflight_stylesheet(&format!(".x {{ color: red; {declaration} }}")).expect_err(declaration);
    }
}

#[test]
fn unsupported_native_features_report_generated_locations() {
    for feature in [
        "@property --x { syntax: '<color>'; inherits: true; initial-value: red }",
        "@container (width > 1px) { .x { color: red } }",
        "@media (prefers-reduced-motion: reduce) { .x { color: red } }",
        "html[dir='rtl'] .x { color: red }",
        "html:not([dir='rtl']) .x { color: red }",
        ".x { background-image: url('asset.png') }",
        ".x { background-image: image-set('asset.png' 1x) }",
        "@import 'other.css';",
        "@font-face { font-family: app; src: local('App') }",
        ".x { --asset: u\\72l('asset.png') }",
        "h\\74ml /* split */ [d\\69r] .x { color: red }",
    ] {
        let css = format!("/* generated */\n{feature}");
        let errors = preflight_stylesheet(&css).expect_err(feature);
        assert!(errors.iter().any(|d| d.location.line == 1 && d.message.contains("zgui-stylex-v1")), "missing located profile diagnostic for {feature}: {errors:?}");
    }
}

#[test]
fn valid_themes_keyframes_and_strings_are_not_rejected_by_feature_scanning() {
    let summary = preflight_stylesheet(r#"
        :root { --motion: 100ms; --message: "url(fake) @import prefers-reduced-motion" }
        .theme.theme:root { --motion: 0ms }
        .x:not(#\#) { animation: pulse var(--motion) linear }
        @keyframes pulse { from { opacity: 0 } to { opacity: 1 } }
        html[role=dir] { --valid-attribute-value: 1; --label: prefers-reduced-motion }
        /* @property --not-a-rule; html[dir] */
    "#).expect("tokens, themes, priority selectors and keyframes are supported");
    assert_eq!(summary.rule_count, 5);
}

#[test]
fn summary_reports_static_named_fonts_from_native_parsed_declarations() {
    let summary = preflight_stylesheet(r#"
        .x { font-family: "App Sans", sans-serif, serif; --font: "Not a parsed family" }
        @media (min-width: 99999px) { .x { font: italic 16px App Display, monospace } }
        @layer components { .y { font-family: "App Sans", cursive, "serif" } }
        @keyframes fonts { from { font-family: App Animated } to { font-family: inherit } }
        .dynamic { font-family: var(--font) }
    "#).expect("native font grammar");
    assert_eq!(summary.font_families, ["App Sans", "App Display", "serif", "App Animated"]);
}

const DYNAMIC_REGISTRATION: &str = r#"@property --dynamic { syntax: "*"; inherits: false; }"#;

#[test]
fn dynamic_registration_is_consumed_by_the_real_cascade_and_lives_with_its_sheet() {
    let mut h = Harness::new();
    let child = h.append(h.root, "control");
    let own = h.append(h.root, "control");
    h.set_classes(own, &["own"]);
    h.add_author("root { --dynamic: 123px; width: var(--dynamic) } control { width: var(--dynamic, 7px) } .own { --dynamic: 19px }");
    h.frame();
    assert_eq!(support::width(&h, child), 123.0, "unregistered variables inherit normally");
    h.retire_all();

    preflight_stylesheet(DYNAMIC_REGISTRATION).expect("official dynamic registration is supported");
    assert!(h.add_author(DYNAMIC_REGISTRATION).is_empty(), "the actual installation parser consumes the same registration");
    h.frame();
    assert_eq!(support::width(&h, h.root), 123.0, "own-element value remains available");
    assert_eq!(support::width(&h, child), 7.0, "registered variable must not leak from parent");
    assert_eq!(support::width(&h, own), 19.0, "a child's own assignment still resolves");
    h.retire_all();

    assert!(h.replace(1, "").is_empty());
    h.frame();
    assert_eq!(support::width(&h, child), 123.0, "replacing registration sheet restores ordinary inheritance");
    h.retire_all();

    assert!(h.replace(1, DYNAMIC_REGISTRATION).is_empty());
    h.frame();
    assert_eq!(support::width(&h, child), 7.0, "replacement restores noninheritance");
    h.retire_all();

    h.drop_sheet(1);
    h.frame();
    assert_eq!(support::width(&h, child), 123.0, "removal removes registration, not merely its style rules");
}

#[test]
fn only_universal_noninherited_registration_without_initial_value_is_admitted() {
    assert_eq!(preflight_stylesheet(DYNAMIC_REGISTRATION).expect("dynamic registration").rule_count, 1);
    for css in [
        "@property --dynamic { syntax: '<length>'; inherits: false; initial-value: 0px }",
        "@property --dynamic { syntax: '*'; inherits: true }",
        "@property --dynamic { syntax: '*'; inherits: false; initial-value: 0px }",
        "@property --dynamic { syntax: '*'; inherits: false; initial-value: initial }",
        "@property --dynamic { syntax: '*' }",
        "@property --dynamic { inherits: false }",
        "@property --dynamic { syntax: '*'; inherits: false; unsupported: value }",
        "@media (min-width: 1px) { @property --dynamic { syntax: '*'; inherits: true } }",
    ] {
        assert!(preflight_stylesheet(css).is_err(), "non-profile registration accepted: {css}");
    }
}

