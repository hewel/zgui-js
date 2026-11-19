use super::*;
use std::{sync::mpsc, time::Duration};
use zgui::{platform::{SurfaceId, WakeReason, Waker}, view::stub::StubHost};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;
use crate::{integration::{self, ExternalWake, IntegrationHarness}, session::{ResourceCounts, SessionModule}, test_package::Fixture};

struct ChannelWaker(mpsc::Sender<WakeReason>);
impl Waker for ChannelWaker { fn wake(&self, reason: WakeReason) { let _ = self.0.send(reason); } }

/// Records surface bindings and frame requests; no GPU required.
struct StubSurface {
    document: DocumentId,
    bindings: HashMap<DocumentId, Weak<Session>>,
    images: Option<(u64, zgui::runtime::ImageBlobs)>,
    frames: RefCell<Vec<DocumentId>>,
}
impl SessionSurface for StubSurface {
    type Checkpoint = ();
    type MountWrites = ();

    fn surface(&self, document: DocumentId) -> Option<SurfaceId> {
        (document == self.document).then(|| SurfaceId::new(7))
    }
    fn bind(&mut self, document: DocumentId, session: &Rc<Session>) {
        self.bindings.insert(document, Rc::downgrade(session));
    }
    fn image_source_checkpoint(&mut self, _document: DocumentId) {}
    fn seal_image_source_writes(&self, _document: DocumentId, _checkpoint: ()) {}
    fn discard_image_source_writes(&mut self, _document: DocumentId, _checkpoint: ()) {}
    fn commit_image_policy(&mut self, _document: DocumentId, session: &Session, _mount_writes: ()) {
        if let Some((_, old)) = self.images.replace((session.generation, session.image_blobs().clone())) {
            old.clear();
        }
    }
    fn request_frame(&self, document: DocumentId) { self.frames.borrow_mut().push(document); }
}

struct Native {
    host: SessionHost,
    surface: StubSurface,
    stub: Rc<StubHost>,
    region: MountRegion,
    waker: Arc<dyn Waker>,
    backend: Rc<DocumentDom>,
}
impl Native {
    fn new(recovery: RecoveryPolicy, module: SessionModule) -> Self {
        let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let stub = Rc::new(StubHost::default());
        let region = MountRegion { document: backend.document_id(), root, dom, host: HostHandle::from_rc(stub.clone()), fonts: zgui::app::Fonts::shipped_only(), root_anchor: Rc::new(Cell::new(root)) };
        let pending = Rc::new(RefCell::new(vec![region.clone()]));
        let (sender, _wakes) = mpsc::channel();
        Self {
            host: SessionHost::new(pending, module, SessionPolicy::live(host_services::Executor::new(2, 64).unwrap()), recovery),
            surface: StubSurface { document: region.document, bindings: HashMap::new(), images: None, frames: RefCell::new(Vec::new()) },
            stub, region, waker: Arc::new(ChannelWaker(sender)),
            backend,
        }
    }
    fn attach(&mut self) {
        let retry = no_retry();
        self.host.attach(&mut self.surface, self.waker.clone(), retry).unwrap();
    }
    fn text(&self) -> String { self.region.dom.text_content(self.region.root_anchor.get()) }
    fn session(&self) -> Rc<Session> { self.host.session(self.region.document).expect("mounted session").clone() }
}

fn no_retry() -> Rc<dyn Fn(DocumentId, u64)> { Rc::new(|_, _| {}) }

const SIMPLE: &str = r#"
export const hostApiVersion = 1;
export function mount(root) {
    const label = __zgui.createTextNode('active');
    __zgui.insertNode(root, label, null);
    return () => __zgui.clearRoot(root);
}
"#;

const BAD_EVAL: &str = "throw Error('evaluation exploded');\nexport const hostApiVersion = 1;\nexport function mount() { return () => {}; }";

const BAD_MOUNT: &str = r#"
export const hostApiVersion = 1;
export function mount(root) {
    __zgui.insertNode(root, __zgui.createTextNode('partial'), null);
    setTimeout(() => {}, 60000);
    requestAnimationFrame(() => {});
    throw Error('mount exploded');
}
"#;

