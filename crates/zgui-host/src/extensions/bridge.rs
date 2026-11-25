//! Host-owned `globalThis.__zguiShellHost` bridge enforcing base-shell exclusivity,
//! shared runtime isolation, bounded handoff hooks, and 3-phase generation status notifications.
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
};

use rquickjs::{
    Array, Ctx, Exception, Function, Module, Object, Persistent, Promise, Value,
    function::Func,
    object::Property,
};

use super::{
    manifest::VerifiedOverlay,
    provider::{SharedRuntimeRegistry, is_forbidden_shared_specifier},
};
use crate::handoff::{SessionHandoffSnapshot, SessionHandoffStore};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OverlayFailureInfo {
    pub overlay_id: Option<String>,
    pub phase: &'static str,
    pub reason: String,
    pub rollback_blocked_by_policy: bool,
    pub client_metadata: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OverlayPhase {
    Base,
    Staged,
    Observing,
    Committed,
    SafeBaseShell,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OverlayStatusSnapshot {
    pub generation: u64,
    pub overlay_id: Option<String>,
    pub phase: OverlayPhase,
    pub safe_base_shell: bool,
    pub active_extension_ids: Vec<String>,
    pub client_metadata: Option<serde_json::Value>,
    pub last_failure: Option<OverlayFailureInfo>,
}

impl OverlayStatusSnapshot {
    pub(crate) fn initial(
        generation: u64,
        overlay: Option<&VerifiedOverlay>,
        safe_base_shell: bool,
        last_failure: Option<OverlayFailureInfo>,
    ) -> Self {
        let (overlay_id, active_extension_ids, client_metadata, phase) = if safe_base_shell {
            (None, Vec::new(), None, OverlayPhase::SafeBaseShell)
        } else if let Some(overlay) = overlay {
            (
                Some(overlay.overlay_id.clone()),
                overlay.extension_ids.clone(),
                overlay.client_metadata.clone(),
                OverlayPhase::Staged,
            )
        } else {
            (None, Vec::new(), None, OverlayPhase::Base)
        };
        Self {
            generation,
            overlay_id,
            phase,
            safe_base_shell,
            active_extension_ids,
            client_metadata,
            last_failure,
        }
    }
}

struct GateReset(Rc<Cell<bool>>);
impl Drop for GateReset {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

pub(crate) struct ShellHostBridge {
    overlay: Option<Rc<VerifiedOverlay>>,
    shared_runtime: Rc<RefCell<SharedRuntimeRegistry>>,
    approved_load_gate: Rc<Cell<bool>>,
    handoff: RefCell<SessionHandoffStore>,
    status: RefCell<OverlayStatusSnapshot>,
    shared_module_objects: RefCell<BTreeMap<String, Persistent<Object<'static>>>>,
    before_commit_callbacks: RefCell<Vec<(u64, Persistent<Function<'static>>)>>,
    committed_callbacks: RefCell<Vec<(u64, Persistent<Function<'static>>)>>,
    next_cb_id: Cell<u64>,
    committed: Cell<bool>,
    notified_committed: Cell<bool>,
}

impl ShellHostBridge {
    pub(crate) fn new(
        generation: u64,
        overlay: Option<Rc<VerifiedOverlay>>,
        shared_runtime: Rc<RefCell<SharedRuntimeRegistry>>,
        approved_load_gate: Rc<Cell<bool>>,
        incoming_handoff: Option<SessionHandoffSnapshot>,
        safe_base_shell: bool,
        last_failure: Option<OverlayFailureInfo>,
    ) -> Rc<Self> {
        let status = OverlayStatusSnapshot::initial(
            generation,
            overlay.as_deref(),
            safe_base_shell,
            last_failure,
        );
        Rc::new(Self {
            overlay,
            shared_runtime,
            approved_load_gate,
            handoff: RefCell::new(SessionHandoffStore::new(incoming_handoff)),
            status: RefCell::new(status),
            shared_module_objects: RefCell::new(BTreeMap::new()),
            before_commit_callbacks: RefCell::new(Vec::new()),
            committed_callbacks: RefCell::new(Vec::new()),
            next_cb_id: Cell::new(1),
            committed: Cell::new(false),
            notified_committed: Cell::new(false),
        })
    }

    pub(crate) fn set_phase(&self, phase: OverlayPhase) {
        self.status.borrow_mut().phase = phase;
        self.committed.set(matches!(
            phase,
            OverlayPhase::Observing | OverlayPhase::Committed | OverlayPhase::SafeBaseShell | OverlayPhase::Base
        ));
    }

    pub(crate) fn set_failure(&self, failure: OverlayFailureInfo) {
        self.status.borrow_mut().last_failure = Some(failure);
    }

    pub(crate) fn set_incoming_handoff(&self, snapshot: Option<SessionHandoffSnapshot>) {
        self.handoff.borrow_mut().set_incoming(snapshot);
    }

    pub(crate) fn capture_handoff(&self, ctx: &Ctx<'_>) -> Option<SessionHandoffSnapshot> {
        self.handoff.borrow().capture(ctx)
    }

    pub(crate) fn invoke_before_commit(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let cbs: Vec<Persistent<Function<'static>>> = self
            .before_commit_callbacks
            .borrow()
            .iter()
            .map(|(_, cb)| cb.clone())
            .collect();
        for cb in cbs {
            let func = cb.restore(ctx)?;
            let _: Value<'_> = func.call(())?;
        }
        Ok(())
    }

    pub(crate) fn invoke_session_committed(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if self.notified_committed.replace(true) {
            return Ok(());
        }
        let status_val = status_to_js(ctx, &self.status.borrow())?;
        let cbs: Vec<Persistent<Function<'static>>> = self
            .committed_callbacks
            .borrow()
            .iter()
            .map(|(_, cb)| cb.clone())
            .collect();
        for cb in cbs {
            let func = cb.restore(ctx)?;
            let _: Value<'_> = func.call((status_val.clone(),))?;
        }
        Ok(())
    }

    /// Must be called before `JsRuntime` drops so no `Persistent` roots outlive QuickJS.
    pub(crate) fn clear_roots(&self) {
        self.handoff.borrow_mut().clear_all();
        self.shared_module_objects.borrow_mut().clear();
        self.before_commit_callbacks.borrow_mut().clear();
        self.committed_callbacks.borrow_mut().clear();
    }

    pub(crate) fn install<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        let obj = Object::new(ctx.clone())?;

        let b = Rc::downgrade(self);
        obj.prop(
            "listApprovedEntries",
            Property::from(Func::from(move |ctx: Ctx<'js>| list_entries(&b, ctx))).enumerable(),
        )?;

        let b = Rc::downgrade(self);
        obj.prop(
            "loadApprovedEntry",
            Property::from(Func::from(move |ctx: Ctx<'js>, entry_id: String| {
                load_approved_entry(&b, ctx, entry_id)
            }))
            .enumerable(),
        )?;

        let b = Rc::downgrade(self);
        obj.prop(
            "registerSharedRuntimeModules",
            Property::from(Func::from(move |ctx: Ctx<'js>, table: Object<'js>| {
                register_shared_runtime_modules(&b, ctx, table)
            }))
            .enumerable(),
        )?;

        let b = Rc::downgrade(self);
        obj.prop(
            "registerSessionHandoff",
            Property::from(Func::from(move |ctx: Ctx<'js>, cb: Function<'js>| {
                register_session_handoff(&b, ctx, cb)
            }))
            .enumerable(),
        )?;

        let b = Rc::downgrade(self);
        obj.prop(
            "takeSessionHandoff",
            Property::from(Func::from(move |ctx: Ctx<'js>| take_session_handoff(&b, ctx))).enumerable(),
        )?;

        let b = Rc::downgrade(self);
        obj.prop(
            "onSessionCommitted",
            Property::from(Func::from(move |ctx: Ctx<'js>, cb: Function<'js>| {
                subscribe_committed(&b, ctx, cb)
            }))
            .enumerable(),
        )?;

        let b = Rc::downgrade(self);
        obj.prop(
            "onBeforeCommit",
            Property::from(Func::from(move |ctx: Ctx<'js>, cb: Function<'js>| {
                subscribe_before_commit(&b, ctx, cb)
            }))
            .enumerable(),
        )?;

        let b = Rc::downgrade(self);
        obj.prop(
            "getOverlayStatus",
            Property::from(Func::from(move |ctx: Ctx<'js>| get_overlay_status(&b, ctx))).enumerable(),
        )?;

        if let Ok(object_ctor) = ctx.globals().get::<_, Object<'js>>("Object")
            && let Ok(freeze) = object_ctor.get::<_, Function<'js>>("freeze")
        {
            let _: Value<'js> = freeze.call((obj.clone(),))?;
        }
        ctx.globals().set("__zguiShellHost", obj)?;

        let b = Rc::downgrade(self);
        ctx.globals().prop(
            "__zguiSharedRuntimeGet",
            Property::from(Func::from(move |ctx: Ctx<'js>, specifier: String| {
                get_shared_runtime_module(&b, ctx, specifier)
            })),
        )?;
        Ok(())
    }
}

fn list_entries<'js>(weak: &Weak<ShellHostBridge>, ctx: Ctx<'js>) -> rquickjs::Result<Array<'js>> {
    reject_if_extension_caller(&ctx)?;
    let arr = Array::new(ctx.clone())?;
    let Some(bridge) = weak.upgrade() else {
        return Ok(arr);
    };
    let Some(overlay) = &bridge.overlay else {
        return Ok(arr);
    };
    for (idx, entry) in overlay.entries_by_id.values().enumerate() {
        let item = Object::new(ctx.clone())?;
        item.set("entryId", entry.entry_id.as_str())?;
        item.set("extensionId", entry.extension_id.as_str())?;
        item.set("version", entry.version.as_str())?;
        item.set("kind", entry.kind.as_str())?;
        item.set("bundle", entry.bundle.as_str())?;
        item.set("canonicalSpecifier", entry.canonical_specifier.as_str())?;
        arr.set(idx, item)?;
    }
    Ok(arr)
}

fn load_approved_entry<'js>(
    weak: &Weak<ShellHostBridge>,
    ctx: Ctx<'js>,
    entry_id: String,
) -> rquickjs::Result<Promise<'js>> {
    reject_if_extension_caller(&ctx)?;
    let Some(bridge) = weak.upgrade() else {
        return Err(Exception::throw_reference(&ctx, "session disposed"));
    };
    let Some(overlay) = &bridge.overlay else {
        return Err(Exception::throw_reference(&ctx, "no extension overlay is active"));
    };
    let Some(entry) = overlay.entries_by_id.get(&entry_id) else {
        return Err(Exception::throw_reference(
            &ctx,
            &format!("unapproved extension entry: {entry_id:?}"),
        ));
    };
    bridge.approved_load_gate.set(true);
    let _guard = GateReset(Rc::clone(&bridge.approved_load_gate));
    Module::import(&ctx, entry.canonical_specifier.as_bytes())
}

fn register_shared_runtime_modules<'js>(
    weak: &Weak<ShellHostBridge>,
    ctx: Ctx<'js>,
    table: Object<'js>,
) -> rquickjs::Result<()> {
    reject_if_extension_caller(&ctx)?;
    let Some(bridge) = weak.upgrade() else {
        return Err(Exception::throw_reference(&ctx, "session disposed"));
    };
    let mut staged_objects = Vec::new();
    let mut staged_exports = Vec::new();
    for entry in table.props::<String, Object<'js>>() {
        let (specifier, module_obj) = entry?;
        if is_forbidden_shared_specifier(&specifier) {
            return Err(Exception::throw_type(
                &ctx,
                &format!("cannot register forbidden shared module specifier {specifier:?}"),
            ));
        }
        let sanitized = Object::new(ctx.clone())?;
        let mut keys = Vec::new();
        for prop in module_obj.props::<String, Value<'js>>() {
            let (key, val) = prop?;
            if specifier == "@hewel/zgui"
                && matches!(key.as_str(), "SessionLockSurface" | "requestSessionLock")
            {
                continue;
            }
            sanitized.set(key.as_str(), val)?;
            keys.push(key);
        }
        keys.sort();
        keys.dedup();
        staged_objects.push((specifier.clone(), Persistent::save(&ctx, sanitized)));
        staged_exports.push((specifier, keys));
    }
    let mut shared = bridge.shared_runtime.borrow_mut();
    let mut stored_objs = bridge.shared_module_objects.borrow_mut();
    for (specifier, keys) in staged_exports {
        shared
            .register_module_exports(&specifier, keys)
            .map_err(|msg| Exception::throw_type(&ctx, &msg))?;
    }
    for (specifier, obj) in staged_objects {
        stored_objs.insert(specifier, obj);
    }
    Ok(())
}

fn register_session_handoff<'js>(
    weak: &Weak<ShellHostBridge>,
    ctx: Ctx<'js>,
    cb: Function<'js>,
) -> rquickjs::Result<Function<'js>> {
    reject_if_extension_caller(&ctx)?;
    let Some(bridge) = weak.upgrade() else {
        return Err(Exception::throw_reference(&ctx, "session disposed"));
    };
    let id = bridge.handoff.borrow_mut().register_capture(&ctx, cb)?;
    let weak_bridge = Rc::downgrade(&bridge);
    Function::new(ctx, move || {
        if let Some(bridge) = weak_bridge.upgrade() {
            bridge.handoff.borrow_mut().unregister_capture(id);
        }
    })
}

