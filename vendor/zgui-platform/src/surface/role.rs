//! What a surface is to the desktop: an ordinary window, a pop-up, or a shell layer.

use zgui_geom::{Css, CssPx, Edges, Point, Rect, Size};

use crate::error::PlatformError;
use crate::monitor::MonitorTarget;
use crate::surface::id::SurfaceId;
use crate::surface::serial::InputSerial;

/// What a surface is to the desktop.
///
/// A backend that cannot make a role refuses the surface with
/// [`PlatformError::Unsupported`](crate::PlatformError::Unsupported) rather than making an
/// ordinary window instead, because a menu silently opening as a second application window is
/// worse than a menu that does not open. Whether a role is available at all is
/// [`PlatformCapabilities`](crate::PlatformCapabilities)' to answer, and a component asks that
/// before it asks for the surface.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum SurfaceRole {
    /// An ordinary window.
    #[default]
    Toplevel,
    /// A surface placed against a rectangle of another surface, and dismissed with it.
    Popup(PopupPlacement),
    /// A part of the desktop shell: a wallpaper, a dock, a notification.
    Layer(LayerPlacement),
    /// The lock screen on one output, while the application holds the session lock.
    ///
    /// The desktop sizes it to cover the whole output, and it takes the keyboard. Only an
    /// application that asked for the lock with
    /// [`PlatformCx::lock_session`](crate::PlatformCx::lock_session) can make one, and it makes
    /// one for each output. The surface cannot be hidden. It closes with
    /// [`CloseReason::OutputGone`](crate::CloseReason::OutputGone) when its output goes away.
    Lock(MonitorTarget),
}

/// Where a pop-up sits relative to the surface that owns it.
///
/// The rectangle and both corners are the same model X11, Wayland and every menu library agree on:
/// a rectangle on the parent, a corner of that rectangle the pop-up is anchored to, and the corner
/// of the pop-up that meets it. What happens when the result would leave the screen is
/// [`Constrain`]'s.
///
/// The parent is an ordinary window, another pop-up or a shell layer. A pop-up on a shell layer is
/// how a bar opens a menu that extends past the bar's own edges.
#[derive(Clone, Debug, PartialEq)]
pub struct PopupPlacement {
    /// The surface the pop-up belongs to and is measured against.
    pub parent: SurfaceId,
    /// The rectangle on the parent to anchor against, in the parent's own coordinates.
    pub anchor_rect: Rect<CssPx, Css>,
    /// Which corner of the anchor rectangle the pop-up hangs from.
    pub anchor: Anchor,
    /// Which corner of the pop-up meets it.
    pub gravity: Anchor,
    /// What to do when the pop-up would not fit.
    pub constrain: Constrain,
    /// Translation from the chosen anchor and gravity, in the parent's CSS pixels.
    pub offset: Point<CssPx, Css>,
    /// Whether the compositor should adjust the placement when the parent changes.
    pub reactive: bool,
    /// Outer drawable bleed around the visual popup geometry, in nonnegative whole CSS pixels.
    ///
    /// A popup requested at visual size `G` with insets `(L, T, R, B)` allocates a drawable
    /// logical surface of `S = (G.w + L + R, G.h + T + B)` while declaring `xdg_surface` window
    /// geometry `(L, T, G.w, G.h)`.
    pub geometry_insets: Edges<CssPx>,
    /// The press that lets the pop-up take the pointer and the keyboard until it is dismissed.
    ///
    /// A menu takes them; a tooltip does not, and has `None`. A desktop grants the grab only
    /// against a press the user made, so the serial comes from the handler of the press that opens
    /// the pop-up. A grab that quotes anything else makes the desktop dismiss the pop-up at once.
    /// A grabbing pop-up closes with [`CloseReason::Dismissed`](crate::CloseReason::Dismissed)
    /// when the user presses outside it.
    pub grab: Option<InputSerial>,
}