#[test]
fn rejected_preflight_preserves_active_generation_tree_and_stylesheet() {
    let good = Fixture::new(SIMPLE, "text { color: red }");
    let bad = Fixture::new(BAD_EVAL, "text { color: blue }");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(good.package()));
    native.attach();
    let active = native.session();
    let stylesheet = native.stub.stylesheet("application:0").expect("active app owns its stylesheet");
    let generation = active.generation;
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), native.region.document, SessionModule::package(bad.package()), no_retry());
    let ReloadOutcome::Rejected(error) = outcome else { panic!("evaluation failure must reject") };
    let SessionError::Diagnostic(diagnostic) = error else { panic!("evaluation must stay a diagnostic") };
    assert_eq!(diagnostic.message, "evaluation exploded");
    assert!(Rc::ptr_eq(&active, &native.session()));
    assert_eq!(native.text(), "active");
    assert_eq!(native.stub.stylesheet("application:0").as_ref(), Some(&stylesheet));
    // A rejected candidate still consumed its generation; the next mount is fresh.
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), native.region.document, SessionModule::package(good.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Mounted { generation: g, .. } if g == generation + 2), "{outcome:?}");
}

#[test]
fn failed_mount_preserves_successful_session_and_rejects_candidate() {
    let good = Fixture::new(SIMPLE, "text { color: red }");
    let bad = Fixture::new(BAD_MOUNT, "text { color: blue }");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(good.package()));
    native.attach();
    let old = native.session();
    let document = native.region.document;
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(bad.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Rejected(_)), "{outcome:?}");
    assert!(Rc::ptr_eq(&old, &native.session()), "failed candidate displaced the successful session");
    assert_eq!(native.text(), "active");
    assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text { color: red }"));
    assert!(native.host.fallback_generation(document).is_none());
    assert_eq!(native.host.acceptance().unwrap()["status"], "rejected");
    assert_eq!(native.host.acceptance().unwrap()["lastSuccessfulBuildId"], good.package().manifest.build_id);
    assert!(native.host.retry(&mut native.surface, native.waker.clone(), document, 2, no_retry()).is_none());
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(good.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Mounted { generation: 3, .. }));
    assert_eq!(old.counts(), ResourceCounts::default());
}

#[test]
fn image_policy_commits_only_after_acceptance_and_previous_owner_cleanup() {
    let good = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            const image = __zguiServices.registerImageAsset('test.txt');
            return () => image.release();
        }
    "#, "");
    let bad = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            __zguiServices.registerImageAsset('test.txt');
            throw Error('mount exploded');
        }
    "#, "");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(good.package()));
    native.attach();
    let old = native.session();
    let document = native.region.document;
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(bad.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Rejected(_)), "{outcome:?}");
    assert!(Rc::ptr_eq(&native.surface.bindings[&document].upgrade().unwrap(), &old));
    let (generation, blobs) = native.surface.images.as_ref().unwrap();
    assert_eq!((*generation, blobs.len()), (old.generation, 1));
    assert_eq!(old.counts().image_handles, 1);

    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(good.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Mounted { generation: 3, .. }), "{outcome:?}");
    let (generation, blobs) = native.surface.images.as_ref().unwrap();
    assert_eq!((*generation, blobs.len()), (3, 1));
    let report = old.retire();
    assert_eq!(report.before.image_handles, 1);
    assert_eq!(report.counts.image_handles, 0);
    assert_eq!(native.session().counts().image_handles, 1);
    assert!(native.host.acceptance().unwrap().get("predecessorLeak").is_none());
}

#[test]
fn reload_records_predecessor_leaks_without_rejecting_the_successor() {
    let first = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            setTimeout(() => {}, 60000);
            requestAnimationFrame(() => {});
            __zguiServices.registerImageAsset('test.txt');
            return () => {};
        }
    "#, "");
    let second = Fixture::new(SIMPLE, "");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(first.package()));
    native.attach();
    let old = native.session();
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), native.region.document, SessionModule::package(second.package()), no_retry());
    let acceptance = native.host.acceptance().unwrap();
    assert_eq!(acceptance["status"], "accepted", "{outcome:?}");
    let leak = &acceptance["predecessorLeak"];
    assert_eq!(leak["code"], "E_TEARDOWN");
    assert_eq!(leak["generation"], old.generation);
    assert_eq!(leak["resources"], serde_json::to_value(ResourceCounts { image_handles: 1, ..Default::default() }).unwrap());
    assert_eq!(old.counts(), ResourceCounts::default());
    assert_eq!(native.text(), "active");
}

