//! Required visibility uses the same placed fragment and clip identities as native painting.
use super::*;
use zgui_geom::{DevicePx, Point, Rect};
use zgui_paint::lower::anim::AnimOverrides;

pub(crate) struct ImageVisibility<'a> {
    pub(crate) surface: Size<DevicePx, Device>,
    pub(crate) clips: &'a zgui_scene::ClipTable,
    pub(crate) placements: &'a zgui_scene::Placements,
    pub(crate) anim: &'a dyn AnimOverrides,
}

impl ImageVisibility<'_> {
    pub(super) fn visible(&self, layout: &LayoutStore, node: NodeKey, unprobed: bool) -> bool {
        let viewport = Rect::new(Point::new(DevicePx(0.0), DevicePx(0.0)), self.surface);
        for box_ in layout.boxes_of(node) {
            let current = layout.node(*box_);
            if current.style.get_inherited_box().visibility != zgui_css::values::size::VisibilityValue::Visible {
                continue;
            }
            let mut ancestor = Some(*box_);
            let mut transparent = false;
            while let Some(key) = ancestor {
                let box_ = layout.node(key);
                let opacity = box_.source.and_then(|node| self.anim.get(node)).and_then(|over| over.opacity)
                    .unwrap_or(box_.style.get_effects().opacity);
                if opacity <= 0.0 { transparent = true; break; }
                ancestor = box_.parent;
            }
            if transparent { continue; }
            for fragment in layout.fragments_of_box(*box_) {
                let Some(fragment) = layout.fragment(*fragment) else { continue; };
                let mut content = fragment.content_box();
                // Intrinsic-only images can have zero extent until the header lands. Their
                // on-screen origin still represents potential content, never an empty pass.
                if unprobed {
                    content.size.width.0 = content.size.width.0.max(1.0);
                    content.size.height.0 = content.size.height.0.max(1.0);
                }
                let placed = zgui_layout::fragment::transform::placed::onto_device(
                    content, fragment.transform, self.placements,
                );
                let Some(on_screen) = placed.intersection(viewport) else { continue; };
                let clip = self.clips.bounds_placed(fragment.clip, &|id| self.placements.get(id).copied());
                if on_screen.intersects(clip) { return true; }
            }
        }
        false
    }
}
