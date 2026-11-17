import { onCleanup, splitProps, useContext } from "#solid";
import { insert, memo, mergeProps } from "../renderer/index.js";
import { focus, getWindowRect, observeGeometry } from "../nodes.js";
import type { NativeChild, NativeElementProps, NativeKeyEvent, NativeNode, NativePointerBoundaryEvent, NativePointerEvent, NativePoint, NativeWindowRect } from "../types.js";
import { OverlayContext, createOverlayState } from "./overlay.js";
import type { OverlayRootProps, OverlayState } from "./overlay.js";
import { createDelayed } from "./delay.js";
import { callHandler } from "./state.js";
import { classes } from "./shared.js";
import { Icon } from "./icons.js";
import { CHEVRON_RIGHT } from "./generated/icons.js";
import { menuRow, menuSurface } from "./menu.js";
import type { MenuContentProps } from "./menu.js";

export type MenuSubProps = OverlayRootProps;
export interface MenuSubTriggerProps extends NativeElementProps { inset?: boolean; }
export type MenuSubContentProps = MenuContentProps;
const subClosings = new WeakMap<OverlayState, ReturnType<typeof createDelayed>>();
const useSub = () => {
  const state = useContext(OverlayContext);
  const closing = state && subClosings.get(state);
  return closing ? { state, closing } : undefined;
};

export function MenuSub(props: MenuSubProps = {}): NativeChild {
  const state = createOverlayState(props);
  const closing = createDelayed();
  subClosings.set(state, closing);
  onCleanup(closing.cancel);
  return OverlayContext.Provider({ value: state, get children() { return props.children; } });
}

export function headingToward(from: NativePoint, to: NativePoint, target: NativeWindowRect): boolean {
  const near = from.x <= target.x ? target.x : target.x + target.width;
  const top = { x: near, y: target.y }, bottom = { x: near, y: target.y + target.height };
  const cross = (a: NativePoint, b: NativePoint) =>
    (b.x - a.x) * (to.y - a.y) - (b.y - a.y) * (to.x - a.x);
  const first = cross(from, top), second = cross(top, bottom), third = cross(bottom, from);
  return !((first < 0 || second < 0 || third < 0) && (first > 0 || second > 0 || third > 0));
}

export function MenuSubTrigger(props: MenuSubTriggerProps = {}): NativeNode {
  const sub = useSub(), state = sub?.state;
  const opening = createDelayed(), closing = sub?.closing ?? createDelayed();
  const [own, rest] = splitProps(props, ["children", "inset", "ref", "onPointerMove", "onKeyDown"]);
  let alive = true, scale = 1;
  let last: NativePoint = { x: 0, y: 0 };
  onCleanup(() => { alive = false; opening.cancel(); if (!sub) closing.cancel(); });
  const device = (point: NativePoint): NativePoint => ({ x: Math.fround(point.x * scale), y: Math.fround(point.y * scale) });
  const node = menuRow(mergeProps({
    ariaHasPopup: "menu",
    get expanded() { return state?.open() ?? false; },
    get ariaControls() { return state?.content.node() ?? null; },
    get "data-state"() { return state?.open() ? "open" : "closed"; },
    get "data-inset"() { return own.inset ? "true" : null; },
  }, rest, {
    ref(node: NativeNode) { state?.trigger.ref(node); if (typeof own.ref === "function") own.ref(node); },
    onPointerMove(event: NativePointerEvent) {
      if (callHandler(event, own.onPointerMove) && alive) last = device(event.position);
    },
    onKeyDown(event: NativeKeyEvent) {
      if (!callHandler(event, own.onKeyDown) || !alive || rest.disabled || !state || event.character != null) return;
      if (event.key === "ArrowRight" || event.key === "Enter" || event.key === " ") {
        event.preventDefault(); event.stopPropagation(); state.setOpen(true);
      }
    },
  }), {
    role: "menuItem", class: () => classes(own.inset && "zui-menu__item--inset"), activateOnPress: false,
    select() { opening.cancel(); closing.cancel(); state?.setOpen(true); },
    enter(activate) {
      closing.cancel(); activate();
      if (alive && state) opening.after(120, () => { if (alive) state.setOpen(true); });
    },
    leave(event, defocus) {
      opening.cancel();
      if (!state?.open()) { defocus(); return; }
      const surface = state.content.node();
      const rect = surface ? getWindowRect(surface) : null;
      if (rect && headingToward(last, device(event.position), {
        ...device(rect), width: Math.fround(rect.width * scale), height: Math.fround(rect.height * scale),
      })) return;
      closing.after(180, () => state.setOpen(false));
    },
  });
  observeGeometry(node, "viewport", viewport => { scale = viewport.scale; });
  insert(node, memo(() => own.children), null);
  insert(node, Icon({ icon: CHEVRON_RIGHT, class: "zui-menu__chevron" }), null);
  return node;
}

export function MenuSubContent(props: MenuSubContentProps = {}): NativeChild {
  const sub = useSub(), state = sub?.state;
  const [own, rest] = splitProps(props, ["onPointerEnter", "onKeyDown"]);
  let alive = true;
  onCleanup(() => { alive = false; });
  return menuSurface(state, mergeProps({ placement: { side: "right", align: "start" } as const, offset: -4 }, rest, {
    get class() { return classes("zui-menu--sub", rest.class); },
    onPointerEnter(event: NativePointerBoundaryEvent) {
      if (callHandler(event, own.onPointerEnter) && alive) sub?.closing.cancel();
    },
    onKeyDown(event: NativeKeyEvent) {
      if (!callHandler(event, own.onKeyDown) || !alive || !state || event.character != null || event.key !== "ArrowLeft") return;
      event.preventDefault(); event.stopPropagation(); state.setOpen(false);
      const trigger = state.trigger.node();
      if (alive && trigger) focus(trigger);
    },
  }));
}
