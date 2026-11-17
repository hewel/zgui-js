import { Show, createEffect, splitProps, untrack, useContext } from "#solid";
import { createElement, memo, mergeProps, spread } from "../renderer/index.js";
import { lockScroll, registerDismissLayer, trapFocus } from "../nodes.js";
import type { NativeAnimationEvent, NativeChild, NativeElementProps, NativeEvent, NativeNode, NativeTransitionEvent } from "../types.js";
import { Button } from "./button.js";
import type { ButtonProps } from "./button.js";
import { Icon, CROSS } from "./icons.js";
import { OverlayContext, OverlayRoot, overlayPortal, raiseSurface } from "./overlay.js";
import type { OverlayRootProps } from "./overlay.js";
import { createPresence } from "./presence.js";
import { callHandler } from "./state.js";
import { classes, element, forbidChildren, part, slot, useStylesheet } from "./shared.js";
import { sheet } from "./generated/DialogStyle.js";
import { sheet as overlaySheet } from "./generated/OverlayStyle.js";

export type DialogProps = OverlayRootProps;
export type DialogTriggerProps = ButtonProps;
export type DialogCloseProps = ButtonProps;
export type DialogHeaderProps = NativeElementProps;
export type DialogTitleProps = NativeElementProps;
export type DialogDescriptionProps = NativeElementProps;
export interface DialogContentProps extends NativeElementProps {
  dismissOnOutsidePress?: boolean;
  dismissOnEscape?: boolean;
  dismissControl?: boolean;
}
export interface DialogFooterProps extends NativeElementProps { closeControl?: boolean; }
export interface DialogDismissProps extends Omit<NativeElementProps, "children"> { children?: never; }

export function Dialog(props: DialogProps = {}): NativeChild { return OverlayRoot(props); }

export function DialogTrigger(props: DialogTriggerProps = {}): NativeNode {
  const context = useContext(OverlayContext);
  const [own, rest] = splitProps(props, ["onClick", "ref"]);
  return Button(mergeProps({
    get expanded() { return context?.open(); },
    get ariaHasPopup() { return context ? "dialog" as const : null; },
    get ariaControls() { return context?.content.node() ?? null; },
    get "data-state"() { return context ? context.open() ? "open" : "closed" : null; },
  }, rest, {
    ref(node: NativeNode) { context?.trigger.ref(node); if (typeof own.ref === "function") own.ref(node); },
    onClick(event: NativeEvent) {
      if (callHandler(event, own.onClick) && !props.disabled) context?.setOpen(!context.open());
    },
  }));
}

export function DialogClose(props: DialogCloseProps = {}): NativeNode {
  const context = useContext(OverlayContext);
  const [own, rest] = splitProps(props, ["onClick"]);
  return Button(mergeProps({ variant: "ghost" as const }, rest, {
    onClick(event: NativeEvent) {
      if (callHandler(event, own.onClick) && !props.disabled) context?.setOpen(false);
    },
  }));
}

export function DialogDismiss(props: DialogDismissProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("DialogDismiss", props);
  const context = useContext(OverlayContext);
  const [own, rest] = splitProps(props, ["onClick"]);
  return element("control", mergeProps({ tabIndex: 0, role: "button" as const, ariaLabel: "Close" }, rest, {
    get class() { return classes("zui-dialog__dismiss", rest.class); },
    onClick(event: NativeEvent) {
      if (callHandler(event, own.onClick) && !props.disabled) context?.setOpen(false);
    },
    get children() { return Icon({ icon: CROSS }); },
  }));
}

export const DialogHeader = part("column", sheet, "zui-dialog__header");

export function DialogTitle(props: DialogTitleProps = {}): NativeNode {
  useStylesheet(sheet);
  const context = useContext(OverlayContext);
  const [own, rest] = splitProps(props, ["ref"]);
  return element("label", mergeProps(rest, {
    ref(node: NativeNode) { context?.title.ref(node); if (typeof own.ref === "function") own.ref(node); },
    get class() { return classes("zui-dialog__title", rest.class); },
  }));
}

export function DialogDescription(props: DialogDescriptionProps = {}): NativeNode {
  useStylesheet(sheet);
  const context = useContext(OverlayContext);
  const [own, rest] = splitProps(props, ["ref"]);
  return element("box", mergeProps(rest, {
    ref(node: NativeNode) { context?.description.ref(node); if (typeof own.ref === "function") own.ref(node); },
    get class() { return classes("zui-dialog__description", rest.class); },
  }));
}

export function DialogFooter(props: DialogFooterProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["children", "closeControl"]);
  const content = memo(() => own.children);
  const close = memo(() => own.closeControl ? DialogClose({ variant: "outline", children: "Close" }) : null);
  return element("row", mergeProps(rest, {
    get class() { return classes("zui-dialog__footer", rest.class); },
    get children() { return [content, close]; },
  }));
}

export function DialogContent(props: DialogContentProps = {}): NativeChild {
  useStylesheet(sheet);
  useStylesheet(overlaySheet);
  const context = useContext(OverlayContext);
  const [own, rest] = splitProps(props, ["children", "dismissOnOutsidePress", "dismissOnEscape", "dismissControl",
    "onAnimationEnd", "onAnimationCancel", "onTransitionEnd", "onTransitionCancel"]);
  const presence = createPresence(() => context?.open() ?? false);
  const elevation = raiseSurface("modal");
  const holder = element("box", { style: { display: "none" } });
  slot(holder, Show({ get when() { return presence.mounted(); }, get children() {
    return overlayPortal(elevation, () => {
      const state = context!;
      const surface = createElement("box");
      state.content.ref(surface);
      presence.ref(surface);
      trapFocus(surface);
      const options = () => ({
        layer: elevation.layer,
        dismissOnOutsidePress: own.dismissOnOutsidePress ?? true,
        dismissOnEscape: own.dismissOnEscape ?? true,
        exclude: state.trigger.node(), leaving: !state.open(),
      });
      const dismiss = registerDismissLayer(surface, () => state.setOpen(false), untrack(options));
      createEffect(() => dismiss.update(options()));
      createEffect(() => { if (state.open()) lockScroll(surface); });
      spread(surface, mergeProps({
        tabIndex: -1, role: "dialog" as const, ariaModal: true,
        get ariaLabelledBy() { return state.title.node(); },
        get ariaDescribedBy() { return state.description.node(); },
        get ariaPopupFor() { return state.trigger.node(); },
        get "data-state"() { return presence.state(); },
      }, rest, {
        get class() { return classes("zui-surface", sheet.className, "zui-dialog", rest.class); },
        onAnimationEnd(event: NativeAnimationEvent) { own.onAnimationEnd?.(event); presence.onEnd(event); },
        onAnimationCancel(event: NativeAnimationEvent) { own.onAnimationCancel?.(event); presence.onEnd(event); },
        onTransitionEnd(event: NativeTransitionEvent) { own.onTransitionEnd?.(event); presence.onEnd(event); },
        onTransitionCancel(event: NativeTransitionEvent) { own.onTransitionCancel?.(event); presence.onEnd(event); },
      }), true);
      const content = memo(() => own.children);
      const close = memo(() => (own.dismissControl ?? true) ? DialogDismiss() : null);
      slot(surface, [content, close]);
      const scrim = element("box", {
        class: classes(overlaySheet.className, "zui-overlay-scrim"), ariaHidden: true,
        get "data-state"() { return presence.state(); },
      });
      return [scrim, surface];
    });
  } }));
  return holder;
}
