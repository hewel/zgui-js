//! A surface that is a buffer, with no window behind it.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use accesskit::TreeUpdate;
use zgui_geom::{Css, CssPx, Device, DevicePx, Size};
use zgui_platform::{
    CursorStyle, Decorations, FullscreenMode, InputRegion, LayerPlacement, PlatformError,
    PopupPlacement, Surface, SurfaceAttributes, SurfaceId, SurfaceRole, TextInput, Unsupported,
};

/// How many accessibility nodes the surface keeps across the updates it retains.
///
/// A bound on nodes rather than on updates, because the two kinds of driver differ in exactly
/// that: a test publishes hundreds of small updates and replays them from the first, which a
/// bound on their number would cut short; a benchmark over a long list publishes an update of
/// tens of thousands of nodes per frame, which is what must not be held for thousands of frames.
/// Once the bound is passed the oldest updates go first, and a driver that wants the whole
/// sequence drains it as it goes.
pub const A11Y_LOG_NODES: usize = 1 << 20;

/// The retained accessibility updates and how many nodes they hold between them.
#[derive(Debug, Default)]
struct A11yLog {
    /// The updates, oldest first.
    updates: VecDeque<TreeUpdate>,
    /// How many nodes `updates` carry in total.
    nodes: usize,
}

impl A11yLog {
    /// Keeps `update`, letting the oldest updates go once the bound is passed.
    ///
    /// The one just pushed is never let go: a log holding the latest update is what
    /// [`last_a11y_update`](OffscreenSurface::last_a11y_update) answers from.
    fn push(&mut self, update: TreeUpdate) {
        self.nodes += update.nodes.len();
        self.updates.push_back(update);
        while self.nodes > A11Y_LOG_NODES && self.updates.len() > 1 {
            if let Some(oldest) = self.updates.pop_front() {
                self.nodes -= oldest.nodes.len();
            }
        }
    }
}

/// The currently presented solid-color fallback, in physical pixels.
///
/// Every pixel has the same RGBA value, so this frame need not store an output-sized buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LockFallbackFrame {
    size: Size<DevicePx, Device>,
}

impl LockFallbackFrame {
    /// The full extent covered by this presentation.
    #[must_use]
    pub fn size(self) -> Size<DevicePx, Device> {
        self.size
    }

    /// The presented RGBA8 pixel at `(x, y)`, or `None` outside the frame.
    #[must_use]
    pub fn pixel(self, x: u32, y: u32) -> Option<[u8; 4]> {
        if f64::from(x) < f64::from(self.size.width.0)
            && f64::from(y) < f64::from(self.size.height.0)
        {
            Some([0, 0, 0, 255])
        } else {
            None
        }
    }
}

#[derive(Debug)]
enum Presentation {
    Window { visible: bool },
    UnclaimedLock(LockFallbackFrame),
    ClaimedLock(Option<LockFallbackFrame>),
    RemovedLock,
}

