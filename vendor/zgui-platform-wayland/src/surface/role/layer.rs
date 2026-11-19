//! A part of the desktop shell: what the contract asks for, in the shell's own terms.

use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerSurface,
};
use smithay_client_toolkit::shell::wlr_layer::SurfaceKind;
use wayland_client::Proxy;
use zgui_geom::{Css, CssPx, Size};
use zgui_platform::{LayerPlacement, PlatformError, Unsupported};

/// Which layer of the desktop, in the shell's numbering.
pub const fn layer(layer: zgui_platform::Layer) -> Layer {
    match layer {
        zgui_platform::Layer::Background => Layer::Background,
        zgui_platform::Layer::Bottom => Layer::Bottom,
        zgui_platform::Layer::Overlay => Layer::Overlay,
        _ => Layer::Top,
    }
}

/// Which edges the surface is fastened to.
pub fn anchor(anchors: zgui_geom::Edges<bool>) -> Anchor {
    let mut edges = Anchor::empty();
    edges.set(Anchor::TOP, anchors.top);
    edges.set(Anchor::RIGHT, anchors.right);
    edges.set(Anchor::BOTTOM, anchors.bottom);
    edges.set(Anchor::LEFT, anchors.left);
    edges
}

/// How much of the keyboard the surface takes.
pub const fn keyboard(
    interactivity: zgui_platform::KeyboardInteractivity,
) -> KeyboardInteractivity {
    match interactivity {
        zgui_platform::KeyboardInteractivity::Exclusive => KeyboardInteractivity::Exclusive,
        zgui_platform::KeyboardInteractivity::OnDemand => KeyboardInteractivity::OnDemand,
        _ => KeyboardInteractivity::None,
    }
}

/// How much room the compositor should reserve, in whole logical pixels.
///
/// An absent value uses protocol `-1`: ignore the space reserved by other surfaces.
pub fn exclusive_zone(zone: Option<CssPx>) -> i32 {
    zone.map_or(-1, |zone| zone.0.round().max(0.0) as i32)
}

/// Applies the placement shared by initial creation and live reconfiguration.
pub(crate) fn apply(surface: &LayerSurface, placement: &LayerPlacement) {
    surface.set_anchor(anchor(placement.anchors));
    apply_fields(surface, placement);
}

fn apply_fields(surface: &LayerSurface, placement: &LayerPlacement) {
    surface.set_keyboard_interactivity(keyboard(placement.keyboard));
    surface.set_exclusive_zone(exclusive_zone(placement.exclusive_zone));
    let margin = placement.margin.map(|side| side.0.round() as i32);
    surface.set_margin(margin.top, margin.right, margin.bottom, margin.left);
}

/// Encodes a logical size without requesting a compositor extent on an unanchored axis.
pub(crate) fn extent(
    size: Size<CssPx, Css>,
    placement: &LayerPlacement,
) -> Result<(u32, u32), PlatformError> {
    fn axis(value: CssPx, stretch: bool) -> Option<u32> {
        if !(0.0..=f64::from(u32::MAX)).contains(&f64::from(value.0)) {
            None
        } else if value.0 == 0.0 {
            stretch.then_some(0)
        } else {
            Some(value.0.round().max(1.0) as u32)
        }
    }
    let anchors = placement.anchors;
    match (
        axis(size.width, anchors.left && anchors.right),
        axis(size.height, anchors.top && anchors.bottom),
    ) {
        (Some(width), Some(height)) => Ok((width, height)),
        _ => Err(PlatformError::Backend(
            "layer size must fit the unsigned protocol range and be positive \
             unless stretched between opposite anchors"
                .to_owned(),
        )),
    }
}

pub(crate) fn check_keyboard(version: u32, placement: &LayerPlacement) -> Result<(), PlatformError> {
    if version < 4 && placement.keyboard == zgui_platform::KeyboardInteractivity::OnDemand {
        return Err(Unsupported.into());
    }
    Ok(())
}

pub(crate) fn version(surface: &LayerSurface) -> u32 {
    match surface.kind() {
        SurfaceKind::Wlr(role) => role.version(),
        _ => 1,
    }
}

