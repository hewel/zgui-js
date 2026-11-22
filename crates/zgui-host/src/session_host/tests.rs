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

#[test]
fn staged_network_deferral_and_reload_under_sse_flood() {
    use crate::{
        assets::fixture::{FixtureDefinition, FixtureRuntime},
        environment::Environment,
        package::{Capabilities, NetworkCapabilities},
    };
    use std::time::Duration;

    let caps = || Capabilities {
        network: NetworkCapabilities {
            origins: vec!["https://api.example.com".into()],
            allow_insecure_http: false,
            allow_private_networks: false,
        },
        ..Capabilities::default()
    };

    let gen1_pkg = Fixture::new(
        r#"
        export const hostApiVersion = 1;
        let received = 0;
        let closed = false;
        let label;
        export function mount(root) {
            label = __zgui.createElement('text');
            __zgui.insertNode(label, __zgui.createTextNode('g1:0'), null);
            __zgui.insertNode(root, label, null);
            const stream = __zguiServices.openEventStream(
                { url: 'https://api.example.com/flood' },
                {
                    onMessage() {
                        if (closed) throw Error('stale callback delivered to retired generation 1');
                        received += 1;
                        __zgui.replaceText(__zgui.getFirstChild(label), `g1:${received}`);
                    },
                },
            );
            return () => {
                closed = true;
                stream.close();
                __zgui.clearRoot(root);
            };
        }
        "#,
        "",
    )
    .with_capabilities(caps());

    // Gen 2 starts an irreversible HTTP request during mount(), then fails before acceptance!
    let rejected_pkg = Fixture::new(
        r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            __zguiServices.httpRequest({ url: 'https://api.example.com/must-not-send' });
            throw Error('candidate mount aborted');
        }
        "#,
        "",
    )
    .with_capabilities(caps());

    // Gen 3 accepts cleanly under the same flood and verifies timers still fire fairly.
    let gen3_pkg = Fixture::new(
        r#"
        export const hostApiVersion = 1;
        let received = 0;
        let timerFired = 0;
        let label;
        export function mount(root) {
            label = __zgui.createElement('text');
            __zgui.insertNode(label, __zgui.createTextNode('g3:0:t0'), null);
            __zgui.insertNode(root, label, null);
            const stream = __zguiServices.openEventStream(
                { url: 'https://api.example.com/flood' },
                {
                    onMessage() {
                        received += 1;
                        __zgui.replaceText(__zgui.getFirstChild(label), `g3:${received}:t${timerFired}`);
                    },
                },
            );
            const t1 = setTimeout(() => {
                timerFired += 1;
                __zgui.replaceText(__zgui.getFirstChild(label), `g3:${received}:t${timerFired}`);
            }, 5);
            return () => {
                clearTimeout(t1);
                stream.close();
                __zgui.clearRoot(root);
            };
        }
        "#,
        "",
    )
    .with_capabilities(caps());

    let flood_events: Vec<serde_json::Value> = (1..=60)
        .map(|i| serde_json::json!({ "kind": "message", "atMs": i / 4, "data": format!("m{i}") }))
        .collect();
    let fixture_def: FixtureDefinition = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "id": "network-reload-flood",
        "services": {
            "network": {
                "http": [],
                "sse": [{
                    "url": "https://api.example.com/flood",
                    "repeat": true,
                    "atMs": 0,
                    "events": flood_events
                }]
            }
        },
        "events": []
    }))
    .expect("decode fixture");

    let cx = zgui_platform_headless::Headless::new();
    let clock = cx.clock();
    let env = Environment::new(clock.clone(), Some(1_700_000_000_000), 7);
    let fixture_rt = FixtureRuntime::new(fixture_def, env.clone());
    let deadline = Rc::new(Cell::new(std::time::Instant::now() + Duration::from_secs(30)));
    let policy = SessionPolicy::preview(
        js_runtime::RuntimeLimits::default(),
        deadline,
        env.clone(),
        fixture_rt.clone(),
    );

    let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
    let dom = DomHandle::from_rc(backend.clone());
    let root = dom.create_element(ElementName::new("column"));
    dom.insert(backend.root_node(), root, None);
    let stub = Rc::new(StubHost::default());
    let region = MountRegion {
        document: backend.document_id(),
        root,
        dom,
        host: HostHandle::from_rc(stub.clone()),
        fonts: zgui::app::Fonts::shipped_only(),
        root_anchor: Rc::new(Cell::new(root)),
    };
    let pending = Rc::new(RefCell::new(vec![region.clone()]));
    let mut surface = StubSurface {
        document: region.document,
        bindings: HashMap::new(),
        images: None,
        frames: RefCell::new(Vec::new()),
    };
    let mut host = SessionHost::new(
        pending,
        SessionModule::package(gen1_pkg.package()),
        policy,
        RecoveryPolicy::Propagate,
        SurfaceOpacity::Opaque,
    );
    host.attach(&mut surface, cx.waker(), no_retry()).expect("mount gen 1");
    let gen1 = host.session(region.document).expect("gen1 session").clone();

    // Advance 5ms so gen1 starts receiving flood events.
    cx.virtual_clock().advance(Duration::from_millis(5));
    fixture_rt.release_due().expect("release gen1 flood events");
    while gen1.checkpoint().expect("pump gen1") {}

    // Attempt staged reload with Gen 2: it calls httpRequest then throws.
    // Because staged outbound ops are deferred until acceptance, ZERO HTTP requests hit the fixture!
    let outcome = host.replace(
        &mut surface,
        cx.waker(),
        region.document,
        SessionModule::package(rejected_pkg.package()),
        no_retry(),
    );
    assert!(matches!(outcome, ReloadOutcome::Rejected(_)), "gen2 must be rejected");
    fixture_rt
        .assert_calls("network", "httpRequest", 0, None)
        .expect("rejected candidate must leak zero outbound HTTP calls to fixture");
    assert!(fixture_rt.contract_failure().is_none(), "rejected candidate must not trigger unmatched fixture failure");

    // Reload to Gen 3: Gen 1 retires cleanly and Gen 3 commits its staged SSE stream.
    let outcome = host.replace(
        &mut surface,
        cx.waker(),
        region.document,
        SessionModule::package(gen3_pkg.package()),
        no_retry(),
    );
    assert!(matches!(outcome, ReloadOutcome::Mounted { .. }), "gen3 must mount cleanly");
    assert_eq!(gen1.counts(), ResourceCounts::default(), "gen1 must retire cleanly");

    let gen3 = host.session(region.document).expect("gen3 session").clone();
    // Advance virtual clock across the full 60-event flood and fire StubHost timers.
    for _ in 0..25 {
        cx.virtual_clock().advance(Duration::from_millis(1));
        stub.advance(Duration::from_millis(1));
        fixture_rt.release_due().expect("release gen3 flood events");
        while gen3.checkpoint().expect("pump gen3") {}
    }
    let text = region.dom.text_content(region.root_anchor.get());
    assert_eq!(text, "g3:60:t1", "gen3 must receive all 60 SSE events and fire timer fairly under flood");
    assert!(fixture_rt.contract_failure().is_none());

    let report = gen3.retire();
    assert!(report.failure.is_none(), "gen3 teardown failed: {:?}", report.failure);
    assert_eq!(report.counts, ResourceCounts::default());
}

