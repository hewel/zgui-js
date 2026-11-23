//! Native overlay scrollbar visibility, auto-fade, and hover/drag emphasis state machine.
//!
//! All state lives sideband in the runtime keyed by `(NodeKey, Axis)` and is passed to
//! `zgui-paint` via [`OverlayScrollbarSource`] and to `zgui-input` via [`ScrollbarHitFilter`].
//! Revealing, widening, dragging, and fading an overlay scrollbar never mutate DOM style or
//! trigger relayout or hit-index rebuilds.

use rustc_hash::FxHashMap;
use web_time::{Duration, Instant};
use zgui_dom::{DocumentStore, NodeKey};
use zgui_geom::{Device, DevicePx, Point, Rect};
use zgui_input::ScrollbarHitFilter;
use zgui_layout::scroll_region::ScrollbarMode;
use zgui_layout::scroll_region::bar::live;
use zgui_layout::{Axis, BoxKey, FragmentKind, HitIndex, LayoutStore};
use zgui_paint::emit::scrollbar::{OverlayBarVisual, OverlayScrollbarSource};
use zgui_scene::{ClipTable, Placements, SpatialTree};

/// How long an overlay scrollbar stays fully visible after scrolling/hover/drag stops.
///
/// During this hold phase, the window parks on a single scheduled deadline rather than waking on
/// every refresh interval.
pub(crate) const HOLD_DURATION: Duration = Duration::from_millis(800);

/// How long an overlay scrollbar takes to fade out smoothly once its hold expires.
pub(crate) const FADE_DURATION: Duration = Duration::from_millis(200);

/// How long the resting (`4px`) <-> hovered/dragged (`6px`) emphasis transition takes.
pub(crate) const EMPHASIS_DURATION: Duration = Duration::from_millis(120);

/// Edge hotzone width/height inside a scroller's padding box, in CSS pixels.
///
/// Moving the pointer into this right/bottom edge strip reveals the overflowing overlay bar and
/// widens it for easy grabbing.
pub(crate) const HOTZONE_CSS: f32 = 16.0;

#[derive(Clone, Copy, Debug)]
enum FadePhase {
    /// Held visible while hovered in the edge hotzone or actively dragged.
    Active,
    /// Held visible after scrolling/hover/drag ended until `until` (zero frames rendered).
    Holding { until: Instant },
    /// Smoothly fading from `from_alpha` to `0.0` over [`FADE_DURATION`].
    Fading { started: Instant, from_alpha: f32 },
}

#[derive(Clone, Copy, Debug)]
struct EmphasisState {
    current: f32,
    target: f32,
    transition: Option<(Instant, f32)>,
}

impl EmphasisState {
    fn settled(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            transition: None,
        }
    }

    fn is_animating(&self) -> bool {
        self.transition.is_some()
    }
}

#[derive(Clone, Debug)]
struct BarEntry {
    phase: FadePhase,
    emphasis: EmphasisState,
    painted: OverlayBarVisual,
    last_rect: Option<Rect<DevicePx, Device>>,
}

/// Runtime state machine managing all live overlay scrollbars for one window.
#[derive(Debug, Default)]
pub(crate) struct OverlayScrollbars {
    bars: FxHashMap<(NodeKey, Axis), BarEntry>,
    pending_scrolled: Vec<(NodeKey, Option<Axis>)>,
}

impl OverlayScrollbars {
    /// Creates an empty overlay scrollbar tracker (all overlay bars start hidden).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Notes that `container` scrolled (wheel, touchpad, keyboard, or programmatic scroll).
    pub(crate) fn note_scrolled(&mut self, container: NodeKey) {
        if !self
            .pending_scrolled
            .iter()
            .any(|(node, axis)| *node == container && axis.is_none())
        {
            self.pending_scrolled.push((container, None));
        }
    }

    /// Returns the current visual state of `(container, axis)` (`HIDDEN` when inactive).
    pub(crate) fn visual_of(&self, container: NodeKey, axis: Axis) -> OverlayBarVisual {
        self.bars
            .get(&(container, axis))
            .map_or(OverlayBarVisual::HIDDEN, |entry| entry.painted)
    }

    /// Whether any overlay scrollbar is actively fading or widening/narrowing on vsync.
    ///
    /// False during the 800ms hold period (which wakes via [`Self::next_deadline`]) and false when
    /// all overlay bars are hidden or settled.
    pub(crate) fn is_animating(&self) -> bool {
        self.bars.values().any(|entry| {
            matches!(entry.phase, FadePhase::Fading { .. }) || entry.emphasis.is_animating()
        })
    }

