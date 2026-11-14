//! Asking a settled window what is under a point, without dispatching anything.
//!
//! Routing an event answers the same question and then acts on the answer: the pointer enters and
//! leaves elements, hover states are written, and a press starts a gesture. A caller that only
//! wants to know what is there — a fixture aiming at a control, a check that what is drawn and what
//! is hit are the same thing — must be able to ask without any of that happening, or the asking
//! changes the answer.

use zgui_dom::NodeKey;
use zgui_geom::{Device, DevicePx, Point};
use zgui_vocab::PointerId;

use crate::window::Window;

impl Window {
    /// The most recently completed native frame, when this window has a frame probe.
    ///
    /// `None` before the first frame or when no probe was configured. Updated immediately before
    /// [`FrameProbe::frame_ended`](crate::FrameProbe::frame_ended), after cleanup, even when the
    /// renderer skipped drawing. Reading it never requests work or changes the window.
    ///
    /// The record is historical: later writes can advance `self.dom().revision()` without a new
    /// frame. Compare that revision with the record before associating live tree data with it.
    #[must_use]
    pub fn completed_frame(&self) -> Option<crate::probe::CompletedFrame> {
        self.completed_frame
    }

    /// Throws away what this window holds about where things are, and builds it again.
    ///
    /// The index that answers what is under a point is kept up to date one entry at a time, by the
    /// same pass that writes the fragments: an entry is moved when its fragment moves, and nothing
    /// rebuilds the whole of it unless the painting order itself changed. That is what makes it
    /// cheap, and it is also what makes "the answers are right" a claim about every frame since the
    /// window opened rather than about this one. This is the other way of arriving at it — one pass
    /// over the fragments as they are now, holding nothing from before — so the two can be held
    /// against each other.
    pub fn forget_hit_index(&mut self) {
        let mut layout = self.layout.borrow_mut();
        self.hit.rebuild(&mut layout, self.scale);
    }

    /// The elements under a point on the surface, the document's root first.
    ///
    /// Empty when the point is over nothing at all, which for a window is the backdrop outside
    /// everything the document drew.
    ///
    /// The path rather than only the element: an event travels every step of it, so a difference
    /// three levels above the target is a difference in which handlers a press reaches even when
    /// the element under the pointer is the same one.
    ///
    /// Absolute device pixels, and the answer is against the frame that was last drawn — the same
    /// coordinate systems, resolved the same way, as the frame in front of the person pointing.
    pub fn chain_at(&self, point: Point<DevicePx, Device>) -> Vec<NodeKey> {
        let document = self.document.borrow();
        let layout = self.layout.borrow();
        let filter = self.engine.filter();
        let world = zgui_input::World {
            document: &document,
            layout: &layout,
            hit: &self.hit,
            clips: &self.scene.clips,
            spatial: &self.scene.spatial,
            scale: zgui_geom::Scale::new(self.scale),
            filter: &filter,
        };
        world.chain_at(point).path().to_vec()
    }

    /// Whether a live node or its descendant has a hit-eligible fragment at this device point.
    ///
    /// Unlike [`Window::chain_at`], this also sees fragments behind an overlapping element.
    /// It uses the native clip, transform, rounded-corner and pointer-events tests, so callers can
    /// distinguish an obscured point from one the target does not admit. It dispatches no input.
    pub fn hit_contains(&self, node: zgui_view::NodeId, point: Point<DevicePx, Device>) -> bool {
        let document = self.document.borrow();
        let Some(target) = zgui_view_dom::id::to_document(node) else { return false; };
        if document.store().index_of(target).is_none() { return false; }
        let layout = self.layout.borrow();
        for fragment in self.hit.hit(point, &self.scene.clips, &self.scene.spatial) {
            let Some(key) = zgui_input::hit::element_of(&layout, fragment) else { continue; };
            let mut index = document.store().index_of(key);
            while let Some(at) = index {
                if document.store().key_of(at) == target { return true; }
                index = document.store().core(at).parent();
            }
        }
        false
    }

    /// Returns the element that holds `pointer`'s capture, if one does.
    ///
    /// A handler captures a pointer with
    /// [`EventCx::capture_pointer`](zgui_view::EventCx::capture_pointer), and every later event of
    /// that pointer goes to that element until the button is released. A host that shares the
    /// pointer with other content, for example a game view under the interface, reads this to
    /// keep a drag that started on the interface with the interface.
    ///
    /// The answer is the router's state after the last dispatch: a press or release still in the
    /// queue has not changed it yet.
    pub fn pointer_capture(&self, pointer: PointerId) -> Option<NodeKey> {
        self.router.capture().of(pointer)
    }

    /// Returns `true` where an element holds the capture of any pointer.
    ///
    /// See [`Window::pointer_capture`].
    pub fn has_pointer_capture(&self) -> bool {
        !self.router.capture().is_empty()
    }
}
