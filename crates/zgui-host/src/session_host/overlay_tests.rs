use super::*;
use crate::{
    package::Capabilities,
    session::SessionModule,
    test_package::Fixture,
};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs,
    path::PathBuf,
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
use zgui::{
    platform::{SessionLockState, SurfaceId, WakeReason, Waker},
    view::stub::StubHost,
};
use zgui_dom::Document;
use zgui_view_dom::DocumentDom;

static SEQ: AtomicU64 = AtomicU64::new(1);

struct NoopWaker(mpsc::Sender<WakeReason>);
impl Waker for NoopWaker {
    fn wake(&self, reason: WakeReason) {
        let _ = self.0.send(reason);
    }
}

struct LockableStubSurface {
    document: DocumentId,
    bindings: HashMap<DocumentId, Weak<Session>>,
    lock_state: Rc<Cell<SessionLockState>>,
}

impl SessionSurface for LockableStubSurface {
    type Checkpoint = ();
    type MountWrites = ();

    fn surface(&self, document: DocumentId) -> Option<SurfaceId> {
        (document == self.document).then(|| SurfaceId::new(9))
    }
    fn bind(&mut self, document: DocumentId, session: &Rc<Session>) {
        self.bindings.insert(document, Rc::downgrade(session));
    }
    fn image_source_checkpoint(&mut self, _document: DocumentId) {}
    fn seal_image_source_writes(&self, _document: DocumentId, _checkpoint: ()) {}
    fn discard_image_source_writes(&mut self, _document: DocumentId, _checkpoint: ()) {}
    fn commit_image_policy(&mut self, _document: DocumentId, _session: &Session, _mount_writes: ()) {}
    fn request_frame(&self, _document: DocumentId) {}
    fn session_lock_support(&self) -> Option<SessionLockState> {
        Some(self.lock_state.get())
    }
    fn close_shell_surface(&mut self, _token: WindowToken, _id: Option<SurfaceId>) -> bool {
        false
    }
    fn stage_shell_surfaces(
        &mut self,
        _staged: &mut ShellWindows,
        _previous: &ShellWindows,
        _monitors: &[Monitor],
        _session: &Rc<Session>,
    ) -> SessionResult<()> {
        Ok(())
    }
}

struct Harness {
    _fixture: Fixture,
    dir: PathBuf,
    host: SessionHost,
    surface: LockableStubSurface,
    stub: Rc<StubHost>,
    document: DocumentId,
    waker: Arc<dyn Waker>,
    lock_state: Rc<Cell<SessionLockState>>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.host.stop(&mut self.surface);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn setup_harness(base_js: &str) -> Harness {
    let dir = std::env::temp_dir().join(format!(
        "zgui-ovl-host-test-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let fixture = Fixture::new(base_js, "").with_capabilities(Capabilities {
        shell_surfaces: true,
        session_lock: true,
        ..Capabilities::default()
    });
    let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(Document::new()))));
    let dom = DomHandle::from_rc(backend.clone());
    let root = dom.create_element(ElementName::new("column"));
    dom.insert(backend.root_node(), root, None);
    let stub = Rc::new(StubHost::default());
    let document = backend.document_id();
    let region = MountRegion {
        document,
        root,
        dom,
        host: HostHandle::from_rc(stub.clone()),
        fonts: zgui::app::Fonts::shipped_only(),
        root_anchor: Rc::new(Cell::new(root)),
    };
    let pending = Rc::new(RefCell::new(vec![region]));
    let (sender, _wakes) = mpsc::channel();
    let waker: Arc<dyn Waker> = Arc::new(NoopWaker(sender));
    let lock_state = Rc::new(Cell::new(SessionLockState::Unlocked));
    let surface = LockableStubSurface {
        document,
        bindings: HashMap::new(),
        lock_state: Rc::clone(&lock_state),
    };
    let mut host = SessionHost::new(
        pending,
        SessionModule::package(fixture.package()),
        SessionPolicy::live(host_services::Executor::new(2, 64).unwrap()),
        RecoveryPolicy::Propagate,
        SurfaceOpacity::Transparent,
    );
    host.set_observation_duration(Duration::from_millis(35));
    Harness {
        _fixture: fixture,
        dir,
        host,
        surface,
        stub,
        document,
        waker,
        lock_state,
    }
}

fn no_retry() -> Rc<dyn Fn(DocumentId, u64)> {
    Rc::new(|_, _| {})
}

