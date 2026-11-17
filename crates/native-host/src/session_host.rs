//! Shared session lifecycle: mount-region intake, generation allocation,
//! candidate preflight, replacement, fallback recovery and retirement.
//! `Application` keeps the event loop and `PreviewHost` keeps renderer probes;
//! neither owns this algorithm anymore.

use std::{cell::RefCell, collections::HashMap, rc::{Rc, Weak}, sync::Arc};

use zgui::{
    platform::{SurfaceId, Waker},
    runtime::{HostBinding, ImagePolicy},
    view::{Anchor, BuildCx, DocumentId, DomHandle, ElementName, HostHandle, NodeId, View},
};

use crate::{
    fallback,
    session::{Result as SessionResult, Session, SessionError, SessionModule, SessionPolicy},
};

/// The portion of a native document owned by one application session at a time.
#[derive(Clone)]
pub(crate) struct MountRegion {
    pub(crate) document: DocumentId,
    pub(crate) root: NodeId,
    pub(crate) dom: DomHandle,
    pub(crate) host: HostHandle,
}

pub(crate) struct Root(pub(crate) Rc<RefCell<Vec<MountRegion>>>);
pub(crate) struct RootAnchor(NodeId);
impl View for Root {
    type State = RootAnchor;
    fn build(self, cx: &mut BuildCx<'_>) -> RootAnchor {
        let root = cx.dom().create_element(ElementName::new("column"));
        // These are editing semantics, not application chrome. Keep them owned
        // by the Rust root across JS generations; application CSS may override.
        let host = cx.host_handle();
        host.install_stylesheet("zgui-js-controls", "field { white-space: pre; } editor { white-space: pre-wrap; }");
        self.0.borrow_mut().push(MountRegion { document: cx.document(), root, dom: cx.dom_handle(), host });
        RootAnchor(root)
    }
    fn rebuild(self, _state: &mut RootAnchor, _cx: &mut BuildCx<'_>) {}
}
impl Anchor for RootAnchor {
    fn mount(&mut self, dom: &DomHandle, parent: NodeId, before: Option<NodeId>) { dom.insert(parent, self.0, before); }
    fn unmount(&mut self, dom: &DomHandle) { dom.detach(self.0); }
    fn first_node(&self) -> Option<NodeId> { Some(self.0) }
}

pub(crate) struct Binding(Weak<Session>);
impl HostBinding for Binding {
    fn checkpoint(&mut self) {
        if let Some(session) = self.0.upgrade() { session.frame_checkpoint(); }
    }
    fn before_paint(&mut self, _timestamp: zgui::vocab::Timestamp) {
        if let Some(session) = self.0.upgrade() {
            session.frames.set(session.frames.get() + 1);
            if session.frames.get() == 1 { eprintln!("first-native-frame: {}", session.region.dom.text_content(session.region.root)); }
        }
    }
    fn shutting_down(&mut self) { if let Some(session) = self.0.upgrade() { let _ = session.retire(); } }
}

/// What a mount or checkpoint failure does to the affected mount region.
#[derive(Clone, Copy)]
pub(crate) enum RecoveryPolicy {
    /// Retire the failed generation and show the Rust fallback with an
    /// optional retry of the last successfully mounted package.
    Fallback,
    /// Keep the failed session owned and hand the typed error to the caller,
    /// which persists its cause before deciding to retire it.
    Propagate,
}

#[derive(Debug)]
pub(crate) enum ReloadOutcome {
    Mounted(u64),
    Rejected(SessionError),
    Fallback(SessionError),
}

pub(crate) fn report_reload(outcome: &ReloadOutcome) {
    match outcome {
        ReloadOutcome::Mounted(generation) => eprintln!("reload-mounted: generation={generation}"),
        ReloadOutcome::Rejected(error) => eprintln!("reload-preflight-rejected: {error}"),
        ReloadOutcome::Fallback(error) => eprintln!("reload-fallback: {error}"),
    }
}

/// Native-surface seam consumed by [`SessionHost`]. `RuntimeSurface` adapts the
/// real `zgui` runtime; tests substitute a stub that records bindings and
/// frame requests. This is deliberately narrow: the host does not see the rest
/// of the renderer.
pub(crate) trait SessionSurface {
    fn surface(&self, document: DocumentId) -> Option<SurfaceId>;
    /// Install the generation's binding and image policy before `Session::mount`.
    fn bind(&mut self, document: DocumentId, session: &Rc<Session>);
    fn request_frame(&self, document: DocumentId);
}

/// Image authority installed by [`RuntimeSurface::bind`]. Preview publishes
/// package bytes in strict launch order; the live host uses ordinary
/// asynchronous package-asset publication.
#[derive(Clone, Copy)]
pub(crate) enum ImageChoice { Live, PackageOnly }

pub(crate) struct RuntimeSurface<'a> {
    runtime: &'a mut zgui::runtime::Runtime,
    images: ImageChoice,
}
impl<'a> RuntimeSurface<'a> {
    pub(crate) fn new(runtime: &'a mut zgui::runtime::Runtime, images: ImageChoice) -> Self {
        Self { runtime, images }
    }
}
impl SessionSurface for RuntimeSurface<'_> {
    fn surface(&self, document: DocumentId) -> Option<SurfaceId> {
        self.runtime.windows().iter().find(|window| window.document_id() == document).map(|window| window.surface().id())
    }
    fn bind(&mut self, document: DocumentId, session: &Rc<Session>) {
        let window = self.runtime.windows_mut().iter_mut().find(|window| window.document_id() == document)
            .expect("pure preflight cannot destroy its native window");
        // Binding replacement itself neither shuts down the old owner nor
        // requests a frame. Both are explicit at this owner-thread boundary.
        window.replace_image_policy(match (self.images, session.package()) {
            (ImageChoice::Live, Some(package)) => ImagePolicy::package_assets(package.image_assets()),
            (ImageChoice::Live, None) => ImagePolicy::default(),
            (ImageChoice::PackageOnly, Some(package)) => ImagePolicy::package_only(package.image_assets()),
            (ImageChoice::PackageOnly, None) => ImagePolicy::package_only(std::iter::empty()),
        });
        window.install_binding(Box::new(Binding(Rc::downgrade(session))));
        eprintln!("binding-installed: {document:?} generation={}", session.generation);
    }
    fn request_frame(&self, document: DocumentId) {
        if let Some(window) = self.runtime.windows().iter().find(|window| window.document_id() == document) { window.request_frame(); }
    }
}

