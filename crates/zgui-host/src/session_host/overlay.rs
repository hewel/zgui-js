//! Overlay host state machine: observation window, lock-screen coalescing slot,
//! and anti-resurrection Safe Base Shell rollback tracking.
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use zgui::{platform::{PlatformCx, Waker}, runtime::Runtime, view::DocumentId};

use super::{ImageChoice, ReloadOutcome, RuntimeSurface, SessionHost, SessionSurface, surfaces::platform_lock_state};
use crate::session::{Result as SessionResult, Session};

use crate::{
    bytecode_cache::BytecodeCache,
    extensions::{
        bridge::{OverlayFailureInfo, OverlayPhase},
        manifest::{FailedOverlayMeta, SharedRuntimeAbi, VerifiedOverlay},
    },
    handoff::rollback_permitted,
};

pub(crate) const DEFAULT_OBSERVATION_MS: u64 = 1500;

pub(super) struct ObservationWindow {
    pub(super) generation: u64,
    pub(super) overlay: Rc<VerifiedOverlay>,
    pub(super) started_at: Instant,
    pub(super) duration: Duration,
}

pub(super) struct OverlayHostState {
    pub(super) enabled: bool,
    pub(super) active_overlay: Option<Rc<VerifiedOverlay>>,
    pub(super) last_good_overlay: Option<Rc<VerifiedOverlay>>,
    pub(super) pending_overlay: Option<PathBuf>,
    pub(super) observation: Option<ObservationWindow>,
    pub(super) observation_duration: Duration,
    pub(super) bytecode_cache: Option<Rc<BytecodeCache>>,
    pub(super) host_abi: SharedRuntimeAbi,
    pub(super) safe_base_shell: bool,
    pub(super) last_failure: Option<OverlayFailureInfo>,
    pub(super) is_rollback_mount: bool,
}

struct ActiveOverlaySnapshot {
    active_overlay: Option<Rc<VerifiedOverlay>>,
    safe_base_shell: bool,
    last_failure: Option<OverlayFailureInfo>,
}

impl Default for OverlayHostState {
    fn default() -> Self {
        Self {
            enabled: false,
            active_overlay: None,
            last_good_overlay: None,
            pending_overlay: None,
            observation: None,
            observation_duration: Duration::from_millis(DEFAULT_OBSERVATION_MS),
            bytecode_cache: None,
            host_abi: SharedRuntimeAbi::CURRENT,
            safe_base_shell: false,
            last_failure: None,
            is_rollback_mount: false,
        }
    }
}

pub(super) struct OverlayRecovery<'a> {
    pub(super) phase: &'static str,
    pub(super) reason: String,
    pub(super) failed_meta: &'a FailedOverlayMeta,
}

pub(super) enum RollbackPlan {
    KeepActive(OverlayFailureInfo),
    MountLastGood {
        overlay: Rc<VerifiedOverlay>,
        failure: OverlayFailureInfo,
    },
    SafeBaseShell(OverlayFailureInfo),
}

fn failed_meta(overlay: &VerifiedOverlay) -> FailedOverlayMeta {
    FailedOverlayMeta {
        overlay_id: Some(overlay.overlay_id.clone()),
        rollback_allowed_extensions: overlay.rollback_allowed_extensions.clone(),
        client_metadata: overlay.client_metadata.clone(),
    }
}

impl OverlayHostState {
    pub(super) fn policy_overlay(&self) -> Option<Rc<VerifiedOverlay>> {
        if self.safe_base_shell {
            None
        } else {
            self.active_overlay.clone()
        }
    }

    pub(super) fn initial_notification_phase(&self) -> OverlayPhase {
        if self.safe_base_shell {
            OverlayPhase::SafeBaseShell
        } else if self.active_overlay.is_some() {
            OverlayPhase::Observing
        } else {
            OverlayPhase::Base
        }
    }

    pub(super) fn start_observation(&mut self, generation: u64, overlay: Rc<VerifiedOverlay>) {
        self.observation = Some(ObservationWindow {
            generation,
            overlay,
            started_at: Instant::now(),
            duration: self.observation_duration,
        });
    }

    fn replace_active(
        &mut self,
        overlay: Option<Rc<VerifiedOverlay>>,
        failure: Option<OverlayFailureInfo>,
    ) -> ActiveOverlaySnapshot {
        let safe_base_shell = overlay.is_none();
        ActiveOverlaySnapshot {
            active_overlay: std::mem::replace(&mut self.active_overlay, overlay),
            safe_base_shell: std::mem::replace(&mut self.safe_base_shell, safe_base_shell),
            last_failure: std::mem::replace(&mut self.last_failure, failure),
        }
    }

