import type { NativeChild, NativeElementProps, NativeNode } from "../types.js";
import type { PopperPlacement } from "./popper.js";
import { HoverRoot, hoverSurface, hoverTrigger } from "./hover.js";
import type { HoverProps } from "./hover.js";
import { sheet } from "./generated/HoverCardStyle.js";

export type HoverCardProps = HoverProps;
export type HoverCardTriggerProps = NativeElementProps;
export interface HoverCardContentProps extends NativeElementProps {
  placement?: PopperPlacement;
  offset?: number;
}

export function HoverCard(props: HoverCardProps = {}): NativeChild {
  return HoverRoot(props, { delay: () => 700, closeDelay: () => 300 });
}

export function HoverCardTrigger(props: HoverCardTriggerProps = {}): NativeNode {
  return hoverTrigger(
    {
      sheet,
      class: "zui-hover-card__trigger",
      focus: "always",
      attrs: state => ({
        tabIndex: 0,
        ariaHasPopup: "dialog" as const,
        get expanded() {
          return state?.open() ?? false;
        },
        get ariaControls() {
          return state?.content.node() ?? null;
        },
      }),
    },
    props,
  );
}

export function HoverCardContent(props: HoverCardContentProps = {}): NativeChild {
  return hoverSurface({ sheet, class: "zui-hover-card", role: "dialog" }, props);
}
