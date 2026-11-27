//! Shared session lifecycle: mount-region intake, generation allocation,
//! candidate preflight, replacement, fallback recovery and retirement.
//! `Application` keeps the event loop and `PreviewHost` keeps renderer probes;
//! neither owns this algorithm anymore.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
    rc::{Rc, Weak},
    sync::Arc,
};

use zgui::{
    platform::{PlatformCx, SessionLockState, SurfaceId, Waker},
    runtime::window::{ImageSourceCheckpoint, ImageSourceMountWrites},
    runtime::{HostBinding, ImagePolicy, WindowToken},
    view::{Anchor, BuildCx, DocumentId, DomHandle, ElementName, HostHandle, NodeId, View},
};

use crate::{
    assets::MAX_SAFE_ID,
    bytecode_cache::BytecodeCache,
    dev_banner,
    extensions::{bridge::OverlayFailureInfo, manifest::FailedOverlayMeta},
    fallback,
    session::{
        ResourceCounts, Result as SessionResult, Session, SessionError, SessionModule,
        SessionPolicy,
    },
};

mod overlay;
mod surfaces;
use crate::shell::Monitor;
use overlay::OverlayHostState;
use surfaces::ShellWindows;

/// The portion of a native document owned by one application session at a time.
#[derive(Clone)]
pub(crate) struct MountRegion {
    pub(crate) document: DocumentId,
    pub(crate) root: NodeId,
    pub(crate) dom: DomHandle,
    pub(crate) host: HostHandle,
    pub(crate) fonts: zgui::app::Fonts,
    pub(crate) root_anchor: Rc<Cell<NodeId>>,
}

pub(crate) struct Root(pub(crate) Rc<RefCell<Vec<MountRegion>>>);
pub(crate) struct RootAnchor(Rc<Cell<NodeId>>);
impl View for Root {
    type State = RootAnchor;
    fn build(self, cx: &mut BuildCx<'_>) -> RootAnchor {
        let root = cx.dom().create_element(ElementName::new("column"));
        // These are editing semantics, not application chrome. Keep them owned
        // by the Rust root across JS generations; application CSS may override.
        let host = cx.host_handle();
        host.install_stylesheet(
            "zgui-js-controls",
            "field { white-space: pre; } editor { white-space: pre-wrap; }",
        );
        let fonts = zgui::reactive::use_context::<zgui::app::Fonts>()
            .expect("native mount roots require the renderer font collection");
        let root_anchor = Rc::new(Cell::new(root));
        self.0.borrow_mut().push(MountRegion {
            document: cx.document(),
            root,
            dom: cx.dom_handle(),
            host,
            fonts,
            root_anchor: root_anchor.clone(),
        });
        RootAnchor(root_anchor)
    }
    fn rebuild(self, _state: &mut RootAnchor, _cx: &mut BuildCx<'_>) {}
}
impl Anchor for RootAnchor {
    fn mount(&mut self, dom: &DomHandle, parent: NodeId, before: Option<NodeId>) {
        dom.insert(parent, self.0.get(), before);
    }
    fn unmount(&mut self, dom: &DomHandle) {
        dom.detach(self.0.get());
    }
    fn first_node(&self) -> Option<NodeId> {
        Some(self.0.get())
    }
}

pub(crate) struct Binding {
    owner: Weak<Session>,
    document: DocumentId,
}
impl HostBinding for Binding {
    fn checkpoint(&mut self) {
        if let Some(session) = self.owner.upgrade() {
            session.frame_checkpoint();
        }
    }
    fn before_paint(&mut self, _timestamp: zgui::vocab::Timestamp) {
        if let Some(session) = self.owner.upgrade() {
            session.frames.set(session.frames.get() + 1);
            if session.frames.get() == 1
                && let Some(owned) = session.mount_region(self.document)
            {
                let region = &owned.region;
                eprintln!(
                    "first-native-frame: {}",
                    region.dom.text_content(region.root)
                );
            }
        }
    }
    fn shutting_down(&mut self) {
        if let Some(session) = self.owner.upgrade()
            && session
                .primary()
                .is_some_and(|primary| primary.region.document == self.document)
        {
            session.dispose([]);
        }
    }
    fn stylesheet_rejected(&mut self, name: &str, diagnostics: zgui_style::CssDiagnostics) {
        if let Some(session) = self.owner.upgrade() {
            session.stylesheet_rejected(name, diagnostics);
        }
    }
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
    Mounted {
        generation: u64,
        predecessor_leak: Option<PredecessorLeak>,
        css_only: bool,
    },
    Rejected(SessionError),
    Fallback(SessionError),
}

pub(crate) struct CurrentReplacement {
    pub(crate) document: Option<DocumentId>,
    pub(crate) outcome: ReloadOutcome,
}

pub(crate) enum TurnError {
    Checkpoint(SessionError),
    Shell(SessionError),
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct PredecessorLeak {
    code: &'static str,
    generation: u64,
    resources: ResourceCounts,
}

pub(crate) fn report_reload(outcome: &ReloadOutcome) {
    match outcome {
        ReloadOutcome::Mounted {
            generation,
            css_only: true,
            ..
        } => {
            eprintln!("reload-css-swapped: generation={generation}");
        }
        ReloadOutcome::Mounted {
            generation,
            css_only: false,
            ..
        } => {
            eprintln!("reload-mounted: generation={generation}");
        }
        ReloadOutcome::Rejected(error) => eprintln!("reload-preflight-rejected: {error}"),
        ReloadOutcome::Fallback(error) => eprintln!("reload-fallback: {error}"),
    }
}

/// Native-surface seam consumed by [`SessionHost`]. `RuntimeSurface` adapts the
/// real `zgui` runtime; tests substitute a stub that records bindings and
/// frame requests. This is deliberately narrow: the host does not see the rest
/// of the renderer.
pub(crate) trait SessionSurface {
    type Checkpoint;
    type MountWrites;

