//! Pure value-oriented frame coherence ledger and step frame budget tracker.

use zgui::render::{FrameOutcome, SkipReason};

use crate::preview::evidence::{FrameStamp, Presentation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameStatus {
    Presented,
    Undamaged,
    Other,
}

impl From<FrameOutcome> for FrameStatus {
    fn from(outcome: FrameOutcome) -> Self {
        match outcome {
            FrameOutcome::Presented(_) => Self::Presented,
            FrameOutcome::Skipped(SkipReason::Undamaged) => Self::Undamaged,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FrameInput {
    pub ordinal: u64,
    pub time_us: u64,
    pub layout_document_coherent: bool,
    pub status: FrameStatus,
    pub outcome_debug: String,
    pub draw_id: u64,
    pub presentation_id: u64,
    pub configuration_id: u64,
    pub presented_draw: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SettledQuery {
    pub generation: u64,
    pub document_revision: u64,
    pub metadata_revision: u64,
    pub resource_revision: u64,
    pub configuration_id: u64,
    pub presentation_id: u64,
    pub presented_draw: Option<u64>,
    pub completed_coherent: bool,
    pub completed_ordinal: Option<u64>,
    pub completed_status: Option<FrameStatus>,
}

#[derive(Clone, Debug)]
pub(crate) struct CommittedMeta {
    pub generation: u64,
    pub document_revision: u64,
    pub metadata_revision: u64,
    pub resource_revision: u64,
    pub surface_configuration: u64,
    pub configuration_id: u64,
    pub presentation_id: u64,
    pub ordinal: u64,
    pub time_us: u64,
}

#[derive(Default)]
pub(crate) struct FrameLedger {
    frames: u64,
    action: u64,
    surface_configuration: u64,
    presentation: Option<Presentation>,
    committed: Option<CommittedMeta>,
}

impl FrameLedger {
    /// Records one completed native frame outcome and returns `(stamp, presentation)`
    /// only when the frame is layout-coherent, presented or undamaged-skipped, and
    /// matches the renderer's active configuration.
    pub fn record_outcome(&mut self, input: &FrameInput) -> Option<(FrameStamp, Presentation)> {
        self.frames += 1;
        let stamp = FrameStamp {
            ordinal: input.ordinal,
            time_us: input.time_us,
            draw_id: input.draw_id,
            outcome: input.outcome_debug.clone(),
        };
        if input.status == FrameStatus::Presented {
            self.presentation = Some(Presentation {
                frame: stamp.clone(),
                sequence: input.presentation_id,
                configuration_id: input.configuration_id,
                draw_id: input.presented_draw,
            });
        }
        if !input.layout_document_coherent
            || !matches!(input.status, FrameStatus::Presented | FrameStatus::Undamaged)
        {
            return None;
        }
        let presentation = self.presentation.clone()?;
        if presentation.configuration_id != input.configuration_id {
            return None;
        }
        Some((stamp, presentation))
    }

    /// Commits a checkpointed frame's identity only after sink persistence succeeds.
    pub fn commit(&mut self, meta: CommittedMeta) {
        self.committed = Some(meta);
    }

    /// Enforces all 9 coherence criteria tying committed frame identity to the live surface.
    pub fn is_settled(&self, query: &SettledQuery) -> bool {
        let Some(last) = &self.committed else {
            return false;
        };
        last.document_revision == query.document_revision
            && last.configuration_id == query.configuration_id
            && last.presentation_id == query.presentation_id
            && query.presented_draw.is_some()
            && last.generation == query.generation
            && last.metadata_revision == query.metadata_revision
            && last.resource_revision == query.resource_revision
            && last.surface_configuration == self.surface_configuration
            && query.completed_coherent
            && query.completed_ordinal == Some(last.ordinal)
            && matches!(query.completed_status, Some(FrameStatus::Presented | FrameStatus::Undamaged))
    }

    pub fn exceeded_step_budget(&self, step_start_frame: u64, max_frames_per_step: u32) -> bool {
        self.frames.saturating_sub(step_start_frame) > u64::from(max_frames_per_step)
    }

    pub fn begin_step(&mut self, action: u64) -> u64 {
        self.action = action;
        self.frames
    }

    pub fn bump_surface_configuration(&mut self) {
        self.surface_configuration += 1;
    }

    /// Counts a completed native frame whose observation was skipped early (e.g.
    /// prior fault or inactive surface) without mutating presentation state.
    pub fn note_ignored_frame(&mut self) {
        self.frames += 1;
    }

    pub fn reset_surface(&mut self) {
        self.presentation = None;
        self.committed = None;
    }

    pub fn action(&self) -> u64 {
        self.action
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn surface_configuration(&self) -> u64 {
        self.surface_configuration
    }

    pub fn committed_time_us(&self) -> Option<u64> {
        self.committed.as_ref().map(|meta| meta.time_us)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_ledger_tracks_settled_skips_revisions_resizes_and_budgets() {
        let mut ledger = FrameLedger::default();
        let step_start = ledger.begin_step(1);

        let initial = FrameInput {
            ordinal: 1,
            time_us: 1000,
            layout_document_coherent: true,
            status: FrameStatus::Presented,
            outcome_debug: "Presented".into(),
            draw_id: 1,
            presentation_id: 1,
            configuration_id: 1,
            presented_draw: Some(1),
        };
        assert!(ledger.record_outcome(&initial).is_some());
        ledger.commit(CommittedMeta {
            generation: 1,
            document_revision: 5,
            metadata_revision: 2,
            resource_revision: 0,
            surface_configuration: ledger.surface_configuration(),
            configuration_id: 1,
            presentation_id: 1,
            ordinal: 1,
            time_us: 1000,
        });

        let mut query = SettledQuery {
            generation: 1,
            document_revision: 5,
            metadata_revision: 2,
            resource_revision: 0,
            configuration_id: 1,
            presentation_id: 1,
            presented_draw: Some(1),
            completed_coherent: true,
            completed_ordinal: Some(1),
            completed_status: Some(FrameStatus::Presented),
        };
        assert!(ledger.is_settled(&query));

        let skipped = FrameInput {
            ordinal: 2,
            time_us: 2000,
            layout_document_coherent: true,
            status: FrameStatus::Undamaged,
            outcome_debug: "Skipped(Undamaged)".into(),
            draw_id: 2,
            presentation_id: 1,
            configuration_id: 1,
            presented_draw: Some(1),
        };
        assert!(ledger.record_outcome(&skipped).is_some());
        ledger.commit(CommittedMeta {
            generation: 1,
            document_revision: 5,
            metadata_revision: 2,
            resource_revision: 0,
            surface_configuration: ledger.surface_configuration(),
            configuration_id: 1,
            presentation_id: 1,
            ordinal: 2,
            time_us: 2000,
        });
        query.completed_ordinal = Some(2);
        query.completed_status = Some(FrameStatus::Undamaged);
        assert!(ledger.is_settled(&query));

        assert!(!ledger.is_settled(&SettledQuery { document_revision: 6, ..query }));

        let failed_next = FrameInput {
            ordinal: 3,
            time_us: 3000,
            layout_document_coherent: true,
            status: FrameStatus::Presented,
            outcome_debug: "Presented".into(),
            draw_id: 3,
            presentation_id: 2,
            configuration_id: 1,
            presented_draw: Some(3),
        };
        assert!(ledger.record_outcome(&failed_next).is_some());
        assert_eq!(ledger.committed_time_us(), Some(2000));

        ledger.bump_surface_configuration();
        assert!(!ledger.is_settled(&query));
        let lag = FrameInput {
            ordinal: 4,
            time_us: 4000,
            layout_document_coherent: true,
            status: FrameStatus::Undamaged,
            outcome_debug: "Skipped(Undamaged)".into(),
            draw_id: 4,
            presentation_id: 2,
            configuration_id: 2,
            presented_draw: Some(3),
        };
        assert!(ledger.record_outcome(&lag).is_none());
        assert!(ledger.exceeded_step_budget(step_start, 3));
        assert!(!ledger.exceeded_step_budget(step_start, 4));
    }
}