    /// Earliest hold-expiration deadline across all holding overlay bars.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.bars
            .values()
            .filter_map(|entry| match entry.phase {
                FadePhase::Holding { until } => Some(until),
                FadePhase::Active | FadePhase::Fading { .. } => None,
            })
            .min()
    }

    /// Advances all overlay scrollbar state against the frame's deterministic clock `now`, returning
    /// the device-space rectangles whose painted visual changed and must be damaged.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn plan(
        &mut self,
        now: Instant,
        document: &DocumentStore,
        layout: &LayoutStore,
        hit: &HitIndex,
        clips: &ClipTable,
        spatial: &SpatialTree,
        placements: &Placements,
        scale: f32,
        pointer: Option<Point<DevicePx, Device>>,
        held_scrollbars: &[(NodeKey, Axis)],
        scrolled_this_frame: &[zgui_scroll::report::Scrolled],
        reduced_motion: bool,
    ) -> Vec<Rect<DevicePx, Device>> {
        let hovered = hotzone_bars(document, layout, hit, clips, spatial, pointer, scale);

        let mut woken: Vec<(NodeKey, Axis)> = Vec::new();
        let pending = core::mem::take(&mut self.pending_scrolled);
        for (node, opt_axis) in pending {
            push_woken(layout, node, opt_axis, &mut woken);
        }
        for moved in scrolled_this_frame {
            let moved_y = (moved.to.y.0 - moved.from.y.0).abs() > 0.01;
            let moved_x = (moved.to.x.0 - moved.from.x.0).abs() > 0.01;
            if moved_y {
                push_woken(layout, moved.container, Some(Axis::Vertical), &mut woken);
            }
            if moved_x {
                push_woken(layout, moved.container, Some(Axis::Horizontal), &mut woken);
            }
        }

        // Fast path: nothing live, nothing hovered/held/woken.
        if self.bars.is_empty()
            && hovered.is_empty()
            && held_scrollbars.is_empty()
            && woken.is_empty()
        {
            return Vec::new();
        }

        let mut candidates: Vec<(NodeKey, Axis)> = self.bars.keys().copied().collect();
        for &key in hovered
            .iter()
            .chain(held_scrollbars.iter())
            .chain(woken.iter())
        {
            if !candidates.contains(&key) {
                candidates.push(key);
            }
        }

        let mut damage = Vec::new();
        for key @ (container, axis) in candidates {
            let current_rect = bar_device_rect(document, layout, placements, container, axis);
            let Some(rect) = current_rect else {
                if let Some(removed) = self.bars.remove(&key)
                    && removed.painted.alpha > 0.0
                    && let Some(prev) = removed.last_rect
                {
                    damage.push(prev);
                }
                continue;
            };

            let is_held = held_scrollbars.contains(&key);
            let is_hovered = hovered.contains(&key);
            let is_woken = woken.contains(&key);
            let is_pinned = is_held || is_hovered;

            let entry = match self.bars.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(slot) => {
                    if !is_pinned && !is_woken {
                        continue;
                    }
                    let phase = if is_pinned {
                        FadePhase::Active
                    } else {
                        FadePhase::Holding {
                            until: now + HOLD_DURATION,
                        }
                    };
                    let emphasis = if is_pinned {
                        if reduced_motion {
                            EmphasisState::settled(1.0)
                        } else {
                            EmphasisState {
                                current: 0.0,
                                target: 1.0,
                                transition: Some((now, 0.0)),
                            }
                        }
                    } else {
                        EmphasisState::settled(0.0)
                    };
                    slot.insert(BarEntry {
                        phase,
                        emphasis,
                        painted: OverlayBarVisual::HIDDEN,
                        last_rect: None,
                    })
                }
            };

            let mut snap_hidden = false;
            if is_pinned {
                entry.phase = FadePhase::Active;
            } else if is_woken {
                entry.phase = FadePhase::Holding {
                    until: now + HOLD_DURATION,
                };
            } else {
                match entry.phase {
                    FadePhase::Active => {
                        entry.phase = FadePhase::Holding {
                            until: now + HOLD_DURATION,
                        };
                    }
                    FadePhase::Holding { until } if now >= until => {
                        if reduced_motion {
                            snap_hidden = true;
                        } else {
                            let from_alpha = if entry.painted.alpha > 0.0 {
                                entry.painted.alpha.clamp(0.0, 1.0)
                            } else {
                                1.0
                            };
                            entry.phase = FadePhase::Fading {
                                started: until,
                                from_alpha,
                            };
                        }
                    }
                    FadePhase::Holding { .. } => {}
                    FadePhase::Fading { .. } if reduced_motion => {
                        snap_hidden = true;
                    }
                    FadePhase::Fading { .. } => {}
                }
            }

            let alpha = if snap_hidden {
                0.0
            } else {
                match entry.phase {
                    FadePhase::Active | FadePhase::Holding { .. } => 1.0,
                    FadePhase::Fading {
                        started,
                        from_alpha,
                    } => {
                        let elapsed = now.saturating_duration_since(started);
                        let t = (elapsed.as_secs_f32() / FADE_DURATION.as_secs_f32())
                            .clamp(0.0, 1.0);
                        let value = from_alpha * (1.0 - t);
                        if t >= 1.0 || value <= 0.004 {
                            0.0
                        } else {
                            value
                        }
                    }
                }
            };

            let target_emphasis = if is_pinned { 1.0 } else { 0.0 };
            if reduced_motion {
                entry.emphasis = EmphasisState::settled(target_emphasis);
            } else {
                if (entry.emphasis.target - target_emphasis).abs() > f32::EPSILON {
                    entry.emphasis.target = target_emphasis;
                    entry.emphasis.transition = Some((now, entry.emphasis.current));
                }
                if let Some((started, from)) = entry.emphasis.transition {
                    let elapsed = now.saturating_duration_since(started);
                    let t = (elapsed.as_secs_f32() / EMPHASIS_DURATION.as_secs_f32())
                        .clamp(0.0, 1.0);
                    if t >= 1.0 {
                        entry.emphasis.current = target_emphasis;
                        entry.emphasis.transition = None;
                    } else {
                        entry.emphasis.current = from + (target_emphasis - from) * t;
                    }
                }
            }

            let new_painted = if alpha <= 0.0 {
                OverlayBarVisual::HIDDEN
            } else {
                OverlayBarVisual {
                    alpha,
                    emphasis: entry.emphasis.current,
                }
            };

            let changed = entry.painted.signature() != new_painted.signature();
            if changed {
                damage.push(rect);
                if let Some(prev) = entry.last_rect
                    && prev != rect
                {
                    damage.push(prev);
                }
            }
            entry.painted = new_painted;
            entry.last_rect = Some(rect);

            if new_painted.alpha <= 0.0 {
                self.bars.remove(&key);
            }
        }

        damage
    }
}