#[test]
fn retired_shutdown_cannot_remove_the_successor_stylesheet() {
    let first = Fixture::new(SIMPLE, "text { color: red }");
    let second = Fixture::new(SIMPLE, "text { color: green }");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(first.package()));
    native.attach();
    let retired = native.session();
    let document = native.region.document;
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(second.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Mounted { .. }));
    let stylesheet = native.stub.stylesheet("application:0").expect("successor owns its stylesheet");
    retired.retire();
    drop(retired);
    assert_eq!(native.text(), "active");
    assert_eq!(native.stub.stylesheet("application:0").as_ref(), Some(&stylesheet));
    assert_eq!(native.session().counts().nodes, 1);
}

#[test]
fn generation_never_reused_across_surface_loss_and_reattach() {
    let fixture = Fixture::new(SIMPLE, "");
    let module = SessionModule::package(fixture.package());
    let mut native = Native::new(RecoveryPolicy::Fallback, module.clone());
    native.attach();
    assert_eq!(native.session().generation, 1);
    // Surface loss retires everything; the rebuilt region mounts a new generation.
    native.host.stop();
    native.host.pending.borrow_mut().push(native.region.clone());
    native.attach();
    assert_eq!(native.session().generation, 2);
    // A rejection between mounts still consumes its generation.
    let bad = Fixture::new(BAD_EVAL, "");
    let document = native.region.document;
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(bad.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Rejected(_)));
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, module, no_retry());
    assert!(matches!(outcome, ReloadOutcome::Mounted { generation: 4, .. }), "{outcome:?}");
}

#[test]
fn propagate_replacement_preserves_last_success_on_failed_mount() {
    let good = Fixture::new(SIMPLE, "text { color: red }");
    let bad = Fixture::new(BAD_MOUNT, "text { color: blue }");
    let mut native = Native::new(RecoveryPolicy::Propagate, SessionModule::package(good.package()));
    native.attach();
    let document = native.region.document;
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(bad.package()), no_retry());
    let ReloadOutcome::Rejected(error) = outcome else { panic!("propagated mount failure must hand back the typed error") };
    let SessionError::Diagnostic(diagnostic) = error else { panic!("mount failure must stay a diagnostic") };
    assert_eq!(diagnostic.message, "mount exploded");
    let failed = native.host.session(document).cloned().expect("previous successful session stays owned");
    assert_eq!(failed.generation, 1);
    assert!(native.host.fallback_generation(document).is_none());
    assert!(!native.text().contains("Application error"), "no fallback view is created under Propagate");
    // The caller inspects the retained session, then explicitly retires it.
    let retired = native.host.retire(document).expect("failed session must retire through the host");
    assert!(Rc::ptr_eq(&failed, &retired));
    assert_eq!(retired.counts(), ResourceCounts::default());
    assert!(native.host.session(document).is_none());
}

#[test]
fn propagate_checkpoint_fault_hands_back_the_typed_error_with_session_owned() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            __zgui.insertNode(root, __zgui.createTextNode('active'), null);
            queueMicrotask(() => { throw Error('checkpoint exploded'); });
            return () => __zgui.clearRoot(root);
        }
    "#, "");
    let mut native = Native::new(RecoveryPolicy::Propagate, SessionModule::package(fixture.package()));
    native.attach();
    let session = native.session();
    let error = native.host.checkpoint(&mut native.surface, no_retry()).unwrap_err();
    let SessionError::Diagnostic(diagnostic) = error else { panic!("checkpoint fault must stay a diagnostic") };
    assert_eq!(diagnostic.message, "checkpoint exploded");
    assert!(Rc::ptr_eq(&session, &native.session()));
    let retired = native.host.retire(native.region.document).unwrap();
    assert_eq!(retired.counts(), ResourceCounts::default());
}