    fn surface(&self, document: DocumentId) -> Option<SurfaceId>;
    /// Install the callback binding before mounting, independently of image authority.
    fn bind(&mut self, document: DocumentId, session: &Rc<Session>);
    fn image_source_checkpoint(&mut self, document: DocumentId) -> Self::Checkpoint;
    fn seal_image_source_writes(
        &self,
        document: DocumentId,
        checkpoint: Self::Checkpoint,
    ) -> Self::MountWrites;
    fn discard_image_source_writes(&mut self, document: DocumentId, checkpoint: Self::Checkpoint);
    /// Publish image authority after predecessor cleanup without discarding the new mount's sources.
    fn commit_image_policy(
        &mut self,
        document: DocumentId,
        session: &Session,
        mount_writes: Self::MountWrites,
    );
    fn request_frame(&self, document: DocumentId);
    fn validate_fonts(&self, _session: &Session) -> SessionResult<()> {
        Ok(())
    }
    fn session_lock_support(&self) -> Option<SessionLockState> {
        None
    }
    fn portal_reduced_motion(&self) -> Option<bool> {
        None
    }
    fn background_blur_supported(&self) -> bool {
        false
    }
    /// Close an auxiliary window; layer surfaces must also disappear from the platform.
    fn close_shell_surface(
        &mut self,
        token: WindowToken,
        removed_surface: Option<SurfaceId>,
    ) -> bool;
    fn stage_shell_surfaces(
        &mut self,
        staged: &mut ShellWindows,
        previous: &ShellWindows,
        monitors: &[Monitor],
        session: &Rc<Session>,
    ) -> SessionResult<()>;
}

/// Image authority installed by [`RuntimeSurface::commit_image_policy`]. Preview publishes
/// package bytes in strict launch order; the live host uses ordinary
/// asynchronous package-asset publication.
#[derive(Clone, Copy)]
pub(crate) enum ImageChoice {
    Live,
    PackageOnly,
}

/// Backing target opacity for layer and popup surfaces; lock surfaces are always opaque.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SurfaceOpacity {
    /// Every presented pixel must be opaque.
    #[default]
    Opaque,
    /// Preserve the rendered alpha channel.
    Transparent,
}

pub(crate) struct RuntimeSurface<'a> {
    runtime: &'a mut zgui::runtime::Runtime,
    cx: &'a dyn PlatformCx,
    images: ImageChoice,
}
impl<'a> RuntimeSurface<'a> {
    pub(crate) fn new(
        runtime: &'a mut zgui::runtime::Runtime,
        cx: &'a dyn PlatformCx,
        images: ImageChoice,
    ) -> Self {
        Self {
            runtime,
            cx,
            images,
        }
    }
}
impl SessionSurface for RuntimeSurface<'_> {
    type Checkpoint = ImageSourceCheckpoint;
    type MountWrites = ImageSourceMountWrites;

    fn surface(&self, document: DocumentId) -> Option<SurfaceId> {
        self.runtime
            .windows()
            .iter()
            .find(|window| window.document_id() == document)
            .map(|window| window.surface().id())
    }
    fn bind(&mut self, document: DocumentId, session: &Rc<Session>) {
        session.set_app_timers(self.runtime.app_timers());
        let window = self
            .runtime
            .windows_mut()
            .iter_mut()
            .find(|window| window.document_id() == document)
            .expect("pure preflight cannot destroy its native window");
        let surface_id = window.surface().id();
        if matches!(self.images, ImageChoice::Live) {
            session.frame_streams().set_gpu(window.renderer());
        }
        let caps_lock = {
            use zgui::reactive::prelude::GetUntracked;
            window.handle().lock_keys().get_untracked().caps_lock()
        };
        session.seed_surface_caps_lock(surface_id, caps_lock);
        if let Some(native) = self.cx.surface(surface_id) {
            session.register_platform_surface(surface_id, &native);
        }
        // Binding replacement itself neither shuts down the old owner nor
        // requests a frame. Both are explicit at this owner-thread boundary.
        window.install_binding(Box::new(Binding {
            owner: Rc::downgrade(session),
            document,
        }));
        eprintln!(
            "binding-installed: {document:?} generation={}",
            session.generation
        );
    }
    fn image_source_checkpoint(&mut self, document: DocumentId) -> ImageSourceCheckpoint {
        self.runtime
            .windows_mut()
            .iter_mut()
            .find(|window| window.document_id() == document)
            .expect("staged mount requires its native window")
            .image_source_checkpoint()
    }
    fn seal_image_source_writes(
        &self,
        document: DocumentId,
        checkpoint: ImageSourceCheckpoint,
    ) -> ImageSourceMountWrites {
        self.runtime
            .windows()
            .iter()
            .find(|window| window.document_id() == document)
            .expect("mounting cannot destroy its native window")
            .seal_image_source_writes(checkpoint)
    }
    fn discard_image_source_writes(
        &mut self,
        document: DocumentId,
        checkpoint: ImageSourceCheckpoint,
    ) {
        self.runtime
            .windows_mut()
            .iter_mut()
            .find(|window| window.document_id() == document)
            .expect("mounting cannot destroy its native window")
            .discard_image_source_writes(checkpoint);
    }
    fn commit_image_policy(
        &mut self,
        document: DocumentId,
        session: &Session,
        mount_writes: ImageSourceMountWrites,
    ) {
        let window = self
            .runtime
            .windows_mut()
            .iter_mut()
            .find(|window| window.document_id() == document)
            .expect("mounting cannot destroy its native window");
        let package = session.package();
        let prefixes = session.resolved_capabilities().image_prefixes();
        let policy = match (self.images, package) {
            (ImageChoice::Live, Some(package)) => {
                ImagePolicy::package_assets(package.image_assets()).with_file_prefixes(prefixes)
            }
            (ImageChoice::Live, None) => ImagePolicy::default(),
            (ImageChoice::PackageOnly, package) => ImagePolicy::package_only(
                package
                    .into_iter()
                    .flat_map(|package| package.image_assets()),
            )
            .with_fixture_files(prefixes, session.fixture_image_files()),
        };
        let (include_system_fonts, svg_font_faces) = session.fonts.svg_font_snapshot();
        window.replace_image_policy_with_mount_writes(
            policy
                .with_blobs(session.image_blobs().clone())
                .with_svg_fonts(include_system_fonts, svg_font_faces),
            mount_writes,
        );
        window.install_embed_host(Box::new(crate::frame_stream::Embed::new(
            session.frame_streams(),
        )));
    }
    fn request_frame(&self, document: DocumentId) {
        if let Some(window) = self
            .runtime
            .windows()
            .iter()
            .find(|window| window.document_id() == document)
        {
            window.request_frame();
        }
    }
    fn validate_fonts(&self, session: &Session) -> SessionResult<()> {
        for window in self.runtime.windows() {
            session.validate_font_families(window)?;
        }
        Ok(())
    }
    fn session_lock_support(&self) -> Option<SessionLockState> {
        surfaces::platform_lock_state(self.cx)
    }
    fn portal_reduced_motion(&self) -> Option<bool> {
        self.cx.reduced_motion()
    }
    fn background_blur_supported(&self) -> bool {
        self.cx.capabilities().supports_background_blur
    }
    fn close_shell_surface(&mut self, token: WindowToken, id: Option<SurfaceId>) -> bool {
        surfaces::close_native_surface(self.runtime, self.cx, token, id)
    }
    fn stage_shell_surfaces(
        &mut self,
        staged: &mut ShellWindows,
        previous: &ShellWindows,
        monitors: &[Monitor],
        session: &Rc<Session>,
    ) -> SessionResult<()> {
        staged.mount(
            self.runtime,
            self.cx,
            self.images,
            previous,
            monitors,
            session,
        )
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
    dev_banner_enabled: bool,
    dev_banners: HashMap<DocumentId, dev_banner::DevBanner>,
    generation: u64,
    module: SessionModule,
    last_good: Option<SessionModule>,
    policy: SessionPolicy,
    recovery: RecoveryPolicy,
    acceptance: Option<serde_json::Value>,
    monitors: Vec<Monitor>,
    pure_fonts: Option<zgui::app::Fonts>,
    pure_session: Option<Rc<Session>>,
    pure_attempted: bool,
    shell_windows: ShellWindows,
    overlay: OverlayHostState,
}