    fn restore(&mut self, snapshot: ActiveOverlaySnapshot) {
        self.active_overlay = snapshot.active_overlay;
        self.safe_base_shell = snapshot.safe_base_shell;
        self.last_failure = snapshot.last_failure;
    }

    pub(super) fn plan_failure_recovery(
        &mut self,
        phase: &'static str,
        reason: String,
        failed_meta: &FailedOverlayMeta,
        active_is_live: bool,
    ) -> RollbackPlan {
        self.observation = None;
        let allowed = &failed_meta.rollback_allowed_extensions;
        let failed_overlay_id = failed_meta.overlay_id.as_deref();

        // Check whether keeping the still-live active generation is allowed when Phase 1 rejected.
        if active_is_live
            && matches!(phase, "preflight" | "mount")
            && let Some(active) = &self.active_overlay
            && rollback_permitted(&active.extension_ids, allowed)
        {
            let info = OverlayFailureInfo {
                overlay_id: failed_meta.overlay_id.clone(),
                phase,
                reason,
                rollback_blocked_by_policy: false,
                client_metadata: failed_meta.client_metadata.clone(),
            };
            return RollbackPlan::KeepActive(info);
        }

        let rollback_blocked_by_policy = self
            .last_good_overlay
            .as_ref()
            .is_some_and(|lg| !rollback_permitted(&lg.extension_ids, allowed));
        if rollback_blocked_by_policy {
            // Never allow later resurrection of an extension absent from rollback_allowed_extensions.
            self.last_good_overlay = None;
        }

        if let Some(lg) = self.last_good_overlay.clone()
            && Some(lg.overlay_id.as_str()) != failed_overlay_id
            && rollback_permitted(&lg.extension_ids, allowed)
        {
            let info = OverlayFailureInfo {
                overlay_id: failed_meta.overlay_id.clone(),
                phase,
                reason,
                rollback_blocked_by_policy: false,
                client_metadata: failed_meta.client_metadata.clone(),
            };
            return RollbackPlan::MountLastGood { overlay: lg, failure: info };
        }

        let info = OverlayFailureInfo {
            overlay_id: failed_meta.overlay_id.clone(),
            phase,
            reason,
            rollback_blocked_by_policy,
            client_metadata: failed_meta.client_metadata.clone(),
        };
        RollbackPlan::SafeBaseShell(info)
    }
}

impl SessionHost {
    #[cfg(test)]
    pub(crate) fn set_observation_duration(&mut self, duration: Duration) {
        self.overlay.observation_duration = duration;
    }

    /// Coalesce publications until this host has an unlocked mount target.
    pub(crate) fn queue_overlay_update(&mut self, path: PathBuf) {
        self.overlay.enabled = true;
        self.overlay.pending_overlay = Some(path);
    }

    pub(super) fn drain_pending_overlay_if_unlocked(
        &mut self,
        runtime: &mut Runtime,
        cx: &dyn PlatformCx,
        images: ImageChoice,
        retry: &Rc<dyn Fn(DocumentId, u64)>,
    ) {
        if self.overlay.pending_overlay.is_none() || cx.is_exiting()
            || self.is_session_locked(platform_lock_state(cx))
        {
            return;
        }
        let document = if self.is_pure_shell() {
            None
        } else {
            let Some(document) = self.regions.keys().copied().find(|document| {
                runtime.windows().iter().any(|window| window.document_id() == *document)
            }) else { return; };
            Some(document)
        };
        let Some(path) = self.overlay.pending_overlay.take() else { return; };
        if let Some(document) = document {
            let mut surface = RuntimeSurface::new(runtime, cx, images);
            self.apply_overlay_update_primary(&mut surface, cx.waker(), retry, document, &path);
        } else {
            self.apply_overlay(&path, None, |host| host.replace_shell(runtime, cx, images, host.module.clone()));
        }
    }

    pub(super) fn apply_overlay_update_primary(
        &mut self,
        surface: &mut impl SessionSurface,
        waker: Arc<dyn Waker>,
        retry: &Rc<dyn Fn(DocumentId, u64)>,
        document: DocumentId,
        path: &Path,
    ) {
        self.apply_overlay(path, Some(document),
            |host| host.mount_module(surface, waker.clone(), document, host.module.clone(), retry));
    }

    fn overlay_session(&self, document: Option<DocumentId>) -> Option<&Session> {
        match document {
            Some(document) => self.sessions.get(&document),
            None => self.pure_session.as_ref(),
        }.map(Rc::as_ref).filter(|session| !session.is_retired())
    }

