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
            host: SessionHost::new(pending, module, SessionPolicy::live(host_services::Executor::new(2, 64).unwrap()), recovery, SurfaceOpacity::Transparent),
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
        RecoveryPolicy::Propagate, SurfaceOpacity::Opaque);
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

const POPUP_TREE: &str = r#"
    export const hostApiVersion = 1;
    let menu;
    let dismissals = 0;
    let expectedCleanup = 'menu';
    const cleaned = [];
    function place(parent, width = 24) {
        return {
            parent, width, height: 20,
            anchorRect: { x: 1, y: 1, width: 6, height: 6 },
            offset: { x: 0, y: 0 }, anchor: 'bottom-left', gravity: 'bottom-right',
            grab: false, slide: true, flip: true, resize: false, reactive: false,
        };
    }
    function cleanup(name) { return () => () => cleaned.push(name); }
    export function mount() {
        __zguiShell.openLayer('panel', {}, cleanup('panel'));
        menu = __zguiShell.openPopup('menu', place('panel'), cleanup('menu'), () => {
            if (cleaned.join() !== 'menu') throw Error('dismissed before unmount');
            dismissals++;
        });
        return () => {
            if (cleaned.join() !== expectedCleanup) throw Error('primary cleaned before: ' + cleaned.join());
        };
    }
    export function queuePlacement() { __zguiShell.updatePopup(menu, place('panel', 28)); }
    export function queueChild() { __zguiShell.openPopup('child', place('menu'), cleanup('child'), () => {}); }
    export function expectChildCleanup() { expectedCleanup = 'child,menu'; }
    export function checkDismissal() {
        if (dismissals !== 1) throw Error('expected one dismissal, got ' + dismissals);
    }
"#;

struct PopupTree {
    host: SessionHost,
    runtime: zgui::runtime::Runtime,
    cx: zgui_platform_headless::Headless,
    session: Rc<Session>,
}

impl PopupTree {
    fn new() -> Self {
        use zgui::platform::AppHandler;

        let package = Fixture::new(POPUP_TREE, "").with_capabilities(crate::package::Capabilities {
            shell_surfaces: true, ..Default::default()
        });
        let pending = Rc::new(RefCell::new(Vec::new()));
        let roots = pending.clone();
        let fonts = zgui::app::Fonts::shipped_only();
        let mut runtime = zgui::runtime::App::new()
            .with_renderer(Box::new(|_, target| {
                let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
                zgui::render::Renderer::configure(&mut renderer, target);
                Ok(Box::new(renderer))
            }))
            .into_handler(move |cx| {
                zgui::reactive::provide_context(fonts.clone());
                Box::new(Root(roots.clone()).build(cx))
            })
            .expect("create popup test runtime");
        let cx = zgui_platform_headless::Headless::new();
        runtime.surfaces_available(&cx);
        let mut host = SessionHost::new(pending, SessionModule::package(package.package()),
            SessionPolicy::live(host_services::Executor::new(2, 64).expect("create service executor")),
            RecoveryPolicy::Propagate, SurfaceOpacity::Opaque);
        host.attach(&mut RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly), cx.waker(), no_retry())
            .expect("mount the primary and popup tree");
        let session = host.sessions().next().expect("active session").clone();
        Self { host, runtime, cx, session }
    }

    fn event(&mut self, surface: SurfaceId, event: zgui::platform::SurfaceEvent) -> SessionResult<()> {
        self.host.surface_event(&mut self.runtime, &self.cx, surface, event)
    }

    fn turn(&mut self) -> SessionResult<()> {
        self.host.shell_turn(&mut self.runtime, &self.cx, ImageChoice::PackageOnly, &[])
    }

    fn surface(&self, name: &str) -> SurfaceId {
        self.session.mount_regions().into_iter().find(|(region, _)| region == name)
            .unwrap_or_else(|| panic!("missing mount region {name:?}")).1.surface
    }
}