impl PopupPlacement {
    /// A pop-up under `anchor_rect`, aligned to its left, sliding to stay on screen.
    ///
    /// It takes no grab. Add one with [`PopupPlacement::with_grab`].
    pub fn below(parent: SurfaceId, anchor_rect: Rect<CssPx, Css>) -> Self {
        Self {
            parent,
            anchor_rect,
            anchor: Anchor::BottomLeft,
            gravity: Anchor::BottomRight,
            constrain: Constrain::SLIDE,
            offset: Point::new(CssPx(0.0), CssPx(0.0)),
            reactive: false,
            geometry_insets: Edges::new(CssPx(0.0), CssPx(0.0), CssPx(0.0), CssPx(0.0)),
            grab: None,
        }
    }

    /// The same placement, translated by `offset` in the parent's CSS pixels.
    pub const fn with_offset(mut self, offset: Point<CssPx, Css>) -> Self {
        self.offset = offset;
        self
    }

    /// The same placement, opting into reactive compositor repositioning when `reactive` is true.
    pub const fn with_reactive(mut self, reactive: bool) -> Self {
        self.reactive = reactive;
        self
    }

    /// The same placement, reserving outer drawable bleed around the visual popup geometry.
    pub const fn with_geometry_insets(mut self, insets: Edges<CssPx>) -> Self {
        self.geometry_insets = insets;
        self
    }

    /// The same placement, taking the pointer and the keyboard against the press `serial`.
    pub fn with_grab(mut self, serial: InputSerial) -> Self {
        self.grab = Some(serial);
        self
    }

    /// Validates `geometry_insets` as nonnegative finite whole CSS pixels whose opposite-axis sums
    /// fit in `i32`.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::SurfaceCreation`] if any edge is nonfinite, negative, fractional,
    /// or exceeds the signed 32-bit range (including opposite-side sums).
    pub fn validated_geometry_insets(&self) -> Result<Edges<i32>, PlatformError> {
        fn edge(value: CssPx) -> Option<i32> {
            let v = f64::from(value.0);
            if v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= f64::from(i32::MAX) {
                Some(v as i32)
            } else {
                None
            }
        }
        let invalid = || {
            PlatformError::SurfaceCreation(
                "pop-up geometry insets must be finite nonnegative whole CSS pixels".to_owned(),
            )
        };
        let top = edge(self.geometry_insets.top).ok_or_else(invalid)?;
        let right = edge(self.geometry_insets.right).ok_or_else(invalid)?;
        let bottom = edge(self.geometry_insets.bottom).ok_or_else(invalid)?;
        let left = edge(self.geometry_insets.left).ok_or_else(invalid)?;
        if left.checked_add(right).is_none() || top.checked_add(bottom).is_none() {
            return Err(invalid());
        }
        Ok(Edges::new(top, right, bottom, left))
    }

    /// Validates a visual extent `visual` and `geometry_insets`, returning the quantized visual
    /// size, validated integer insets, and checked expanded drawable logical size.
    ///
    /// # Errors
    ///
    /// Returns [`PlatformError::SurfaceCreation`] if `visual` is invalid, any inset is invalid, or
    /// the expanded drawable extent overflows `i32`.
    pub fn expand_visual_size(
        &self,
        visual: Size<CssPx, Css>,
    ) -> Result<(Size<i32, Css>, Edges<i32>, Size<i32, Css>), PlatformError> {
        fn axis(value: CssPx) -> Option<i32> {
            let v = f64::from(value.0);
            if v.is_finite() && v > 0.0 && v <= f64::from(i32::MAX) {
                Some((v.round() as i32).max(1))
            } else {
                None
            }
        }
        let vw = axis(visual.width)
            .ok_or_else(|| PlatformError::SurfaceCreation("invalid pop-up extent".to_owned()))?;
        let vh = axis(visual.height)
            .ok_or_else(|| PlatformError::SurfaceCreation("invalid pop-up extent".to_owned()))?;
        let insets = self.validated_geometry_insets()?;
        let dw = vw
            .checked_add(insets.left)
            .and_then(|v| v.checked_add(insets.right))
            .ok_or_else(|| {
                PlatformError::SurfaceCreation("pop-up drawable extent overflows i32".to_owned())
            })?;
        let dh = vh
            .checked_add(insets.top)
            .and_then(|v| v.checked_add(insets.bottom))
            .ok_or_else(|| {
                PlatformError::SurfaceCreation("pop-up drawable extent overflows i32".to_owned())
            })?;
        Ok((Size::new(vw, vh), insets, Size::new(dw, dh)))
    }
}

