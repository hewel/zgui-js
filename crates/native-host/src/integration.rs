//! Integration harness: the host-owned baseline module and the native timer /
//! external-wake schedule that verifies JavaScript updates, scheduling and
//! wakes outside a packaged application. Harness registrations are keyed by
//! document/generation so replacement or retirement cancels them instead of
//! keeping an old session alive.

use std::{cell::Cell, collections::HashMap, rc::{Rc, Weak}, time::Duration};

use zgui::{
    platform::SurfaceId,
    view::{DocumentId, HostHandle, host::{Repeat, TimerId}},
};

use crate::{
    session::{Session, SessionModule},
    session_host::SessionHost,
};

/// Deliver an external wake to the owning event loop. The surface lets the
/// platform coalesce the wake with real reactive work for that surface only.
/// `Arc` because the detached baseline thread holds a copy.
pub(crate) type ExternalWake = std::sync::Arc<dyn Fn(DocumentId, u64, SurfaceId) + Send + Sync>;

const BASELINE_SOURCE: &str = r#"
export const hostApiVersion = 1;
let textNode = null;
export function mount(root) {
    textNode = __zgui.createTextNode('JavaScript mounted');
    __zgui.insertNode(root, textNode, null);
    return () => __zgui.clearRoot(root);
}
export function advance() { __zgui.replaceText(textNode, 'Native timer updated JavaScript'); }
export function external() { __zgui.replaceText(textNode, 'External wake updated JavaScript'); }
"#;

/// The package-less module mounted when the live host runs without `--bundle`.
pub(crate) fn baseline_module() -> SessionModule {
    SessionModule {
        name: "zgui-js-baseline.mjs".into(),
        source: BASELINE_SOURCE.into(),
        styles: Vec::new(),
        package: None,
    }
}

/// Existing callers that used `Session::advance` now invoke the module export.
#[cfg(test)]
pub(crate) fn advance(session: &Session) -> crate::session::Result<()> { session.invoke("advance") }

struct Armed {
    host: HostHandle,
    timer: Cell<Option<TimerId>>,
}
impl Armed {
    fn cancel(&self) {
        if let Some(timer) = self.timer.take() { self.host.cancel_timer(timer); }
    }
}

/// Owns every native registration the baseline/exercise workflow creates.
/// Weak session callbacks mean a retired generation is never kept alive by a
/// still-registered native timer.
pub(crate) struct IntegrationHarness {
    pub(crate) exercise: bool,
    armed: HashMap<(DocumentId, u64), Rc<Armed>>,
}
impl IntegrationHarness {
    pub(crate) fn new() -> Self { Self { exercise: false, armed: HashMap::new() } }

    /// Arm baseline/exercise work for newly mounted sessions and cancel
    /// registrations whose generation is no longer owned by the host. The
    /// application calls this after every host operation that can change
    /// session ownership, before the event loop continues.
    pub(crate) fn reconcile(&mut self, host: &SessionHost, external: &ExternalWake) {
        self.armed.retain(|(document, generation), registration| {
            let live = host.session(*document).is_some_and(|session| session.generation == *generation);
            if !live { registration.cancel(); }
            live
        });
        for session in host.sessions() {
            let key = (session.region.document, session.generation);
            if self.armed.contains_key(&key) { continue; }
            self.arm(session, external);
        }
    }

    fn arm(&mut self, session: &Rc<Session>, external: &ExternalWake) {
        let baseline = session.package().is_none();
        if !baseline && !self.exercise { return; }
        let key = (session.region.document, session.generation);
        let registration = Rc::new(Armed { host: session.region.host.clone(), timer: Cell::new(None) });
        let remaining = Cell::new(if self.exercise { 4 } else { 1 });
        let repeat = if self.exercise { Repeat::Every } else { Repeat::Once };
        let weak: Weak<Session> = Rc::downgrade(session);
        let cancel = registration.clone();
        registration.timer.set(Some(registration.host.schedule(Duration::from_secs(1), repeat, Rc::new(move || {
            if let Some(session) = weak.upgrade() {
                if let Err(error) = session.invoke("advance") {
                    session.record_fault(error);
                    remaining.set(1);
                }
                remaining.set(remaining.get() - 1);
                if remaining.get() == 0 { cancel.cancel(); }
            }
        }))));
        self.armed.insert(key, registration);
        if baseline {
            let external = external.clone();
            let document = session.region.document;
            let generation = session.generation;
            let surface = session.surface();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                external(document, generation, surface);
            });
        }
    }

    /// Cancel every armed registration; used before the host releases sessions.
    pub(crate) fn stop(&mut self) {
        for registration in self.armed.values() { registration.cancel(); }
        self.armed.clear();
    }
}
// Armed registrations hold a weak session and a native cancel; owner drop must
// not leave the cycle scheduled.
impl Drop for IntegrationHarness {
    fn drop(&mut self) { self.stop(); }
}
