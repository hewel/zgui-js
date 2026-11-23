//! How a scroll container presents its scrollbars, read off its computed style.
//!
//! `--zgui-scrollbar-mode: classic | overlay` (default `classic`; invalid values fall back to
//! `classic`). Read through an interned [`Property`] + [`CustomProperties`] bit so it inherits
//! naturally from `:root` and costs nothing on documents that never declare it.

use zgui_css::ComputedStyle;
use zgui_css::values::custom::Property;

use crate::fragment::custom_properties::CustomProperties;

/// The custom property a scroll container names its scrollbar presentation mode through.
pub const SCROLLBAR_MODE: &str = "zgui-scrollbar-mode";

/// [`SCROLLBAR_MODE`], interned.
pub(crate) static SCROLLBAR_MODE_PROPERTY: Property = Property::new(SCROLLBAR_MODE);

/// How a scroll container reserves space for and draws its scrollbars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollbarMode {
    /// Traditional layout-reserving gutter scrollbar (default, 100% backwards compatible).
    #[default]
    Classic,
    /// Zero-gutter floating capsule scrollbar drawn over the scrollport edge.
    Overlay,
}

/// The scrollbar mode `style` names.
///
/// Defaults to [`ScrollbarMode::Classic`] when absent or unrecognised.
#[must_use]
pub fn of(style: &ComputedStyle) -> ScrollbarMode {
    of_with(style, CustomProperties::ALL)
}

/// [`of`], for a style that declares `declared`.
#[must_use]
pub fn of_with(style: &ComputedStyle, declared: CustomProperties) -> ScrollbarMode {
    if !declared.scrollbar_mode() {
        return ScrollbarMode::Classic;
    }
    let Some(written) = SCROLLBAR_MODE_PROPERTY.text(style) else {
        return ScrollbarMode::Classic;
    };
    parse(written).unwrap_or(ScrollbarMode::Classic)
}

fn parse(written: &str) -> Option<ScrollbarMode> {
    let written = written.trim();
    if written.eq_ignore_ascii_case("overlay") {
        return Some(ScrollbarMode::Overlay);
    }
    if written.eq_ignore_ascii_case("classic") {
        return Some(ScrollbarMode::Classic);
    }
    None
}

#[cfg(test)]
mod tests {
    use zgui_css::StyleDraft;

    use super::{SCROLLBAR_MODE, ScrollbarMode, of, of_with, parse};
    use crate::fragment::custom_properties::CustomProperties;

    #[test]
    fn parses_classic_and_overlay_case_insensitively() {
        assert_eq!(parse("classic"), Some(ScrollbarMode::Classic));
        assert_eq!(parse("overlay"), Some(ScrollbarMode::Overlay));
        assert_eq!(parse("  OVERLAY  "), Some(ScrollbarMode::Overlay));
        assert_eq!(parse("Classic"), Some(ScrollbarMode::Classic));
        assert_eq!(parse("auto"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn inherits_from_root_and_allows_local_override_and_invalid_fallback() {
        let root = StyleDraft::initial()
            .with_custom_property(SCROLLBAR_MODE, "overlay")
            .build();
        let child = zgui_css::inherited_style(&root);
        let declared = CustomProperties::of(&child);
        assert!(declared.scrollbar_mode());
        assert_eq!(of(&root), ScrollbarMode::Overlay);
        assert_eq!(of_with(&child, declared), ScrollbarMode::Overlay);

        let override_classic = StyleDraft::initial()
            .with_custom_property(SCROLLBAR_MODE, "classic")
            .build();
        assert_eq!(of(&override_classic), ScrollbarMode::Classic);

        let invalid = StyleDraft::initial()
            .with_custom_property(SCROLLBAR_MODE, "floating")
            .build();
        assert_eq!(of(&invalid), ScrollbarMode::Classic);
    }
}