#[test]
fn dev_state_never_snapshots_locked_generations_and_hydrates_unlocked_reloads() {
    let source = r#"
        export const hostApiVersion = 1;
        export function mount(root) {
            const hydrated = globalThis.__zguiDevState.read("secret", 1) ?? "cold";
            const label = __zgui.createElement("label");
            const text = __zgui.createTextNode(hydrated);
            __zgui.insertNode(label, text, null);
            __zgui.insertNode(root, label, null);
            globalThis.__zguiDevState.write("secret", JSON.stringify("persisted-value"), 1);
            return () => {};
        }
    "#;
    let module = SessionModule::inline("dev-lock-test", source, vec![]);
    let cx = zgui_platform_headless::Headless::new();
    let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
    let dom = DomHandle::from_rc(backend.clone());
    let root = dom.create_element(ElementName::new("column"));
    dom.insert(backend.root_node(), root, None);
    let stub = Rc::new(StubHost::default());
    let region = MountRegion {
        document: backend.document_id(),
        root,
        dom,
        host: HostHandle::from_rc(stub),
        fonts: zgui::app::Fonts::shipped_only(),
        root_anchor: Rc::new(Cell::new(root)),
    };
    let pending = Rc::new(RefCell::new(vec![region.clone()]));
    let policy = SessionPolicy::live(host_services::Executor::new(1, 16).expect("executor"))
        .with_dev_state(true);
    let mut host = SessionHost::new(
        pending,
        module.clone(),
        policy,
        RecoveryPolicy::Propagate,
        SurfaceOpacity::Opaque,
    );
    let mut surface = StubSurface {
        document: region.document,
        bindings: HashMap::new(),
        images: None,
        frames: RefCell::new(Vec::new()),
    };
    let waker = cx.waker();

    host.attach(&mut surface, waker.clone(), no_retry()).expect("initial mount");
    assert_eq!(region.dom.text_content(region.root_anchor.get()), "cold");

    // Unlocked reload hydrates persisted plain data cleanly.
    let outcome = host.replace(&mut surface, waker.clone(), region.document, module.clone(), no_retry());
    assert!(matches!(outcome, ReloadOutcome::Mounted { .. }));
    assert_eq!(region.dom.text_content(region.root_anchor.get()), "\"persisted-value\"");

    // Locking the session blocks snapshot capture across reload (forces clean cold start).
    let live = host.session(region.document).expect("live session").clone();
    let _ = live.update_lock_state(SessionLockState::Locked);
    assert!(live.capture_dev_state(&module).is_none(), "locked generation must never snapshot dev state");
    let outcome = host.replace(&mut surface, waker, region.document, module, no_retry());
    assert!(matches!(outcome, ReloadOutcome::Mounted { .. }));
    assert_eq!(region.dom.text_content(region.root_anchor.get()), "cold", "reloading from locked generation must cold-mount");

    host.stop(&mut surface);
}

