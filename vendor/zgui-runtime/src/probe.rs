//! The seam a diagnostic tool attaches to.
//!
//! A window's interesting state — what it computed, what it damaged, what it emitted, what it holds
//! on the device — exists only for the instant between the frame that produced it and the next
//! frame that overwrites it. A tool that wants to show that state cannot go and ask for it later,
//! and it cannot ask for it from inside the view either, because a view sees the document and not
//! the frame that painted it.
//!
//! So a probe is *called*, once, at the end of every frame, and handed the window as it stands. It
//! is deliberately the narrowest possible seam: one method, no return value, nothing it can change.
//! A probe that mutated the window would be a probe that changed the thing it was measuring, and a
//! probe that could refuse a frame would be a second frame loop.
//!
//! Nothing in this framework implements it. It exists so that an inspector — or a recorder, or a
//! test that wants to assert about the frames a script produced — has one object to install rather
//! than a fork of the loop.
//!
//! ```
//! use std::cell::Cell;
//! use std::rc::Rc;
//! use zgui_runtime::{FrameProbe, Window};
//!
//! /// A probe that counts frames and nothing else.
//! #[derive(Default)]
//! struct Counting {
//!     frames: Cell<u64>,
//! }
//!
//! impl FrameProbe for Counting {
//!     fn frame_ended(&self, _window: &Window) {
//!         self.frames.set(self.frames.get() + 1);
//!     }
//! }
//!
//! let probe: Rc<dyn FrameProbe> = Rc::new(Counting::default());
//! // `App::with_probe(probe)` installs it into every window the application opens.
//! # let _ = probe;
//! ```

use crate::window::Window;

/// Read-only evidence about one completed native frame.
///
/// Available through [`Window::completed_frame`] before [`FrameProbe::frame_ended`] is called,
/// including frames the renderer held, recovered, or skipped. This does not imply presentation,
/// resource readiness, or that another frame is not owed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompletedFrame {
    /// One-based ordinal of completed frames on this probed window.
    pub ordinal: u64,
    /// The native frame moment, shared with this frame's timers and animations.
    pub timestamp: zgui_vocab::Timestamp,
    /// The actual result of this frame's paint/draw stage, not a cached renderer report.
    pub outcome: zgui_render::FrameOutcome,
    /// The document's view-mutation revision after all frame cleanup.
    pub document_revision: u64,
    /// Stylesheet revision consumed by this frame's last restyle.
    ///
    /// May trail [`Window::style_revision`] when a late callback applies a stylesheet after
    /// restyling. This is consumed style evidence, not a claim about presentation.
    pub style_revision: u64,
    /// The view-mutation revision entering the last completed layout/fragment pass.
    ///
    /// `None` when this frame completed no such pass. Sampled at layout, including relayouts
    /// caused by scroll or geometry delivery, never inferred from the post-frame document.
    /// A pass whose measurement gate reuses standing boxes still completes fragment composition
    /// and records this frame's input revision, including semantic-only and undamaged frames.
    pub layout_revision: Option<u64>,
    /// Whether layout and the completed document can conservatively be associated.
    ///
    /// True only when a layout/fragment pass completed, no view mutation occurred since that
    /// pass began, and no style, box, text, layout, or imperative-scroll obligation remains.
    /// Late focus, observation, re-hit, or script callbacks therefore cannot certify a newer
    /// document against older geometry. False may conservatively reject a harmless mutation.
    ///
    /// This describes frame completion, not a live guarantee: before inspecting later, also
    /// compare [`Self::document_revision`] with `window.dom().revision()`. It says nothing about
    /// whether this frame's pixels were presented; consult [`Self::outcome`] separately.
    pub layout_document_coherent: bool,
}

/// Something told about each frame after it has been produced.
///
/// Called with the window in the state the frame left it. [`Window::completed_frame`] reports the
/// actual outcome even when no presentation occurred, and whether late document mutations left
/// its geometry coherent. The scene, layout and renderer report are still available for inspection.
///
/// The method takes `&self` rather than `&mut self` because the window is borrowed for the call, so
/// a probe that wanted to keep something writes it through a cell or a signal. That is not a
/// restriction in practice — everything a probe does with what it is handed is either recording it
/// or publishing it — and it is what lets the same probe be installed into several windows.
pub trait FrameProbe {
    /// A frame has finished on `window`.
    fn frame_ended(&self, window: &Window);

    /// What to call this probe in a diagnostic rendering of the window's options.
    ///
    /// Defaulted, because a probe is identified by what it does rather than by what it is called,
    /// and requiring a name of every implementation would be requiring a decision nobody has an
    /// opinion about.
    fn describe(&self) -> &str {
        "a frame probe"
    }
}

impl core::fmt::Debug for dyn FrameProbe {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.describe())
    }
}