impl OverlayScrollbarSource for OverlayScrollbars {
    fn visual(&self, container: NodeKey, axis: Axis) -> Option<OverlayBarVisual> {
        Some(self.visual_of(container, axis))
    }
}

impl ScrollbarHitFilter for OverlayScrollbars {
    fn is_scrollbar_interactive(&self, container: NodeKey, axis: Axis) -> bool {
        self.visual_of(container, axis).alpha > 0.0
    }
}

/// Pushes overflowing overlay axes of `container` into `out`.
fn push_woken(
    layout: &LayoutStore,
    container: NodeKey,
    only_axis: Option<Axis>,
    out: &mut Vec<(NodeKey, Axis)>,
) {
    let Some(&box_key) = layout.boxes_of(container).first() else {
        return;
    };
    if layout.scrollbar_mode(box_key) != ScrollbarMode::Overlay {
        return;
    }
    let (has_v, has_h) = bar_axes(layout, box_key);
    let mut push_axis = |axis: Axis| {
        if !out.contains(&(container, axis)) {
            out.push((container, axis));
        }
    };
    match only_axis {
        Some(Axis::Vertical) if has_v => push_axis(Axis::Vertical),
        Some(Axis::Horizontal) if has_h => push_axis(Axis::Horizontal),
        Some(_) => {}
        None => {
            if has_v {
                push_axis(Axis::Vertical);
            }
            if has_h {
                push_axis(Axis::Horizontal);
            }
        }
    }
}

/// Returns `(has_vertical, has_horizontal)` scrollbar fragments on `box_key`.
fn bar_axes(layout: &LayoutStore, box_key: BoxKey) -> (bool, bool) {
    let mut has_v = false;
    let mut has_h = false;
    for &frag_key in layout.fragments_of_box(box_key) {
        let Some(fragment) = layout.fragment(frag_key) else {
            continue;
        };
        match fragment.kind {
            FragmentKind::Scrollbar {
                axis: Axis::Vertical,
                ..
            } => has_v = true,
            FragmentKind::Scrollbar {
                axis: Axis::Horizontal,
                ..
            } => has_h = true,
            _ => {}
        }
    }
    (has_v, has_h)
}