#[test]
fn css_only_live_reload_preserves_session_generation_and_swaps_stylesheets() {
    let first = Fixture::new(SIMPLE, "text { color: rgb(255, 0, 0); }");
    let css_updated = Fixture::new(SIMPLE, "text { color: rgb(0, 180, 0); }");
    let bad_css = Fixture::new(SIMPLE, "text { background: url(https://evil.invalid/x.png); }");
    let bad_font_css = Fixture::new(SIMPLE, "text { font-family: UnregisteredDevFont; }");
    let js_updated_src = SIMPLE.replace("active", "updated-js");
    let js_updated = Fixture::new(&js_updated_src, "text { color: rgb(0, 0, 255); }");

    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(first.package()));
    native.host.set_dev_state(true);
    native.attach();
    let initial = native.session();
    let document = native.region.document;
    assert_eq!(initial.generation, 1);
    assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text { color: rgb(255, 0, 0); }"));

    // CSS-only successor swaps stylesheets in place without bumping generation or restarting QuickJS.
    let outcome = native.host.replace(
        &mut native.surface,
        native.waker.clone(),
        document,
        SessionModule::package(css_updated.package()),
        no_retry(),
    );
    assert!(
        matches!(outcome, ReloadOutcome::Mounted { generation: 1, predecessor_leak: None, css_only: true }),
        "expected CSS-only in-place swap, got {outcome:?}"
    );
    assert!(Rc::ptr_eq(&initial, &native.session()), "CSS-only reload must keep the exact live JS Session");
    assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text { color: rgb(0, 180, 0); }"));
    assert_eq!(native.host.acceptance().unwrap()["cssOnly"], true);

    // Invalid CSS and unregistered fonts fail closed and keep the active green stylesheet + live session.
    for invalid in [bad_css.package(), bad_font_css.package()] {
        let rejected = native.host.replace(
            &mut native.surface,
            native.waker.clone(),
            document,
            SessionModule::package(invalid),
            no_retry(),
        );
        assert!(matches!(rejected, ReloadOutcome::Rejected(_)), "bad CSS must reject fail-closed: {rejected:?}");
        assert!(Rc::ptr_eq(&initial, &native.session()));
        assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text { color: rgb(0, 180, 0); }"));
    }

    // JS change triggers normal staged generation reload.
    let js_outcome = native.host.replace(
        &mut native.surface,
        native.waker.clone(),
        document,
        SessionModule::package(js_updated.package()),
        no_retry(),
    );
    assert!(
        matches!(js_outcome, ReloadOutcome::Mounted { css_only: false, .. }),
        "JS change must trigger full staged reload: {js_outcome:?}"
    );
    assert!(!Rc::ptr_eq(&initial, &native.session()));
    assert_eq!(native.text(), "updated-js");
    assert_eq!(native.stub.stylesheet("application:0").as_deref(), Some("text { color: rgb(0, 0, 255); }"));
}

#[test]
fn dev_banner_respects_dev_flag_and_session_lock_and_preserves_healthy_tree() {
    let good = Fixture::new(SIMPLE, "text { color: red }");
    let mut native = Native::new(RecoveryPolicy::Fallback, SessionModule::package(good.package()));
    native.attach();
    let document = native.region.document;

    // Outside dev --watch mode, banner is strictly suppressed.
    native.host.show_dev_banner(&mut native.surface, document, "Build failed", "src/main.tsx:1:1");
    assert!(!native.host.has_dev_banner(document));
    assert!(native.stub.stylesheet(dev_banner::SHEET_NAME).is_none());

    // In dev --watch mode, banner renders alongside healthy content and clears cleanly.
    native.host.set_dev_banner_enabled(true);
    native.host.show_dev_banner(&mut native.surface, document, "Build failed", "src/main.tsx:1:1");
    assert!(native.host.has_dev_banner(document));
    assert!(native.stub.stylesheet(dev_banner::SHEET_NAME).is_some());
    assert_eq!(native.text(), "active", "healthy app tree must stay intact");

    // Repeated identical error keeps the banner mounted without losing its stylesheet.
    native.host.show_dev_banner(&mut native.surface, document, "Build failed", "src/main.tsx:1:1");
    assert!(native.host.has_dev_banner(document));
    assert!(native.stub.stylesheet(dev_banner::SHEET_NAME).is_some());

    // Clean recovery clears the banner and removes its stylesheet.
    native.host.clear_dev_banner(&mut native.surface, document);
    assert!(!native.host.has_dev_banner(document));
    assert!(native.stub.stylesheet(dev_banner::SHEET_NAME).is_none());

    // Session lock surface strictly suppresses the banner even in dev mode.
    struct LockedSurface<'a>(&'a mut StubSurface);
    impl SessionSurface for LockedSurface<'_> {
        type Checkpoint = ();
        type MountWrites = ();
        fn surface(&self, d: DocumentId) -> Option<SurfaceId> { self.0.surface(d) }
        fn bind(&mut self, d: DocumentId, s: &Rc<Session>) { self.0.bind(d, s); }
        fn image_source_checkpoint(&mut self, d: DocumentId) { self.0.image_source_checkpoint(d); }
        fn seal_image_source_writes(&self, d: DocumentId, c: ()) { self.0.seal_image_source_writes(d, c); }
        fn discard_image_source_writes(&mut self, d: DocumentId, c: ()) { self.0.discard_image_source_writes(d, c); }
        fn commit_image_policy(&mut self, d: DocumentId, s: &Session, w: ()) { self.0.commit_image_policy(d, s, w); }
        fn request_frame(&self, d: DocumentId) { self.0.request_frame(d); }
        fn session_lock_support(&self) -> Option<SessionLockState> { Some(SessionLockState::Locked) }
        fn close_shell_surface(&mut self, t: WindowToken, id: Option<SurfaceId>) -> bool { self.0.close_shell_surface(t, id) }
        fn stage_shell_surfaces(&mut self, st: &mut ShellWindows, p: &ShellWindows, m: &[Monitor], s: &Rc<Session>) -> SessionResult<()> {
            self.0.stage_shell_surfaces(st, p, m, s)
        }
    }
    let mut locked = LockedSurface(&mut native.surface);
    native.host.show_dev_banner(&mut locked, document, "Build failed", "must never appear on lock screen");
    assert!(!native.host.has_dev_banner(document));
    assert!(native.stub.stylesheet(dev_banner::SHEET_NAME).is_none());
}

