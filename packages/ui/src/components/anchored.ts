import { Show, createEffect, splitProps, untrack } from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import { registerDismissLayer, trapFocus } from "../nodes.js";
import type { NativeAnimationEvent, NativeChild, NativeDismissReason, NativeElementProps, NativeRole, NativeTransitionEvent } from "../types.js";
import { overlayPortal, raiseSurface } from "./overlay.js";
import type { OverlayState } from "./overlay.js";
import { Popper } from "./popper.js";
import type { PopperPlacement } from "./popper.js";
import { createPresence } from "./presence.js";
import { classes, element, slot, useStylesheet } from "./shared.js";
import { sheet } from "./generated/OverlayStyle.js";
export interface AnchoredContentProps extends NativeElementProps {
  placement?: PopperPlacement;
  offset?: number;
  dismissOnOutsidePress?: boolean;
  dismissOnEscape?: boolean;
}
export function anchoredSurface(state: OverlayState | undefined, props: AnchoredContentProps, defaults: {
  class: string; role: NativeRole; trap?: boolean; surfaceClass?: boolean; onDismiss?: (reason: NativeDismissReason) => void;
  children?: () => NativeChild;
}): NativeChild {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["children", "placement", "offset", "dismissOnOutsidePress", "dismissOnEscape",
    "onAnimationEnd", "onAnimationCancel", "onTransitionEnd", "onTransitionCancel"]);
  const presence = createPresence(() => state?.open() ?? false);
  const elevation = raiseSurface("popover");
  const holder = element("box", { style: { display: "none" } });
  slot(holder, Show({ get when() { return presence.mounted(); }, get children() {
    return overlayPortal(elevation, () => {
      const context = state!;
      const surface = createElement("box");
      context.content.ref(surface);
      presence.ref(surface);
      if (defaults.trap) trapFocus(surface);
      const options = () => ({
        layer: elevation.layer,
        dismissOnOutsidePress: own.dismissOnOutsidePress ?? true,
        dismissOnEscape: own.dismissOnEscape ?? true,
        exclude: context.trigger.node(), leaving: !context.open(),
      });
      const dismiss = registerDismissLayer(surface, reason => {
        if (defaults.onDismiss) defaults.onDismiss(reason); else context.setOpen(false);
      }, untrack(options));
      createEffect(() => dismiss.update(options()));
      spread(surface, mergeProps({
        tabIndex: defaults.trap ? -1 : undefined, role: defaults.role, ariaModal: defaults.trap ?? false,
        get ariaPopupFor() { return context.trigger.node(); },
        get "data-state"() { return presence.state(); },
      }, rest, {
        get class() { return classes(defaults.surfaceClass === false ? null : "zui-surface", defaults.class, rest.class); },
        onAnimationEnd(event: NativeAnimationEvent) { own.onAnimationEnd?.(event); presence.onEnd(event); },
        onAnimationCancel(event: NativeAnimationEvent) { own.onAnimationCancel?.(event); presence.onEnd(event); },
        onTransitionEnd(event: NativeTransitionEvent) { own.onTransitionEnd?.(event); presence.onEnd(event); },
        onTransitionCancel(event: NativeTransitionEvent) { own.onTransitionCancel?.(event); presence.onEnd(event); },
      }), true);
      slot(surface, defaults.children ?? (() => own.children));
      return Popper({
        get anchor() { return context.trigger.node(); },
        get placement() { return own.placement; },
        get offset() { return own.offset; },
        class: "zui-overlay-positioner", children: surface,
      });
    });
  } }));
  return holder;
}