fn take_session_handoff<'js>(weak: &Weak<ShellHostBridge>, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    reject_if_extension_caller(&ctx)?;
    let Some(bridge) = weak.upgrade() else {
        return Ok(Value::new_null(ctx));
    };
    bridge.handoff.borrow_mut().take_incoming(&ctx)
}

fn subscribe_committed<'js>(
    weak: &Weak<ShellHostBridge>,
    ctx: Ctx<'js>,
    cb: Function<'js>,
) -> rquickjs::Result<Function<'js>> {
    reject_if_extension_caller(&ctx)?;
    let Some(bridge) = weak.upgrade() else {
        return Err(Exception::throw_reference(&ctx, "session disposed"));
    };
    let id = next_id(&ctx, &bridge.next_cb_id)?;
    if bridge.committed.get() {
        let status_val = status_to_js(&ctx, &bridge.status.borrow())?;
        let _: Value<'js> = cb.call((status_val,))?;
    }
    bridge
        .committed_callbacks
        .borrow_mut()
        .push((id, Persistent::save(&ctx, cb)));
    let weak_bridge = Rc::downgrade(&bridge);
    Function::new(ctx, move || {
        if let Some(bridge) = weak_bridge.upgrade() {
            bridge
                .committed_callbacks
                .borrow_mut()
                .retain(|(cid, _)| *cid != id);
        }
    })
}

