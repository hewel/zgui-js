import { splitProps, useContext } from "#solid";
import { memo, mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeElementProps, NativeEvent, NativeNode } from "../types.js";
import type { ButtonProps } from "./button.js";
import { Icon, CROSS } from "./icons.js";
import { OverlayContext, OverlayRoot, modalSurface, overlayClose, overlayDescription, overlayTitle, overlayTrigger } from "./overlay.js";
import type { OverlayRootProps } from "./overlay.js";
import { callHandler } from "./state.js";
import { classes, element, forbidChildren, part, useStylesheet } from "./shared.js";
import { sheet } from "./generated/DialogStyle.js";

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
  return overlayTrigger("dialog", props);
}

export function DialogClose(props: DialogCloseProps = {}): NativeNode {
  return overlayClose(props);
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
  return overlayTitle("label", sheet, "zui-dialog__title", props);
}

export function DialogDescription(props: DialogDescriptionProps = {}): NativeNode {
  return overlayDescription(sheet, "zui-dialog__description", props);
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
  const context = useContext(OverlayContext);
  const [own, rest] = splitProps(props, ["children", "dismissControl"]);
  return modalSurface(context, mergeProps({
    get ariaLabelledBy() { return context?.title.node() ?? null; },
    get ariaDescribedBy() { return context?.description.node() ?? null; },
  }, rest), {
    class: `${sheet.className} zui-dialog`,
    role: "dialog",
    children() {
      const content = memo(() => own.children);
      const close = memo(() => (own.dismissControl ?? true) ? DialogDismiss() : null);
      return [content, close];
    },
  });
}