#[test]
fn fault_classification_keeps_contract_diagnostic_and_state_categories() {
    use crate::preview::driver::Fault;
    let contract = Fault::application(SessionError::Contract(crate::assets::ContractFailure { code: "E_ASSERTION", message: "assert failed".into() }));
    assert_eq!((contract.error.code, contract.error.exit_code), ("E_ASSERTION", 4));
    let contract = Fault::application(SessionError::Contract(crate::assets::ContractFailure { code: "E_FIXTURE_MISSING_SERVICE", message: "missing".into() }));
    assert_eq!((contract.error.code, contract.error.exit_code), ("E_FIXTURE_MISSING_SERVICE", 6));
    let diagnostic = Fault::application(SessionError::Diagnostic(js_runtime::Diagnostic {
        kind: js_runtime::DiagnosticKind::Exception, phase: "mount",
        message: "boom".into(), stack: None, generated_stack: None,
    }));
    assert_eq!((diagnostic.error.code, diagnostic.error.exit_code), ("E_JS_EXCEPTION", 3));
    let budget = Fault::application(SessionError::Diagnostic(js_runtime::Diagnostic {
        kind: js_runtime::DiagnosticKind::BudgetExceeded, phase: "preflight",
        message: "preflight jobs exceeded their bounded pump".into(), stack: None, generated_stack: None,
    }));
    assert_eq!((budget.error.code, budget.error.exit_code), ("E_JOB_BUDGET", 5));
    let state = Fault::application(SessionError::InvalidState("mount region has no window".into()));
    assert_eq!((state.error.code, state.error.exit_code), ("E_HOST_INVARIANT", 9));
}

#[test]
fn harness_arms_baseline_timer_and_external_wake_then_retirement_cancels() {
    let mut native = Native::new(RecoveryPolicy::Fallback, integration::baseline_module());
    let (sender, wakes) = mpsc::channel();
    let external: ExternalWake = Arc::new(move |document, generation, surface| {
        let _ = sender.send((document, generation, surface));
    });
    let mut harness = IntegrationHarness::new();
    native.attach();
    harness.reconcile(&native.host, &external);
    assert_eq!(native.stub.live_timers(), 1, "baseline arms exactly one native timer");
    assert_eq!(native.text(), "JavaScript mounted");
    native.stub.advance(Duration::from_secs(1));
    assert_eq!(native.text(), "Native timer updated JavaScript");
    assert_eq!(native.stub.live_timers(), 0, "one-shot baseline timer consumed itself");
    // The detached external wake still fires on the host's schedule.
    let (document, generation, _surface) = wakes.recv_timeout(Duration::from_secs(10)).expect("external wake did not arrive");
    assert_eq!((document, generation), (native.region.document, 1));
    // A packaged session without --exercise keeps no harness timer; enabling
    // exercise arms four callbacks.
    let exercise = Fixture::new(r#"
        export const hostApiVersion = 1;
        let label = null;
        export function mount(root) {
            label = __zgui.createTextNode('active');
            __zgui.insertNode(root, label, null);
            return () => __zgui.clearRoot(root);
        }
        export function advance() { __zgui.replaceText(label, 'exercised'); }
    "#, "");
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(exercise.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Mounted { .. }));
    harness.reconcile(&native.host, &external);
    assert_eq!(native.stub.live_timers(), 0);
    harness.exercise = true;
    harness.reconcile(&native.host, &external);
    assert_eq!(native.stub.live_timers(), 1, "exercise arms one repeating timer");
    for _ in 0..3 { native.stub.advance(Duration::from_secs(1)); }
    assert_eq!(native.text(), "exercised");
    assert_eq!(native.stub.live_timers(), 1, "exercise timer still armed before its last call");
    native.stub.advance(Duration::from_secs(1));
    assert_eq!(native.stub.live_timers(), 0, "fourth exercise call cancels the timer");
    // A checkpoint fault retires the armed generation; reconciliation cancels
    // its outstanding exercise timer instead of dropping it silently.
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(exercise.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Mounted { .. }));
    harness.reconcile(&native.host, &external);
    assert_eq!(native.stub.live_timers(), 1, "new exercised generation armed");
    native.session().record_fault(SessionError::from("injected harness fault"));
    native.host.checkpoint(&mut native.surface, no_retry()).unwrap();
    harness.reconcile(&native.host, &external);
    assert_eq!(native.stub.live_timers(), 0, "no native timer survives checkpoint retirement");
    assert!(native.host.session(document).is_none(), "faulted generation must not remain mounted");
    // Retire the recovered generation while its timer is still pending.
    let failed_generation = native.host.fallback_generation(document).expect("Rust fallback missing");
    let outcome = native.host.retry(&mut native.surface, native.waker.clone(), document, failed_generation, no_retry());
    assert!(matches!(outcome, Some(ReloadOutcome::Mounted { .. })));
    harness.reconcile(&native.host, &external);
    assert_eq!(native.stub.live_timers(), 1, "retried generation armed its exercise timer");
    let retired = native.host.retire(document).unwrap();
    harness.reconcile(&native.host, &external);
    assert_eq!(native.stub.live_timers(), 0, "no native timer survives retirement");
    assert_eq!(retired.counts(), ResourceCounts::default());
}

