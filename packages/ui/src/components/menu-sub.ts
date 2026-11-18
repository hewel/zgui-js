import { onCleanup, omit, untrack } from "#solid";
import { memo, mergeProps } from "../renderer/index.js";
import { focus, getWindowRect, observeGeometry } from "../nodes.js";
import type { NativeChild, NativeElementProps, NativeKeyEvent, NativeNode, NativePointerBoundaryEvent, NativePointerEvent, NativePoint, NativeWindowRect } from "../types.js";
import type { OverlayRootProps } from "./overlay.js";
import { createDelayed } from "./delay.js";
import { callHandler } from "./state.js";
import { append, classes, slot, withDefaults } from "./shared.js";
import { Icon } from "./icons.js";
import { CHEVRON_RIGHT } from "./generated/icons.js";
import type { MenuContentProps } from "./menu.js";
import { menuRow, provideMenuSub, renderMenuSurface, useMenuSub } from "./menu-core.js";

export type MenuSubProps = OverlayRootProps;
export interface MenuSubTriggerProps extends NativeElementProps { inset?: boolean; }
export type MenuSubContentProps = MenuContentProps;

export function MenuSub(props: MenuSubProps = {}): NativeChild {
  return provideMenuSub(props);
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
  const sub = useMenuSub(), state = sub?.state;
  const opening = createDelayed(), closing = sub?.closing ?? createDelayed();
  const rest = omit(props, "children", "inset", "ref", "onPointerMove", "onKeyDown");
  let alive = true, scale = 1;
  let last: NativePoint = { x: 0, y: 0 };
  onCleanup(() => { alive = false; opening.cancel(); if (!sub) closing.cancel(); });
  const device = (point: NativePoint): NativePoint => ({ x: Math.fround(point.x * scale), y: Math.fround(point.y * scale) });
  const node = menuRow(mergeProps(rest, withDefaults({
    ariaHasPopup: "menu" as const,
    get expanded() { return state?.open() ?? false; },
    get ariaControls() { return state?.content.node() ?? null; },
    get "data-state"() { return state?.open() ? "open" : "closed"; },
    get "data-inset"() { return props.inset ? "true" : null; },
  }, rest), {
    ref(node: NativeNode) { state?.trigger.ref(node); if (typeof props.ref === "function") props.ref(node); },
    onPointerMove(event: NativePointerEvent) {
      if (callHandler(event, props.onPointerMove) && alive) last = device(event.position);
    },
    onKeyDown(event: NativeKeyEvent) {
      if (!callHandler(event, props.onKeyDown) || !alive || rest.disabled || !state || event.character != null) return;
      if (event.key === "ArrowRight" || event.key === "Enter" || event.key === " ") {
        event.preventDefault(); event.stopPropagation(); state.setOpen(true);
      }
    },
  }), {
    role: "menuItem", class: () => classes(props.inset && "zui-menu__item--inset"), activateOnPress: false,
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
  slot(node, memo(() => props.children));
  append(node, Icon({ icon: CHEVRON_RIGHT, class: "zui-menu__chevron" }));
  return node;
}

export function MenuSubContent(props: MenuSubContentProps = {}): NativeChild {
  const sub = useMenuSub(), state = sub?.state;
  const rest = omit(props, "onPointerEnter", "onKeyDown");
  let alive = true;
  onCleanup(() => { alive = false; });
  const closeToTrigger = () => {
    sub?.closing.cancel();
    state?.setOpen(false);
    const trigger = state ? untrack(state.trigger.node) : null;
    if (alive && trigger) focus(trigger);
  };
  return renderMenuSurface(state, mergeProps(rest, withDefaults({ placement: { side: "right", align: "start" } as const, offset: -4 }, rest), {
    get __zguiClass() { return classes("zui-menu--sub"); },
    onPointerEnter(event: NativePointerBoundaryEvent) {
      if (callHandler(event, props.onPointerEnter) && alive) sub?.closing.cancel();
    },
    onKeyDown(event: NativeKeyEvent) {
      if (!callHandler(event, props.onKeyDown) || !alive || !state || event.character != null || event.key !== "ArrowLeft") return;
      event.preventDefault(); event.stopPropagation();
      closeToTrigger();
    },
  }), {
    onDismiss(reason) {
      if (reason === "escape") closeToTrigger();
      else { sub?.closing.cancel(); state?.setOpen(false); }
    },
  });
}
