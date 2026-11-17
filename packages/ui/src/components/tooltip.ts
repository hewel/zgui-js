import { Show, createContext, splitProps, useContext } from "#solid";
import { memo, mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeElementProps, NativeFocusEvent, NativeKeyEvent, NativeNode, NativePointerBoundaryEvent } from "../types.js";
import { anchoredSurface } from "./anchored.js";
import type { PopperPlacement } from "./popper.js";
import { HoverContext, HoverRoot, createHandoff } from "./hover.js";
import type { HoverProps } from "./hover.js";
import { callHandler } from "./state.js";
import { classes, element, forbidChildren, provide, useStylesheet } from "./shared.js";
import { sheet } from "./generated/TooltipStyle.js";
export type TooltipProps = HoverProps;
export interface TooltipProviderProps { delay?: number; closeDelay?: number; children?: NativeChild; }
export type TooltipTriggerProps = NativeElementProps;
export interface TooltipArrowProps extends Omit<NativeElementProps, "children"> { children?: never; }
export interface TooltipContentProps extends NativeElementProps { placement?: PopperPlacement; offset?: number; arrow?: boolean; }
const Group = createContext<{ delay: () => number; closeDelay: () => number; handoff: ReturnType<typeof createHandoff> }>();
const defaultOffset = 10 * Math.SQRT2 / 2 - 1 + 4;
export function TooltipProvider(props: TooltipProviderProps = {}): NativeChild {
  const value = { delay: () => props.delay ?? 0, closeDelay: () => props.closeDelay ?? 150, handoff: createHandoff() };
  return provide(Group, value, props);
}
export function Tooltip(props: TooltipProps = {}): NativeChild {
  const group = useContext(Group);
  return HoverRoot(props, { delay: () => group?.delay() ?? 0, closeDelay: () => group?.closeDelay() ?? 150, handoff: group?.handoff });
}
export function TooltipTrigger(props: TooltipTriggerProps = {}): NativeNode {
  useStylesheet(sheet);
  const intent = useContext(HoverContext), state = intent?.state;
  const [own, rest] = splitProps(props, ["ref", "onPointerEnter", "onPointerLeave", "onFocus", "onBlur", "onKeyDown"]);
  return element("box", mergeProps({
    get "data-state"() { return state?.open() ? "open" : "closed"; },
    get ariaDescribedBy() { return state?.content.node() ?? null; },
  }, rest, {
    get class() { return classes("zui-tooltip__trigger", rest.class); },
    ref(node: NativeNode) { state?.trigger.ref(node); if (typeof own.ref === "function") own.ref(node); },
    onPointerEnter(event: NativePointerBoundaryEvent) { if (callHandler(event, own.onPointerEnter)) intent?.enter(); },
    onPointerLeave(event: NativePointerBoundaryEvent) { if (callHandler(event, own.onPointerLeave)) intent?.leave(); },
    onFocus(event: NativeFocusEvent) { if (callHandler(event, own.onFocus) && event.cause === "keyboard") intent?.openNow(); },
    onBlur(event: NativeFocusEvent) { if (callHandler(event, own.onBlur)) intent?.closeNow(); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, own.onKeyDown) && event.character == null && event.key === "Escape" && state?.open()) { event.stopPropagation(); intent?.closeNow(); }
    },
  }));
}
export function TooltipArrow(props: TooltipArrowProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("TooltipArrow", props);
  return element("box", mergeProps({ ariaHidden: true }, props, {
    get class() { return classes("zui-tooltip__arrow", props.class); },
  }));
}
export function TooltipContent(props: TooltipContentProps = {}): NativeChild {
  useStylesheet(sheet);
  const intent = useContext(HoverContext);
  const [own, rest] = splitProps(props, ["children", "placement", "offset", "arrow", "onPointerEnter", "onPointerLeave"]);
  return anchoredSurface(intent?.state, mergeProps({
    dismissOnOutsidePress: false,
    get "data-instant"() { return intent?.instant() ? "" : null; },
  }, rest, {
    get placement(): PopperPlacement { return own.placement ?? { side: "top", align: "center" }; },
    get offset() { const offset = own.offset ?? defaultOffset; return (own.arrow ?? true) ? Math.max(offset, defaultOffset) : offset; },
    onPointerEnter(event: NativePointerBoundaryEvent) { if (callHandler(event, own.onPointerEnter)) intent?.enter(); },
    onPointerLeave(event: NativePointerBoundaryEvent) { if (callHandler(event, own.onPointerLeave)) intent?.leave(); },
    get children() {
      const content = memo(() => own.children);
      return [content, Show({ get when() { return own.arrow ?? true; }, get children() { return TooltipArrow(); } })];
    },
  }), { class: `${sheet.className} zui-tooltip`, role: "tooltip", onDismiss: () => intent?.closeNow() });
}