/// A surface that is a buffer, with no window behind it.
///
/// Everything a real surface would do to a window is recorded instead, because a headless backend
/// exists to be asserted against. A test that cannot see whether a frame was asked for cannot
/// check the loop's parking, and the parking is what the whole of this backend is for.
///
/// Application frames come from whoever answers
/// [`SurfaceEvent::RedrawRequested`](zgui_platform::SurfaceEvent::RedrawRequested). A session
/// lock can also present its own opaque fallback before a renderer exists or after it goes away.
#[derive(Debug)]
pub struct OffscreenSurface {
    /// Which surface this is.
    id: SurfaceId,
    /// How large the buffer is, in physical pixels.
    size: Mutex<Size<DevicePx, Device>>,
    /// How many physical pixels there are to a CSS pixel.
    scale_factor: Mutex<f64>,
    /// How fast the output this surface is on refreshes, in thousandths of a hertz.
    ///
    /// Zero means the platform does not say, which is what a backend that knows no rate reports.
    /// It is settable because how often a window answers a configure, and how often it ticks an
    /// animation, are both derived from it — so a suite that cannot move it cannot tell a loop that
    /// reads the output from one that assumes sixty hertz, and those two differ by a factor of four
    /// on the displays where the difference is felt.
    refresh_rate_millihertz: AtomicU32,
    /// How many frames have been asked for and not yet delivered.
    pending: AtomicU64,
    /// How many have been asked for in total.
    requested: AtomicU64,
    /// How many accessibility updates have been published.
    a11y_updates: AtomicU64,
    /// The most recent accessibility updates published, in order, kept so a test can read what a
    /// screen reader would have been told.
    ///
    /// A count alone cannot tell an update that carried the right tree from one that carried an
    /// empty one, and the tree is the whole point of the channel. A sequence is kept and not only
    /// the last, because an update is a *difference*: what it names is resolved against everything
    /// sent before it, so a check on one update alone would reject every correct incremental one.
    ///
    /// The sequence is bounded at [`A11Y_LOG_NODES`] nodes over all retained updates. A headless
    /// surface has a listener by default, so a whole tree is published for every frame that owes
    /// one, and a driver that runs for thousands of frames would otherwise retain every tree it was
    /// ever sent. A driver that wants the whole sequence drains it as it goes, through
    /// [`take_a11y_log`](OffscreenSurface::take_a11y_log).
    a11y_log: Mutex<A11yLog>,
    /// Whether an accessibility consumer listens. Without one, no update is built.
    a11y_listener: AtomicBool,
    /// Everything the surface was told about text input, in order.
    ///
    /// Whether an input method is wanted at all, and where its candidate window goes, is a state
    /// the surface is *told*: nothing about the document says it, so a test that cannot read what
    /// was said cannot tell a window that reports the caret from one that reports nothing and
    /// leaves every Japanese keyboard unable to type into it.
    text_input_log: Mutex<Vec<Option<TextInput>>>,
    /// Every cursor the surface was told to show, in order.
    ///
    /// Which cursor is over a window is a state the surface is *told*, exactly as text input is:
    /// nothing in the document, the fragment tree or the display list records it, so a test that
    /// cannot read what was said cannot tell a window that honours `cursor` from one that ignores
    /// it.
    cursor_log: Mutex<Vec<CursorStyle>>,
    /// What the offscreen compositor is showing on this surface.
    presentation: Mutex<Presentation>,
    /// The last title it was given.
    title: Mutex<String>,
    /// What the surface was most recently asked to be.
    ///
    /// Kept because most of a request is answered by the desktop and never read back: what a test
    /// can otherwise check about an application identifier, a window level or an icon is nothing at
    /// all, and an attribute nothing reads is an attribute that silently stops being sent.
    requested_attributes: Mutex<SurfaceAttributes>,
    popup_dismissed: AtomicBool,
    /// The input region to use when routing scripted pointer input.
    input_region: Mutex<InputRegion>,
}

impl OffscreenSurface {
    /// A surface of `size`, hidden, with nothing recorded yet.
    pub fn new(id: SurfaceId, size: Size<DevicePx, Device>) -> Self {
        Self {
            id,
            size: Mutex::new(size),
            scale_factor: Mutex::new(1.0),
            refresh_rate_millihertz: AtomicU32::new(0),
            pending: AtomicU64::new(0),
            requested: AtomicU64::new(0),
            a11y_updates: AtomicU64::new(0),
            a11y_log: Mutex::new(A11yLog::default()),
            a11y_listener: AtomicBool::new(true),
            text_input_log: Mutex::new(Vec::new()),
            cursor_log: Mutex::new(Vec::new()),
            presentation: Mutex::new(Presentation::Window { visible: false }),
            title: Mutex::new(String::new()),
            requested_attributes: Mutex::new(SurfaceAttributes::default()),
            popup_dismissed: AtomicBool::new(false),
            input_region: Mutex::new(InputRegion::Full),
        }
    }

    pub(crate) fn new_lock(id: SurfaceId, size: Size<DevicePx, Device>) -> Self {
        let mut surface = Self::new(id, size);
        surface.presentation = Mutex::new(Presentation::UnclaimedLock(LockFallbackFrame { size }));
        surface
    }

    pub(crate) fn claim_lock(&self) -> bool {
        let mut presentation = self.presentation.lock().expect("the presentation is not poisoned");
        let Presentation::UnclaimedLock(fallback) = *presentation else {
            return false;
        };
        *presentation = Presentation::ClaimedLock(Some(fallback));
        true
    }

    pub(crate) fn lock_is_claimed(&self) -> bool {
        matches!(
            *self.presentation.lock().expect("the presentation is not poisoned"),
            Presentation::ClaimedLock(_)
        )
    }

