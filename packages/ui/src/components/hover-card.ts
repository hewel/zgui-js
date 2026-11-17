import { splitProps, useContext } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeElementProps, NativeFocusEvent, NativeKeyEvent, NativeNode, NativePointerBoundaryEvent } from "../types.js";
import { anchoredSurface } from "./anchored.js";
import type { PopperPlacement } from "./popper.js";
import { HoverContext, HoverRoot } from "./hover.js";
import type { HoverProps } from "./hover.js";
import { callHandler } from "./state.js";
import { classes, element, useStylesheet } from "./shared.js";
import { sheet } from "./generated/HoverCardStyle.js";
export type HoverCardProps = HoverProps;
export type HoverCardTriggerProps = NativeElementProps;
export interface HoverCardContentProps extends NativeElementProps { placement?: PopperPlacement; offset?: number; }
export function HoverCard(props: HoverCardProps = {}): NativeChild {
  return HoverRoot(props, { delay: () => 700, closeDelay: () => 300 });
}
export function HoverCardTrigger(props: HoverCardTriggerProps = {}): NativeNode {
  useStylesheet(sheet);
  const intent = useContext(HoverContext), state = intent?.state;
  const [own, rest] = splitProps(props, ["ref", "onPointerEnter", "onPointerLeave", "onFocus", "onBlur", "onKeyDown"]);
  return element("box", mergeProps({
    tabIndex: 0, ariaHasPopup: "dialog" as const,
    get expanded() { return state?.open() ?? false; },
    get ariaControls() { return state?.content.node() ?? null; },
    get "data-state"() { return state?.open() ? "open" : "closed"; },
  }, rest, {
    get class() { return classes("zui-hover-card__trigger", rest.class); },
    ref(node: NativeNode) { state?.trigger.ref(node); if (typeof own.ref === "function") own.ref(node); },
    onPointerEnter(event: NativePointerBoundaryEvent) { if (callHandler(event, own.onPointerEnter)) intent?.enter(); },
    onPointerLeave(event: NativePointerBoundaryEvent) { if (callHandler(event, own.onPointerLeave)) intent?.leave(); },
    onFocus(event: NativeFocusEvent) { if (callHandler(event, own.onFocus)) { intent?.closeNow(); intent?.state.setOpen(true); } },
    onBlur(event: NativeFocusEvent) { if (callHandler(event, own.onBlur)) intent?.closeNow(); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, own.onKeyDown) && event.character == null && event.key === "Escape" && state?.open()) { event.stopPropagation(); intent?.closeNow(); }
    },
  }));
}
export function HoverCardContent(props: HoverCardContentProps = {}): NativeChild {
  useStylesheet(sheet);
  const intent = useContext(HoverContext);
  const [own, rest] = splitProps(props, ["onPointerEnter", "onPointerLeave"]);
  return anchoredSurface(intent?.state, mergeProps({ dismissOnOutsidePress: false }, rest, {
    onPointerEnter(event: NativePointerBoundaryEvent) { if (callHandler(event, own.onPointerEnter)) intent?.enter(); },
    onPointerLeave(event: NativePointerBoundaryEvent) { if (callHandler(event, own.onPointerLeave)) intent?.leave(); },
  }), { class: `${sheet.className} zui-hover-card`, role: "dialog", onDismiss: () => intent?.closeNow() });
}