/// A corner, an edge or the middle of a rectangle.
///
/// As an anchor it names the point on the parent's rectangle a pop-up hangs from. As a gravity it
/// names the direction the pop-up extends in: [`Anchor::BottomRight`] puts the pop-up below and to
/// the right of the anchor point, which is where a left-aligned drop-down goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Anchor {
    /// The middle.
    #[default]
    Center,
    /// The middle of the top edge.
    Top,
    /// The middle of the bottom edge.
    Bottom,
    /// The middle of the left edge.
    Left,
    /// The middle of the right edge.
    Right,
    /// The top-left corner.
    TopLeft,
    /// The top-right corner.
    TopRight,
    /// The bottom-left corner.
    BottomLeft,
    /// The bottom-right corner.
    BottomRight,
}

/// What a desktop may do to a pop-up that would not fit on the screen.
///
/// Each is allowed rather than requested: the desktop applies the ones it is given, in whatever
/// order it prefers, and only as far as it has to. Granting none of them leaves a menu hanging off
/// the edge of the screen with its items unreachable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Constrain {
    /// Move the pop-up along the axis until it fits.
    pub slide: bool,
    /// Reflect it to the other side of the anchor.
    pub flip: bool,
    /// Shrink it.
    pub resize: bool,
}

impl Constrain {
    /// Nothing: the pop-up appears exactly where it was asked for, on screen or not.
    pub const NONE: Self = Self {
        slide: false,
        flip: false,
        resize: false,
    };
    /// Sliding only, which is what a drop-down under a field wants.
    pub const SLIDE: Self = Self {
        slide: true,
        flip: false,
        resize: false,
    };
    /// Everything, which is what a nested menu wants.
    pub const ANY: Self = Self {
        slide: true,
        flip: true,
        resize: true,
    };
}

/// Where a shell layer sits and how much of the screen it claims.
///
/// ```
/// use zgui_geom::{CssPx, Edges};
/// use zgui_platform::{ExclusiveZone, Layer, LayerPlacement};
///
/// // A bar across the top of the screen that keeps windows out of its 34 pixels.
/// let bar = LayerPlacement::new(Layer::Top)
///     .with_anchors(Edges::new(true, true, false, true))
///     .with_exclusive_zone(ExclusiveZone::Reserve(CssPx(34.0)))
///     .with_namespace("bar");
/// assert_eq!(bar.namespace.as_deref(), Some("bar"));
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LayerPlacement {
    /// Which layer of the desktop it belongs to.
    pub layer: Layer,
    /// Which screen edges it is fastened to.
    ///
    /// Two opposite edges stretch the surface between them and its size on that axis is ignored.
    /// No edges at all centres it.
    pub anchors: Edges<bool>,
    /// How far in from each anchored edge it sits.
    pub margin: Edges<CssPx>,
    /// How the surface and the room other shell surfaces reserve affect each other.
    pub exclusive_zone: ExclusiveZone,
    /// Whether the surface takes the keyboard.
    pub keyboard: KeyboardInteractivity,
    /// The output to place it on. Absent lets the desktop choose.
    ///
    /// A backend refuses the surface when no connected output matches, because a bar that opens
    /// on another screen is a second bar on that screen.
    pub monitor: Option<MonitorTarget>,
    /// What the surface is for, in a word the desktop's rules can match, such as `bar` or
    /// `notifications`.
    ///
    /// Absent uses the application identifier. The desktop reads it once, when the surface is
    /// made.
    pub namespace: Option<String>,
}