#[test]
fn popup_done_wins_over_pending_reposition_and_child_open() {
    for pending in ["queuePlacement", "queueChild"] {
        let mut fixture = PopupTree::new();
        let menu = fixture.surface("menu");
        fixture.cx.offscreen(menu).expect("mounted popup").mark_popup_done();
        fixture.session.invoke(pending).expect("queue shell work before the close event");
        fixture.turn().unwrap_or_else(|error| panic!("{pending} raced with compositor dismissal: {error}"));
        assert!(fixture.cx.surface(menu).is_none(), "dismissed popup must be gone");
        assert_eq!(fixture.cx.surfaces().len(), 2, "only the primary and panel should remain");
        fixture.event(menu, zgui::platform::SurfaceEvent::Destroyed).expect("late close notification is harmless");
        fixture.turn().expect("session continues running after dismissal");
        fixture.session.invoke("checkDismissal").expect("owner was notified exactly once");
        fixture.host.stop_shell_surfaces(&mut fixture.runtime, &fixture.cx);
        let report = fixture.session.retire();
        assert!(report.failure.is_none(), "teardown failed: {:?}", report.failure);
    }
}

#[test]
fn closing_primary_disposes_popup_tree_before_primary_cleanup() {
    let mut fixture = PopupTree::new();
    fixture.session.invoke("queueChild").expect("declare nested popup");
    fixture.session.invoke("expectChildCleanup").expect("check the full tree on shutdown");
    fixture.turn().expect("mount nested popup");
    let child = fixture.surface("child");
    assert!(fixture.cx.surface(child).is_some(), "nested popup is live before closing");
    let primary = fixture.surface("main");
    fixture.event(primary, zgui::platform::SurfaceEvent::CloseRequested).expect("close the primary");
    assert!(fixture.cx.surfaces().is_empty(), "primary and all auxiliary surfaces were closed");
    assert!(fixture.runtime.windows().is_empty(), "all native windows were released");
    let report = fixture.session.retire();
    assert!(report.failure.is_none(), "child-before-parent cleanup failed: {:?}", report.failure);
    assert_eq!(report.counts, ResourceCounts::default(), "retirement leaks no shell surfaces");
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
fn locked_reload_advances_the_daemon_correlation_identity_without_unlocking() {
    let package = lock_package(r#"
        export const hostApiVersion = 1;
        export function mount() {
            const generation = __zguiShell.getSessionGeneration();
            const lock = __zguiShell.requestSessionLock(() => {});
            __zguiShell.openLock('locker', 'A', (root, bridge) => {
                bridge.insertNode(root, bridge.createTextNode(`${generation}:${lock.id}`), null);
                return () => bridge.clearRoot(root);
            });
            return () => {
                if (__zguiShell.getSessionGeneration() !== generation) {
                    throw Error('generation changed during cleanup');
                }
                lock.unsubscribe();
            };
        }
    "#);
    let mut fixture = LockedShell::new(&package);
    let identity = |session: &Session| {
        let (_, lock) = session.mount_regions().pop().expect("active lock mount region");
        let label = lock.region.dom.text_content(lock.region.root);
        let (generation, id) = label.split_once(':').expect("generation and request ID label");
        let generation = generation.parse::<u64>().expect("safe integer generation");
        assert_eq!(generation, session.generation, "bridge identity matches the accepted session");
        (generation, id.parse::<u64>().expect("safe integer lock request ID"))
    };
    let previous = fixture.session();
    let (old_generation, old_id) = identity(&previous);
    let outcome = fixture.replace(&lock_package(BAD_EVAL));
    assert!(matches!(outcome, ReloadOutcome::Rejected(_)), "bad preflight must be rejected: {outcome:?}");
    let outcome = fixture.replace(&package);
    assert!(matches!(outcome, ReloadOutcome::Mounted { .. }), "clean reload must succeed: {outcome:?}");
    let retirement = previous.retire();
    assert!(retirement.failure.is_none(), "predecessor cleanup must see its own generation: {:?}", retirement.failure);
    let (generation, id) = identity(&fixture.session());
    assert_eq!(generation, old_generation + 2, "rejected candidates consume their generation");
    assert!(id > old_id, "reloading must not reset lock request IDs: {old_id}, {id}");
    fixture.assert_locked();
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
            SessionPolicy::live(host_services::Executor::new(2, 64).expect("create service executor")),
            RecoveryPolicy::Propagate, SurfaceOpacity::Transparent);
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

#[test]
fn window_surface_hosts_child_popup_and_closes_on_compositor_close_request() {
    use zgui::platform::AppHandler;

    const WINDOW_APP: &str = r#"
        export const hostApiVersion = 1;
        let winId;
        const cleaned = [];
        let closed = 0;
        export function mount() {
            winId = __zguiShell.openWindow('settings', {
                title: 'Settings', appId: 'z-shell-settings',
                width: 820, height: 600, minWidth: 640, minHeight: 480,
                resizable: true, decorations: 'full', transparent: false,
            }, () => () => cleaned.push('settings'), () => {
                if (cleaned.join() !== 'menu,settings') throw Error('unexpected close order: ' + cleaned.join());
                closed++;
            });
            __zguiShell.openPopup('menu', {
                parent: 'settings', width: 220, height: 'auto', minHeight: 40, maxHeight: 300,
                anchorRect: { x: 16, y: 16, width: 80, height: 28 },
                offset: { x: 0, y: 4 }, anchor: 'bottom-left', gravity: 'bottom-right',
                grab: false, slide: true, flip: true, resize: false, reactive: false,
            }, () => () => cleaned.push('menu'), () => {});
            return () => {};
        }
        export function resizeWindow() {
            __zguiShell.updateWindow(winId, {
                title: 'Settings — Display', appId: 'z-shell-settings',
                width: 900, height: 640, minWidth: 640, minHeight: 480,
                resizable: true, decorations: 'no-title-bar', transparent: false,
            });
        }
        export function checkClosed() {
            if (closed !== 1) throw Error('expected one window close callback, got ' + closed);
        }
    "#;

    let package = Fixture::new(WINDOW_APP, "").with_capabilities(crate::package::Capabilities {
        shell_surfaces: true, ..Default::default()
    });
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let fonts = zgui::app::Fonts::shipped_only();
    let mut runtime = zgui::runtime::App::new()
        .with_renderer(Box::new(|_, target| {
            let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
            zgui::render::Renderer::configure(&mut renderer, target);
            Ok(Box::new(renderer))
        }))
        .into_handler(move |cx| {
            zgui::reactive::provide_context(fonts.clone());
            Box::new(Root(roots.clone()).build(cx))
        })
        .expect("create window surface test runtime");
    let cx = zgui_platform_headless::Headless::new();
    runtime.surfaces_available(&cx);
    let mut host = SessionHost::new(pending, SessionModule::package(package.package()),
        SessionPolicy::live(host_services::Executor::new(2, 64).expect("create service executor")),
        RecoveryPolicy::Propagate, SurfaceOpacity::Opaque);
    host.attach(&mut RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly), cx.waker(), no_retry())
        .expect("mount window surface and popup");
    let session = host.sessions().next().expect("active session").clone();
    let surface_of = |name: &str| {
        session.mount_regions().into_iter().find(|(region, _)| region == name)
            .unwrap_or_else(|| panic!("missing mount region {name:?}")).1.surface
    };
    let settings = surface_of("settings");
    let menu = surface_of("menu");
    assert_eq!(cx.surfaces().len(), 3, "primary, settings window, and child popup should be live");
    assert_eq!(host.shell_geometry(settings), Some((820.0, 600.0, None)));

    session.invoke("resizeWindow").expect("update window title and size");
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("reconcile window update");
    assert_eq!(host.shell_geometry(settings), Some((900.0, 640.0, None)));

    host.surface_event(&mut runtime, &cx, settings, zgui::platform::SurfaceEvent::CloseRequested)
        .expect("close shell window on compositor request");
    assert!(cx.surface(settings).is_none(), "closed window surface must be gone");
    assert!(cx.surface(menu).is_none(), "child popup anchored to window must close with it");
    assert_eq!(cx.surfaces().len(), 1, "primary surface stays alive after auxiliary window closes");
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("dismissed window must not reopen");
    assert_eq!(cx.surfaces().len(), 1, "dismissed window stays closed");
    session.invoke("checkClosed").expect("onClose fired once after child popup + window cleanup");

    host.stop_shell_surfaces(&mut runtime, &cx);
    let report = session.retire();
    assert!(report.failure.is_none(), "teardown failed: {:?}", report.failure);
}

#[test]
fn pure_shell_frame_callbacks_isolate_occlusion_migrate_on_close_and_tick_per_surface() {
    use zgui::platform::{AppHandler, SurfaceEvent};
    use zgui::view::IntoView;

    const SHELL_RAF_APP: &str = r#"
        export const hostApiVersion = 1;
        let wallpaperId;
        let dockBridge;
        const events = [];
        export function mount() {
            const placement = {
                layer: 'top', anchor: ['top', 'left'], width: 120, height: 40,
                margin: { top: 0, right: 0, bottom: 0, left: 0 }, exclusiveZone: 0, keyboard: 'none',
            };
            wallpaperId = __zguiShell.openLayer('wallpaper', placement, () => () => {});
            __zguiShell.openLayer('dock', placement, (_root, bridge) => {
                dockBridge = bridge;
                return () => {};
            });
            return () => {};
        }
        export function scheduleDockOnly() {
            dockBridge.requestAnimationFrame(() => events.push('dock-pinned'));
        }
        export function scheduleUnscoped() {
            requestAnimationFrame(() => events.push('unscoped'));
        }
        export function closeWallpaper() {
            __zguiShell.closeLayer(wallpaperId);
        }
        export function expectStep0() { if (events.join(',') !== '') throw Error('step0: ' + events.join(',')); }
        export function expectStep1() { if (events.join(',') !== 'dock-pinned') throw Error('step1: ' + events.join(',')); }
        export function expectStep2() { if (events.join(',') !== 'dock-pinned,dock-pinned,unscoped') throw Error('step2: ' + events.join(',')); }
        export function expectStep3() { if (events.join(',') !== 'dock-pinned,dock-pinned,unscoped,unscoped') throw Error('step3: ' + events.join(',')); }
    "#;

    let package = Fixture::new(SHELL_RAF_APP, "").with_capabilities(crate::package::Capabilities {
        shell_surfaces: true, ..Default::default()
    });
    let pending = Rc::new(RefCell::new(Vec::new()));
    let fonts = zgui::app::Fonts::shipped_only();
    let mut runtime = zgui::runtime::App::new()
        .with_renderer(Box::new(|_, target| {
            let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
            zgui::render::Renderer::configure(&mut renderer, target);
            Ok(Box::new(renderer))
        }))
        .into_handler(|cx| Box::new(zgui::elements::column().into_view().build(cx)))
        .expect("create pure-shell test runtime");
    runtime.without_primary_surface();
    let cx = zgui_platform_headless::Headless::new();
    runtime.surfaces_available(&cx);

    let mut host = SessionHost::new(
        pending,
        SessionModule::package(package.package()),
        SessionPolicy::live(host_services::Executor::new(2, 64).expect("create service executor")),
        RecoveryPolicy::Propagate,
        SurfaceOpacity::Transparent,
    );
    host.set_pure_shell(fonts);
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("mount pure shell");
    let session = host.sessions().next().expect("active pure-shell session").clone();
    let surface_of = |name: &str| {
        session.mount_regions().into_iter().find(|(region, _)| region == name)
            .unwrap_or_else(|| panic!("missing mount region {name:?}")).1.surface
    };
    let wallpaper = surface_of("wallpaper");
    let dock = surface_of("dock");

    // 1. Per-surface cadence: dock-pinned rAF does NOT fire on wallpaper's redraw, only on dock's.
    session.invoke("scheduleDockOnly").expect("schedule pinned dock frame");
    host.surface_event(&mut runtime, &cx, wallpaper, SurfaceEvent::RedrawRequested).expect("redraw wallpaper");
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("shell turn after wallpaper redraw");
    session.invoke("expectStep0").expect("wallpaper redraw must not fire dock-pinned rAF");
    host.surface_event(&mut runtime, &cx, dock, SurfaceEvent::RedrawRequested).expect("redraw dock");
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("shell turn after dock redraw");
    session.invoke("expectStep1").expect("dock redraw fires dock-pinned rAF");

    // 2. Occluding wallpaper migrates pending unscoped rAF onto visible dock, while dock-pinned keeps firing.
    session.invoke("scheduleUnscoped").expect("schedule unscoped rAF on wallpaper");
    session.invoke("scheduleDockOnly").expect("schedule dock rAF");
    host.surface_event(&mut runtime, &cx, wallpaper, SurfaceEvent::Occluded(true)).expect("occlude wallpaper");
    // Wallpaper redraw while occluded must not stall dock or unscoped rAF.
    host.surface_event(&mut runtime, &cx, wallpaper, SurfaceEvent::RedrawRequested).expect("occluded wallpaper redraw");
    host.surface_event(&mut runtime, &cx, dock, SurfaceEvent::RedrawRequested).expect("redraw visible dock");
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("shell turn after occlusion");
    session.invoke("expectStep2")
        .expect("both migrated unscoped rAF and dock-pinned rAF fire while wallpaper is occluded");

    // 3. Unoccluding wallpaper, scheduling unscoped rAF, and closing wallpaper migrates it to dock instead of cancelling.
    host.surface_event(&mut runtime, &cx, wallpaper, SurfaceEvent::Occluded(false)).expect("unocclude wallpaper");
    session.invoke("scheduleUnscoped").expect("schedule unscoped rAF before wallpaper closes");
    session.invoke("closeWallpaper").expect("close wallpaper layer");
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("reconcile wallpaper close");
    assert!(cx.surface(wallpaper).is_none(), "wallpaper surface must be closed");
    host.surface_event(&mut runtime, &cx, dock, SurfaceEvent::RedrawRequested).expect("redraw remaining dock");
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("shell turn after wallpaper close");
    session.invoke("expectStep3")
        .expect("unscoped rAF survived wallpaper close via migration to dock");

    host.stop_shell_surfaces(&mut runtime, &cx);
    let report = session.retire();
    assert!(report.failure.is_none(), "teardown failed: {:?}", report.failure);
}

#[test]
fn live_portal_reduced_motion_updates_bridges_unless_overridden_by_policy() {
    use zgui::platform::AppHandler;

    const MOTION_APP: &str = r#"
        export const hostApiVersion = 1;
        const log = [];
        export function mount() {
            log.push('init:' + __zgui.getReducedMotion());
            __zgui.subscribeReducedMotion(value => log.push('sub:' + value + ':' + __zgui.getReducedMotion()));
            return () => {};
        }
        export function expectLive() {
            if (log.join(',') !== 'init:false,sub:true:true,sub:false:false') throw Error('live: ' + log.join(','));
        }
        export function expectFalse() {
            if (log.join(',') !== 'init:false') throw Error('false: ' + log.join(','));
        }
        export function expectTrue() {
            if (log.join(',') !== 'init:true') throw Error('true: ' + log.join(','));
        }
    "#;

    let run_case = |explicit_override: Option<bool>, check_export: &str| {
        let package = Fixture::new(MOTION_APP, "");
        let pending = Rc::new(RefCell::new(Vec::new()));
        let roots = pending.clone();
        let fonts = zgui::app::Fonts::shipped_only();
        let mut runtime = zgui::runtime::App::new()
            .with_renderer(Box::new(|_, target| {
                let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
                zgui::render::Renderer::configure(&mut renderer, target);
                Ok(Box::new(renderer))
            }))
            .into_handler(move |cx| {
                zgui::reactive::provide_context(fonts.clone());
                Box::new(Root(roots.clone()).build(cx))
            })
            .expect("create reduced-motion test runtime");
        let cx = zgui_platform_headless::Headless::new();
        runtime.surfaces_available(&cx);
        let mut policy = SessionPolicy::live(host_services::Executor::new(2, 64).expect("executor"));
        if let Some(flag) = explicit_override {
            policy = policy.with_reduced_motion(flag);
        }
        let mut host = SessionHost::new(
            pending,
            SessionModule::package(package.package()),
            policy,
            RecoveryPolicy::Propagate,
            SurfaceOpacity::Opaque,
        );
        host.attach(&mut RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly), cx.waker(), no_retry())
            .expect("mount reduced-motion app");
        let session = host.sessions().next().expect("active session").clone();
        host.sync_reduced_motion(Some(true), &mut RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly))
            .expect("apply portal reduced-motion=true");
        host.sync_reduced_motion(Some(false), &mut RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly))
            .expect("apply portal reduced-motion=false");
        session.invoke(check_export).expect("reduced-motion log matches");
        let report = session.retire();
        assert!(report.failure.is_none(), "teardown failed: {:?}", report.failure);
    };

    run_case(None, "expectLive");
    run_case(Some(false), "expectFalse");
    run_case(Some(true), "expectTrue");
}