struct Recovery {
    generation: u64,
    view: fallback::Fallback,
}

/// Owns sessions, mount regions, fallbacks, the generation counter and the
/// last successfully mounted package for every attached native document.
pub(crate) struct SessionHost {
    pending: Rc<RefCell<Vec<MountRegion>>>,
    regions: HashMap<DocumentId, MountRegion>,
    sessions: HashMap<DocumentId, Rc<Session>>,
    fallbacks: HashMap<DocumentId, Recovery>,
    generation: u64,
    module: SessionModule,
    last_good: Option<SessionModule>,
    policy: SessionPolicy,
    recovery: RecoveryPolicy,
}

impl SessionHost {
    pub(crate) fn new(pending: Rc<RefCell<Vec<MountRegion>>>, module: SessionModule, policy: SessionPolicy, recovery: RecoveryPolicy) -> Self {
        Self {
            pending, regions: HashMap::new(), sessions: HashMap::new(), fallbacks: HashMap::new(),
            generation: 0, module, last_good: None, policy, recovery,
        }
    }

    pub(crate) fn generation(&self) -> u64 { self.generation }
    /// The module a fresh mount region attaches, or the last mounted one.
    pub(crate) fn module(&self) -> &SessionModule { &self.module }
    /// Adopt a publication that arrived while no mount region existed.
    pub(crate) fn set_module(&mut self, module: SessionModule) { self.module = module; }
    #[cfg(test)]
    pub(crate) fn region(&self, document: DocumentId) -> Option<&MountRegion> { self.regions.get(&document) }
    pub(crate) fn region_documents(&self) -> impl Iterator<Item = DocumentId> + '_ { self.regions.keys().copied() }
    #[cfg(test)]
    pub(crate) fn fallback_generation(&self, document: DocumentId) -> Option<u64> { self.fallbacks.get(&document).map(|recovery| recovery.generation) }
    pub(crate) fn pending_regions(&self) -> usize { self.pending.borrow().len() }
    pub(crate) fn session(&self, document: DocumentId) -> Option<&Rc<Session>> { self.sessions.get(&document) }
    pub(crate) fn sessions(&self) -> impl Iterator<Item = &Rc<Session>> { self.sessions.values() }