impl SessionHost {
    pub(crate) fn new(
        pending: Rc<RefCell<Vec<MountRegion>>>,
        module: SessionModule,
        policy: SessionPolicy,
        recovery: RecoveryPolicy,
        shell_opacity: SurfaceOpacity,
    ) -> Self {
        Self {
            pending,
            regions: HashMap::new(),
            sessions: HashMap::new(),
            fallbacks: HashMap::new(),
            dev_banner_enabled: false,
            dev_banners: HashMap::new(),
            generation: 0,
            module,
            last_good: None,
            policy,
            recovery,
            acceptance: None,
            monitors: Vec::new(),
            pure_fonts: None,
            pure_session: None,
            pure_attempted: false,
            shell_windows: ShellWindows::new(shell_opacity),
            overlay: OverlayHostState::default(),
        }
    }

    pub(crate) fn set_bytecode_cache(&mut self, cache: Rc<BytecodeCache>) {
        self.overlay.bytecode_cache = Some(cache);
    }

    /// Configures `--extensions-overlay <path>` and performs initial verified load before first mount.
    pub(crate) fn configure_initial_overlay(&mut self, path: &Path) {
        self.overlay.enabled = true;
        match self.verify_candidate_overlay(path) {
            Ok(verified) => {
                self.overlay.active_overlay = Some(verified);
                self.overlay.safe_base_shell = false;
                self.overlay.last_failure = None;
            }
            Err((reason, failed)) => {
                self.overlay.safe_base_shell = true;
                self.overlay.active_overlay = None;
                self.overlay.last_failure = Some(OverlayFailureInfo {
                    overlay_id: failed.overlay_id,
                    phase: "preflight",
                    reason,
                    rollback_blocked_by_policy: false,
                    client_metadata: failed.client_metadata,
                });
            }
        }
    }

    pub(crate) fn initial_overlay_digest(&self) -> Option<[u8; 32]> {
        self.overlay.active_overlay.as_ref().map(|o| o.raw_sha256)
    }

    pub(crate) fn next_overlay_deadline(&self) -> Option<std::time::Instant> {
        self.overlay
            .observation
            .as_ref()
            .map(|obs| obs.started_at + obs.duration)
    }

    pub(crate) fn tick_observation(&mut self) {
        let Some(obs) = &self.overlay.observation else {
            return;
        };
        if obs.started_at.elapsed() < obs.duration {
            return;
        }
        let target_gen = obs.generation;
        let overlay = Rc::clone(&obs.overlay);
        self.overlay.observation = None;
        let matched = self
            .sessions()
            .find(|s| s.generation == target_gen)
            .cloned();
        if let Some(session) = matched {
            session.promote_overlay_committed();
            self.overlay.last_good_overlay = Some(overlay);
        }
    }

    pub(crate) fn is_session_locked(&self, lock_state: Option<SessionLockState>) -> bool {
        lock_state.is_some_and(SessionLockState::is_retained)
            || self.shell_windows.has_active_lock()
    }