    pub(crate) fn release_lock(&self) {
        let mut presentation = self.presentation.lock().expect("the presentation is not poisoned");
        if matches!(*presentation, Presentation::ClaimedLock(_)) {
            *presentation = Presentation::UnclaimedLock(LockFallbackFrame { size: self.size() });
            self.pending.store(0, Ordering::Relaxed);
            *self.input_region.lock().expect("the input region is not poisoned") = InputRegion::Full;
        }
    }

    pub(crate) fn remove_lock(&self) {
        *self.presentation.lock().expect("the presentation is not poisoned") =
            Presentation::RemovedLock;
        self.pending.store(0, Ordering::Relaxed);
    }

    /// The fallback pixels if they are what this output is presenting right now.
    #[must_use]
    pub fn lock_fallback_frame(&self) -> Option<LockFallbackFrame> {
        match *self.presentation.lock().expect("the presentation is not poisoned") {
            Presentation::UnclaimedLock(frame) => Some(frame),
            Presentation::ClaimedLock(frame) => frame,
            Presentation::Window { .. } | Presentation::RemovedLock => None,
        }
    }

    /// Records what the surface was asked to be.
    pub fn set_requested_attributes(&self, attributes: &SurfaceAttributes) {
        *self
            .requested_attributes
            .lock()
            .expect("the attributes are not poisoned") = attributes.clone();
    }

    /// What the surface was most recently asked to be.
    pub fn requested_attributes(&self) -> SurfaceAttributes {
        self.requested_attributes
            .lock()
            .expect("the attributes are not poisoned")
            .clone()
    }

    /// Simulates protocol dismissal before the corresponding close event is delivered.
    pub fn mark_popup_done(&self) {
        if self.popup_parent().is_some() {
            self.popup_dismissed.store(true, Ordering::Release);
        }
    }

    #[must_use]
    pub(crate) fn can_parent_popup(&self) -> bool {
        if self.popup_dismissed() {
            return false;
        }
        let attributes = self.requested_attributes.lock().expect("the attributes are not poisoned");
        matches!(
            attributes.role,
            SurfaceRole::Toplevel | SurfaceRole::Layer(_) | SurfaceRole::Popup(_)
        )
    }

    #[must_use]
    pub(crate) fn popup_parent(&self) -> Option<SurfaceId> {
        let attributes = self.requested_attributes.lock().expect("the attributes are not poisoned");
        match &attributes.role {
            SurfaceRole::Popup(placement) => Some(placement.parent),
            _ => None,
        }
    }

    /// The pointer input region most recently applied to the surface.
    pub fn input_region(&self) -> InputRegion {
        self.input_region.lock().expect("the input region is not poisoned").clone()
    }

    /// How many frames have been asked for since this surface was created.
    pub fn redraws_requested(&self) -> u64 {
        self.requested.load(Ordering::Relaxed)
    }

    /// Whether a frame has been asked for and not yet delivered.
    pub fn has_pending_redraw(&self) -> bool {
        self.pending.load(Ordering::Relaxed) > 0
    }

    /// Takes the pending request, reporting whether there was one.
    ///
    /// Coalescing lives here: a hundred requests between two frames are one pending request and
    /// therefore one frame, which is exactly what the contract promises.
    pub fn take_pending_redraw(&self) -> bool {
        self.pending.swap(0, Ordering::Relaxed) > 0
    }

    /// Sets whether an accessibility consumer listens. The default is yes.
    ///
    /// With no listener the surface builds no update, as a platform surface does when no assistive
    /// technology has connected.
    pub fn set_a11y_listener(&self, listening: bool) {
        self.a11y_listener.store(listening, Ordering::Relaxed);
    }

    /// How many accessibility updates have been published.
    pub fn a11y_updates(&self) -> u64 {
        self.a11y_updates.load(Ordering::Relaxed)
    }

    /// The last accessibility update published, if one has been.
    pub fn last_a11y_update(&self) -> Option<TreeUpdate> {
        self.a11y_log
            .lock()
            .expect("the log is not poisoned")
            .updates
            .back()
            .cloned()
    }

    /// The accessibility updates published, in the order they were published, back to the bound
    /// of [`A11Y_LOG_NODES`].
    ///
    /// This is what a consumer holds: each update is applied over the ones before it, so a
    /// question about what the consumer can resolve is a question about the whole sequence.
    pub fn a11y_log(&self) -> Vec<TreeUpdate> {
        self.a11y_log
            .lock()
            .expect("the log is not poisoned")
            .updates
            .iter()
            .cloned()
            .collect()
    }

