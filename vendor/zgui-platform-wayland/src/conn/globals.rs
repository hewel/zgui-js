//! The globals this backend binds itself, beside the ones the toolkit owns.

use smithay_client_toolkit::globals::GlobalData;
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::{Dispatch, QueueHandle};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;

use crate::driver::WaylandState;
use crate::driver::background_effect::NiriBlurShapeManagerV1;

/// The optional protocols the toolkit has no wrapper for.
///
/// Fractional scaling globals are used as a pair; both background blur globals are optional.
#[derive(Debug, Default)]
pub struct Extras {
    /// `wp_fractional_scale_manager_v1`: what scale the compositor wants for a surface.
    pub fractional_scale: Option<WpFractionalScaleManagerV1>,
    /// `wp_viewporter`: the mapping from a buffer's extent to the extent it appears at.
    pub viewporter: Option<WpViewporter>,
    /// `ext_background_effect_manager_v1` along with its Wayland registry global name.
    pub background_effect: Option<(u32, ExtBackgroundEffectManagerV1)>,
    /// `niri_blur_shape_manager_v1` along with its Wayland registry global name.
    pub niri_blur_shape: Option<(u32, NiriBlurShapeManagerV1)>,
}

impl Extras {
    /// Binds whichever of these the compositor advertised.
    ///
    /// A missing global is not an error. Every one here is an improvement on a path that already
    /// works, so a compositor without them gets whole-number scaling rather than no window.
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<WaylandState>) -> Self {
        let fractional_scale = bind(globals, qh);
        let viewporter = bind(globals, qh);
        let background_effect = bind_named::<ExtBackgroundEffectManagerV1>(globals, qh);
        let niri_blur_shape = bind_named::<NiriBlurShapeManagerV1>(globals, qh);
        if fractional_scale.is_some() != viewporter.is_some() {
            tracing::debug!(
                fractional_scale = fractional_scale.is_some(),
                viewporter = viewporter.is_some(),
                "fractional scaling needs both globals; falling back to whole-number scaling"
            );
        }
        Self {
            fractional_scale,
            viewporter,
            background_effect,
            niri_blur_shape,
        }
    }

    /// Whether this compositor can scale a surface by something other than a whole number.
    pub const fn scales_fractionally(&self) -> bool {
        self.fractional_scale.is_some() && self.viewporter.is_some()
    }
}

/// Binds one optional global at version one.
fn bind<I>(globals: &GlobalList, qh: &QueueHandle<WaylandState>) -> Option<I>
where
    I: wayland_client::Proxy + 'static,
    WaylandState: Dispatch<I, GlobalData>,
{
    globals.bind(qh, 1..=1, GlobalData).ok()
}

/// Binds one optional global at version one and retains its registry global name for removal tracking.
fn bind_named<I>(globals: &GlobalList, qh: &QueueHandle<WaylandState>) -> Option<(u32, I)>
where
    I: wayland_client::Proxy + 'static,
    WaylandState: Dispatch<I, GlobalData>,
{
    let interface = I::interface().name;
    let name = globals.contents().with_list(|list| {
        list.iter()
            .find(|global| global.interface == interface)
            .map(|global| global.name)
    })?;
    let proxy: I = globals.bind(qh, 1..=1, GlobalData).ok()?;
    Some((name, proxy))
}