#[test]
fn ordered_sheet_slots_survive_replacement_and_recreated_documents_install_their_own() {
    let first = Fixture::with_styles(SIMPLE, &[("base.css", "text {color:red}"), ("app.css", "text {color:blue}")]);
    let second = Fixture::with_styles(SIMPLE, &[("app.css", "text {color:green}")]);
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(first.package()));
    native.attach();
    assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text {color:red}"));
    assert_eq!(native.stub.stylesheet("application:1").as_deref(), Some("text {color:blue}"));
    let old = native.session();
    let document = native.region.document;
    assert!(matches!(native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(second.package()), no_retry()), ReloadOutcome::Mounted { .. }));
    old.retire();
    assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text {color:green}"));
    assert_eq!(native.stub.stylesheet("application:1"), None);
    native.host.stop();
    assert_eq!(native.stub.stylesheet("application:0"), None);
    native.host.pending.borrow_mut().push(native.region.clone());
    native.attach();
    assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text {color:green}"));
    native.host.stop();
}

#[test]
fn rejected_sheet_and_unregistered_font_preflight_preserve_successful_session() {
    let good = Fixture::new(SIMPLE, "text {color:red}");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(good.package()));
    native.attach();
    let active = native.session();
    for css in ["text {color: definitely-not-a-color}", "text {font-family: MisspelledAppSans}", "@font-face {font-family:AppSans;src:url(font.ttf)}"] {
        let bad = Fixture::new(SIMPLE, css);
        let outcome = native.host.replace(&mut native.surface, native.waker.clone(), native.region.document, SessionModule::package(bad.package()), no_retry());
        assert!(matches!(outcome, ReloadOutcome::Rejected(_)), "accepted {css}: {outcome:?}");
        assert!(Rc::ptr_eq(&active, &native.session()));
        assert_eq!(native.text(), "active");
        assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text {color:red}"));
    }
}

#[test]
fn mount_generated_sheet_rejection_preserves_successful_session() {
    let good = Fixture::new(SIMPLE, "text {color:red}");
    let bad = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            __zgui.installStylesheet('dynamic-theme', 'text { width: invalid }');
            __zgui.insertNode(root, __zgui.createTextNode('partial'), null);
            return () => __zgui.clearRoot(root);
        }
    "#, "text {color:blue}");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(good.package()));
    native.attach();
    let active = native.session();
    let outcome = native.host.replace(&mut native.surface, native.waker.clone(), native.region.document, SessionModule::package(bad.package()), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Rejected(_)), "{outcome:?}");
    assert!(Rc::ptr_eq(&active, &native.session()), "mount-generated invalid sheet displaced the predecessor");
    assert!(native.stub.stylesheet_names().iter().all(|name| !name.ends_with("dynamic-theme")), "rejected candidate command reached the host queue");
    assert_eq!(native.text(), "active");
    assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text {color:red}"));
}