    /// Takes every accessibility update published since the last take, in order.
    ///
    /// For a driver that folds the updates as they arrive: the log is emptied, so nothing is
    /// cloned twice and nothing is retained past its reading. The count from
    /// [`a11y_updates`](OffscreenSurface::a11y_updates) is untouched.
    pub fn take_a11y_log(&self) -> Vec<TreeUpdate> {
        let mut log = self.a11y_log.lock().expect("the log is not poisoned");
        log.nodes = 0;
        log.updates.drain(..).collect()
    }

    /// Everything the surface was told about text input, in the order it was told.
    pub fn text_input_log(&self) -> Vec<Option<TextInput>> {
        self.text_input_log
            .lock()
            .expect("the log is not poisoned")
            .clone()
    }

    /// What the surface was last told about text input, if it has been told anything.
    ///
    /// The outer option is whether it was ever told; the inner one is what it was told — `None`
    /// being "no text is being typed here", which is a thing that has to be said.
    pub fn last_text_input(&self) -> Option<Option<TextInput>> {
        self.text_input_log
            .lock()
            .expect("the log is not poisoned")
            .last()
            .cloned()
    }

    /// Every cursor the surface was told to show, in the order it was told.
    pub fn cursor_log(&self) -> Vec<CursorStyle> {
        self.cursor_log
            .lock()
            .expect("the log is not poisoned")
            .clone()
    }

    /// The cursor the surface was last told to show, if it has been told at all.
    pub fn last_cursor(&self) -> Option<CursorStyle> {
        self.cursor_log
            .lock()
            .expect("the log is not poisoned")
            .last()
            .copied()
    }

    /// Whether the surface has been shown.
    pub fn is_visible(&self) -> bool {
        match *self.presentation.lock().expect("the presentation is not poisoned") {
            Presentation::Window { visible } => visible,
            Presentation::UnclaimedLock(_) | Presentation::ClaimedLock(_) => true,
            Presentation::RemovedLock => false,
        }
    }

    /// The title the surface was last given.
    pub fn title(&self) -> String {
        self.title
            .lock()
            .expect("the title is not poisoned")
            .clone()
    }

    /// Resizes the buffer, as a window manager would.
    pub fn resize(&self, size: Size<DevicePx, Device>) {
        let mut presentation = self.presentation.lock().expect("the presentation is not poisoned");
        if let Presentation::UnclaimedLock(frame) | Presentation::ClaimedLock(Some(frame)) =
            &mut *presentation
        {
            frame.size = size;
        }
        *self.size.lock().expect("the size is not poisoned") = size;
    }

    /// Changes the scale the surface is presented at, as moving to another output would.
    pub fn set_scale_factor(&self, scale_factor: f64) {
        *self.scale_factor.lock().expect("the scale is not poisoned") = scale_factor;
    }

    /// Puts the surface on an output that refreshes at `millihertz`, or on one that does not say.
    ///
    /// Dragging a window from a seventy-five hertz panel onto a two-hundred-and-forty hertz one is
    /// this call, and the two are four times apart in everything paced against the output.
    pub fn set_refresh_rate_millihertz(&self, millihertz: Option<u32>) {
        self.refresh_rate_millihertz
            .store(millihertz.unwrap_or(0), Ordering::Relaxed);
    }
}

impl Surface for OffscreenSurface {
    fn id(&self) -> SurfaceId {
        self.id
    }

    fn size(&self) -> Size<DevicePx, Device> {
        *self.size.lock().expect("the size is not poisoned")
    }

    fn scale_factor(&self) -> f64 {
        *self.scale_factor.lock().expect("the scale is not poisoned")
    }

    fn refresh_rate_millihertz(&self) -> Option<u32> {
        match self.refresh_rate_millihertz.load(Ordering::Relaxed) {
            0 => None,
            rate => Some(rate),
        }
    }

    fn request_size(&self, size: Size<CssPx, Css>) -> Option<Size<DevicePx, Device>> {
        if !matches!(
            *self.presentation.lock().expect("the presentation is not poisoned"),
            Presentation::Window { .. }
        ) {
            return None;
        }
        let scale = self.scale_factor();
        let taken = Size::new(
            DevicePx(size.width.0 * scale as f32),
            DevicePx(size.height.0 * scale as f32),
        );
        self.resize(taken);
        Some(taken)
    }

    fn set_min_size(&self, _size: Option<Size<CssPx, Css>>) {}

    fn set_max_size(&self, _size: Option<Size<CssPx, Css>>) {}