    fn apply_overlay(
        &mut self,
        path: &Path,
        document: Option<DocumentId>,
        mut mount: impl FnMut(&mut Self) -> ReloadOutcome,
    ) {
        let verified = match self.verify_candidate_overlay(path) {
            Ok(verified) => verified,
            Err((reason, failed_meta)) => {
                let _ = self.recover_overlay(document, OverlayRecovery { phase: "preflight", reason, failed_meta: &failed_meta }, mount);
                return;
            }
        };
        let failed_meta = failed_meta(&verified);
        let previous = self.overlay.replace_active(Some(verified), None);
        if let ReloadOutcome::Rejected(error) | ReloadOutcome::Fallback(error) = mount(self) {
            let phase = if self.overlay_session(document).is_some() { "mount" } else { "commit" };
            self.overlay.restore(previous);
            let _ = self.recover_overlay(document, OverlayRecovery {
                phase, reason: error.to_string(), failed_meta: &failed_meta,
            }, mount);
        }
    }

    pub(super) fn verify_candidate_overlay(
        &self,
        path: &Path,
    ) -> Result<Rc<VerifiedOverlay>, (String, FailedOverlayMeta)> {
        let verified = VerifiedOverlay::load_from_path(path, self.overlay.host_abi)?;
        if let Some(pkg) = self.module.as_package()
            && let Err(reason) = pkg.manifest.capabilities.merge_overlay(&verified.effective_host_capabilities)
        {
            let meta = FailedOverlayMeta {
                overlay_id: Some(verified.overlay_id),
                rollback_allowed_extensions: verified.rollback_allowed_extensions,
                client_metadata: verified.client_metadata,
            };
            return Err((reason, meta));
        }
        Ok(Rc::new(verified))
    }

    pub(super) fn recover_primary_overlay(
        &mut self,
        surface: &mut impl SessionSurface,
        waker: Arc<dyn Waker>,
        retry: &Rc<dyn Fn(DocumentId, u64)>,
        document: DocumentId,
        recovery: OverlayRecovery<'_>,
    ) -> SessionResult<()> {
        self.recover_overlay(Some(document), recovery,
            |host| host.mount_module(surface, waker.clone(), document, host.module.clone(), retry))
    }

    pub(super) fn recover_overlay(
        &mut self,
        document: Option<DocumentId>,
        recovery: OverlayRecovery<'_>,
        mut mount: impl FnMut(&mut Self) -> ReloadOutcome,
    ) -> SessionResult<()> {
        let active_is_live = self.overlay_session(document).is_some();
        let failure = match self.overlay.plan_failure_recovery(
            recovery.phase, recovery.reason, recovery.failed_meta, active_is_live,
        ) {
            RollbackPlan::KeepActive(info) => {
                if let Some(session) = self.overlay_session(document) {
                    session.record_overlay_failure(info.clone());
                }
                self.overlay.last_failure = Some(info);
                return Ok(());
            }
            RollbackPlan::MountLastGood { overlay, failure } => {
                if matches!(self.mount_overlay_recovery(Some(overlay), failure.clone(), &mut mount), ReloadOutcome::Mounted { .. }) {
                    return Ok(());
                }
                failure
            }
            RollbackPlan::SafeBaseShell(failure) => failure,
        };
        match self.mount_overlay_recovery(None, failure, mount) {
            ReloadOutcome::Mounted { .. } => Ok(()),
            ReloadOutcome::Rejected(error) | ReloadOutcome::Fallback(error) => Err(error),
        }
    }

    fn mount_overlay_recovery(
        &mut self,
        overlay: Option<Rc<VerifiedOverlay>>,
        failure: OverlayFailureInfo,
        mut mount: impl FnMut(&mut Self) -> ReloadOutcome,
    ) -> ReloadOutcome {
        self.overlay.replace_active(overlay, Some(failure));
        self.overlay.is_rollback_mount = true;
        let outcome = mount(self);
        self.overlay.is_rollback_mount = false;
        outcome
    }

    #[cfg(test)]
    pub(crate) fn is_safe_base_shell(&self) -> bool { self.overlay.safe_base_shell }
    #[cfg(test)]
    pub(crate) fn active_overlay_id(&self) -> Option<&str> {
        self.overlay.active_overlay.as_deref().map(|o| o.overlay_id.as_str())
    }
    #[cfg(test)]
    pub(crate) fn last_good_overlay_id(&self) -> Option<&str> {
        self.overlay.last_good_overlay.as_deref().map(|o| o.overlay_id.as_str())
    }
    #[cfg(test)]
    pub(crate) fn pending_overlay_path(&self) -> Option<&Path> {
        self.overlay.pending_overlay.as_deref()
    }
}
