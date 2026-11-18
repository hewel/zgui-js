//! Applied stylesheet revisions and FIFO named-slot lifecycle at the public window seam.

mod support;

use std::{cell::RefCell, rc::Rc};
use zgui_runtime::{App, FrameProbe, HostBinding, Window};

#[derive(Default)]
struct Rejected(Rc<RefCell<Vec<(String, String)>>>);
impl HostBinding for Rejected {
    fn stylesheet_rejected(&mut self, name: &str, diagnostics: zgui_style::CssDiagnostics) {
        self.0.borrow_mut().push((name.to_owned(), diagnostics.iter().map(|diagnostic| diagnostic.message.clone()).collect::<Vec<_>>().join("; ")));
    }
}
use zgui_view::{BuildCx, IntoView, View};

struct Probe;
impl FrameProbe for Probe {
    fn frame_ended(&self, window: &Window) {
        assert!(window.completed_frame().expect("completed frame").style_revision <= window.style_revision());
    }
}

fn revision(h: &zgui_platform_headless::Harness<zgui_runtime::Runtime>) -> u64 {
    h.app().windows()[0].style_revision()
}

fn widths(h: &zgui_platform_headless::Harness<zgui_runtime::Runtime>) -> Vec<f32> {
    let layout = h.app().windows()[0].layout().borrow();
    layout.keys().into_iter().filter_map(|key| layout.layout_of(key))
        .map(|l| l.border_box().size.width.0).collect()
}

#[test]
fn applied_installs_replacements_removals_and_noops_have_exact_revisions() {
    let runtime = App::new().with_size(400.0, 300.0)
        .with_stylesheet("root { display: block } .panel { display: block; height: 20px; width: 10px }")
        .with_probe(Rc::new(Probe))
        .with_renderer(Box::new(support::capture))
        .into_handler(|cx: &mut BuildCx<'_>| {
            Box::new(zgui_elements::r#box().class("panel").into_view().build(cx)) as Box<dyn zgui_view::Anchor>
        }).expect("runtime");
    let mut h = zgui_platform_headless::Harness::new(runtime);
    h.settle(8);
    assert_eq!(revision(&h), 1, "application sheet counts, UA sheet does not");
    let host = h.app().windows()[0].host_handle().clone();
    let a = ".panel { width: 100px }";
    let b = ".panel { width: 200px }";
    host.install_stylesheet("a", a);
    host.install_stylesheet("b", b);
    assert_eq!(revision(&h), 1, "queued commands have not applied");
    assert!(h.app().windows()[0].has_pending_stylesheets(), "queued-only theme changes are not fresh");
    h.settle(8);
    assert_eq!(revision(&h), 3);
    assert!(!h.app().windows()[0].has_pending_stylesheets());
    assert!(widths(&h).contains(&200.0));
    host.install_stylesheet("a", ".panel { width: 300px }");
    h.settle(8);
    assert_eq!(revision(&h), 4);
    assert!(widths(&h).contains(&200.0), "replacement retains cascade position");
    host.install_stylesheet("a", ".panel { width: 300px }");
    host.remove_stylesheet("absent");
    h.settle(8);
    assert_eq!(revision(&h), 4, "byte identical replacement and absent remove do nothing");
    assert!(!h.app().windows()[0].has_pending_stylesheets(), "no-op requests do not leave pending work");
    host.remove_stylesheet("b");
    h.settle(8);
    assert_eq!(revision(&h), 5);
    assert!(widths(&h).contains(&300.0));
    host.install_stylesheet("a", a);
    host.remove_stylesheet("a");
    host.install_stylesheet("a", b);
    h.settle(8);
    assert_eq!(revision(&h), 8, "install/remove/install applies in request order");
    assert!(widths(&h).contains(&200.0));
    let frame = h.app().windows()[0].completed_frame().expect("probed frame");
    assert_eq!(frame.style_revision, 8);
    host.remove_stylesheet("a");
    host.remove_stylesheet("a");
    h.settle(8);
    assert_eq!(revision(&h), 9);
    assert!(widths(&h).contains(&10.0));
    h.shut_down();
}

#[test]
fn queued_dynamic_registration_install_replace_remove_updates_the_native_cascade() {
    const REGISTRATION: &str = r#"@property --dynamic { syntax: "*"; inherits: false; }"#;
    zgui_style::preflight_stylesheet(REGISTRATION).expect("official dynamic variable registration");
    let mut h = support::app(
        "root { display: block; width: 400px; --dynamic: 123px } .panel { display: block; height: 20px; width: var(--dynamic, 7px) }",
        |cx: &mut BuildCx<'_>| {
            Box::new(zgui_elements::r#box().class("panel").into_view().build(cx)) as Box<dyn zgui_view::Anchor>
        },
    );
    h.settle(8);
    let host = h.app().windows()[0].host_handle().clone();
    assert!(widths(&h).contains(&123.0));
    host.install_stylesheet("dynamic", REGISTRATION);
    h.settle(8);
    assert_eq!(revision(&h), 2);
    assert!(widths(&h).contains(&7.0), "registration disables parent inheritance");
    host.install_stylesheet("dynamic", "");
    h.settle(8);
    assert_eq!(revision(&h), 3);
    assert!(widths(&h).contains(&123.0), "replacement cleans the registration registry");
    host.install_stylesheet("dynamic", REGISTRATION);
    h.settle(8);
    assert_eq!(revision(&h), 4);
    assert!(widths(&h).contains(&7.0));
    host.remove_stylesheet("dynamic");
    h.settle(8);
    assert_eq!(revision(&h), 5);
    assert!(widths(&h).contains(&123.0), "removal restores inheritance");
    h.shut_down();
}

#[test]
fn invalid_queued_replacement_retains_the_previous_complete_sheet_and_revision() {
    let mut h = support::app(
        "root { display: block } .panel { display: block; height: 20px; width: 10px }",
        |cx: &mut BuildCx<'_>| {
            Box::new(zgui_elements::r#box().class("panel").into_view().build(cx)) as Box<dyn zgui_view::Anchor>
        },
    );
    h.settle(8);
    let host = h.app().windows()[0].host_handle().clone();
    let rejected = Rejected::default();
    h.app_mut().windows_mut()[0].install_binding(Box::new(Rejected(rejected.0.clone())));
    host.install_stylesheet("theme", ".panel { width: 20px }");
    h.settle(8);
    assert_eq!(revision(&h), 2);
    host.install_stylesheet("theme", ".panel { width: 30px; height: invalid }");
    h.settle(8);
    assert_eq!(revision(&h), 2, "rejected replacement must not advance the applied revision");
    assert!(widths(&h).contains(&20.0), "the previous complete sheet remains active");
    let reports = rejected.0.borrow();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].0, "theme");
    assert!(reports[0].1.contains("height") || reports[0].1.contains("invalid"), "{}", reports[0].1);
    drop(reports);
    host.install_stylesheet("theme", ".panel { width: 30px }");
    h.settle(8);
    assert_eq!(revision(&h), 3);
    assert!(widths(&h).contains(&30.0));
    h.shut_down();
}
