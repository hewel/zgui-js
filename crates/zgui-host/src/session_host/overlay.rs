//! Overlay host state machine: observation window, lock-screen coalescing slot,
//! and anti-resurrection Safe Base Shell rollback tracking.
use std::{
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

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

impl super::SessionHost {
    #[cfg(test)]
    pub(crate) fn set_observation_duration(&mut self, duration: Duration) {
        self.overlay.observation_duration = duration;
    }

    pub(crate) fn drain_pending_overlay_pure_if_unlocked(
        &mut self,
        runtime: &mut zgui::runtime::Runtime,
        cx: &dyn zgui::platform::PlatformCx,
        images: super::ImageChoice,
    ) {
        let lock_state = super::surfaces::platform_lock_state(cx);
        if self.is_session_locked(lock_state) {
            return;
        }
        if let Some(path) = self.overlay.pending_overlay.take() {
            self.apply_overlay_update_pure(runtime, cx, images, &path);
        }
    }

    pub(crate) fn drain_pending_overlay_primary_if_unlocked(
        &mut self,
        surface: &mut impl super::SessionSurface,
        waker: std::sync::Arc<dyn zgui::platform::Waker>,
        retry: Rc<dyn Fn(zgui::view::DocumentId, u64)>,
        document: zgui::view::DocumentId,
    ) {
        if self.is_session_locked(surface.session_lock_support()) {
            return;
        }
        if let Some(path) = self.overlay.pending_overlay.take() {
            self.apply_overlay_update_primary(surface, waker, retry, document, &path);
        }
    }

    pub(crate) fn apply_overlay_update_pure(
        &mut self,
        runtime: &mut zgui::runtime::Runtime,
        cx: &dyn zgui::platform::PlatformCx,
        images: super::ImageChoice,
        path: &std::path::Path,
    ) {
        self.overlay.enabled = true;
        let lock_state = super::surfaces::platform_lock_state(cx);
        if self.is_session_locked(lock_state) {
            self.overlay.pending_overlay = Some(path.to_path_buf());
            return;
        }
        let verified = match self.verify_candidate_overlay(path) {
            Ok(v) => v,
            Err((reason, failed_meta)) => {
                let _ = self.recover_pure_shell_overlay(runtime, cx, images, "preflight", reason, &failed_meta);
                return;
            }
        };
        let failed_meta = FailedOverlayMeta {
            overlay_id: Some(verified.overlay_id.clone()),
            rollback_allowed_extensions: verified.rollback_allowed_extensions.clone(),
            client_metadata: verified.client_metadata.clone(),
        };
        let prev_active = self.overlay.active_overlay.clone();
        let prev_safe = self.overlay.safe_base_shell;
        let prev_failure = self.overlay.last_failure.clone();

        self.overlay.active_overlay = Some(verified);
        self.overlay.safe_base_shell = false;
        self.overlay.last_failure = None;
        self.overlay.is_rollback_mount = false;
        match self.replace_shell(runtime, cx, images, self.module.clone()) {
            super::ReloadOutcome::Mounted { .. } => {}
            super::ReloadOutcome::Rejected(err) | super::ReloadOutcome::Fallback(err) => {
                let phase = if self.pure_session.as_ref().is_some_and(|s| s.generation == self.generation) {
                    "commit"
                } else {
                    "mount"
                };
                self.overlay.active_overlay = prev_active;
                self.overlay.safe_base_shell = prev_safe;
                self.overlay.last_failure = prev_failure;
                let _ = self.recover_pure_shell_overlay(runtime, cx, images, phase, err.to_string(), &failed_meta);
            }
        }
    }

    pub(crate) fn apply_overlay_update_primary(
        &mut self,
        surface: &mut impl super::SessionSurface,
        waker: std::sync::Arc<dyn zgui::platform::Waker>,
        retry: Rc<dyn Fn(zgui::view::DocumentId, u64)>,
        document: zgui::view::DocumentId,
        path: &std::path::Path,
    ) {
        self.overlay.enabled = true;
        if self.is_session_locked(surface.session_lock_support()) {
            self.overlay.pending_overlay = Some(path.to_path_buf());
            return;
        }
        let verified = match self.verify_candidate_overlay(path) {
            Ok(v) => v,
            Err((reason, failed_meta)) => {
                self.recover_primary_overlay(
                    surface,
                    waker,
                    retry,
                    document,
                    OverlayRecovery { phase: "preflight", reason, failed_meta: &failed_meta },
                );
                return;
            }
        };
        let failed_meta = FailedOverlayMeta {
            overlay_id: Some(verified.overlay_id.clone()),
            rollback_allowed_extensions: verified.rollback_allowed_extensions.clone(),
            client_metadata: verified.client_metadata.clone(),
        };
        let prev_active = self.overlay.active_overlay.clone();
        let prev_safe = self.overlay.safe_base_shell;
        let prev_failure = self.overlay.last_failure.clone();

        self.overlay.active_overlay = Some(verified);
        self.overlay.safe_base_shell = false;
        self.overlay.last_failure = None;
        self.overlay.is_rollback_mount = false;
        match self.mount_module(surface, waker.clone(), document, self.module.clone(), &retry) {
            super::ReloadOutcome::Mounted { .. } => {}
            super::ReloadOutcome::Rejected(err) | super::ReloadOutcome::Fallback(err) => {
                let live_still_prev = self.sessions.contains_key(&document);
                let phase = if live_still_prev { "mount" } else { "commit" };
                self.overlay.active_overlay = prev_active;
                self.overlay.safe_base_shell = prev_safe;
                self.overlay.last_failure = prev_failure;
                self.recover_primary_overlay(
                    surface,
                    waker,
                    retry,
                    document,
                    OverlayRecovery { phase, reason: err.to_string(), failed_meta: &failed_meta },
                );
            }
        }
    }

    fn verify_candidate_overlay(
        &self,
        path: &std::path::Path,
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

    pub(super) fn recover_pure_shell_overlay(
        &mut self,
        runtime: &mut zgui::runtime::Runtime,
        cx: &dyn zgui::platform::PlatformCx,
        images: super::ImageChoice,
        phase: &'static str,
        reason: String,
        failed_meta: &FailedOverlayMeta,
    ) -> crate::session::Result<()> {
        let active_is_live = self.pure_session.is_some();
        match self.overlay.plan_failure_recovery(phase, reason, failed_meta, active_is_live) {
            RollbackPlan::KeepActive(info) => {
                self.overlay.last_failure = Some(info.clone());
                if let Some(session) = &self.pure_session {
                    session.record_overlay_failure(info);
                }
                Ok(())
            }
            RollbackPlan::MountLastGood { overlay, failure } => {
                self.overlay.active_overlay = Some(overlay);
                self.overlay.safe_base_shell = false;
                self.overlay.last_failure = Some(failure.clone());
                self.overlay.is_rollback_mount = true;
                let res = self.replace_shell(runtime, cx, images, self.module.clone());
                self.overlay.is_rollback_mount = false;
                if matches!(res, super::ReloadOutcome::Mounted { .. }) {
                    return Ok(());
                }
                self.mount_safe_base_shell_pure(runtime, cx, images, failure)
            }
            RollbackPlan::SafeBaseShell(info) => self.mount_safe_base_shell_pure(runtime, cx, images, info),
        }
    }

    fn mount_safe_base_shell_pure(
        &mut self,
        runtime: &mut zgui::runtime::Runtime,
        cx: &dyn zgui::platform::PlatformCx,
        images: super::ImageChoice,
        info: OverlayFailureInfo,
    ) -> crate::session::Result<()> {
        self.overlay.active_overlay = None;
        self.overlay.observation = None;
        self.overlay.safe_base_shell = true;
        self.overlay.last_failure = Some(info);
        self.overlay.is_rollback_mount = true;
        let outcome = self.replace_shell(runtime, cx, images, self.module.clone());
        self.overlay.is_rollback_mount = false;
        match outcome {
            super::ReloadOutcome::Mounted { .. } => Ok(()),
            super::ReloadOutcome::Rejected(err) | super::ReloadOutcome::Fallback(err) => Err(err),
        }
    }

    pub(super) fn recover_primary_overlay(
        &mut self,
        surface: &mut impl super::SessionSurface,
        waker: std::sync::Arc<dyn zgui::platform::Waker>,
        retry: Rc<dyn Fn(zgui::view::DocumentId, u64)>,
        document: zgui::view::DocumentId,
        recovery: OverlayRecovery<'_>,
    ) {
        let active_is_live = self.sessions.contains_key(&document);
        match self.overlay.plan_failure_recovery(
            recovery.phase,
            recovery.reason,
            recovery.failed_meta,
            active_is_live,
        ) {
            RollbackPlan::KeepActive(info) => {
                self.overlay.last_failure = Some(info.clone());
                if let Some(session) = self.sessions.get(&document) {
                    session.record_overlay_failure(info);
                }
            }
            RollbackPlan::MountLastGood { overlay, failure } => {
                self.overlay.active_overlay = Some(overlay);
                self.overlay.safe_base_shell = false;
                self.overlay.last_failure = Some(failure.clone());
                self.overlay.is_rollback_mount = true;
                let res = self.mount_module(surface, waker.clone(), document, self.module.clone(), &retry);
                self.overlay.is_rollback_mount = false;
                if matches!(res, super::ReloadOutcome::Mounted { .. }) {
                    return;
                }
                self.mount_safe_base_shell_primary(surface, waker, retry, document, failure);
            }
            RollbackPlan::SafeBaseShell(info) => {
                self.mount_safe_base_shell_primary(surface, waker, retry, document, info);
            }
        }
    }

    fn mount_safe_base_shell_primary(
        &mut self,
        surface: &mut impl super::SessionSurface,
        waker: std::sync::Arc<dyn zgui::platform::Waker>,
        retry: Rc<dyn Fn(zgui::view::DocumentId, u64)>,
        document: zgui::view::DocumentId,
        info: OverlayFailureInfo,
    ) {
        self.overlay.active_overlay = None;
        self.overlay.observation = None;
        self.overlay.safe_base_shell = true;
        self.overlay.last_failure = Some(info);
        self.overlay.is_rollback_mount = true;
        let _ = self.mount_module(surface, waker, document, self.module.clone(), &retry);
        self.overlay.is_rollback_mount = false;
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
    pub(crate) fn pending_overlay_path(&self) -> Option<&std::path::Path> {
        self.overlay.pending_overlay.as_deref()
    }
}