/// Device-space union rectangle of all scrollbar fragments for `(container, axis)` when `container`
/// is a live overlay scroller with an active bar on `axis`.
fn bar_device_rect(
    document: &DocumentStore,
    layout: &LayoutStore,
    placements: &Placements,
    container: NodeKey,
    axis: Axis,
) -> Option<Rect<DevicePx, Device>> {
    if document.index_of(container).is_none() {
        return None;
    }
    let &box_key = layout.boxes_of(container).first()?;
    if layout.scrollbar_mode(box_key) != ScrollbarMode::Overlay {
        return None;
    }
    let mut union: Option<Rect<DevicePx, Device>> = None;
    for &frag_key in layout.fragments_of_box(box_key) {
        let Some(fragment) = layout.fragment(frag_key) else {
            continue;
        };
        if !matches!(
            fragment.kind,
            FragmentKind::Scrollbar { axis: frag_axis, .. } if frag_axis == axis
        ) {
            continue;
        }
        let rect = fragment.on_device(fragment.border_box, placements);
        union = Some(match union {
            Some(existing) => existing.union(rect),
            None => rect,
        });
    }
    union
}

/// Finds which overflowing overlay scrollbar edge hotzones `pointer` currently rests inside.
///
/// Uses the topmost non-inert fragment under `pointer` and walks only its box ancestor chain
/// (`O(depth)`), automatically respecting clips, transforms, inert subtrees, and occlusion by
/// overlays or modals above the scroll container.
fn hotzone_bars(
    document: &DocumentStore,
    layout: &LayoutStore,
    hit: &HitIndex,
    clips: &ClipTable,
    spatial: &SpatialTree,
    pointer: Option<Point<DevicePx, Device>>,
    scale: f32,
) -> Vec<(NodeKey, Axis)> {
    let mut result = Vec::new();
    let Some(point) = pointer else {
        return result;
    };

    let mut topmost = None;
    for frag_key in hit.hit(point, clips, spatial) {
        let Some(fragment) = layout.fragment(frag_key) else {
            continue;
        };
        let owner = match fragment.kind {
            FragmentKind::Scrollbar { .. } => layout.get(fragment.box_).and_then(|b| b.source),
            _ => zgui_input::hit::element_of(layout, frag_key),
        };
        let Some(owner) = owner else {
            continue;
        };
        if document.is_effectively_inert(owner) {
            continue;
        }
        topmost = Some((fragment, owner));
        break;
    }
    let Some((top_frag, top_owner)) = topmost else {
        return result;
    };

    if let FragmentKind::Scrollbar { axis, .. } = top_frag.kind
        && layout.scrollbar_mode(top_frag.box_) == ScrollbarMode::Overlay
    {
        result.push((top_owner, axis));
        return result;
    }

    let hotzone = HOTZONE_CSS * scale.max(0.0);
    let mut cursor = Some(top_frag.box_);
    while let Some(box_key) = cursor {
        let Some(box_node) = layout.get(box_key) else {
            break;
        };
        if layout.scrollbar_mode(box_key) == ScrollbarMode::Overlay
            && let Some(owner) = box_node.source
            && !document.is_effectively_inert(owner)
        {
            let (has_v, has_h) = bar_axes(layout, box_key);
            if (has_v || has_h)
                && let Some(main_frag) = layout
                    .fragments_of_box(box_key)
                    .first()
                    .and_then(|k| layout.fragment(*k))
                && let Some(local) = live::into_bar_space(layout, spatial, box_key, point)
            {
                let padding = main_frag.padding_box();
                if has_v
                    && local.y.0 >= padding.top().0
                    && local.y.0 <= padding.bottom().0
                    && local.x.0 >= padding.right().0 - hotzone
                    && local.x.0 <= padding.right().0
                    && !result.contains(&(owner, Axis::Vertical))
                {
                    result.push((owner, Axis::Vertical));
                }
                if has_h
                    && local.x.0 >= padding.left().0
                    && local.x.0 <= padding.right().0
                    && local.y.0 >= padding.bottom().0 - hotzone
                    && local.y.0 <= padding.bottom().0
                    && !result.contains(&(owner, Axis::Horizontal))
                {
                    result.push((owner, Axis::Horizontal));
                }
            }
        }
        cursor = box_node.parent;
    }

    result
}