    fn active_overlay_failed_meta(&self) -> FailedOverlayMeta {
        self.overlay
            .active_overlay
            .as_ref()
            .map_or_else(FailedOverlayMeta::default, |ovl| FailedOverlayMeta {
                overlay_id: Some(ovl.overlay_id.clone()),
                rollback_allowed_extensions: ovl.rollback_allowed_extensions.clone(),
                client_metadata: ovl.client_metadata.clone(),
            })
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) fn acceptance(&self) -> Option<&serde_json::Value> {
        self.acceptance.as_ref()
    }
    pub(crate) fn set_reduced_motion(&mut self, reduced_motion: bool) {
        self.policy = self.policy.clone().with_reduced_motion(reduced_motion);
    }
    pub(crate) fn set_stack_limit_bytes(&mut self, stack_limit_bytes: usize) {
        self.policy = self
            .policy
            .clone()
            .with_stack_limit_bytes(stack_limit_bytes);
    }
    pub(crate) fn set_dev_state(&mut self, enabled: bool) {
        self.policy = self.policy.clone().with_dev_state(enabled);
    }
    pub(crate) fn set_dev_banner_enabled(&mut self, enabled: bool) {
        self.dev_banner_enabled = enabled;
        if !enabled {
            for banner in self.dev_banners.values_mut() {
                banner.clear();
            }
            self.dev_banners.clear();
        }
    }
    pub(crate) fn show_dev_banner(
        &mut self,
        surface: &mut impl SessionSurface,
        document: DocumentId,
        title: &str,
        detail: &str,
    ) {
        if !self.dev_banner_enabled
            || self.shell_windows.has_active_lock()
            || surface
                .session_lock_support()
                .is_some_and(SessionLockState::is_retained)
        {
            self.clear_dev_banner(surface, document);
            return;
        }
        let Some(region) = self.regions.get(&document).cloned() else {
            return;
        };
        let signature = dev_banner::banner_signature(title, detail);
        if self
            .dev_banners
            .get(&document)
            .is_some_and(|existing| existing.is_visible() && existing.signature() == signature)
        {
            return;
        }
        if let Some(mut old) = self.dev_banners.remove(&document) {
            old.clear();
        }
        let next = dev_banner::DevBanner::new(region, title, detail);
        self.dev_banners.insert(document, next);
        surface.request_frame(document);
    }
    pub(crate) fn clear_dev_banner(
        &mut self,
        surface: &mut impl SessionSurface,
        document: DocumentId,
    ) {
        if let Some(mut banner) = self.dev_banners.remove(&document) {
            banner.clear();
            surface.request_frame(document);
        }
    }
    #[cfg(test)]
    pub(crate) fn has_dev_banner(&self, document: DocumentId) -> bool {
        self.dev_banners
            .get(&document)
            .is_some_and(dev_banner::DevBanner::is_visible)
    }
    /// The module a fresh mount region attaches, or the last mounted one.
    pub(crate) fn module(&self) -> &SessionModule {
        &self.module
    }
    /// Adopt a publication that arrived while no mount region existed.
    pub(crate) fn set_module(&mut self, module: SessionModule) {
        self.module = module;
    }
    #[cfg(test)]
    pub(crate) fn region(&self, document: DocumentId) -> Option<&MountRegion> {
        self.regions.get(&document)
    }
    pub(crate) fn region_documents(&self) -> impl Iterator<Item = DocumentId> + '_ {
        self.regions.keys().copied()
    }
    #[cfg(test)]
    pub(crate) fn fallback_generation(&self, document: DocumentId) -> Option<u64> {
        self.fallbacks
            .get(&document)
            .map(|recovery| recovery.generation)
    }
    pub(crate) fn pending_regions(&self) -> usize {
        self.pending.borrow().len()
    }
    pub(crate) fn session(&self, document: DocumentId) -> Option<&Rc<Session>> {
        self.sessions.get(&document)
    }
    pub(crate) fn sessions(&self) -> impl Iterator<Item = &Rc<Session>> {
        self.sessions.values().chain(self.pure_session.iter())
    }

    pub(crate) fn set_launch(
        &mut self,
        initial: crate::invocation::Activation,
        inbox: Option<Arc<crate::instance::Inbox>>,
    ) {
        self.policy.set_launch(initial, inbox);
    }

    /// Execute queued session clipboard writes and reads against the platform clipboard.
    pub(crate) fn drain_clipboard(&self, cx: &dyn PlatformCx) -> bool {
        let mut produced = false;
        for session in self.sessions() {
            produced |= session.drain_clipboard(cx.clipboard());
        }
        produced
    }

    /// Route a completed platform clipboard read to the owning session if initiated via JS services.
    /// Returns `Ok(())` when consumed, or `Err(result)` when the read belongs to the native runtime.
    pub(crate) fn resolve_clipboard_read(
        &self,
        serial: zgui::platform::ClipboardSerial,
        result: Result<zgui::platform::ClipboardData, zgui::platform::ClipboardError>,
    ) -> Result<(), Result<zgui::platform::ClipboardData, zgui::platform::ClipboardError>> {
        if let Some(session) = self
            .sessions()
            .find(|session| session.owns_clipboard_read(serial))
        {
            session.finish_clipboard_read(serial, result);
            Ok(())
        } else {
            Err(result)
        }
    }

    /// Route a completed external `xdg_activation_v1` token to the owning session.
    pub(crate) fn resolve_activation_token(
        &self,
        serial: zgui::platform::ActivationTokenSerial,
        token: zgui::platform::ActivationToken,
    ) {
        if let Some(session) = self
            .sessions()
            .find(|session| session.owns_activation_token(serial))
        {
            session.finish_activation_token(serial, token);
        }
    }

    /// Deliver a platform event, retiring shell windows before their primary window closes.
    pub(crate) fn surface_event(
        &mut self,
        runtime: &mut zgui::runtime::Runtime,
        cx: &dyn PlatformCx,
        surface: SurfaceId,
        event: zgui::platform::SurfaceEvent,
    ) -> SessionResult<()> {
        use zgui::platform::{AppHandler, SurfaceEvent};
        match event {
            SurfaceEvent::CloseRequested
                if self.shell_window_close_requested(runtime, cx, surface)? => {}
            SurfaceEvent::Destroyed | SurfaceEvent::Closed(_)
                if self.shell_surface_destroyed(runtime, cx, surface)? => {}
            SurfaceEvent::CloseRequested => {
                let owner = self
                    .sessions
                    .values()
                    .find(|session| {
                        session
                            .primary()
                            .is_some_and(|primary| primary.surface == surface)
                    })
                    .cloned();
                let mut close_error = None;
                let mut vetoed_primary = false;
                runtime.surface_event_with_close_veto(
                    cx,
                    surface,
                    event,
                    || {
                        let Some(owner) = owner.as_ref() else {
                            return zgui::runtime::CloseResponse::Close;
                        };
                        match owner.request_close(surface) {
                            Ok(true) => {
                                vetoed_primary = true;
                                zgui::runtime::CloseResponse::Veto
                            }
                            Ok(false) => zgui::runtime::CloseResponse::Close,
                            Err(error) => {
                                close_error = Some(error);
                                zgui::runtime::CloseResponse::Veto
                            }
                        }
                    },
                    |runtime| {
                        if let Some(owner) = owner.as_ref() {
                            self.shell_windows.retire(owner, |token, id| {
                                surfaces::close_native_surface(runtime, cx, token, id)
                            });
                        }
                    },
                );
                if let Some(error) = close_error {
                    return Err(error);
                }
                if vetoed_primary
                    && let Some(window) = runtime
                        .windows_mut()
                        .iter_mut()
                        .find(|w| w.surface().id() == surface)
                {
                    window.request_frame();
                }
            }
            SurfaceEvent::Destroyed | SurfaceEvent::Closed(_) => {
                let sessions: Vec<Rc<Session>> = self.sessions().cloned().collect();
                for session in sessions {
                    let _ = session.cancel_drags_for_surface(surface);
                }
                runtime.surface_event_with_close(cx, surface, event, |runtime| {
                    let owner = self.sessions.values().find(|session| {
                        session
                            .primary()
                            .is_some_and(|primary| primary.surface == surface)
                    });
                    if let Some(owner) = owner {
                        self.shell_windows.retire(owner, |token, id| {
                            surfaces::close_native_surface(runtime, cx, token, id)
                        });
                    }
                });
            }
            SurfaceEvent::Drag(drag) => {
                let sessions: Vec<Rc<Session>> = self.sessions().cloned().collect();
                if let Some(window) = runtime
                    .windows()
                    .iter()
                    .find(|w| w.surface().id() == surface)
                {
                    for session in &sessions {
                        if session.bridge_for_surface(surface).is_some() {
                            session.reconcile_inert_drags()?;
                            session.handle_external_drag(window, surface, &drag)?;
                        }
                    }
                }
                runtime.surface_event(cx, surface, SurfaceEvent::Drag(drag));
            }
            SurfaceEvent::Pointer {
                action, event: ptr, ..
            } => {
                let pos = ptr.position;
                if self
                    .sessions()
                    .any(|s| s.bridge_for_surface(surface).is_some() && s.dnd.active_count() > 0)
                    && let Some(window) = runtime
                        .windows_mut()
                        .iter_mut()
                        .find(|w| w.surface().id() == surface)
                {
                    window.clear_press_grants();
                }
                runtime.surface_event(cx, surface, event);
                let sessions: Vec<Rc<Session>> = self.sessions().cloned().collect();
                if let Some(window) = runtime
                    .windows()
                    .iter()
                    .find(|w| w.surface().id() == surface)
                {
                    for session in &sessions {
                        if session.bridge_for_surface(surface).is_some() {
                            session.reconcile_inert_drags()?;
                            session.handle_internal_drag_pointer(window, surface, action, pos)?;
                        }
                    }
                }
            }
            SurfaceEvent::ModifiersChanged(modifiers) => {
                runtime.surface_event(cx, surface, event);
                let caps_lock = modifiers.caps_lock();
                let sessions: Vec<Rc<Session>> = self.sessions().cloned().collect();
                for session in sessions {
                    if session.sync_surface_caps_lock(surface, caps_lock)?
                        && let Some(window) = runtime
                            .windows_mut()
                            .iter_mut()
                            .find(|w| w.surface().id() == surface)
                    {
                        window.request_frame();
                    }
                }
            }
            SurfaceEvent::Key {
                state,
                event: ref key_ev,
                ..
            } => {
                let mut consumed = false;
                let sessions: Vec<Rc<Session>> = self.sessions().cloned().collect();
                for session in &sessions {
                    if session.bridge_for_surface(surface).is_some()
                        && session.handle_drag_key(state, key_ev)?
                    {
                        consumed = true;
                        break;
                    }
                }
                if !consumed {
                    runtime.surface_event(cx, surface, event);
                }
            }
            SurfaceEvent::Occluded(occluded) => {
                runtime.surface_event(cx, surface, event);
                for session in self.sessions.values().chain(self.pure_session.iter()) {
                    session.set_surface_occluded(surface, occluded);
                }
            }
            SurfaceEvent::Resized(_) => {
                runtime.surface_event(cx, surface, event);
                if let Some(native) = cx.surface(surface) {
                    let fullscreen = native.fullscreen().is_some();
                    let maximized = native.is_maximized();
                    let minimized = native.is_minimized();
                    let sessions: Vec<Rc<Session>> = self.sessions().cloned().collect();
                    for session in sessions {
                        if session.sync_window_flags(surface, fullscreen, maximized, minimized)?
                            && let Some(window) = runtime
                                .windows_mut()
                                .iter_mut()
                                .find(|w| w.surface().id() == surface)
                        {
                            window.request_frame();
                        }
                    }
                }
            }
            _ => runtime.surface_event(cx, surface, event),
        }
        Ok(())
    }

    /// Drain newly built mount regions and mount the current module on each,
    /// then retire ownership for documents whose native surface is gone.
    pub(crate) fn attach(
        &mut self,
        surface: &mut impl SessionSurface,
        waker: Arc<dyn Waker>,
        retry: Rc<dyn Fn(DocumentId, u64)>,
    ) -> SessionResult<()> {
        let regions = std::mem::take(&mut *self.pending.borrow_mut());
        for mut region in regions {
            region.root = region.root_anchor.get();
            let document = region.document;
            self.regions.insert(document, region);
            let outcome = self.mount_module(
                surface,
                waker.clone(),
                document,
                self.module.clone(),
                &retry,
            );
            if let (RecoveryPolicy::Propagate, ReloadOutcome::Rejected(error)) =
                (self.recovery, outcome)
            {
                return Err(error);
            }
        }
        while let Some(document) = self
            .sessions
            .keys()
            .copied()
            .find(|&document| surface.surface(document).is_none())
        {
            let session = self
                .sessions
                .remove(&document)
                .expect("retire the session whose surface disappeared");
            self.shell_windows
                .retire(&session, |token, id| surface.close_shell_surface(token, id));
        }
        self.fallbacks.retain(|document, recovery| {
            let present = surface.surface(*document).is_some();
            if !present {
                recovery.view.clear();
            }
            present
        });
        self.dev_banners.retain(|document, banner| {
            let present = surface.surface(*document).is_some();
            if !present {
                banner.clear();
            }
            present
        });
        self.regions
            .retain(|document, _| surface.surface(*document).is_some());
        Ok(())
    }

    /// Complete owner work for a native event before realizing shell requests.
    pub(crate) fn turn(
        &mut self,
        runtime: &mut zgui::runtime::Runtime,
        cx: &dyn PlatformCx,
        images: ImageChoice,
        monitors: &[Monitor],
        retry: &Rc<dyn Fn(DocumentId, u64)>,
    ) -> Result<(), TurnError> {
        self.checkpoint_native(runtime, cx, images, retry)
            .map_err(TurnError::Checkpoint)?;
        self.shell_turn(runtime, cx, images, monitors, retry)
            .map_err(TurnError::Shell)
    }

    /// Synchronize primary preferences, checkpoint, then dispatch any platform clipboard work.
    /// Pure shells synchronize and checkpoint when reconciling their shell surfaces.
    fn checkpoint_native(
        &mut self,
        runtime: &mut zgui::runtime::Runtime,
        cx: &dyn PlatformCx,
        images: ImageChoice,
        retry: &Rc<dyn Fn(DocumentId, u64)>,
    ) -> SessionResult<()> {
        let mut surface = RuntimeSurface::new(runtime, cx, images);
        for session in self.sessions.values() {
            if Self::sync_preferences(session, cx)? {
                session.request_frames(|document| surface.request_frame(document));
            }
        }
        self.checkpoint(&mut surface, retry.clone())?;
        if self.drain_clipboard(cx) {
            self.checkpoint(&mut surface, retry.clone())?;
        }
        Ok(())
    }

    fn sync_preferences(session: &Session, cx: &dyn PlatformCx) -> SessionResult<bool> {
        let motion = session.update_reduced_motion(cx.reduced_motion())?;
        let blur =
            session.update_background_blur_supported(cx.capabilities().supports_background_blur)?;
        Ok(motion || blur)
    }

    /// Run each mounted session's work turn. Under `Fallback` a session fault
    /// retires that generation and shows the fallback; under `Propagate` the
    /// first fault is handed to the caller with the session still owned.
    pub(crate) fn checkpoint(
        &mut self,
        surface: &mut impl SessionSurface,
        retry: Rc<dyn Fn(DocumentId, u64)>,
    ) -> SessionResult<()> {
        let mut faults = Vec::new();
        for (&document, session) in &self.sessions {
            if let Err(error) = surface.validate_fonts(session) {
                faults.push((document, session.generation, error));
                continue;
            }
            match session.checkpoint() {
                Ok(true) => {
                    if let Err(error) = session.reconcile_inert_drags() {
                        faults.push((document, session.generation, error));
                        continue;
                    }
                    session.request_frames(|document| surface.request_frame(document));
                }
                Ok(false) => {
                    if let Err(error) = session.reconcile_inert_drags() {
                        faults.push((document, session.generation, error));
                        continue;
                    }
                }
                Err(error) => faults.push((document, session.generation, error)),
            }
        }
        for (document, generation, error) in faults {
            eprintln!("session-fault: generation={generation} {error}");
            let observing = self
                .overlay
                .observation
                .as_ref()
                .is_some_and(|obs| obs.generation == generation);
            if observing && self.is_session_locked(surface.session_lock_support()) {
                self.overlay.observation = None;
            } else if observing {
                let failed_meta = self.active_overlay_failed_meta();
                if let Some(session) = self.sessions.remove(&document) {
                    let waker = session.waker();
                    self.shell_windows
                        .retire(&session, |token, id| surface.close_shell_surface(token, id));
                    let _ = self.recover_primary_overlay(
                        surface,
                        waker,
                        &retry,
                        document,
                        overlay::OverlayRecovery {
                            phase: "observation",
                            reason: error.to_string(),
                            failed_meta: &failed_meta,
                        },
                    );
                }
                continue;
            }
            match self.recovery {
                RecoveryPolicy::Fallback => {
                    self.show_fallback(surface, document, generation, &error.to_string(), &retry)
                }
                RecoveryPolicy::Propagate => {
                    if let Some(session) = self.sessions.get(&document) {
                        self.shell_windows.hide_lock_ui(session);
                    }
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Explicitly retry the last successfully mounted package after a fallback.
    /// A retry for any other generation is a stale delivery and is ignored.
    pub(crate) fn retry(
        &mut self,
        surface: &mut impl SessionSurface,
        waker: Arc<dyn Waker>,
        document: DocumentId,
        failed_generation: u64,
        retry: Rc<dyn Fn(DocumentId, u64)>,
    ) -> Option<ReloadOutcome> {
        if self
            .fallbacks
            .get(&document)
            .is_some_and(|recovery| recovery.generation == failed_generation)
            && let Some(module) = self.last_good.clone()
        {
            return Some(self.mount_module(surface, waker, document, module, &retry));
        }
        None
    }

    /// Shut the document's session down once and return it so the caller can
    /// inspect post-cleanup counts, cleanup failure, or contract failure.
    #[cfg(test)]
    pub(crate) fn retire(&mut self, document: DocumentId) -> Option<Rc<Session>> {
        if let Some(mut recovery) = self.fallbacks.remove(&document) {
            recovery.view.clear();
        }
        if let Some(mut banner) = self.dev_banners.remove(&document) {
            banner.clear();
        }
        self.regions.remove(&document);
        let session = self.sessions.remove(&document)?;
        let _ = session.retire();
        Some(session)
    }

    /// Release every owned session, fallback view, region and pending intake.
    pub(crate) fn stop(&mut self, surface: &mut impl SessionSurface) {
        let generation = self.shell_windows.generation;
        let owner = self
            .sessions()
            .find(|session| Some(session.generation) == generation)
            .cloned();
        if let Some(owner) = owner {
            self.shell_windows
                .retire(&owner, |token, id| surface.close_shell_surface(token, id));
        }
        for session in self.sessions() {
            let _ = session.retire();
        }
        for recovery in self.fallbacks.values_mut() {
            recovery.view.clear();
        }
        for banner in self.dev_banners.values_mut() {
            banner.clear();
        }
        self.sessions.clear();
        self.pure_session = None;
        self.pure_attempted = false;
        self.fallbacks.clear();
        self.dev_banners.clear();
        self.regions.clear();
        self.pending.borrow_mut().clear();
    }

    fn mount_module(
        &mut self,
        surface: &mut impl SessionSurface,
        waker: Arc<dyn Waker>,
        document: DocumentId,
        module: SessionModule,
        retry: &Rc<dyn Fn(DocumentId, u64)>,
    ) -> ReloadOutcome {
        let outcome = self.mount_module_inner(surface, waker, document, module.clone(), retry);
        self.record_outcome(&module, Some(document), &outcome);
        outcome
    }

    pub(super) fn record_outcome(
        &mut self,
        module: &SessionModule,
        document: Option<DocumentId>,
        outcome: &ReloadOutcome,
    ) {
        let package = module.as_package();
        let identity = module.as_package().map(|package| serde_json::json!({
            "id":package.manifest.id, "buildId":package.manifest.build_id, "styleSystem":package.manifest.style_system
        }));
        if let Some(identity) = identity {
            let (status, diagnostic) = match &outcome {
                ReloadOutcome::Mounted { .. } => ("accepted", None),
                ReloadOutcome::Rejected(error) | ReloadOutcome::Fallback(error) => {
                    ("rejected", Some(error.to_string()))
                }
            };
            let generation = match outcome {
                ReloadOutcome::Mounted { generation, .. } => *generation,
                _ => self.generation,
            };
            let mut record = serde_json::json!({"schemaVersion":1,"status":status,"candidate":identity,"generation":generation,
                "document":document.map(|document| format!("{document:?}")),"diagnostic":diagnostic,
                "lastSuccessfulBuildId":self.last_good.as_ref().and_then(SessionModule::as_package).map(|package|&package.manifest.build_id)});
            if let ReloadOutcome::Mounted {
                predecessor_leak: Some(leak),
                ..
            } = &outcome
            {
                record["predecessorLeak"] = serde_json::json!(leak);
            }
            if let ReloadOutcome::Mounted { css_only: true, .. } = &outcome {
                record["cssOnly"] = serde_json::json!(true);
            }
            eprintln!("native-candidate: {record}");
            if let Some(package) = package
                && let Err(error) = package.record_acceptance(&record)
            {
                eprintln!("native-candidate-status-error: {error}");
            }
            self.acceptance = Some(record);
        }
        report_reload(outcome);
    }

    /// Rejected candidates still consume their generation; no unrepresentable ID is issued.
    fn allocate_generation(current: &mut u64) -> Option<u64> {
        let generation = current.checked_add(1).filter(|&id| id <= MAX_SAFE_ID)?;
        *current = generation;
        Some(generation)
    }

    fn mount_module_inner(
        &mut self,
        surface: &mut impl SessionSurface,
        waker: Arc<dyn Waker>,
        document: DocumentId,
        module: SessionModule,
        retry: &Rc<dyn Fn(DocumentId, u64)>,
    ) -> ReloadOutcome {
        let Some(region) = self.regions.get(&document).cloned() else {
            return ReloadOutcome::Rejected("mount region no longer exists".into());
        };
        let Some(target) = surface.surface(document) else {
            return ReloadOutcome::Rejected("mount region has no window".into());
        };
        let lock_state = surface.session_lock_support();
        let previous = self.sessions.get(&document).cloned();

        // Fast path (dev mode): CSS-only live reload swaps Stylo stylesheets in
        // place without restarting QuickJS or losing local component state.
        if self.policy.dev_state_enabled()
            && let Some(prev_session) = &previous
            && !self.fallbacks.contains_key(&document)
            && !lock_state.is_some_and(SessionLockState::is_retained)
            && !self.shell_windows.has_active_lock()
            && self.overlay.observation.is_none()
            && !self.overlay.is_rollback_mount
            && let Some(prev_pkg) = prev_session.active_package()
            && let Some(next_pkg) = module.as_package()
            && prev_pkg.is_css_only_successor(next_pkg)
        {
            match prev_session.replace_packaged_stylesheets(next_pkg) {
                Ok(()) => {
                    self.module = module.clone();
                    self.last_good = Some(module);
                    prev_session.request_frames(|doc| surface.request_frame(doc));
                    return ReloadOutcome::Mounted {
                        generation: prev_session.generation,
                        predecessor_leak: None,
                        css_only: true,
                    };
                }
                Err(error) => {
                    eprintln!("css-live-swap-fallback: {error}");
                }
            }
        }

        let Some(generation) = Self::allocate_generation(&mut self.generation) else {
            return ReloadOutcome::Rejected("session generation exhausted".into());
        };
        let mut candidate_region = region.clone();
        let staging_parent = if previous.is_some() {
            let Some(parent) = region.dom.parent(region.root) else {
                return ReloadOutcome::Rejected("active mount root is detached".into());
            };
            candidate_region.root = region.dom.create_element(ElementName::new("column"));
            Some(parent)
        } else {
            None
        };
        // Candidate evaluation owns no native children or stylesheet and cannot
        // create work. Rejection must leave the active session untouched.
        let hydrated = if self.policy.dev_state_enabled()
            && !lock_state.is_some_and(SessionLockState::is_retained)
            && !self.shell_windows.has_active_lock()
        {
            previous
                .as_ref()
                .and_then(|prev| prev.capture_dev_state(&module))
        } else {
            None
        };
        let policy = self
            .policy
            .clone()
            .with_session_lock(lock_state)
            .with_hydrated_dev_state(hydrated)
            .with_extension_overlay(self.overlay.policy_overlay())
            .with_bytecode_cache(self.overlay.bytecode_cache.clone())
            .with_safe_base_shell(self.overlay.safe_base_shell)
            .with_last_overlay_failure(self.overlay.last_failure.clone());
        let candidate = match Session::prepare(
            candidate_region.clone(),
            module.clone(),
            generation,
            waker,
            target,
            policy,
        ) {
            Ok(candidate) => candidate,
            Err(error) => {
                if let Some(parent) = staging_parent {
                    region
                        .dom
                        .insert(parent, candidate_region.root, Some(region.root));
                    region.dom.detach(candidate_region.root);
                }
                if matches!(self.recovery, RecoveryPolicy::Fallback)
                    && !self.sessions.contains_key(&document)
                {
                    self.show_fallback(surface, document, generation, &error.to_string(), retry);
                }
                return ReloadOutcome::Rejected(error);
            }
        };
        if let Err(error) = candidate.update_monitors(&self.monitors) {
            return ReloadOutcome::Rejected(error);
        }
        let _ = candidate.update_reduced_motion(surface.portal_reduced_motion());
        let _ = candidate.update_background_blur_supported(surface.background_blur_supported());
        surface.bind(document, &candidate);
        let image_checkpoint = surface.image_source_checkpoint(document);
        let mut staged = self.shell_windows.empty_successor();
        candidate.begin_staged_mount();
        let mounted = candidate.mount().and_then(|()| {
            surface.stage_shell_surfaces(
                &mut staged,
                &self.shell_windows,
                &self.monitors,
                &candidate,
            )
        });
        if let Err(error) = mounted {
            if let Some(previous) = previous {
                candidate.relinquish_stylesheets();
                staged.retire(&candidate, |token, id| {
                    surface.close_shell_surface(token, id)
                });
                candidate.remove_surplus_stylesheets(&previous);
                previous.restore_stylesheets();
                if let Some(parent) = staging_parent {
                    region
                        .dom
                        .insert(parent, candidate_region.root, Some(region.root));
                    region.dom.detach(candidate_region.root);
                }
                surface.discard_image_source_writes(document, image_checkpoint);
                surface.bind(document, &previous);
                surface.request_frame(document);
                return ReloadOutcome::Rejected(error);
            }
            match self.recovery {
                RecoveryPolicy::Fallback => {
                    staged.retire(&candidate, |token, id| {
                        surface.close_shell_surface(token, id)
                    });
                    surface.discard_image_source_writes(document, image_checkpoint);
                    self.show_fallback(surface, document, generation, &error.to_string(), retry);
                    return ReloadOutcome::Fallback(error);
                }
                RecoveryPolicy::Propagate => {
                    surface.discard_image_source_writes(document, image_checkpoint);
                    if staged.has_windows() {
                        staged.retire(&candidate, |token, id| {
                            surface.close_shell_surface(token, id)
                        });
                    }
                    self.sessions.insert(document, candidate);
                    return ReloadOutcome::Rejected(error);
                }
            }
        }
        let mount_writes = surface.seal_image_source_writes(document, image_checkpoint);
        let capture_handoff = self.overlay.enabled
            && !self.overlay.safe_base_shell
            && !self.overlay.is_rollback_mount;
        let mut handoff = None;
        let locked = self.is_session_locked(surface.session_lock_support());
        let predecessor_leak = if let Some(previous) = previous {
            if let Some(parent) = staging_parent {
                region
                    .dom
                    .insert(parent, candidate_region.root, Some(region.root));
                region.root_anchor.set(candidate_region.root);
                self.regions.insert(document, candidate_region);
            }
            previous.invoke_before_commit();
            if capture_handoff && !locked {
                handoff = previous.capture_session_handoff();
            }
            previous.relinquish_stylesheets();
            previous.remove_surplus_stylesheets(&candidate);
            let report = self.shell_windows.retire(&previous, |token, id| {
                surface.close_shell_surface(token, id)
            });
            region.dom.detach(region.root);
            staged.fence_after_predecessor(
                report,
                &candidate,
                surface.session_lock_support(),
                |token, id| surface.close_shell_surface(token, id),
            )
        } else {
            None
        };
        self.shell_windows = staged;
        surface.commit_image_policy(document, &candidate, mount_writes);
        if capture_handoff {
            candidate.set_incoming_handoff(handoff);
        }
        candidate.commit_staged_mount();
        if self.overlay.enabled {
            let phase = self.overlay.initial_notification_phase();
            if let Err(error) = candidate.notify_session_committed(phase) {
                self.shell_windows.retire(&candidate, |token, id| {
                    surface.close_shell_surface(token, id)
                });
                // The previous generation has already retired; do not offer it for rollback.
                self.sessions.remove(&document);
                return ReloadOutcome::Rejected(error);
            }
            if !self.overlay.safe_base_shell
                && let Some(overlay) = self.overlay.active_overlay.clone()
            {
                self.overlay.start_observation(generation, overlay);
            }
        }
        self.clear_fallback(document);
        self.module = module.clone();
        if module.as_package().is_some() {
            self.last_good = Some(module);
        }
        self.sessions.insert(document, candidate);
        surface.request_frame(document);
        ReloadOutcome::Mounted {
            generation,
            predecessor_leak,
            css_only: false,
        }
    }

    fn clear_fallback(&mut self, document: DocumentId) {
        if let Some(mut previous) = self.fallbacks.remove(&document) {
            previous.view.clear();
        }
    }

    fn show_fallback(
        &mut self,
        surface: &mut impl SessionSurface,
        document: DocumentId,
        generation: u64,
        diagnostic: &str,
        retry: &Rc<dyn Fn(DocumentId, u64)>,
    ) {
        if let Some(session) = self.sessions.remove(&document) {
            self.shell_windows
                .retire(&session, |token, id| surface.close_shell_surface(token, id));
        }
        self.clear_fallback(document);
        let Some(region) = self.regions.get(&document).cloned() else {
            return;
        };
        let action = self.last_good.as_ref().map(|_| {
            let retry = retry.clone();
            Rc::new(move || retry(document, generation)) as Rc<dyn Fn()>
        });
        let view = fallback::Fallback::new(region, diagnostic, action);
        self.fallbacks
            .insert(document, Recovery { generation, view });
        surface.request_frame(document);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod overlay_tests;

#[cfg(test)]
mod activation_tests;