#[test]
fn valid_mount_sheets_install_in_request_order() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            __zgui.installStylesheet('first', 'text { color: red }');
            __zgui.installStylesheet('second', 'text { color: blue }');
            __zgui.installStylesheet('first', 'text { color: green }');
            __zgui.installStylesheet('first', 'text { color: green }');
            __zgui.insertNode(root, __zgui.createTextNode('active'), null);
            return () => __zgui.clearRoot(root);
        }
    "#, "");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(fixture.package()));
    native.attach();
    let names = native.stub.stylesheet_names();
    let ordered: Vec<_> = names.iter().filter(|name| name.ends_with("first") || name.ends_with("second")).collect();
    assert_eq!(ordered.len(), 2);
    assert!(ordered[0].ends_with("first"));
    assert!(ordered[1].ends_with("second"));
    let installs = native.stub.stylesheet_installs();
    assert_eq!(installs, 3, "first, second, and the first replacement must each change installation state once");
    assert_eq!(native.stub.stylesheet(ordered[0]).as_deref(), Some("text { color: green }"));
}

#[test]
fn verified_package_fonts_are_registered_before_mount_and_retired_without_withdrawing_successors() {
    let data = include_bytes!("../../../../preview/fonts/NotoSans-Regular.ttf");
    let first = Fixture::new(SIMPLE, "text {font-family: ApplicationSans}").with_font("ApplicationSans", data);
    let second = Fixture::new(SIMPLE, "text {font-family: ApplicationSans}").with_font("ApplicationSans", data);
    let malformed = Fixture::new(SIMPLE, "text {font-family: ApplicationSans}").with_font("ApplicationSans", b"not a font");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(first.package()));
    native.attach();
    let first_session = native.session();
    assert_eq!(first_session.counts().fonts, 1);
    assert!(native.region.fonts.contains_family("ApplicationSans"));
    let document = native.region.document;
    assert!(matches!(native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(malformed.package()), no_retry()), ReloadOutcome::Rejected(_)));
    assert!(Rc::ptr_eq(&first_session, &native.session()));
    assert!(native.region.fonts.contains_family("ApplicationSans"));
    assert!(matches!(native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(second.package()), no_retry()), ReloadOutcome::Mounted { .. }));
    first_session.retire();
    assert_eq!(first_session.counts().fonts, 0);
    assert!(native.region.fonts.contains_family("ApplicationSans"));
    native.host.stop();
    assert!(!native.region.fonts.contains_family("ApplicationSans"));
}

#[test]
fn explicit_reduced_motion_policy_reaches_the_live_bridge() {
    let fixture = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            __zgui.insertNode(root, __zgui.createTextNode(String(__zgui.getReducedMotion())), null);
            return () => __zgui.clearRoot(root);
        }
    "#, "");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(fixture.package()));
    native.host.set_reduced_motion(true);
    native.attach();
    assert_eq!(native.text(), "true");
}

#[test]
fn initial_propagated_mount_failure_remains_owned_until_failure_evidence_is_persisted() {
    let bad = Fixture::new(BAD_MOUNT, "text {color:red}");
    let mut native = Native::new(RecoveryPolicy::Propagate, SessionModule::package(bad.package()));
    assert!(native.host.attach(&mut native.surface, native.waker.clone(), no_retry()).is_err());
    let failed = native.session();
    assert_eq!(failed.generation, 1);
    assert_eq!(native.host.acceptance().unwrap()["status"], "rejected");
    native.host.stop();
    assert_eq!(failed.counts(), ResourceCounts::default());
}

#[test]
fn failed_candidate_root_mutations_never_touch_the_successful_mount_root() {
    let good = Fixture::new(SIMPLE, "text {color:red}");
    let bad = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            __zgui.setProperty(root, 'class', 'rejected-root');
            __zgui.setProperty(root, 'style', 'color:blue');
            throw Error('root mutation rejected');
        }
    "#, "");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(good.package()));
    native.attach();
    let active = native.session();
    let document = native.region.document;
    assert!(matches!(native.host.replace(&mut native.surface, native.waker.clone(), document, SessionModule::package(bad.package()), no_retry()), ReloadOutcome::Rejected(_)));
    assert!(Rc::ptr_eq(&active, &native.session()));
    assert_eq!(native.backend.attribute(active.region.root, zgui::view::AttrName::new("class")), None);
    assert_eq!(native.text(), "active");
}
