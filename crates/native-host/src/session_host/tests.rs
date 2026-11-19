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
    fn close_shell_surface(&mut self, _token: WindowToken, _id: Option<SurfaceId>) -> bool {
        panic!("the primary-only fixture cannot open shell surfaces");
    }
    fn stage_shell_surfaces(&mut self, _staged: &mut ShellWindows, _previous: &ShellWindows, _monitors: &[Monitor], _session: &Rc<Session>) -> SessionResult<()> {
        Ok(())
    }
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
    assert_eq!(leak["code"], "E_TEARDOWN", "acceptance records the predecessor's leaked lease");
    assert_eq!(leak["generation"], old.generation, "the leaked lease belongs to the predecessor");
    assert_eq!(leak["resources"], serde_json::to_value(ResourceCounts { image_handles: 1, ..Default::default() }).unwrap(),
        "the predecessor report identifies the image lease, not the canceled callbacks");
    assert_eq!(old.counts(), ResourceCounts::default(), "forced teardown releases the predecessor's leases");
    assert_eq!(native.text(), "active", "a teardown leak does not replace an ordinary successor with fallback");
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
    native.host.stop(&mut native.surface);
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
    native.host.stop(&mut native.surface);
    assert_eq!(native.stub.stylesheet("application:0"), None);
    native.host.pending.borrow_mut().push(native.region.clone());
    native.attach();
    assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text {color:green}"));
    native.host.stop(&mut native.surface);
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
    native.host.stop(&mut native.surface);
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
    native.host.stop(&mut native.surface);
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
    assert_eq!(native.backend.attribute(active.primary_region().root, zgui::view::AttrName::new("class")), None);
    assert_eq!(native.text(), "active");
}

#[test]
fn failed_shell_child_mount_keeps_the_predecessor_and_last_successful_build() {
    use zgui::platform::{AppHandler, PlatformCx};
    use zgui_platform_headless::Headless;

    let cap = crate::package::Capabilities { shell_surfaces: true, ..Default::default() };
    let good = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            const id = __zguiShell.openLayer('panel', {}, (root, bridge) => {
                bridge.insertNode(root, bridge.createTextNode('working'), null);
                return () => bridge.clearRoot(root);
            });
            return () => __zguiShell.closeLayer(id);
        }
    "#, "").with_capabilities(cap.clone());
    let bad = Fixture::new(r#"
        export const hostApiVersion = 1;
        export function mount() {
            __zguiShell.openLayer('partial', {}, () => () => {});
            __zguiShell.openLayer('broken', {}, () => { throw Error('shell child mount failed'); });
            return () => {};
        }
    "#, "").with_capabilities(cap);
    let mut runtime = zgui::runtime::App::new()
        .with_renderer(Box::new(|_, target| {
            let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
            zgui::render::Renderer::configure(&mut renderer, target);
            Ok(Box::new(renderer))
        }))
        .into_handler(|_| panic!("pure shell must not build a primary window"))
        .expect("create shell test runtime");
    runtime.without_primary_surface();
    let cx = Headless::new();
    runtime.surfaces_available(&cx);
    let good = good.package();
    let mut host = SessionHost::new(Rc::default(), SessionModule::package(good.clone()),
        SessionPolicy::live(host_services::Executor::new(2, 64).expect("create test service executor")),
        RecoveryPolicy::Propagate);
    host.set_pure_shell(zgui::app::Fonts::shipped_only());
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("mount the working shell");
    let previous = host.sessions().next().expect("working session").clone();
    let (_, panel) = previous.mount_regions().pop().expect("working shell region");
    assert_eq!(previous.counts().shell_surfaces, 1, "the working panel owns a native surface");

    let outcome = host.replace_shell(&mut runtime, &cx, ImageChoice::PackageOnly, SessionModule::package(bad.package()));
    assert!(matches!(&outcome, ReloadOutcome::Rejected(error) if error.to_string().contains("shell child mount failed")),
        "child mount error must reject the replacement, got {outcome:?}");
    assert!(Rc::ptr_eq(&previous, host.sessions().next().expect("predecessor still active")),
        "rejection must keep the same running session");
    let acceptance = host.acceptance().expect("record rejected reload");
    assert_eq!(acceptance["status"], "rejected", "native acceptance must include child mounting");
    assert_eq!(acceptance["lastSuccessfulBuildId"], good.manifest.build_id, "the working build is still last-good");
    assert_eq!(panel.region.dom.text_content(panel.region.root), "working", "the predecessor's tree was not disposed");
    assert!(previous.counts().engine_alive, "the predecessor's JS engine remains live");
    assert_eq!(cx.surfaces().iter().map(|surface| surface.id()).collect::<Vec<_>>(), [panel.surface],
        "both candidate windows, including the one that mounted, must be closed");

    host.stop_shell_surfaces(&mut runtime, &cx);
    assert_eq!(previous.retire().counts, ResourceCounts::default(), "shutdown releases the working shell without leaks");
}