fn sha(s: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(s.as_bytes()))
}

fn write_overlay_file(
    base_dir: &Path,
    filename: &str,
    overlay_id: &str,
    rollback_allowed: &[&str],
    client_metadata: serde_json::Value,
    ext_specs: &[(&str, &str, Option<&str>)],
    extra_caps: serde_json::Value,
) -> PathBuf {
    let mut extensions = Vec::new();
    for (ext_id, entry_js, css_opt) in ext_specs {
        let ext_dir = base_dir.join(format!("bundle-{overlay_id}-{}", ext_id.replace('/', "_")));
        fs::create_dir_all(ext_dir.join("dist")).unwrap();
        fs::write(ext_dir.join("dist/entry.mjs"), entry_js).unwrap();
        let mut files = serde_json::Map::new();
        files.insert("dist/entry.mjs".into(), serde_json::json!(sha(entry_js)));
        let mut styles = Vec::new();
        if let Some(css) = css_opt {
            fs::write(ext_dir.join("dist/style.css"), css).unwrap();
            files.insert("dist/style.css".into(), serde_json::json!(sha(css)));
            styles.push("dist/style.css");
        }
        let build_manifest = serde_json::json!({
            "schema_version": 1,
            "shared_runtime_abi": "1.0",
            "files": files,
        });
        fs::write(
            ext_dir.join("dist/manifest.build.json"),
            serde_json::to_vec(&build_manifest).unwrap(),
        )
        .unwrap();
        extensions.push(serde_json::json!({
            "id": ext_id,
            "version": "1.0.0",
            "shared_runtime_abi": "1.0",
            "content_digest": sha(entry_js),
            "bundle_root": ext_dir,
            "build_manifest": "dist/manifest.build.json",
            "styles": styles,
            "entries": [{
                "entry_id": format!("{ext_id}:main"),
                "kind": "panel",
                "bundle": "dist/entry.mjs",
            }],
        }));
    }
    let overlay_path = base_dir.join(filename);
    let payload = serde_json::json!({
        "schema_version": 1,
        "overlay_id": overlay_id,
        "rollback_allowed_extensions": rollback_allowed,
        "effective_host_capabilities": extra_caps,
        "extensions": extensions,
        "client_metadata": client_metadata,
    });
    fs::write(&overlay_path, serde_json::to_vec(&payload).unwrap()).unwrap();
    overlay_path
}

const SHELL_BASE_JS: &str = r#"
export const hostApiVersion = 1;
globalThis.__phaseEvents = [];
globalThis.__handoffSeq = 0;

export function mount(root) {
    let status = globalThis.__zguiShellHost.getOverlayStatus();
    globalThis.__phaseEvents.push("mount:" + status.phase + ":" + (status.overlayId ?? "none"));
    const entries = globalThis.__zguiShellHost.listApprovedEntries();
    globalThis.__loadedCount = entries.length;
    globalThis.__beforeCommitRan = false;
    const unsubBefore = globalThis.__zguiShellHost.onBeforeCommit(() => {
        globalThis.__beforeCommitRan = true;
        globalThis.__phaseEvents.push("beforeCommit:" + globalThis.__zguiShellHost.getOverlayStatus().generation);
    });
    const unsubCommit = globalThis.__zguiShellHost.onSessionCommitted((st) => {
        globalThis.__committedStatus = st;
        globalThis.__takenHandoff = globalThis.__zguiShellHost.takeSessionHandoff();
        globalThis.__secondTakeHandoff = globalThis.__zguiShellHost.takeSessionHandoff();
        globalThis.__phaseEvents.push("committed:" + st.phase + ":" + (st.overlayId ?? "none"));
        for (const entry of entries) {
            globalThis.__zguiShellHost.loadApprovedEntry(entry.entryId).then(mod => {
                if (mod.onMount) mod.onMount();
            });
        }
    });
    const unsubHandoff = globalThis.__zguiShellHost.registerSessionHandoff(() => {
        if (globalThis.__oversizeHandoff) {
            return "x".repeat(70000);
        }
        return { tab: "status", overlay: status.overlayId };
    });
    return () => {
        unsubBefore();
        unsubCommit();
        unsubHandoff();
        __zgui.clearRoot(root);
    };
}
export function pumpEvents() {}
"#;