#[test]
fn auto_to_auto_popup_anchor_update_with_geometry_insets_does_not_inflate_bleed() {
    use zgui::geom::{DevicePx, Size};
    use zgui::platform::{AppHandler, SurfaceEvent};
    use zgui::view::IntoView;

    const POPUP_INSET_APP: &str = r#"
        export const hostApiVersion = 1;
        let popId;
        const base = {
            parent: 'bar', width: 320, height: 'auto', minHeight: 80, maxHeight: 400,
            anchor: 'bottom-left', gravity: 'bottom-right', offset: { x: 0, y: 0 },
            slide: true, flip: true, resize: false, reactive: false, grab: false,
            geometryInsets: { top: 28, right: 28, bottom: 28, left: 28 },
        };
        export function mount() {
            const placement = {
                layer: 'top', anchor: ['top', 'left'], width: 400, height: 40,
                margin: { top: 0, right: 0, bottom: 0, left: 0 }, exclusiveZone: 0, keyboard: 'none',
            };
            __zguiShell.openLayer('bar', placement, () => () => {});
            popId = __zguiShell.openPopup(
                'card',
                { ...base, anchorRect: { x: 10, y: 10, width: 40, height: 24 } },
                (root, bridge) => {
                    const bleed = bridge.createElement('column');
                    bridge.setProperty(bleed, 'style', { padding: '28px', boxSizing: 'border-box' });
                    const body = bridge.createElement('column');
                    bridge.setProperty(body, 'style', { height: '140px' });
                    bridge.insertNode(bleed, body, null);
                    bridge.insertNode(root, bleed, null);
                    return () => {};
                },
                () => {},
            );
            return () => {};
        }
        export function moveAnchor() {
            __zguiShell.updatePopup(popId, {
                ...base,
                anchorRect: { x: 80, y: 10, width: 40, height: 24 },
            });
        }
    "#;

    let package = Fixture::new(POPUP_INSET_APP, "").with_capabilities(crate::package::Capabilities {
        shell_surfaces: true,
        ..Default::default()
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
        .expect("create pure-shell runtime");
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
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("mount shell");
    let session = host.sessions().next().expect("session").clone();
    let popup_surface = session
        .mount_regions()
        .into_iter()
        .find(|(name, _)| name == "card")
        .expect("card popup")
        .1
        .surface;

    // Frame 1 settles auto-height to visual 140px -> drawable (320+56)x(140+56) = 376x196.
    host.surface_event(&mut runtime, &cx, popup_surface, SurfaceEvent::RedrawRequested)
        .expect("settle popup auto-height");
    assert_eq!(
        cx.surface(popup_surface).expect("popup").size(),
        Size::new(DevicePx(376.0), DevicePx(196.0)),
    );

    // Anchor-only auto-to-auto update must preserve visual 140px (drawable 376x196), NOT inflate to 196+56=252!
    session.invoke("moveAnchor").expect("update popup anchor");
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("reconcile anchor move");
    assert_eq!(
        cx.surface(popup_surface).expect("popup").size(),
        Size::new(DevicePx(376.0), DevicePx(196.0)),
        "anchor-only update must subtract existing vertical bleed before clamping visual height"
    );

    host.stop_shell_surfaces(&mut runtime, &cx);
    let report = session.retire();
    assert!(report.failure.is_none(), "teardown failed: {:?}", report.failure);
}

#[test]
fn background_blur_capability_seeds_and_syncs_across_zero_surface_shell_and_mounted_layers() {
    use zgui::platform::{AppHandler, PlatformCapabilities, WakeReason};
    use zgui::view::IntoView;

    const BLUR_CAP_APP: &str = r#"
        export const hostApiVersion = 1;
        const events = [];
        let unsubShell;
        let unsubLayer;
        let layerId;
        export function mount() {
            events.push('init:' + __zguiShell.getBackgroundBlurSupported());
            unsubShell = __zguiShell.subscribeBackgroundBlurSupported(v => {
                events.push('shell:' + v);
            });
            return () => {
                if (unsubLayer) unsubLayer();
                if (unsubShell) unsubShell();
                if (layerId !== undefined) __zguiShell.closeLayer(layerId);
            };
        }
        export function openBar() {
            const placement = {
                layer: 'top', anchor: ['top', 'left'], width: 160, height: 32,
                margin: { top: 0, right: 0, bottom: 0, left: 0 }, exclusiveZone: 0, keyboard: 'none',
            };
            layerId = __zguiShell.openLayer('bar', placement, (_root, bridge) => {
                events.push('layerInit:' + bridge.getBackgroundBlurSupported());
                unsubLayer = bridge.subscribeBackgroundBlurSupported(v => {
                    events.push('layer:' + v);
                });
                return () => {
                    if (unsubLayer) { unsubLayer(); unsubLayer = undefined; }
                };
            });
        }
        function assertEvents(expected) {
            const actual = events.join(',');
            if (actual !== expected) throw Error('expected ' + expected + ', got ' + actual);
        }
        export function checkInit() {
            assertEvents('init:false');
        }
        export function checkZeroSurfaceTrue() {
            assertEvents('init:false,shell:true');
        }
        export function checkLayerSeededTrue() {
            assertEvents('init:false,shell:true,layerInit:true');
        }
        export function checkBothFalse() {
            assertEvents('init:false,shell:true,layerInit:true,shell:false,layer:false');
        }
    "#;

    let package = Fixture::new(BLUR_CAP_APP, "").with_capabilities(crate::package::Capabilities {
        shell_surfaces: true,
        ..Default::default()
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
        .expect("create pure-shell runtime");
    runtime.without_primary_surface();
    let mut caps = PlatformCapabilities::none();
    caps.supports_background_blur = false;
    let mut cx = zgui_platform_headless::Headless::new().with_capabilities(caps);
    runtime.surfaces_available(&cx);

    let mut host = SessionHost::new(
        pending,
        SessionModule::package(package.package()),
        SessionPolicy::live(host_services::Executor::new(2, 64).expect("create service executor")),
        RecoveryPolicy::Propagate,
        SurfaceOpacity::Transparent,
    );
    host.set_pure_shell(fonts);
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("mount zero-surface pure shell");
    let session = host.sessions().next().expect("active pure-shell session").clone();
    session.invoke("checkInit").expect("seeded false at zero surfaces");

    // Toggle false -> true while still at zero surfaces: Headless wakes AppWork, shell_turn delivers live!
    cx.set_background_blur_supported(true);
    assert!(cx.drain_wakes().iter().any(|wake| matches!(wake, WakeReason::AppWork)));
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("sync true at zero surfaces");
    session.invoke("checkZeroSurfaceTrue").expect("zero-surface shell received true");

    // Now open first LayerSurface: its UiBridge must inherit current true!
    session.invoke("openBar").expect("request layer");
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("mount bar layer");
    session.invoke("checkLayerSeededTrue").expect("layer seeded true");

    // Toggle true -> false: both shell and mounted layer bridge receive false.
    cx.set_background_blur_supported(false);
    assert!(cx.drain_wakes().iter().any(|wake| matches!(wake, WakeReason::AppWork)));
    host.shell_turn(&mut runtime, &cx, ImageChoice::PackageOnly, &[]).expect("sync false across shell and layer");
    session.invoke("checkBothFalse").expect("shell and layer received false");

    host.stop_shell_surfaces(&mut runtime, &cx);
    let report = session.retire();
    assert!(report.failure.is_none(), "teardown failed: {:?}", report.failure);
    assert_eq!(report.counts.callbacks, 0, "all blur subscriptions must clean up with zero leaks");
}

#[test]
fn aria_active_descendant_accesskit_initial_incremental_unmount_and_remount() {
    use accesskit_consumer::{Node, NodeId as ConsumerNodeId, Tree, TreeChangeHandler};
    use zgui::platform::{AppHandler, PlatformCx, Surface, SurfaceEvent};
    use zgui::view::{IntoView, View};

    struct IgnoreChanges;
    impl TreeChangeHandler for IgnoreChanges {
        fn node_added(&mut self, _: &Node) {}
        fn node_updated(&mut self, _: &Node, _: &Node) {}
        fn focus_moved(&mut self, _: Option<&Node>, _: Option<&Node>) {}
        fn node_removed(&mut self, _: &Node) {}
    }

    const APP_JS: &str = r#"
        export const hostApiVersion = 1;
        let combo, list, opt1, opt2, opt3;
        export function mount(root) {
            combo = __zgui.createElement('field');
            __zgui.setProperty(combo, 'role', 'comboBox');
            __zgui.setProperty(combo, 'ariaLabel', 'Fruit');
            __zgui.setProperty(combo, 'expanded', true);
            __zgui.setProperty(combo, 'style', 'width: 160px; height: 24px');

            list = __zgui.createElement('column');
            __zgui.setProperty(list, 'role', 'listBox');
            __zgui.setProperty(list, 'ariaLabel', 'Fruit options');
            __zgui.setProperty(list, 'style', 'width: 160px; height: 96px');

            opt1 = __zgui.createElement('control');
            __zgui.setProperty(opt1, 'role', 'listBoxOption');
            __zgui.setProperty(opt1, 'ariaLabel', 'Apple');
            __zgui.setProperty(opt1, 'style', 'width: 160px; height: 24px');

            opt2 = __zgui.createElement('control');
            __zgui.setProperty(opt2, 'role', 'listBoxOption');
            __zgui.setProperty(opt2, 'ariaLabel', 'Pear');
            __zgui.setProperty(opt2, 'style', 'width: 160px; height: 24px');

            __zgui.insertNode(list, opt1, null);
            __zgui.insertNode(list, opt2, null);
            __zgui.insertNode(root, combo, null);
            __zgui.insertNode(root, list, null);

            __zgui.setProperty(combo, 'ariaControls', list);
            __zgui.setProperty(combo, 'ariaActiveDescendant', opt1);
            __zgui.focus(combo);

            return () => __zgui.clearRoot(root);
        }
        export function selectSecond() {
            __zgui.setProperty(combo, 'ariaActiveDescendant', opt2);
        }
        export function clearActive() {
            __zgui.setProperty(combo, 'ariaActiveDescendant', null);
        }
        export function unmountSecond() {
            __zgui.removeNode(list, opt2);
        }
        export function remountThird() {
            opt3 = __zgui.createElement('control');
            __zgui.setProperty(opt3, 'role', 'listBoxOption');
            __zgui.setProperty(opt3, 'ariaLabel', 'Mango');
            __zgui.setProperty(opt3, 'style', 'width: 160px; height: 24px');
            __zgui.insertNode(list, opt3, null);
            __zgui.setProperty(combo, 'ariaActiveDescendant', opt3);
        }
    "#;

    let package = Fixture::new(APP_JS, "");
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
            Box::new(Root(roots.clone()).into_view().build(cx))
        })
        .expect("create runtime");
    let cx = zgui_platform_headless::Headless::new();
    runtime.surfaces_available(&cx);
    let offscreen = cx.offscreens()[0].clone();
    offscreen.set_a11y_listener(true);
    let surface_id = offscreen.id();
    runtime.surface_event(&cx, surface_id, SurfaceEvent::Focused(true));

    let mut host = SessionHost::new(
        pending,
        SessionModule::package(package.package()),
        SessionPolicy::live(host_services::Executor::new(2, 64).expect("create executor")),
        RecoveryPolicy::Propagate,
        SurfaceOpacity::Opaque,
    );
    {
        let mut surface = RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly);
        host.attach(&mut surface, cx.waker(), no_retry()).expect("mount combobox app");
    }
    let session = host.sessions().next().expect("active session").clone();

    let mut tree: Option<Tree> = None;
    let pump = |runtime: &mut zgui::runtime::Runtime, tree: &mut Option<Tree>| -> usize {
        let mut applied = 0;
        for _ in 0..4 {
            runtime.surface_event(&cx, surface_id, SurfaceEvent::RedrawRequested);
            for update in offscreen.take_a11y_log() {
                applied += 1;
                if let Some(existing) = tree.as_mut() {
                    existing.update_and_process_changes(update, &mut IgnoreChanges);
                } else {
                    *tree = Some(Tree::new(update, true));
                }
            }
        }
        applied
    };

    let find_by_name = |tree: &Tree, name: &str| -> Option<ConsumerNodeId> {
        let state = tree.state();
        let mut stack = vec![state.root_id()];
        while let Some(id) = stack.pop() {
            if let Some(node) = state.node_by_id(id) {
                if node.data().label() == Some(name) {
                    return Some(id);
                }
                stack.extend(node.child_ids());
            }
        }
        None
    };

    let active_descendant_of = |tree: &Tree, id: ConsumerNodeId| -> Option<ConsumerNodeId> {
        tree.state().node_by_id(id)?.active_descendant().map(|node| node.id())
    };

    // 1. Initial AccessKit tree: combo is focused and points active_descendant at sibling Apple.
    assert!(pump(&mut runtime, &mut tree) > 0, "initial frame must publish AccessKit tree");
    let t = tree.as_ref().expect("tree built");
    let combo_id = find_by_name(t, "Fruit").expect("combo node present");
    let apple_id = find_by_name(t, "Apple").expect("Apple option present");
    let pear_id = find_by_name(t, "Pear").expect("Pear option present");
    assert_eq!(t.state().focus_id(), Some(combo_id), "native focus must be on combobox");
    assert_eq!(
        active_descendant_of(t, combo_id),
        Some(apple_id),
        "initial AccessKit tree must expose active_descendant = Apple"
    );

    // 2. Incremental update: switch active_descendant from Apple -> Pear without moving native focus.
    session.invoke("selectSecond").expect("switch to Pear");
    assert!(pump(&mut runtime, &mut tree) > 0, "incremental switch must emit AccessKit update");
    let t = tree.as_ref().unwrap();
    assert_eq!(t.state().focus_id(), Some(combo_id), "native focus must stay on combobox");
    assert_eq!(
        active_descendant_of(t, combo_id),
        Some(pear_id),
        "incremental update must switch active_descendant to Pear"
    );

    // 3. Incremental clear: null clears active_descendant without moving focus.
    session.invoke("clearActive").expect("clear active_descendant");
    assert!(pump(&mut runtime, &mut tree) > 0, "clearing must emit AccessKit update");
    let t = tree.as_ref().unwrap();
    assert_eq!(t.state().focus_id(), Some(combo_id), "native focus must stay on combobox");
    assert_eq!(
        active_descendant_of(t, combo_id),
        None,
        "null must clear active_descendant"
    );

    // 4. Point to Pear again, then unmount Pear while combo still references it: consumer must see zero dangling IDs.
    session.invoke("selectSecond").expect("re-select Pear");
    assert!(pump(&mut runtime, &mut tree) > 0);
    assert_eq!(active_descendant_of(tree.as_ref().unwrap(), combo_id), Some(pear_id));
    session.invoke("unmountSecond").expect("unmount Pear");
    assert!(pump(&mut runtime, &mut tree) > 0, "unmounting referenced option must emit clean AccessKit update");
    let t = tree.as_ref().unwrap();
    assert_eq!(t.state().focus_id(), Some(combo_id), "native focus must stay on combobox after option unmount");
    assert_eq!(
        active_descendant_of(t, combo_id),
        None,
        "unmounted active_descendant must be pruned so AccessKit has no dangling relation"
    );
    assert_eq!(find_by_name(t, "Pear"), None, "unmounted Pear must leave tree");

    // 5. Remount a new option (Mango) and point active_descendant to it.
    session.invoke("remountThird").expect("remount Mango");
    assert!(pump(&mut runtime, &mut tree) > 0, "remounting option must emit incremental AccessKit update");
    let t = tree.as_ref().unwrap();
    let mango_id = find_by_name(t, "Mango").expect("Mango option present");
    assert_eq!(t.state().focus_id(), Some(combo_id), "native focus must still stay on combobox");
    assert_eq!(
        active_descendant_of(t, combo_id),
        Some(mango_id),
        "incremental update must expose remounted Mango as active_descendant"
    );

    let report = session.retire();
    assert!(report.failure.is_none(), "teardown failed: {:?}", report.failure);
}

#[test]
fn ime_preedit_protects_keys_and_escape_dismiss_until_commit() {
    use zgui::platform::{AppHandler, PlatformCx, Surface, SurfaceEvent};
    use zgui::view::{IntoView, View};
    use zgui::vocab::{ImeEvent, Key, KeyCode, KeyEvent, KeyLocation, KeyState, Modifiers, NamedKey, PhysicalKey, Timestamp};

    const IME_APP_JS: &str = r#"
        export const hostApiVersion = 1;
        let overlay, field;
        const observed = [];
        const actions = [];
        const dismisses = [];

        export function mount(root) {
            overlay = __zgui.createElement('box');
            __zgui.setProperty(overlay, 'style', 'width: 300px; height: 200px');
            field = __zgui.createElement('field');
            __zgui.setProperty(field, 'style', 'width: 240px; height: 32px');
            __zgui.insertNode(overlay, field, null);
            __zgui.insertNode(root, overlay, null);

            __zgui.setProperty(field, 'onKeyDown', e => {
                observed.push(['down', e.key, e.isComposing]);
                if (e.isComposing) return;
                actions.push(e.key);
            });
            __zgui.setProperty(field, 'onKeyUp', e => {
                observed.push(['up', e.key, e.isComposing]);
            });
            const dismissLease = __zgui.registerDismissLayer(
                overlay,
                reason => dismisses.push(reason),
                { layer: 'modal', dismissOnOutsidePress: false, dismissOnEscape: true, exclude: null, leaving: false }
            );
            __zgui.focus(field);
            return () => {
                dismissLease.dispose();
                __zgui.clearRoot(root);
            };
        }
        export function assertDuringPreedit() {
            if (actions.length !== 0) throw Error('expected 0 guarded actions during preedit, got ' + JSON.stringify(actions));
            if (dismisses.length !== 0) throw Error('expected 0 dismisses during preedit, got ' + JSON.stringify(dismisses));
            if (observed.length !== 8) throw Error('expected 8 keydown/keyup events during preedit, got ' + JSON.stringify(observed));
            for (const [kind, key, composing] of observed) {
                if (composing !== true) throw Error('expected isComposing=true for ' + kind + ' ' + key);
            }
        }
        export function assertAfterCommitEnter() {
            if (actions.length !== 1 || actions[0] !== 'Enter') {
                throw Error('expected Enter to activate once after commit, got ' + JSON.stringify(actions));
            }
            if (dismisses.length !== 0) {
                throw Error('expected no dismiss after Enter, got ' + JSON.stringify(dismisses));
            }
            const lastTwo = observed.slice(-2);
            if (lastTwo.length !== 2 || lastTwo[0][2] !== false || lastTwo[1][2] !== false) {
                throw Error('expected isComposing=false after commit, got ' + JSON.stringify(lastTwo));
            }
        }
        export function assertAfterOrdinaryEscape() {
            if (dismisses.length !== 1 || dismisses[0] !== 'escape') {
                throw Error('expected single escape dismiss after commit, got ' + JSON.stringify(dismisses));
            }
        }
    "#;

    let package = Fixture::new(IME_APP_JS, "");
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
            Box::new(Root(roots.clone()).into_view().build(cx))
        })
        .expect("create runtime");
    let cx = zgui_platform_headless::Headless::new();
    runtime.surfaces_available(&cx);
    let surface_id = cx.offscreens()[0].id();
    runtime.surface_event(&cx, surface_id, SurfaceEvent::Focused(true));

    let mut host = SessionHost::new(
        pending,
        SessionModule::package(package.package()),
        SessionPolicy::live(host_services::Executor::new(2, 64).expect("create executor")),
        RecoveryPolicy::Propagate,
        SurfaceOpacity::Opaque,
    );
    {
        let mut surface = RuntimeSurface::new(&mut runtime, &cx, ImageChoice::PackageOnly);
        host.attach(&mut surface, cx.waker(), no_retry()).expect("mount IME app");
    }
    let session = host.sessions().next().expect("active session").clone();

    let pump = |runtime: &mut zgui::runtime::Runtime, host: &mut SessionHost| {
        runtime.surface_event(&cx, surface_id, SurfaceEvent::RedrawRequested);
        let mut surface = RuntimeSurface::new(runtime, &cx, ImageChoice::PackageOnly);
        host.checkpoint(&mut surface, no_retry()).expect("checkpoint session");
        runtime.surface_event(&cx, surface_id, SurfaceEvent::RedrawRequested);
    };
    pump(&mut runtime, &mut host);

    let send_key = |runtime: &mut zgui::runtime::Runtime, host: &mut SessionHost, event: KeyEvent| {
        for state in [KeyState::Pressed, KeyState::Released] {
            runtime.surface_event(
                &cx,
                surface_id,
                SurfaceEvent::Key {
                    state,
                    event: event.clone(),
                    modifiers: Modifiers::NONE,
                    timestamp: Timestamp::ORIGIN,
                },
            );
            pump(runtime, host);
        }
    };

    // Start IME preedit on focused field.
    runtime.surface_event(&cx, surface_id, SurfaceEvent::Ime(ImeEvent::Enabled));
    runtime.surface_event(
        &cx,
        surface_id,
        SurfaceEvent::Ime(ImeEvent::Preedit {
            text: "にほん".into(),
            cursor: Some(9..9),
        }),
    );
    pump(&mut runtime, &mut host);

    // During preedit: ArrowDown, Space, Enter, Escape must all carry isComposing === true,
    // must not trigger guarded JS actions, and Escape must NOT dismiss the modal overlay.
    send_key(&mut runtime, &mut host, KeyEvent::named(NamedKey::ArrowDown, PhysicalKey::Code(KeyCode::ArrowDown)));
    send_key(
        &mut runtime,
        &mut host,
        KeyEvent {
            key: Key::Character(" ".into()),
            key_without_modifiers: Key::Character(" ".into()),
            physical: PhysicalKey::Code(KeyCode::Space),
            location: KeyLocation::Standard,
            repeat: false,
        },
    );
    send_key(&mut runtime, &mut host, KeyEvent::named(NamedKey::Enter, PhysicalKey::Code(KeyCode::Enter)));
    send_key(&mut runtime, &mut host, KeyEvent::named(NamedKey::Escape, PhysicalKey::Code(KeyCode::Escape)));
    session.invoke("assertDuringPreedit").expect("preedit protected keys and Escape dismiss");

    // Commit IME text; subsequent ordinary Enter activates exactly once with isComposing === false.
    runtime.surface_event(&cx, surface_id, SurfaceEvent::Ime(ImeEvent::Commit("日本".into())));
    pump(&mut runtime, &mut host);

    send_key(&mut runtime, &mut host, KeyEvent::named(NamedKey::Enter, PhysicalKey::Code(KeyCode::Enter)));
    session.invoke("assertAfterCommitEnter").expect("post-commit Enter activated once");

    // Subsequent ordinary Escape dismisses the modal overlay once.
    send_key(&mut runtime, &mut host, KeyEvent::named(NamedKey::Escape, PhysicalKey::Code(KeyCode::Escape)));
    session.invoke("assertAfterOrdinaryEscape").expect("post-commit Escape dismissed overlay");

    let report = session.retire();
    assert!(report.failure.is_none(), "teardown failed: {:?}", report.failure);
    assert_eq!(report.counts.callbacks, 0, "all listeners cleaned up");
}
