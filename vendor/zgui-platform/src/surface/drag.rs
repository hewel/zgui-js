//! Content being dragged over a surface from outside it.

use std::path::PathBuf;

use zgui_geom::{Css, CssPx, Point};

/// One action negotiated for a desktop drag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DragAction {
    /// No action was accepted or committed.
    #[default]
    None,
    /// Copy data to the destination.
    Copy,
    /// Transfer responsibility for the data to the destination.
    Move,
}

/// The set of supported transfer actions for a drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DragActions {
    /// No transfer action is supported.
    None,
    /// Only Copy is supported.
    Copy,
    /// Only Move is supported.
    Move,
    /// Both Copy and Move are supported.
    CopyMove,
}

impl DragActions {
    /// Whether this set allows the given transfer action.
    #[must_use]
    pub const fn contains(self, action: DragAction) -> bool {
        matches!(
            (self, action),
            (Self::Copy | Self::CopyMove, DragAction::Copy)
                | (Self::Move | Self::CopyMove, DragAction::Move)
        )
    }
}

/// A stage of a drag that started outside the application.
///
/// The whole set of paths arrives at once, with a position, at every stage. That shape is not a
/// convenience: told about one file at a time, a drop target cannot know whether more are coming,
/// so it either acts once per file or waits for an end that never comes. It also lets a target
/// decide whether it *would* accept the drop while the pointer is still moving, which is what
/// makes a highlight appear before the user lets go rather than after.
///
/// ```
/// use std::path::PathBuf;
/// use zgui_geom::{CssPx, Point};
/// use zgui_platform::DragEvent;
///
/// let entered = DragEvent::Entered {
///     paths: vec![PathBuf::from("/tmp/a.png")],
///     position: Point::new(CssPx(4.0), CssPx(4.0)),
/// };
/// assert_eq!(entered.paths().len(), 1);
/// assert!(!entered.is_drop());
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DragEvent {
    /// A drag came over the surface, carrying these paths.
    Entered {
        /// Everything being dragged.
        paths: Vec<PathBuf>,
        /// Where the pointer is, in CSS pixels from the surface's top-left corner.
        position: Point<CssPx, Css>,
    },
    /// The drag moved while over the surface.
    Moved {
        /// Where the pointer is now.
        position: Point<CssPx, Css>,
    },
    /// The drag was released on a backend that does not negotiate actions. Always Copy.
    Dropped {
        /// Everything dropped.
        paths: Vec<PathBuf>,
        /// Where it was dropped.
        position: Point<CssPx, Css>,
    },
    /// The drag was released with an action chosen by the compositor.
    DroppedWithAction {
        /// Everything dropped.
        paths: Vec<PathBuf>,
        /// Where it was dropped.
        position: Point<CssPx, Css>,
        /// The action chosen by the compositor, which the destination must confirm.
        action: DragAction,
    },
    /// The drag left the surface without being dropped.
    Left,
    /// An outbound drag initiated from this surface completed or was cancelled.
    OutboundFinished {
        /// The compositor-selected action committed by the destination, or None on cancellation.
        action: DragAction,
    },
}

/// Owned MIME payload offered during a platform-driven outbound drag.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutboundDragPayload {
    /// Ordered `(mime_type, bytes)` representations advertised to the desktop.
    pub items: Vec<(String, Vec<u8>)>,
    /// Transfer actions authorized by the source.
    pub allowed_actions: DragActions,
}

impl OutboundDragPayload {
    /// Constructs a Copy-only drag payload from `(mime_type, bytes)` pairs.
    #[must_use]
    pub fn new(items: Vec<(String, Vec<u8>)>) -> Self {
        Self { items, allowed_actions: DragActions::Copy }
    }

    /// Sets the supported transfer actions; a drag needs at least one.
    #[must_use]
    pub fn with_allowed_actions(mut self, actions: DragActions) -> Self {
        self.allowed_actions = actions;
        self
    }
}

impl DragEvent {
    /// Everything being dragged, which is nothing on the stages that do not carry it.
    pub fn paths(&self) -> &[PathBuf] {
        match self {
            Self::Entered { paths, .. }
            | Self::Dropped { paths, .. }
            | Self::DroppedWithAction { paths, .. } => paths,
            Self::Moved { .. } | Self::Left | Self::OutboundFinished { .. } => &[],
        }
    }

    /// Where the pointer is, when this stage says.
    pub fn position(&self) -> Option<Point<CssPx, Css>> {
        match self {
            Self::Entered { position, .. }
            | Self::Moved { position }
            | Self::Dropped { position, .. }
            | Self::DroppedWithAction { position, .. } => Some(*position),
            Self::Left | Self::OutboundFinished { .. } => None,
        }
    }

    /// Whether this stage is the release.
    pub const fn is_drop(&self) -> bool {
        self.drop_action().is_some()
    }

    /// Transfer action on a drop, including Copy on backends without action negotiation.
    pub const fn drop_action(&self) -> Option<DragAction> {
        match self {
            Self::Dropped { .. } => Some(DragAction::Copy),
            Self::DroppedWithAction { action, .. } => Some(*action),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DragEvent;
    use std::path::PathBuf;
    use zgui_geom::{Css, CssPx, Point};

    fn at(x: f32) -> Point<CssPx, Css> {
        Point::new(CssPx(x), CssPx(0.0))
    }

    #[test]
    fn only_the_carrying_stages_report_paths() {
        let entered = DragEvent::Entered {
            paths: vec![PathBuf::from("/a")],
            position: at(1.0),
        };
        assert_eq!(entered.paths().len(), 1);
        assert!(DragEvent::Moved { position: at(2.0) }.paths().is_empty());
        assert!(DragEvent::Left.paths().is_empty());
    }

    #[test]
    fn leaving_reports_no_position_and_every_other_stage_does() {
        assert_eq!(DragEvent::Left.position(), None);
        assert_eq!(
            DragEvent::Moved { position: at(3.0) }.position(),
            Some(at(3.0))
        );
    }

    #[test]
    fn exactly_one_stage_is_the_release() {
        let dropped = DragEvent::Dropped {
            paths: Vec::new(),
            position: at(0.0),
        };
        assert!(dropped.is_drop());
        assert_eq!(dropped.drop_action(), Some(super::DragAction::Copy));
        assert_eq!(
            DragEvent::DroppedWithAction {
                paths: Vec::new(),
                position: at(0.0),
                action: super::DragAction::Move,
            }
            .drop_action(),
            Some(super::DragAction::Move),
        );
        assert!(!DragEvent::Left.is_drop());
    }
}