impl LayerPlacement {
    /// A surface on `layer`, anchored to no edge, that avoids the room other surfaces reserve.
    pub fn new(layer: Layer) -> Self {
        Self {
            layer,
            ..Self::default()
        }
    }

    /// The same placement fastened to `anchors`.
    pub fn with_anchors(mut self, anchors: Edges<bool>) -> Self {
        self.anchors = anchors;
        self
    }

    /// The same placement, `margin` in from each anchored edge.
    pub fn with_margin(mut self, margin: Edges<CssPx>) -> Self {
        self.margin = margin;
        self
    }

    /// The same placement with `zone` as its exclusive zone.
    pub fn with_exclusive_zone(mut self, zone: ExclusiveZone) -> Self {
        self.exclusive_zone = zone;
        self
    }

    /// The same placement, taking the keyboard as `keyboard` says.
    pub fn with_keyboard(mut self, keyboard: KeyboardInteractivity) -> Self {
        self.keyboard = keyboard;
        self
    }

    /// The same placement on `monitor`: a [`MonitorId`](crate::MonitorId) or an output name.
    pub fn with_monitor(mut self, monitor: impl Into<MonitorTarget>) -> Self {
        self.monitor = Some(monitor.into());
        self
    }

    /// The same placement under `namespace`.
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }
}

/// How a shell layer and the room other shell surfaces reserve affect each other.
///
/// A bar reserves a strip along its edge so that windows and other layers do not cover it. Every
/// other layer either moves to stay out of the strips the others reserve, or ignores them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum ExclusiveZone {
    /// Reserve nothing, and stay out of the room other surfaces reserve.
    ///
    /// What a pop-over below a bar wants: it opens below the bar, not under it.
    #[default]
    Avoid,
    /// Reserve this much room along the anchored edge, measured from that edge.
    ///
    /// What a bar or a dock wants. The room is reserved only when the surface is anchored to one
    /// edge, or to one edge and the two edges beside it.
    Reserve(CssPx),
    /// Reserve nothing, and extend under the room other surfaces reserve.
    ///
    /// What a wallpaper, a full-screen overlay or a lock screen wants.
    Ignore,
}

/// The parts of a [`LayerPlacement`] that can change after the surface is made.
///
/// Each field is a change when present and is left as it is when absent. The output and the
/// namespace are fixed when the surface is made, so they are not here.
///
/// ```
/// use zgui_geom::{CssPx, Edges};
/// use zgui_platform::{ExclusiveZone, LayerPatch, LayerPlacement};
///
/// let mut placement = LayerPlacement::default();
/// LayerPatch::new()
///     .with_exclusive_zone(ExclusiveZone::Reserve(CssPx(40.0)))
///     .apply_to(&mut placement);
/// assert_eq!(placement.exclusive_zone, ExclusiveZone::Reserve(CssPx(40.0)));
/// assert_eq!(placement.anchors, Edges::default(), "an absent field changes nothing");
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LayerPatch {
    /// A new layer.
    pub layer: Option<Layer>,
    /// New edges to fasten the surface to.
    pub anchors: Option<Edges<bool>>,
    /// New margins.
    pub margin: Option<Edges<CssPx>>,
    /// A new exclusive zone.
    pub exclusive_zone: Option<ExclusiveZone>,
    /// A new keyboard interactivity.
    pub keyboard: Option<KeyboardInteractivity>,
}

impl LayerPatch {
    /// A patch that changes nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same patch, moving the surface to `layer`.
    pub fn with_layer(mut self, layer: Layer) -> Self {
        self.layer = Some(layer);
        self
    }

    /// The same patch, fastening the surface to `anchors`.
    pub fn with_anchors(mut self, anchors: Edges<bool>) -> Self {
        self.anchors = Some(anchors);
        self
    }

    /// The same patch, with `margin` in from each anchored edge.
    pub fn with_margin(mut self, margin: Edges<CssPx>) -> Self {
        self.margin = Some(margin);
        self
    }