fn subscribe_before_commit<'js>(
    weak: &Weak<ShellHostBridge>,
    ctx: Ctx<'js>,
    cb: Function<'js>,
) -> rquickjs::Result<Function<'js>> {
    reject_if_extension_caller(&ctx)?;
    let Some(bridge) = weak.upgrade() else {
        return Err(Exception::throw_reference(&ctx, "session disposed"));
    };
    let id = next_id(&ctx, &bridge.next_cb_id)?;
    bridge
        .before_commit_callbacks
        .borrow_mut()
        .push((id, Persistent::save(&ctx, cb)));
    let weak_bridge = Rc::downgrade(&bridge);
    Function::new(ctx, move || {
        if let Some(bridge) = weak_bridge.upgrade() {
            bridge
                .before_commit_callbacks
                .borrow_mut()
                .retain(|(cid, _)| *cid != id);
        }
    })
}

fn get_overlay_status<'js>(weak: &Weak<ShellHostBridge>, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
    reject_if_extension_caller(&ctx)?;
    let Some(bridge) = weak.upgrade() else {
        return Ok(Value::new_null(ctx));
    };
    status_to_js(&ctx, &bridge.status.borrow())
}

fn get_shared_runtime_module<'js>(
    weak: &Weak<ShellHostBridge>,
    ctx: Ctx<'js>,
    specifier: String,
) -> rquickjs::Result<Object<'js>> {
    let Some(bridge) = weak.upgrade() else {
        return Err(Exception::throw_reference(&ctx, "session disposed"));
    };
    if is_forbidden_shared_specifier(&specifier) {
        return Err(Exception::throw_type(
            &ctx,
            &format!("forbidden shared runtime module {specifier:?}"),
        ));
    }
    let objs = bridge.shared_module_objects.borrow();
    let Some(saved) = objs.get(&specifier) else {
        return Err(Exception::throw_reference(
            &ctx,
            &format!("shared runtime module {specifier:?} is not registered"),
        ));
    };
    saved.clone().restore(&ctx)
}

