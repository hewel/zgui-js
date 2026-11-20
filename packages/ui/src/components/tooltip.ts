import { Show, createContext, omit, useContext } from "#solid";
import { memo, mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeElementProps, NativeNode } from "../types.js";
import type { PopperPlacement } from "./popper.js";
import { HoverRoot, createHandoff, hoverSurface, hoverTrigger } from "./hover.js";
import type { HoverHandoff, HoverProps } from "./hover.js";
import { classes, element, forbidChildren, provide, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/TooltipStyle.js";

export type TooltipProps = HoverProps;
export interface TooltipProviderProps { delay?: number; closeDelay?: number; children?: NativeChild; }
export type TooltipTriggerProps = NativeElementProps;
export interface TooltipArrowProps extends Omit<NativeElementProps, "children"> { children?: never; }
export interface TooltipContentProps extends NativeElementProps { placement?: PopperPlacement; offset?: number; arrow?: boolean; }
const Group = createContext<{ delay: () => number; closeDelay: () => number; handoff: HoverHandoff } | null>(null);
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
  return hoverTrigger({
    sheet,
    class: "zui-tooltip__trigger",
    focus: "keyboard",
    attrs: state => ({
      get ariaDescribedBy() { return state?.content.node() ?? null; },
    }),
  }, props);
}

export function TooltipArrow(props: TooltipArrowProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("TooltipArrow", props);
  return element("box", mergeProps(props, withDefaults({ ariaHidden: true }, props), {
    get __zguiClass() { return classes("zui-tooltip__arrow"); },
  }));
}

export function TooltipContent(props: TooltipContentProps = {}): NativeChild {
  const rest = omit(props, "children", "placement", "offset", "arrow");
  return hoverSurface({
    sheet,
    class: "zui-tooltip",
    role: "tooltip",
    attrs: intent => ({
      get "data-instant"() { return intent?.instant() ? "" : null; },
    }),
  }, mergeProps(rest, {
    get placement(): PopperPlacement { return props.placement ?? { side: "top", align: "center" }; },
    get offset() { const offset = props.offset ?? defaultOffset; return (props.arrow ?? true) ? Math.max(offset, defaultOffset) : offset; },
    get children() {
      const content = memo(() => props.children);
      return [content, Show({ get when() { return props.arrow ?? true; }, get children() { return TooltipArrow(); } })];
    },
  }));
}