    /// The same patch, with `zone` as the exclusive zone.
    pub fn with_exclusive_zone(mut self, zone: ExclusiveZone) -> Self {
        self.exclusive_zone = Some(zone);
        self
    }

    /// The same patch, taking the keyboard as `keyboard` says.
    pub fn with_keyboard(mut self, keyboard: KeyboardInteractivity) -> Self {
        self.keyboard = Some(keyboard);
        self
    }

    /// Whether the patch changes nothing.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Writes every present field into `placement`.
    pub fn apply_to(&self, placement: &mut LayerPlacement) {
        if let Some(layer) = self.layer {
            placement.layer = layer;
        }
        if let Some(anchors) = self.anchors {
            placement.anchors = anchors;
        }
        if let Some(margin) = self.margin {
            placement.margin = margin;
        }
        if let Some(zone) = self.exclusive_zone {
            placement.exclusive_zone = zone;
        }
        if let Some(keyboard) = self.keyboard {
            placement.keyboard = keyboard;
        }
    }

    /// Adds the changes of `later` to this patch. A field present in both takes the later value.
    pub fn merge(&mut self, later: &Self) {
        self.layer = later.layer.or(self.layer);
        self.anchors = later.anchors.or(self.anchors);
        self.margin = later.margin.or(self.margin);
        self.exclusive_zone = later.exclusive_zone.or(self.exclusive_zone);
        self.keyboard = later.keyboard.or(self.keyboard);
    }
}

/// Which layer of the desktop a shell surface belongs to.
///
/// The order is the stacking order, bottom first. Ordinary windows sit between
/// [`Layer::Bottom`] and [`Layer::Top`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Layer {
    /// Behind everything: a wallpaper.
    Background,
    /// Below ordinary windows: a desktop widget.
    Bottom,
    /// Above ordinary windows: a dock, a panel, a bar.
    #[default]
    Top,
    /// Above everything: a notification, a launcher, a lock screen.
    Overlay,
}

/// How much of the keyboard a shell layer takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyboardInteractivity {
    /// None at all; the surface is decoration and the keyboard goes to the window underneath.
    #[default]
    None,
    /// All of it, for as long as the surface exists. A lock screen asks for this.
    Exclusive,
    /// Only while the user is interacting with it. A launcher asks for this.
    OnDemand,
}

#[cfg(test)]
mod tests {
    use super::{
        Anchor, Constrain, ExclusiveZone, KeyboardInteractivity, Layer, LayerPatch, LayerPlacement,
        PopupPlacement, SurfaceRole,
    };
    use crate::surface::id::SurfaceId;
    use zgui_geom::{CssPx, Edges, Point, Rect, Size};

    #[test]
    fn a_surface_that_names_no_role_is_an_ordinary_window() {
        assert_eq!(SurfaceRole::default(), SurfaceRole::Toplevel);
    }

    #[test]
    fn a_drop_down_hangs_from_the_bottom_left_of_its_field_and_slides() {
        let rect = Rect::new(
            Point::new(CssPx(4.0), CssPx(8.0)),
            Size::new(CssPx(120.0), CssPx(24.0)),
        );
        let placement = PopupPlacement::below(SurfaceId::new(1), rect);
        assert_eq!(placement.anchor, Anchor::BottomLeft);
        assert_eq!(placement.gravity, Anchor::BottomRight);
        assert_eq!(placement.constrain, Constrain::SLIDE);
        assert_eq!(placement.grab, None);
        let menu = placement.with_grab(crate::InputSerial::new(12));
        assert_eq!(menu.grab, Some(crate::InputSerial::new(12)));
    }

    #[test]
    fn the_layers_stack_from_the_wallpaper_up() {
        assert!(Layer::Background < Layer::Bottom);
        assert!(Layer::Bottom < Layer::Top);
        assert!(Layer::Top < Layer::Overlay);
    }

    #[test]
    fn a_layer_takes_no_keyboard_unless_it_asks() {
        assert_eq!(
            KeyboardInteractivity::default(),
            KeyboardInteractivity::None
        );
    }