/// Sends one placement transaction. Neither request order exposes a zero dimension without its
/// matching anchors, including while switching the stretch from one axis to the other.
pub(crate) fn reconfigure(
    surface: &LayerSurface,
    current: &LayerPlacement,
    current_extent: (u32, u32),
    next: &LayerPlacement,
    next_extent: (u32, u32),
) -> Result<(), PlatformError> {
    if current.monitor != next.monitor {
        return Err(PlatformError::Backend(
            "moving a layer surface to another output requires reopening it".to_owned(),
        ));
    }
    let version = version(surface);
    check_keyboard(version, next)?;
    if current.layer != next.layer && version < 2 {
        return Err(Unsupported.into());
    }
    let anchors = next.anchors;
    if current_extent.0 == 0 && !(anchors.left && anchors.right)
        || current_extent.1 == 0 && !(anchors.top && anchors.bottom)
    {
        surface.set_size(next_extent.0.max(current_extent.0).max(1), next_extent.1.max(current_extent.1).max(1));
    }
    surface.set_anchor(anchor(anchors));
    surface.set_size(next_extent.0, next_extent.1);
    if current.layer != next.layer {
        surface.set_layer(layer(next.layer));
    }
    apply_fields(surface, next);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{anchor, exclusive_zone, extent, keyboard, layer};
    use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
    use zgui_geom::{CssPx, Edges, Size};
    use zgui_platform::LayerPlacement;

    #[test]
    fn layer_extent_accepts_the_largest_representable_f32_below_the_protocol_limit() {
        let placement = LayerPlacement::default();
        // Converting the integer maximum to f32 rounds up to 2^32.
        let first_out_of_range = u32::MAX as f32;
        let last_in_range = first_out_of_range.next_down();
        assert_eq!(
            extent(Size::new(CssPx(last_in_range), CssPx(1.0)), &placement)
                .expect("the largest in-range f32 extent fits the protocol"),
            (4_294_967_040, 1),
            "the largest representable in-range size must not be clamped"
        );
    }

    #[test]
    fn every_layer_of_the_desktop_has_one_in_the_shell() {
        assert_eq!(layer(zgui_platform::Layer::Background), Layer::Background);
        assert_eq!(layer(zgui_platform::Layer::Bottom), Layer::Bottom);
        assert_eq!(layer(zgui_platform::Layer::Top), Layer::Top);
        assert_eq!(layer(zgui_platform::Layer::Overlay), Layer::Overlay);
    }

    #[test]
    fn a_bar_across_the_top_is_anchored_to_three_edges() {
        let edges = anchor(Edges::new(true, true, false, true));
        assert_eq!(edges, Anchor::TOP | Anchor::RIGHT | Anchor::LEFT);
    }

    #[test]
    fn anchoring_to_nothing_is_a_centred_surface_rather_than_an_error() {
        assert_eq!(
            anchor(Edges::new(false, false, false, false)),
            Anchor::empty()
        );
    }

    #[test]
    fn ignoring_other_reservations_differs_from_reserving_nothing() {
        assert_eq!(exclusive_zone(None), -1);
        assert_eq!(exclusive_zone(Some(CssPx(0.0))), 0);
        assert_eq!(exclusive_zone(Some(CssPx(36.4))), 36);
    }

    #[test]
    fn a_negative_reservation_is_clamped_rather_than_read_as_the_default() {
        assert_eq!(exclusive_zone(Some(CssPx(-8.0))), 0);
    }

    #[test]
    fn a_surface_takes_no_keyboard_unless_it_asked() {
        assert_eq!(
            keyboard(zgui_platform::KeyboardInteractivity::None),
            KeyboardInteractivity::None
        );
        assert_eq!(
            keyboard(zgui_platform::KeyboardInteractivity::Exclusive),
            KeyboardInteractivity::Exclusive
        );
        assert_eq!(
            keyboard(zgui_platform::KeyboardInteractivity::OnDemand),
            KeyboardInteractivity::OnDemand
        );
    }
}
