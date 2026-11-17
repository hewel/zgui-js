import { onCleanup, splitProps, useContext } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeEvent, NativeKeyEvent, NativeNode, NativePointerDownEvent } from "../types.js";
import { Button } from "./button.js";
import type { ButtonProps } from "./button.js";
import type { OverlayRootProps } from "./overlay.js";
import { MenuContent, MenuContext, menuRoot } from "./menu.js";
import type { MenuContentProps } from "./menu.js";
import { callHandler } from "./state.js";
export type DropdownMenuProps = OverlayRootProps;
export type DropdownMenuTriggerProps = ButtonProps;
export type DropdownMenuContentProps = MenuContentProps;
export function DropdownMenu(props: DropdownMenuProps = {}): NativeChild { return menuRoot(props); }
export function DropdownMenuTrigger(props: DropdownMenuTriggerProps = {}): NativeNode {
  const state = useContext(MenuContext)?.state;
  const [own, rest] = splitProps(props, ["ref", "onClick", "onPointerDown", "onKeyDown"]);
  let alive = true;
  onCleanup(() => { alive = false; });
  return Button(mergeProps({
    ariaHasPopup: "menu",
    get expanded() { return state?.open() ?? false; },
    get ariaControls() { return state?.content.node() ?? null; },
    get "data-state"() { return state?.open() ? "open" : "closed"; },
  }, rest, {
    ref(node: NativeNode) { state?.trigger.ref(node); if (typeof own.ref === "function") own.ref(node); },
    onClick(event: NativeEvent) { if (callHandler(event, own.onClick) && alive && !rest.disabled) state?.setOpen(!state.open()); },
    onPointerDown(event: NativePointerDownEvent) {
      if (callHandler(event, own.onPointerDown) && alive && !rest.disabled && event.button === "primary") event.activateNow();
    },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, own.onKeyDown) && alive && !rest.disabled && state && !state.open()
        && event.character == null && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
        state.setOpen(true); event.preventDefault(); event.stopPropagation();
      }
    },
  }));
}
export function DropdownMenuContent(props: DropdownMenuContentProps = {}): NativeChild {
  return MenuContent(mergeProps({ placement: { side: "bottom", align: "start" } as const, offset: 4 }, props));
}