#[test]
fn overlay_lock_screen_coalescing_defers_until_unlock_and_applies_latest_only() {
    let mut h = setup_harness(SHELL_BASE_JS);
    let ovl1 = write_overlay_file(
        &h.dir,
        "ovl1.json",
        "ovl-1",
        &["acme/a"],
        serde_json::json!(null),
        &[("acme/a", "export const v = 1;", None)],
        serde_json::json!({}),
    );
    let ovl2 = write_overlay_file(
        &h.dir,
        "ovl2.json",
        "ovl-2",
        &["acme/b"],
        serde_json::json!(null),
        &[("acme/b", "export const v = 2;", None)],
        serde_json::json!({}),
    );
    let ovl3 = write_overlay_file(
        &h.dir,
        "ovl3.json",
        "ovl-3",
        &["acme/c"],
        serde_json::json!(null),
        &[("acme/c", "export const v = 3;", None)],
        serde_json::json!({}),
    );

    h.host.configure_initial_overlay(&ovl1);
    let waker = Arc::clone(&h.waker);
    h.host.attach(&mut h.surface, waker.clone(), no_retry()).expect("initial mount");
    let gen1 = h.host.session(h.document).unwrap().generation;
    assert_eq!(h.host.active_overlay_id(), Some("ovl-1"));

    // Simulate active compositor lock: queued updates must never interrupt active generation.
    h.lock_state.set(SessionLockState::Locked);
    h.host.apply_overlay_update_primary(&mut h.surface, waker.clone(), no_retry(), h.document, &ovl2);
    h.host.apply_overlay_update_primary(&mut h.surface, waker.clone(), no_retry(), h.document, &ovl3);
    assert_eq!(h.host.session(h.document).unwrap().generation, gen1);
    assert_eq!(h.host.active_overlay_id(), Some("ovl-1"));
    assert_eq!(h.host.pending_overlay_path(), Some(ovl3.as_path()));

    // Still locked -> drain does nothing.
    h.host.drain_pending_overlay_primary_if_unlocked(&mut h.surface, waker.clone(), no_retry(), h.document);
    assert_eq!(h.host.session(h.document).unwrap().generation, gen1);

    // Unlock -> drains single-slot coalesced latest overlay (ovl-3), skipping ovl-2.
    h.lock_state.set(SessionLockState::Unlocked);
    h.host.drain_pending_overlay_primary_if_unlocked(&mut h.surface, waker, no_retry(), h.document);
    assert_ne!(h.host.session(h.document).unwrap().generation, gen1);
    assert_eq!(h.host.active_overlay_id(), Some("ovl-3"));
    assert!(h.host.pending_overlay_path().is_none());
}