#[test]
fn rejected_lock_candidate_keeps_the_accepted_auth_surface() {
    let good = lock_package(r#"
        export const hostApiVersion = 1;
        export function mount() {
            const lock = __zguiShell.requestSessionLock(() => {});
            __zguiShell.openLock('locker', 'A', (root, bridge) => {
                bridge.insertNode(root, bridge.createTextNode('accepted auth UI'), null);
                return () => bridge.clearRoot(root);
            });
            return () => lock.unsubscribe();
        }
    "#);
    let failed_child = lock_package(r#"
        export const hostApiVersion = 1;
        export function mount() {
            __zguiShell.requestSessionLock(() => {}).unlock();
            __zguiShell.openLock('replacement', 'A', () => { throw Error('candidate lock child failed'); });
            return () => {};
        }
    "#);
    let mut fixture = LockedShell::new(&good);
    let previous = fixture.session();
    let (_, auth) = previous.mount_regions().pop().expect("accepted auth region");
    let surface = fixture.cx.offscreen(auth.surface).expect("accepted platform claim");
    zgui::platform::AppHandler::surface_event(&mut fixture.runtime, &fixture.cx, auth.surface, zgui::platform::SurfaceEvent::RedrawRequested);
    assert!(fixture.cx.session_lock_snapshot().fallback_outputs.is_empty(), "the accepted auth frame must be visible before reload");
    for (candidate, fault) in [(lock_package(BAD_EVAL), "evaluation exploded"), (failed_child, "candidate lock child failed")] {
        let outcome = fixture.replace(&candidate);
        assert!(matches!(&outcome, ReloadOutcome::Rejected(error) if error.to_string().contains(fault)),
            "the candidate must be rejected for {fault}: {outcome:?}");
        fixture.turn().expect("accepted generation remains active");
        assert!(Rc::ptr_eq(&previous, &fixture.session()), "candidate rejection must not replace the active owner");
        assert!(Rc::ptr_eq(&auth, &previous.mount_regions().pop().expect("accepted auth still mounted").1),
            "rejection must retain the same mounted region and authentication state");
        assert!(Arc::ptr_eq(&surface, &fixture.cx.offscreen(auth.surface).expect("accepted platform claim remains")),
            "candidate rejection must keep the original platform surface");
        assert_eq!(auth.region.dom.text_content(auth.region.root), "accepted auth UI", "candidate rejection must not clear auth UI");
        assert!(fixture.cx.session_lock_snapshot().fallback_outputs.is_empty(), "candidate rejection must not hide the accepted UI");
        fixture.assert_locked();
    }
}

#[test]
fn lock_cleanup_faults_veto_staged_unlock() {
    let good = lock_package(LOCK_UI);
    for (source, fault) in [
        (r#"
            export const hostApiVersion = 1;
            export function mount() {
                const lock = __zguiShell.requestSessionLock(() => {});
                __zguiShell.openLock('locker', 'A', () => () => { throw Error('lock disposer fault'); });
                lock.unlock();
                return () => lock.unsubscribe();
            }
        "#, "lock disposer fault"),
        (r#"
            export const hostApiVersion = 1;
            export function mount() {
                const lock = __zguiShell.requestSessionLock(() => {});
                __zguiShell.openLock('locker', 'A', () => () => { Promise.reject(Error('lock disposal rejected')); });
                lock.unlock();
                return () => lock.unsubscribe();
            }
        "#, "lock disposal rejected"),
        (r#"
            export const hostApiVersion = 1;
            export function mount() {
                const lock = __zguiShell.requestSessionLock(() => {});
                const layer = __zguiShell.openLayer('panel', {}, () => () => { throw Error('layer disposer fault'); });
                __zguiShell.openLock('locker', 'A', () => () => __zguiShell.closeLayer(layer));
                lock.unlock();
                return () => lock.unsubscribe();
            }
        "#, "layer disposer fault"),
    ] {
        let mut fixture = LockedShell::new(&good);
        let outcome = fixture.replace(&lock_package(source));
        assert!(matches!(outcome, ReloadOutcome::Mounted { .. }), "candidate has no fault until disposal: {outcome:?}");
        let error = fixture.turn().expect_err("teardown failure must veto the staged unlock");
        assert!(error.to_string().contains(fault), "expected teardown error {fault}, got {error}");
        fixture.assert_locked();
        assert_eq!(fixture.cx.session_lock_snapshot().fallback_outputs, ["A"], "a teardown fault leaves native lock coverage");
    }
}

#[test]
fn lock_disposer_cannot_defer_an_unlock_to_a_timer_or_frame() {
    let package = lock_package(r#"
        export const hostApiVersion = 1;
        let auth;
        export function closeAuth() { __zguiShell.closeLock(auth); }
        export function mount() {
            const lock = __zguiShell.requestSessionLock(() => {});
            __zguiShell.openLayer('panel', {}, () => () => {});
            auth = __zguiShell.openLock('locker', 'A', () => () => {
                try { setTimeout(() => lock.unlock(), 1); } catch {}
                try { requestAnimationFrame(() => lock.unlock()); } catch {}
            });
            return () => lock.unsubscribe();
        }
    "#);
    let mut fixture = LockedShell::new(&package);
    let session = fixture.session();
    session.invoke("closeAuth").expect("close the lock UI but retain the compositor lock");
    fixture.turn().expect("dispose the lock child");
    let counts = session.counts();
    fixture.cx.virtual_clock().advance(Duration::from_millis(20));
    zgui::platform::AppHandler::deadline_reached(&mut fixture.runtime, &fixture.cx);
    fixture.turn().expect("drain callbacks after the teardown turn");
    fixture.assert_locked();
    assert_eq!(counts.timers, 0, "lock disposal must not register a timer");
    assert_eq!(counts.frame_callbacks, 0, "lock disposal must not register a frame callback on the surviving layer");
}

#[test]
fn predecessor_teardown_fault_fences_successor_mount_jobs_until_clean_recovery() {
    let faulty = lock_package(r#"
        export const hostApiVersion = 1;
        export function mount() {
            __zguiShell.requestSessionLock(() => {});
            __zguiShell.openLock('locker', 'A', () => () => {});
            return () => { throw Error('predecessor teardown fault'); };
        }
    "#);
    let successor = lock_package(r#"
        export const hostApiVersion = 1;
        export function mount() {
            const lock = __zguiShell.requestSessionLock(() => {});
            __zguiShell.openLock('next', 'A', () => () => {});
            Promise.resolve().then(() => { try { lock.unlock(); } catch {} });
            return () => lock.unsubscribe();
        }
    "#);
    let mut fixture = LockedShell::new(&faulty);
    let outcome = fixture.replace(&successor);
    assert!(matches!(outcome, ReloadOutcome::Mounted { .. }), "the successor can mount without inheriting unlock authority: {outcome:?}");
    fixture.turn().expect("process the fenced successor's queued mount job");
    fixture.assert_locked();
    assert_eq!(fixture.cx.session_lock_snapshot().fallback_outputs, ["A"], "failed predecessor teardown forces native fallback");

    let outcome = fixture.replace(&successor);
    assert!(matches!(outcome, ReloadOutcome::Mounted { .. }), "a clean authorized replacement can recover: {outcome:?}");
    fixture.turn().expect("process the clean replacement's authorized unlock");
    assert_eq!(fixture.cx.session_lock_snapshot().unlock_count, 1, "only the clean replacement can send unlock");
}

const LOCK_UI: &str = r#"
    export const hostApiVersion = 1;
    export function mount() {
        const lock = __zguiShell.requestSessionLock(() => {});
        __zguiShell.openLock('locker', 'A', () => () => {});
        return () => lock.unsubscribe();
    }
"#;

fn lock_package(source: &str) -> Fixture {
    Fixture::new(source, "").with_capabilities(crate::package::Capabilities {
        session_lock: true, shell_surfaces: true, ..Default::default()
    })
}

struct LockedShell {
    host: SessionHost,
    runtime: zgui::runtime::Runtime,
    cx: zgui_platform_headless::Headless,
    monitors: Vec<Monitor>,
}

impl LockedShell {
    fn new(package: &Fixture) -> Self {
        use zgui::{geom::{DevicePx, Point, Size}, platform::{AppHandler, MonitorInfo, PlatformCapabilities, PlatformCx}};
        let mut runtime = zgui::runtime::App::new()
            .with_renderer(Box::new(|_, target| {
                let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
                zgui::render::Renderer::configure(&mut renderer, target);
                Ok(Box::new(renderer))
            }))
            .into_handler(|_| panic!("pure lock shell has no primary document"))
            .expect("create runtime");
        runtime.without_primary_surface();
        let mut supported = PlatformCapabilities::none();
        supported.session_lock = true;
        let cx = zgui_platform_headless::Headless::new().with_capabilities(supported).with_monitors(vec![
            MonitorInfo::new(Point::new(DevicePx(0.0), DevicePx(0.0)), Size::new(DevicePx(32.0), DevicePx(24.0)), 1.0).with_name("A"),
        ]);
        runtime.surfaces_available(&cx);
        let monitors = Monitor::snapshot(&cx.monitors());
        let mut host = SessionHost::new(Rc::default(), SessionModule::package(package.package()),
            SessionPolicy::live(host_services::Executor::new(2, 64).expect("create service executor")), RecoveryPolicy::Propagate);
        host.set_pure_shell(zgui::app::Fonts::shipped_only());
        let mut fixture = Self { host, runtime, cx, monitors };
        for _ in 0..2 { fixture.turn().expect("lock and attach the UI"); }
        fixture.assert_locked();
        fixture
    }

    fn turn(&mut self) -> SessionResult<()> {
        self.host.shell_turn(&mut self.runtime, &self.cx, ImageChoice::PackageOnly, &self.monitors)
    }

    fn replace(&mut self, package: &Fixture) -> ReloadOutcome {
        self.host.replace_shell(&mut self.runtime, &self.cx, ImageChoice::PackageOnly, SessionModule::package(package.package()))
    }

    fn session(&self) -> Rc<Session> { self.host.sessions().next().expect("active shell session").clone() }

    fn assert_locked(&self) {
        let lock = self.cx.session_lock_snapshot();
        assert_eq!(lock.state, SessionLockState::Locked, "the compositor must remain locked");
        assert_eq!(lock.unlock_count, 0, "no unlock request may reach the compositor");
    }
}