    /// Drain newly built mount regions and mount the current module on each,
    /// then retire ownership for documents whose native surface is gone.
    pub(crate) fn attach(&mut self, surface: &mut impl SessionSurface, waker: Arc<dyn Waker>, retry: Rc<dyn Fn(DocumentId, u64)>) -> SessionResult<()> {
        let regions = std::mem::take(&mut *self.pending.borrow_mut());
        for region in regions {
            let document = region.document;
            self.regions.insert(document, region);
            let outcome = self.mount_module(surface, waker.clone(), document, self.module.clone(), &retry);
            if let (RecoveryPolicy::Propagate, ReloadOutcome::Rejected(error)) = (self.recovery, outcome) {
                return Err(error);
            }
        }
        self.sessions.retain(|document, session| {
            let present = surface.surface(*document).is_some();
            if !present { let _ = session.retire(); }
            present
        });
        self.fallbacks.retain(|document, recovery| {
            let present = surface.surface(*document).is_some();
            if !present { recovery.view.clear(); }
            present
        });
        self.regions.retain(|document, _| surface.surface(*document).is_some());
        Ok(())
    }

    /// Evaluate a candidate generation on an existing mount region.
    pub(crate) fn replace(&mut self, surface: &mut impl SessionSurface, waker: Arc<dyn Waker>, document: DocumentId, module: SessionModule, retry: Rc<dyn Fn(DocumentId, u64)>) -> ReloadOutcome {
        self.mount_module(surface, waker, document, module, &retry)
    }

    /// Run each mounted session's work turn. Under `Fallback` a session fault
    /// retires that generation and shows the fallback; under `Propagate` the
    /// first fault is handed to the caller with the session still owned.
    pub(crate) fn checkpoint(&mut self, surface: &mut impl SessionSurface, retry: Rc<dyn Fn(DocumentId, u64)>) -> SessionResult<()> {
        let mut faults = Vec::new();
        for session in self.sessions.values() {
            match session.checkpoint() {
                Ok(true) => surface.request_frame(session.region.document),
                Ok(false) => {}
                Err(error) => faults.push((session.region.document, session.generation, error)),
            }
        }
        for (document, generation, error) in faults {
            eprintln!("session-fault: generation={generation} {error}");
            match self.recovery {
                RecoveryPolicy::Fallback => self.show_fallback(surface, document, generation, &error.to_string(), &retry),
                RecoveryPolicy::Propagate => return Err(error),
            }
        }
        Ok(())
    }

    /// Explicitly retry the last successfully mounted package after a fallback.
    /// A retry for any other generation is a stale delivery and is ignored.
    pub(crate) fn retry(&mut self, surface: &mut impl SessionSurface, waker: Arc<dyn Waker>, document: DocumentId, failed_generation: u64, retry: Rc<dyn Fn(DocumentId, u64)>) -> Option<ReloadOutcome> {
        if self.fallbacks.get(&document).is_some_and(|recovery| recovery.generation == failed_generation)
            && let Some(module) = self.last_good.clone()
        {
            return Some(self.mount_module(surface, waker, document, module, &retry));
        }
        None
    }

    /// Shut the document's session down once and return it so the caller can
    /// inspect post-cleanup counts, cleanup failure, or contract failure.
    pub(crate) fn retire(&mut self, document: DocumentId) -> Option<Rc<Session>> {
        if let Some(mut recovery) = self.fallbacks.remove(&document) { recovery.view.clear(); }
        self.regions.remove(&document);
        let session = self.sessions.remove(&document)?;
        let _ = session.retire();
        Some(session)
    }

