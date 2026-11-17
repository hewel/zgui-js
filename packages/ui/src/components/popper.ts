import { createEffect, createMemo, createSignal } from "#solid";
import { createElement, spread } from "../renderer/index.js";
import { observeGeometry } from "../nodes.js";
import type { NativeChild, NativeNode, NativePoint, NativeSize, NativeStyle, NativeViewport, NativeWindowRect } from "../types.js";
import { slot } from "./shared.js";

export type PopperSide = "top" | "right" | "bottom" | "left";
export type PopperAlign = "start" | "center" | "end";
export interface PopperPlacement { readonly side: PopperSide; readonly align: PopperAlign; }
export interface PopperOptions {
  placement?: PopperPlacement;
  offset?: number;
  padding?: number;
  flip?: boolean;
  shift?: boolean;
}
export interface PopperSolution {
  origin: NativePoint;
  placement: PopperPlacement;
  available: NativeSize;
  overflow: number;
}
export interface PopperProps extends PopperOptions {
  anchor: NativeNode | null;
  active?: boolean;
  class?: string;
  ref?: (node: NativeNode) => void;
  children?: NativeChild;
}
const opposite: Record<PopperSide, PopperSide> = { top: "bottom", right: "left", bottom: "top", left: "right" };
const vertical = (side: PopperSide) => side === "top" || side === "bottom";
function room(side: PopperSide, anchor: NativeWindowRect, viewport: NativeWindowRect): number {
  switch (side) {
    case "top": return anchor.y - viewport.y;
    case "bottom": return viewport.y + viewport.height - anchor.y - anchor.height;
    case "left": return anchor.x - viewport.x;
    case "right": return viewport.x + viewport.width - anchor.x - anchor.width;
  }
}
const along = (align: PopperAlign, start: number, anchor: number, size: number) =>
  start + (align === "start" ? 0 : align === "center" ? (anchor - size) / 2 : anchor - size);
const clampSpan = (position: number, size: number, start: number, extent: number, padding: number) =>
  Math.max(start + padding, Math.min(position, start + extent - padding - size));
const snap = (value: number, scale: number) => Math.sign(value) * Math.round(Math.abs(value) * scale) / scale;
const px = (value: number) => `${value}px`;

export function solvePopper(anchor: NativeWindowRect, floating: NativeSize, viewport: NativeWindowRect, options: PopperOptions = {}): PopperSolution {
  let { side, align } = options.placement ?? { side: "bottom", align: "center" };
  const offset = options.offset ?? 4, padding = options.padding ?? 8;
  const flip = options.flip ?? true, shift = options.shift ?? true;
  if (!["top", "right", "bottom", "left"].includes(side) || !["start", "center", "end"].includes(align))
    throw new TypeError("Popper placement must have a valid side and alignment");
  if (!Number.isFinite(offset) || !Number.isFinite(padding)) throw new RangeError("Popper offset and padding must be finite");
  if (typeof flip !== "boolean" || typeof shift !== "boolean") throw new TypeError("Popper flip and shift must be booleans");
  if (flip) {
    const asked = room(side, anchor, viewport), across = room(opposite[side], anchor, viewport);
    const needed = (vertical(side) ? floating.height : floating.width) + offset + padding;
    if (asked < needed && (across >= needed || across > asked)) side = opposite[side];
  }
  const isVertical = vertical(side), main = Math.max(0, room(side, anchor, viewport) - offset - padding);
  const available = {
    width: isVertical ? Math.max(0, viewport.width - 2 * padding) : main,
    height: isVertical ? main : Math.max(0, viewport.height - 2 * padding),
  };
  let x = isVertical ? along(align, anchor.x, anchor.width, floating.width)
    : side === "left" ? anchor.x - Math.min(floating.width, available.width) - offset : anchor.x + anchor.width + offset;
  let y = !isVertical ? along(align, anchor.y, anchor.height, floating.height)
    : side === "top" ? anchor.y - Math.min(floating.height, available.height) - offset : anchor.y + anchor.height + offset;
  if (shift) {
    if (isVertical) x = clampSpan(x, floating.width, viewport.x, viewport.width, padding);
    else y = clampSpan(y, floating.height, viewport.y, viewport.height, padding);
  }
  return { origin: { x, y }, placement: { side, align }, available,
    overflow: Math.max(0, viewport.x - x, viewport.y - y,
      x + floating.width - viewport.x - viewport.width, y + floating.height - viewport.y - viewport.height) };
}

export function Popper(props: PopperProps): NativeNode {
  const node = createElement("box");
  const [anchor, setAnchor] = createSignal<NativeWindowRect | null>(null);
  const [floating, setFloating] = createSignal<NativeSize | null>(null);
  const [viewport, setViewport] = createSignal<NativeViewport | null>(null);
  const active = () => props.active ?? true;
  createEffect(() => {
    const target = props.anchor;
    setAnchor(null);
    if (active() && target) observeGeometry(target, "borderBox", setAnchor);
  });
  createEffect(() => {
    setFloating(null); setViewport(null);
    if (active()) {
      observeGeometry(node, "contentSize", setFloating);
      observeGeometry(node, "viewport", setViewport);
    }
  });
  const placed = createMemo(() => {
    if (!active()) return null;
    const a = anchor(), f = floating(), v = viewport();
    if (!a || !f || !v || (f.width <= 0 && f.height <= 0)) return null;
    return { ...solvePopper(a, f, v, props), anchor: a, floating: f, scale: v.scale };
  });
  spread(node, {
    get class() { return props.class; },
    get "data-side"() { return placed()?.placement.side ?? null; },
    get "data-align"() { return placed()?.placement.align ?? null; },
    get style(): NativeStyle {
      const a = anchor(), v = viewport(), p = placed();
      const anchorWidth = active() && a && v ? px(snap(a.width, v.scale)) : undefined;
      if (!p) {
        return {
          position: "fixed",
          visibility: "hidden",
          ...(anchorWidth ? { "--zui-popper-anchor-width": anchorWidth } : {}),
        };
      }
      const x = snap(p.origin.x, p.scale), y = snap(p.origin.y, p.scale);
      const anchorX = Math.max(0, Math.min(p.anchor.x + p.anchor.width / 2 - x, Math.max(0, p.floating.width)));
      const anchorY = Math.max(0, Math.min(p.anchor.y + p.anchor.height / 2 - y, Math.max(0, p.floating.height)));
      return {
        position: "fixed",
        left: px(x),
        top: px(y),
        ...(anchorWidth ? { "--zui-popper-anchor-width": anchorWidth } : {}),
        "--zui-popper-available-width": px(Math.floor(p.available.width)),
        "--zui-popper-available-height": px(Math.floor(p.available.height)),
        "--zui-popper-anchor-x": px(snap(anchorX, p.scale)),
        "--zui-popper-anchor-y": px(snap(anchorY, p.scale)),
      };
    },
  }, true);
  props.ref?.(node);
  slot(node, () => props.children);
  return node;
}