#[test]
fn live_font_validation_skips_generic_fallback_stacks_and_deduplicates_missing_families() {
    use zgui::platform::{AppHandler, SurfaceEvent};

    const FONT_APP: &str = r#"
        export const hostApiVersion = 1;
        let label;
        export function mount(root) {
            const box = __zgui.createElement('column');
            const themed = __zgui.createElement('text');
            __zgui.setProperty(themed, 'style', {
                'font-family': 'Geist, ui-sans-serif, system-ui, sans-serif',
            });
            __zgui.insertNode(themed, __zgui.createTextNode('themed'), null);
            const mono = __zgui.createElement('text');
            __zgui.setProperty(mono, 'style', {
                'font-family': '"Geist Mono", ui-monospace, SFMono-Regular, Menlo, monospace',
            });
            __zgui.insertNode(mono, __zgui.createTextNode('mono'), null);
            label = __zgui.createElement('text');
            __zgui.setProperty(label, 'style', { 'font-family': 'TrulyMissingFont' });
            __zgui.insertNode(label, __zgui.createTextNode('v1'), null);
            __zgui.insertNode(box, themed, null);
            __zgui.insertNode(box, mono, null);
            __zgui.insertNode(box, label, null);
            __zgui.insertNode(root, box, null);
            return () => __zgui.clearRoot(root);
        }
        export function bumpDom() {
            __zgui.replaceText(__zgui.getFirstChild(label), 'v2');
        }
    "#;

    let package = Fixture::new(FONT_APP, "");
    let pending = Rc::new(RefCell::new(Vec::new()));
    let roots = pending.clone();
    let fonts = zgui::app::Fonts::shipped_only();
    let mut runtime = zgui::runtime::App::new()
        .with_renderer(Box::new(|_, target| {
            let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
            zgui::render::Renderer::configure(&mut renderer, target);
            Ok(Box::new(renderer))
        }))
        .into_handler(move |cx| {
            zgui::reactive::provide_context(fonts.clone());
            Box::new(Root(roots.clone()).build(cx))
        })
        .expect("create font validation test runtime");
    let cx = zgui_platform_headless::Headless::new();
    runtime.surfaces_available(&cx);
    let policy = SessionPolicy::live(host_services::Executor::new(2, 64).expect("executor"));
    let mut host = SessionHost::new(
        pending,
        SessionModule::package(package.package()),
        policy,
        RecoveryPolicy::Propagate,
        SurfaceOpacity::Opaque,
    );
    host.attach(&mut RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly), cx.waker(), no_retry())
        .expect("mount font test app");
    let surface_id = runtime.windows()[0].surface().id();
    runtime.surface_event(&cx, surface_id, SurfaceEvent::RedrawRequested);
    let session = host.sessions().next().expect("active session").clone();
    session.validate_font_families(&runtime.windows()[0]).expect("live font validation succeeds");
    assert_eq!(
        session.reported_unregistered_fonts(),
        vec!["TrulyMissingFont".to_owned()],
        "fallback stacks ending in CSS generics must not emit diagnostics"
    );

    // Mutate DOM to bump revision and re-validate; missing family stays deduplicated.
    session.invoke("bumpDom").expect("bump DOM revision");
    runtime.surface_event(&cx, surface_id, SurfaceEvent::RedrawRequested);
    session.validate_font_families(&runtime.windows()[0]).expect("second live validation succeeds");
    assert_eq!(session.reported_unregistered_fonts(), vec!["TrulyMissingFont".to_owned()]);

    let report = session.retire();
    assert!(report.failure.is_none(), "teardown failed: {:?}", report.failure);
}
