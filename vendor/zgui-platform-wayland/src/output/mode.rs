//! One output, as the contract describes it.

use std::time::Duration;

use smithay_client_toolkit::output::OutputInfo;
use wayland_client::protocol::wl_output::Transform;
use zgui_geom::{CssPx, DevicePx, Point, Size};
use zgui_platform::MonitorInfo;

/// What the contract knows about one output.
///
/// The extent is the mode the output is running, in its own pixels. The position is where the
/// compositor placed it in the desktop's coordinates, which the protocol reports in logical units;
/// it is scaled here so that every field of the answer is in the same space. When `xdg-output`
/// reports a logical extent (which reflects fractional scaling such as 1.75x when `wl_output`
/// rounds `scale_factor` up to 2), it is preserved on [`MonitorInfo::logical_size`].
pub fn describe(info: &OutputInfo) -> MonitorInfo {
    let (mut width, mut height) = info
        .modes
        .iter()
        .find(|mode| mode.current)
        .map_or((0, 0), |mode| mode.dimensions);
    if matches!(
        info.transform,
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
    ) {
        std::mem::swap(&mut width, &mut height);
    }
    let scale = f64::from(info.scale_factor.max(1));
    let (x, y) = info.logical_position.unwrap_or(info.location);
    let monitor = MonitorInfo::new(
        Point::new(
            DevicePx((f64::from(x) * scale) as f32),
            DevicePx((f64::from(y) * scale) as f32),
        ),
        Size::new(DevicePx(width as f32), DevicePx(height as f32)),
        scale,
    );
    let monitor = match info.logical_size.filter(|&(w, h)| w > 0 && h > 0) {
        Some((lw, lh)) => monitor.with_logical_size(Size::new(CssPx(lw as f32), CssPx(lh as f32))),
        None => monitor,
    };
    let monitor = match &info.name {
        Some(name) => monitor.with_name(name.clone()),
        None => monitor,
    };
    match refresh_rate(info) {
        Some(rate) => monitor.with_refresh_rate_millihertz(rate),
        None => monitor,
    }
}

/// The rate the output's current mode runs at, in thousandths of a hertz.
///
/// The protocol reports exactly that unit, so nothing is converted. A rate of zero means the
/// output did not say, and is reported as unknown rather than as a stopped display.
pub fn refresh_rate(info: &OutputInfo) -> Option<u32> {
    info.modes
        .iter()
        .find(|mode| mode.current)
        .map(|mode| mode.refresh_rate)
        .filter(|rate| *rate > 0)
        .map(|rate| rate as u32)
}

/// The interval between refreshes of the output's current mode.
pub fn interval(info: &OutputInfo) -> Option<Duration> {
    let rate = refresh_rate(info)?;
    Some(Duration::from_secs_f64(1_000.0 / f64::from(rate)))
}

/// The scale of the sharpest output in `infos`, for a surface that is on all of them.
///
/// The largest rather than the first: a window straddling a 1x and a 2x monitor drawn at 1x is
/// visibly soft on half of itself, and drawn at 2x is merely oversampled on the other half.
pub fn sharpest<'a>(infos: impl Iterator<Item = &'a OutputInfo>) -> Option<i32> {
    infos.map(|info| info.scale_factor.max(1)).max()
}

#[cfg(test)]
mod tests {
    use smithay_client_toolkit::output::{Mode, OutputData};
    use wayland_client::protocol::wl_output::Transform;
    use zgui_geom::{CssPx, DevicePx, Size};

    use super::describe;

    #[test]
    fn describe_preserves_xdg_output_logical_size_under_fractional_scaling() {
        let mut info = OutputData::new(1).with_output_info(Clone::clone);
        info.name = Some("DP-1".into());
        info.scale_factor = 2;
        info.transform = Transform::Normal;
        info.logical_position = Some((0, 0));
        info.logical_size = Some((2194, 1234));
        info.modes = vec![Mode {
            dimensions: (3840, 2160),
            refresh_rate: 60_000,
            current: true,
            preferred: true,
        }];

        let monitor = describe(&info);
        assert_eq!(monitor.name.as_deref(), Some("DP-1"));
        assert_eq!(monitor.size, Size::new(DevicePx(3840.0), DevicePx(2160.0)));
        assert_eq!(monitor.scale_factor, 2.0);
        assert_eq!(
            monitor.logical_size,
            Some(Size::new(CssPx(2194.0), CssPx(1234.0)))
        );
        assert_eq!(monitor.logical_size(), Size::new(CssPx(2194.0), CssPx(1234.0)));
    }
}