    fn request_redraw(&self) {
        self.pending.store(1, Ordering::Relaxed);
        self.requested.fetch_add(1, Ordering::Relaxed);
    }

    fn pre_present_notify(&self) {}

    fn set_title(&self, title: &str) {
        *self.title.lock().expect("the title is not poisoned") = title.to_owned();
    }

    fn set_visible(&self, visible: bool) {
        match &mut *self.presentation.lock().expect("the presentation is not poisoned") {
            Presentation::Window { visible: shown } => *shown = visible,
            Presentation::ClaimedLock(fallback) => {
                *fallback = (!visible).then(|| LockFallbackFrame { size: self.size() });
            }
            Presentation::UnclaimedLock(_) | Presentation::RemovedLock => {}
        }
    }

    fn set_decorations(&self, _decorations: Decorations) {}

    fn set_resizable(&self, _resizable: bool) {}

    fn set_maximized(&self, _maximized: bool) {}

    fn set_minimized(&self, _minimized: bool) {}

    fn set_fullscreen(&self, _mode: Option<FullscreenMode>) {}

    fn set_cursor(&self, cursor: CursorStyle) {
        self.cursor_log
            .lock()
            .expect("the log is not poisoned")
            .push(cursor);
    }

    fn set_pointer_passthrough(&self, passthrough: bool) -> Result<(), Unsupported> {
        *self.input_region.lock().expect("the input region is not poisoned") =
            if passthrough { InputRegion::Empty } else { InputRegion::Full };
        Ok(())
    }

    fn set_input_region(&self, region: &InputRegion) -> Result<(), PlatformError> {
        let mut held = self.input_region.lock().expect("the input region is not poisoned");
        if let (InputRegion::Rects(held), InputRegion::Rects(rects)) = (&mut *held, region) {
            held.clone_from(rects);
        } else {
            *held = region.clone();
        }
        Ok(())
    }

    fn reconfigure_layer(
        &self,
        placement: &LayerPlacement,
        size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        let mut attrs = self.requested_attributes.lock().expect("the attributes are not poisoned");
        let SurfaceRole::Layer(old) = &attrs.role else {
            return Err(Unsupported.into());
        };
        if old.monitor != placement.monitor {
            return Err(PlatformError::Backend(
                "moving a layer surface to another output requires reopening it".to_owned(),
            ));
        }
        // A fixture has no compositor to assign stretched dimensions.
        if [size.width.0, size.height.0].into_iter().any(|axis| !axis.is_finite() || axis <= 0.0) {
            return Err(PlatformError::Backend(
                "offscreen layers require resolved positive dimensions".to_owned(),
            ));
        }
        if old != placement || attrs.size != Some(size) {
            attrs.role = SurfaceRole::Layer(placement.clone());
            attrs.size = Some(size);
            self.request_size(size);
            self.request_redraw();
        }
        Ok(())
    }

    fn reconfigure_popup(
        &self,
        placement: &PopupPlacement,
        size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        if self.popup_dismissed() {
            return Err(Unsupported.into());
        }
        let mut attrs = self.requested_attributes.lock().expect("the attributes are not poisoned");
        let SurfaceRole::Popup(old) = &attrs.role else {
            return Err(Unsupported.into());
        };
        if old.parent != placement.parent || old.grab != placement.grab {
            return Err(PlatformError::Backend(
                "changing a pop-up's parent or grab requires reopening it".to_owned(),
            ));
        }
        if [size.width.0, size.height.0].into_iter().any(|axis| !axis.is_finite() || axis <= 0.0) {
            return Err(PlatformError::Backend(
                "offscreen pop-ups require resolved positive dimensions".to_owned(),
            ));
        }
        if old != placement || attrs.size != Some(size) {
            attrs.role = SurfaceRole::Popup(placement.clone());
            attrs.size = Some(size);
            self.request_size(size);
            self.request_redraw();
        }
        Ok(())
    }

    fn popup_dismissed(&self) -> bool {
        self.popup_dismissed.load(Ordering::Acquire)
    }

    fn supports_popup_reposition(&self) -> bool {
        matches!(
            self.requested_attributes
                .lock()
                .expect("the attributes are not poisoned")
                .role,
            SurfaceRole::Popup(_)
        )
    }

    fn set_text_input(&self, state: Option<TextInput>) {
        self.text_input_log
            .lock()
            .expect("the log is not poisoned")
            .push(state);
    }