fn next_id(ctx: &Ctx<'_>, counter: &Cell<u64>) -> rquickjs::Result<u64> {
    let id = counter.get();
    let next = id
        .checked_add(1)
        .ok_or_else(|| Exception::throw_range(ctx, "callback identifiers exhausted"))?;
    counter.set(next);
    Ok(id)
}

fn status_to_js<'js>(ctx: &Ctx<'js>, status: &OverlayStatusSnapshot) -> rquickjs::Result<Value<'js>> {
    let json = serde_json::to_string(status)
        .map_err(|err| Exception::throw_internal(ctx, &format!("serialize overlay status: {err}")))?;
    ctx.json_parse(json)
}

fn reject_if_extension_caller(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    for depth in 0..8 {
        if let Some(atom) = ctx.script_or_module_name(depth)
            && let Ok(name) = atom.to_string()
            && name.starts_with("zgui-ext://")
        {
            return Err(Exception::throw_type(
                ctx,
                "E_CAPABILITY: @hewel/zgui/shell-host is restricted to the base shell",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use js_runtime::{JsRuntime, RuntimeLimits, VerifiedModulePayload, VerifiedModuleProvider};
    use std::fs;

    struct ShimBaseProvider;
    impl VerifiedModuleProvider for ShimBaseProvider {
        fn resolve(&self, _r: &str, s: &str) -> Result<String, String> {
            Err(format!("unused: {s}"))
        }
        fn load(&self, c: &str) -> Result<VerifiedModulePayload, String> {
            Err(format!("unused: {c}"))
        }
    }

    #[test]
    fn shell_host_bridge_isolates_extension_caller_and_strips_session_lock_from_shared_zgui() {
        let dir = std::env::temp_dir().join(format!("zgui-bridge-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dist")).unwrap();
        let ext_src = "import * as zgui from '@hewel/zgui'; export const sameRender = zgui.renderApp; export const hasLock = typeof zgui.SessionLockSurface; export const hasReqLock = typeof zgui.requestSessionLock; export const hasRender = typeof zgui.renderApp; export function probePrivileged() { return globalThis.__zguiShellHost.listApprovedEntries(); }\n";
        let named_lock_src = "import { SessionLockSurface } from '@hewel/zgui'; export const x = SessionLockSurface;\n";
        fs::write(dir.join("dist/widget.mjs"), ext_src).unwrap();
        fs::write(dir.join("dist/named-lock.mjs"), named_lock_src).unwrap();
        let digest = |s: &str| {
            use sha2::{Digest, Sha256};
            format!("sha256:{:x}", Sha256::digest(s.as_bytes()))
        };
        let build_manifest = serde_json::json!({
            "schema_version": 1,
            "shared_runtime_abi": "1.0",
            "files": {
                "dist/widget.mjs": digest(ext_src),
                "dist/named-lock.mjs": digest(named_lock_src),
            }
        });
        fs::write(dir.join("dist/manifest.build.json"), serde_json::to_vec(&build_manifest).unwrap()).unwrap();
        let overlay_json = serde_json::json!({
            "schema_version": 1,
            "overlay_id": "ovl-bridge-1",
            "rollback_allowed_extensions": [],
            "effective_host_capabilities": {},
            "extensions": [{
                "id": "acme/widget",
                "version": "1.0.0",
                "shared_runtime_abi": "1.0",
                "content_digest": digest(ext_src),
                "bundle_root": dir,
                "build_manifest": "dist/manifest.build.json",
                "styles": [],
                "entries": [
                    { "entry_id": "acme/widget:main", "kind": "panel", "bundle": "dist/widget.mjs" },
                    { "entry_id": "acme/widget:named-lock", "kind": "panel", "bundle": "dist/named-lock.mjs" }
                ]
            }]
        });
        let overlay = Rc::new(
            VerifiedOverlay::load_from_bytes(
                &serde_json::to_vec(&overlay_json).unwrap(),
                super::super::manifest::SharedRuntimeAbi::CURRENT,
            )
            .unwrap(),
        );
        let shared = Rc::new(RefCell::new(SharedRuntimeRegistry::new(&[])));
        let gate = Rc::new(Cell::new(false));
        let bridge = ShellHostBridge::new(
            1,
            Some(Rc::clone(&overlay)),
            Rc::clone(&shared),
            Rc::clone(&gate),
            None,
            false,
            None,
        );
        let provider = Rc::new(super::super::provider::CompositeVerifiedModuleProvider::new(
            Rc::new(ShimBaseProvider),
            Some(overlay),
            shared,
            None,
            gate,
        ));

        let mut rt = JsRuntime::new(RuntimeLimits::default()).unwrap();
        rt.set_module_provider(provider);
        rt.with_context(|ctx| bridge.install(&ctx)).unwrap();
        let base_src = r#"
export const refRender = () => "ok";
globalThis.__zguiShellHost.registerSharedRuntimeModules({
    "@hewel/zgui": { renderApp: refRender, SessionLockSurface: () => null, requestSessionLock: () => null }
});
export let loadedMod = null;
export let namedLockErr = null;
globalThis.__zguiShellHost.loadApprovedEntry("acme/widget:main").then(m => { loadedMod = m; });
globalThis.__zguiShellHost.loadApprovedEntry("acme/widget:named-lock").catch(e => { namedLockErr = String(e); });
"#;
        rt.load_module("app.mjs", base_src).unwrap();
        let jobs = rt.pump_jobs();
        assert!(jobs.diagnostics.is_empty(), "{:?}", jobs.diagnostics);

        let (has_lock, has_req_lock, has_render, named_lock_err): (String, String, String, String) = rt
            .with_module(|_ctx, m| {
                let loaded: Object<'_> = m.get("loadedMod")?;
                let same: bool = loaded.get::<_, Function<'_>>("sameRender")? == m.get::<_, Function<'_>>("refRender")?;
                assert!(same, "shared runtime reference identity must be preserved");
                Ok((
                    loaded.get("hasLock")?,
                    loaded.get("hasReqLock")?,
                    loaded.get("hasRender")?,
                    m.get("namedLockErr")?,
                ))
            })
            .unwrap();
        assert_eq!(has_lock, "undefined", "SessionLockSurface must be stripped from extension @hewel/zgui");
        assert_eq!(has_req_lock, "undefined", "requestSessionLock must be stripped from extension @hewel/zgui");
        assert_eq!(has_render, "function");
        assert!(named_lock_err.contains("SessionLockSurface"), "unexpected named lock error: {named_lock_err}");

        let err = rt
            .with_module(|_ctx, m| {
                let loaded: Object<'_> = m.get("loadedMod")?;
                let probe: Function<'_> = loaded.get("probePrivileged")?;
                probe.call::<_, ()>(())
            })
            .expect_err("extension call to __zguiShellHost must throw");
        assert!(err.message.contains("E_CAPABILITY"), "{}", err.message);

        bridge.clear_roots();
        let _ = fs::remove_dir_all(&dir);
    }
}