    #[test]
    fn constraining_nothing_is_not_the_same_as_constraining_everything() {
        const { assert!(!Constrain::NONE.slide) };
        assert_eq!(Constrain::default(), Constrain::NONE);
        const { assert!(Constrain::ANY.slide && Constrain::ANY.flip && Constrain::ANY.resize) };
    }

    #[test]
    fn a_layer_avoids_the_room_others_reserve_unless_it_asks() {
        // A pop-over that ignored the bar's zone would open under the bar.
        assert_eq!(ExclusiveZone::default(), ExclusiveZone::Avoid);
        assert_eq!(
            LayerPlacement::default().exclusive_zone,
            ExclusiveZone::Avoid
        );
    }

    #[test]
    fn a_patch_changes_only_what_it_names() {
        let mut placement = LayerPlacement::new(Layer::Overlay)
            .with_anchors(Edges::new(true, false, false, false))
            .with_namespace("popover");
        LayerPatch::new()
            .with_keyboard(KeyboardInteractivity::OnDemand)
            .apply_to(&mut placement);
        assert_eq!(placement.keyboard, KeyboardInteractivity::OnDemand);
        assert_eq!(placement.layer, Layer::Overlay);
        assert_eq!(placement.anchors, Edges::new(true, false, false, false));
        assert_eq!(placement.namespace.as_deref(), Some("popover"));
    }

    #[test]
    fn a_later_patch_wins_where_both_name_a_field() {
        let mut first = LayerPatch::new()
            .with_layer(Layer::Top)
            .with_exclusive_zone(ExclusiveZone::Ignore);
        first.merge(&LayerPatch::new().with_layer(Layer::Overlay));
        assert_eq!(first.layer, Some(Layer::Overlay));
        assert_eq!(first.exclusive_zone, Some(ExclusiveZone::Ignore));
        assert!(!first.is_empty());
        assert!(LayerPatch::new().is_empty());
    }

    #[test]
    fn popup_geometry_insets_validate_whole_nonnegative_edges_and_checked_expansion() {
        let rect = Rect::new(
            Point::new(CssPx(0.0), CssPx(0.0)),
            Size::new(CssPx(40.0), CssPx(24.0)),
        );
        let base = PopupPlacement::below(SurfaceId::new(1), rect);
        assert_eq!(base.validated_geometry_insets(), Ok(Edges::new(0, 0, 0, 0)));

        let inset = base
            .clone()
            .with_geometry_insets(Edges::new(CssPx(16.0), CssPx(28.0), CssPx(32.0), CssPx(12.0)));
        assert_eq!(
            inset.expand_visual_size(Size::new(CssPx(360.0), CssPx(200.0))),
            Ok((
                Size::new(360, 200),
                Edges::new(16, 28, 32, 12),
                Size::new(400, 248),
            ))
        );

        for bad in [
            Edges::new(CssPx(-1.0), CssPx(0.0), CssPx(0.0), CssPx(0.0)),
            Edges::new(CssPx(0.5), CssPx(0.0), CssPx(0.0), CssPx(0.0)),
            Edges::new(CssPx(f32::NAN), CssPx(0.0), CssPx(0.0), CssPx(0.0)),
            Edges::new(CssPx(f32::INFINITY), CssPx(0.0), CssPx(0.0), CssPx(0.0)),
            Edges::new(CssPx(0.0), CssPx(1_500_000_000.0), CssPx(0.0), CssPx(1_500_000_000.0)),
        ] {
            assert!(
                base.clone()
                    .with_geometry_insets(bad)
                    .validated_geometry_insets()
                    .is_err(),
                "expected invalid insets to fail: {bad:?}"
            );
        }

        let overflow = base
            .with_geometry_insets(Edges::new(CssPx(0.0), CssPx(200.0), CssPx(0.0), CssPx(0.0)));
        assert!(
            overflow
                .expand_visual_size(Size::new(CssPx(2_147_483_520.0), CssPx(10.0)))
                .is_err()
        );
    }
}