    fn push_a11y_update(&self, build: &mut dyn FnMut() -> TreeUpdate) {
        if !self.a11y_listener.load(Ordering::Relaxed) {
            return;
        }
        // By default, whatever drives a headless surface is the listener. What it was told is kept
        // rather than discarded: an assertion about the tree is the only one that can tell a
        // channel that carries the document from one that carries nothing.
        let update = build();
        let mut log = self.a11y_log.lock().expect("the log is not poisoned");
        log.push(update);
        drop(log);
        self.a11y_updates.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::OffscreenSurface;
    use zgui_geom::{DevicePx, Size};
    use zgui_platform::{Surface, SurfaceId};

    /// A surface four hundred pixels square.
    fn surface() -> OffscreenSurface {
        OffscreenSurface::new(
            SurfaceId::new(1),
            Size::new(DevicePx(400.0), DevicePx(400.0)),
        )
    }

    /// An update carrying `nodes` nodes, which is what a frame that moved that many publishes.
    fn update_of(nodes: usize) -> accesskit::TreeUpdate {
        accesskit::TreeUpdate {
            nodes: (0..nodes)
                .map(|n| {
                    (
                        accesskit::NodeId(n as u64),
                        accesskit::Node::new(accesskit::Role::Unknown),
                    )
                })
                .collect(),
            tree: None,
            tree_id: accesskit::TreeId::ROOT,
            focus: accesskit::NodeId(0),
        }
    }

    #[test]
    fn the_accessibility_log_keeps_every_small_update_and_counts_them_all() {
        let surface = surface();
        for _ in 0..1_000 {
            surface.push_a11y_update(&mut || update_of(3));
        }
        assert_eq!(surface.a11y_updates(), 1_000);
        assert_eq!(
            surface.a11y_log().len(),
            1_000,
            "a test replays the sequence from its first update, so small updates are all kept"
        );
        assert_eq!(surface.take_a11y_log().len(), 1_000);
        assert!(surface.a11y_log().is_empty(), "taking the log empties it");
        assert_eq!(
            surface.a11y_updates(),
            1_000,
            "the count is what was published, whatever was taken"
        );
    }

    #[test]
    fn no_listener_builds_no_accessibility_update() {
        let surface = surface();
        surface.set_a11y_listener(false);
        let mut built = 0;
        surface.push_a11y_update(&mut || {
            built += 1;
            update_of(3)
        });
        assert_eq!(built, 0);
        assert_eq!(surface.a11y_updates(), 0);

        surface.set_a11y_listener(true);
        surface.push_a11y_update(&mut || {
            built += 1;
            update_of(3)
        });
        assert_eq!(built, 1, "a listener that arrives gets the next update");
    }

    #[test]
    fn the_accessibility_log_lets_the_oldest_large_updates_go() {
        let surface = surface();
        let each = super::A11Y_LOG_NODES / 4;
        for _ in 0..40 {
            surface.push_a11y_update(&mut || update_of(each));
        }
        let kept = surface.a11y_log();
        assert_eq!(
            kept.len(),
            4,
            "a driver that runs for thousands of frames must not hold a tree per frame"
        );
        assert_eq!(surface.a11y_updates(), 40);
        assert_eq!(
            surface.last_a11y_update().map(|update| update.nodes.len()),
            Some(each)
        );
    }

    #[test]
    fn a_hundred_requests_between_two_frames_are_one_frame() {
        let surface = surface();
        for _ in 0..100 {
            surface.request_redraw();
        }
        assert!(surface.take_pending_redraw());
        assert!(!surface.take_pending_redraw());
        assert_eq!(
            surface.redraws_requested(),
            100,
            "the requests are still counted; it is the frames that coalesce"
        );
    }

    #[test]
    fn an_output_that_says_nothing_about_its_rate_is_told_apart_from_one_that_does() {
        let surface = surface();
        assert_eq!(
            surface.refresh_rate_millihertz(),
            None,
            "a backend that knows no rate must not invent one"
        );
        surface.set_refresh_rate_millihertz(Some(240_000));
        assert_eq!(surface.refresh_rate_millihertz(), Some(240_000));
        surface.set_refresh_rate_millihertz(None);
        assert_eq!(surface.refresh_rate_millihertz(), None);
    }

    #[test]
    fn a_surface_starts_hidden_and_is_shown_only_when_asked() {
        let surface = surface();
        assert!(!surface.is_visible());
        surface.set_visible(true);
        assert!(surface.is_visible());
    }
}