    /// Release every owned session, fallback view, region and pending intake.
    pub(crate) fn stop(&mut self) {
        for session in self.sessions.values() { let _ = session.retire(); }
        for recovery in self.fallbacks.values_mut() { recovery.view.clear(); }
        self.sessions.clear();
        self.fallbacks.clear();
        self.regions.clear();
        self.pending.borrow_mut().clear();
    }

    fn mount_module(&mut self, surface: &mut impl SessionSurface, waker: Arc<dyn Waker>, document: DocumentId, module: SessionModule, retry: &Rc<dyn Fn(DocumentId, u64)>) -> ReloadOutcome {
        let outcome = self.mount_module_inner(surface, waker, document, module, retry);
        report_reload(&outcome);
        outcome
    }

    fn mount_module_inner(&mut self, surface: &mut impl SessionSurface, waker: Arc<dyn Waker>, document: DocumentId, module: SessionModule, retry: &Rc<dyn Fn(DocumentId, u64)>) -> ReloadOutcome {
        let Some(region) = self.regions.get(&document).cloned() else {
            return ReloadOutcome::Rejected("mount region no longer exists".into());
        };
        let Some(target) = surface.surface(document) else {
            return ReloadOutcome::Rejected("mount region has no window".into());
        };
        // A generation is allocated before preflight and never reused, so a
        // rejected candidate still consumes its identity.
        let Some(generation) = self.generation.checked_add(1) else {
            return ReloadOutcome::Rejected("session generation exhausted".into());
        };
        self.generation = generation;
        // Candidate evaluation owns no native children or stylesheet and cannot
        // create work. Rejection must leave the active session untouched.
        let candidate = match Session::prepare(region, module.clone(), generation, waker, target, self.policy.clone()) {
            Ok(candidate) => candidate,
            Err(error) => {
                if matches!(self.recovery, RecoveryPolicy::Fallback) && !self.sessions.contains_key(&document) {
                    self.show_fallback(surface, document, generation, &error.to_string(), retry);
                }
                return ReloadOutcome::Rejected(error);
            }
        };
        if let Some(previous) = self.sessions.remove(&document) { let _ = previous.retire(); }
        self.clear_fallback(document);
        surface.bind(document, &candidate);
        // Propagate keeps a failed candidate owned: its caller persists the
        // cause before any disposal, including a blocked disposer, runs.
        if matches!(self.recovery, RecoveryPolicy::Propagate) {
            self.sessions.insert(document, candidate.clone());
        }
        if let Err(error) = candidate.mount() {
            match self.recovery {
                RecoveryPolicy::Fallback => {
                    let _ = candidate.retire();
                    self.show_fallback(surface, document, generation, &error.to_string(), retry);
                    return ReloadOutcome::Fallback(error);
                }
                RecoveryPolicy::Propagate => return ReloadOutcome::Rejected(error),
            }
        }
        self.module = module.clone();
        if module.package.is_some() { self.last_good = Some(module); }
        self.sessions.insert(document, candidate);
        surface.request_frame(document);
        ReloadOutcome::Mounted(generation)
    }

    fn clear_fallback(&mut self, document: DocumentId) {
        if let Some(mut previous) = self.fallbacks.remove(&document) { previous.view.clear(); }
    }

    fn show_fallback(&mut self, surface: &mut impl SessionSurface, document: DocumentId, generation: u64, diagnostic: &str, retry: &Rc<dyn Fn(DocumentId, u64)>) {
        if let Some(session) = self.sessions.remove(&document) { let _ = session.retire(); }
        self.clear_fallback(document);
        let Some(region) = self.regions.get(&document).cloned() else { return; };
        let action = self.last_good.as_ref().map(|_| {
            let retry = retry.clone();
            Rc::new(move || retry(document, generation)) as Rc<dyn Fn()>
        });
        let view = fallback::Fallback::new(region, diagnostic, action);
        self.fallbacks.insert(document, Recovery { generation, view });
        surface.request_frame(document);
    }
}

#[cfg(test)]
mod tests;