#[test]
fn overlay_three_phase_handoff_bounded_state_and_extension_css_cleanup() {
    let mut h = setup_harness(SHELL_BASE_JS);
    let ovl1 = write_overlay_file(
        &h.dir,
        "ovl1.json",
        "ovl-css-1",
        &["acme/ext-a"],
        serde_json::json!({"seq": 1}),
        &[("acme/ext-a", "export const onMount = () => {};", Some(".extA { padding: 8px; }"))],
        serde_json::json!({}),
    );
    let ovl2 = write_overlay_file(
        &h.dir,
        "ovl2.json",
        "ovl-css-2",
        &["acme/ext-b"],
        serde_json::json!({"seq": 2}),
        &[("acme/ext-b", "export const onMount = () => {};", Some(".extB { padding: 12px; }"))],
        serde_json::json!({}),
    );

    h.host.configure_initial_overlay(&ovl1);
    let waker = Arc::clone(&h.waker);
    h.host.attach(&mut h.surface, waker.clone(), no_retry()).expect("mount ovl1");

    // Verify extension stylesheet from ovl-1 is mounted (slot application:0 alongside empty base).
    let sheets_1: Vec<String> = (0..4).filter_map(|i| h.stub.stylesheet(&format!("application:{i}"))).collect();
    assert!(
        sheets_1.iter().any(|css| css.contains(".extA")),
        "expected ovl1 extension CSS installed, got {sheets_1:?}"
    );

    // Switch to ovl-2: Phase 2 captures bounded handoff from ovl-1 and delivers to ovl-2 onSessionCommitted.
    h.host.apply_overlay_update_primary(&mut h.surface, waker, no_retry(), h.document, &ovl2);
    let sess2 = h.host.session(h.document).unwrap().clone();
    let (taken, second, committed_phase): (String, String, String) = sess2
        .engine
        .borrow_mut()
        .as_mut()
        .unwrap()
        .with_context(|ctx| {
            let taken: rquickjs::Value<'_> = ctx.eval("JSON.stringify(globalThis.__takenHandoff)")?;
            let second: rquickjs::Value<'_> = ctx.eval("JSON.stringify(globalThis.__secondTakeHandoff)")?;
            let phase: String = ctx.eval("globalThis.__committedStatus.phase")?;
            Ok((
                taken.as_string().unwrap().to_string().unwrap(),
                second.as_string().unwrap().to_string().unwrap(),
                phase,
            ))
        })
        .unwrap();
    assert_eq!(taken, r#"{"tab":"status","overlay":"ovl-css-1"}"#);
    assert_eq!(second, "null", "takeSessionHandoff must be single-consume");
    assert_eq!(committed_phase, "observing");

    // Extension CSS switched cleanly to .extB and removed .extA.
    let sheets_2: Vec<String> = (0..4).filter_map(|i| h.stub.stylesheet(&format!("application:{i}"))).collect();
    assert!(!sheets_2.iter().any(|css| css.contains(".extA")), "old ext CSS must be removed");
    assert!(sheets_2.iter().any(|css| css.contains(".extB")), "new ext CSS must be installed");

    // Wait for observation period -> promotes ovl-css-2 to committed & last_good_overlay.
    std::thread::sleep(Duration::from_millis(50));
    h.host.tick_observation();
    assert_eq!(h.host.last_good_overlay_id(), Some("ovl-css-2"));
    let final_phase: String = sess2
        .engine
        .borrow_mut()
        .as_mut()
        .unwrap()
        .with_context(|ctx| ctx.eval("globalThis.__zguiShellHost.getOverlayStatus().phase"))
        .unwrap();
    assert_eq!(final_phase, "committed");
}

#[test]
fn overlay_anti_resurrection_degrades_to_safe_base_shell_on_mount_and_observation_failures() {
    let mut h = setup_harness(SHELL_BASE_JS);
    let ovl_good = write_overlay_file(
        &h.dir,
        "good.json",
        "ovl-revoked-good",
        &["acme/revoked"],
        serde_json::json!({"tag": "good"}),
        &[("acme/revoked", "export const ok = 1;", Some(".extRevoked { padding: 4px; }"))],
        serde_json::json!({"commands": ["echo"]}),
    );
    h.host.configure_initial_overlay(&ovl_good);
    let waker = Arc::clone(&h.waker);
    h.host.attach(&mut h.surface, waker.clone(), no_retry()).expect("mount good overlay");
    assert!(
        h.host.session(h.document).unwrap().effective_capabilities().commands.contains(&"echo".to_owned())
    );
    // Survive observation period so ovl-revoked-good becomes last_good_overlay.
    std::thread::sleep(Duration::from_millis(50));
    h.host.tick_observation();
    assert_eq!(h.host.last_good_overlay_id(), Some("ovl-revoked-good"));

    // Case 1: Fault during 1500ms observation when new overlay revokes "acme/revoked" (`rollback_allowed_extensions: []`).
    // Extension throws asynchronously inside loaded entry (checkpoint drains rejection under strict/observation).
    let ovl_obs_fault = write_overlay_file(
        &h.dir,
        "obs_fault.json",
        "ovl-obs-fault",
        &[],
        serde_json::json!({"release": 99}),
        &[(
            "acme/faulty",
            "export function onMount() { setTimeout(() => { throw new Error('obs-crash'); }, 0); }",
            None,
        )],
        serde_json::json!({}),
    );
    h.host.apply_overlay_update_primary(&mut h.surface, waker.clone(), no_retry(), h.document, &ovl_obs_fault);
    assert_eq!(h.host.active_overlay_id(), Some("ovl-obs-fault"));
    // Pump jobs + timer fault in observation window.
    let s_obs = h.host.session(h.document).unwrap().clone();
    s_obs.record_fault(SessionError::InvalidState("simulated observation fault".into()));
    h.host.checkpoint(&mut h.surface, no_retry()).expect("recovered into Safe Base Shell");

    // Must NOT restore last_good ("ovl-revoked-good"); must degrade to zero-extension Safe Base Shell!
    assert!(h.host.is_safe_base_shell());
    assert!(h.host.last_good_overlay_id().is_none(), "revoked last_good must be cleared");
    assert!(h.host.active_overlay_id().is_none());
    assert!(
        h.host.session(h.document).unwrap().effective_capabilities().commands.is_empty(),
        "Safe Base Shell must strip overlay capabilities"
    );
    let safe_sheets: Vec<String> = (0..4).filter_map(|i| h.stub.stylesheet(&format!("application:{i}"))).collect();
    assert!(
        !safe_sheets.iter().any(|css| css.contains(".extRevoked")),
        "extension CSS must be absent in Safe Base Shell"
    );

    let safe_sess = h.host.session(h.document).unwrap().clone();
    let (safe_flag, blocked, meta_release, taken): (bool, bool, i64, String) = safe_sess
        .engine
        .borrow_mut()
        .as_mut()
        .unwrap()
        .with_context(|ctx| {
            let st: rquickjs::Object<'_> = ctx.eval("globalThis.__committedStatus")?;
            let safe: bool = st.get("safeBaseShell")?;
            let failure: rquickjs::Object<'_> = st.get("lastFailure")?;
            let blocked: bool = failure.get("rollbackBlockedByPolicy")?;
            let meta: rquickjs::Object<'_> = failure.get("clientMetadata")?;
            let rel: i64 = meta.get("release")?;
            let taken: rquickjs::Value<'_> = ctx.eval("JSON.stringify(globalThis.__takenHandoff)")?;
            Ok((safe, blocked, rel, taken.as_string().unwrap().to_string().unwrap()))
        })
        .unwrap();
    assert!(safe_flag);
    assert!(blocked);
    assert_eq!(meta_release, 99);
    assert_eq!(taken, "null", "session handoff must be dropped on Safe Base Shell fallback");

    // Case 2: Allowed rollback when last_good is inside rollback_allowed_extensions.
    let ovl_keep = write_overlay_file(
        &h.dir,
        "keep.json",
        "ovl-keep",
        &["acme/keep"],
        serde_json::json!({"ok": true}),
        &[("acme/keep", "export const ok = 1;", None)],
        serde_json::json!({}),
    );
    h.host.apply_overlay_update_primary(&mut h.surface, waker.clone(), no_retry(), h.document, &ovl_keep);
    std::thread::sleep(Duration::from_millis(50));
    h.host.tick_observation();
    assert_eq!(h.host.last_good_overlay_id(), Some("ovl-keep"));

    // Next candidate fails preflight (missing bundle) while allowing "acme/keep" rollback.
    let bad_path = h.dir.join("bad_preflight.json");
    fs::write(
        &bad_path,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "overlay_id": "ovl-bad-preflight",
            "rollback_allowed_extensions": ["acme/keep"],
            "extensions": [{
                "id": "acme/missing",
                "version": "1.0.0",
                "shared_runtime_abi": "1.0",
                "content_digest": format!("sha256:{}", "0".repeat(64)),
                "bundle_root": h.dir.join("does-not-exist"),
                "build_manifest": "dist/manifest.build.json",
                "entries": []
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    h.host.apply_overlay_update_primary(&mut h.surface, waker.clone(), no_retry(), h.document, &bad_path);
    assert!(!h.host.is_safe_base_shell());
    assert_eq!(h.host.active_overlay_id(), Some("ovl-keep"));

    // Case 3: Phase 1/preflight failure that excludes active "acme/keep" from rollback_allowed_extensions
    // must NOT keep running "acme/keep" and must degrade to Safe Base Shell.
    let revoke_bad = h.dir.join("revoke_preflight.json");
    fs::write(
        &revoke_bad,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "overlay_id": "ovl-revoke-preflight",
            "rollback_allowed_extensions": [],
            "client_metadata": {"release": 100},
            "extensions": [{
                "id": "acme/missing2",
                "version": "1.0.0",
                "shared_runtime_abi": "1.0",
                "content_digest": format!("sha256:{}", "0".repeat(64)),
                "bundle_root": h.dir.join("still-missing"),
                "build_manifest": "dist/manifest.build.json",
                "entries": []
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    h.host.apply_overlay_update_primary(&mut h.surface, waker, no_retry(), h.document, &revoke_bad);
    assert!(h.host.is_safe_base_shell());
    assert!(h.host.active_overlay_id().is_none());
    assert!(h.host.last_good_overlay_id().is_none());
}
