import { splitProps, useContext } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeElementProps, NativeNode } from "../types.js";
import { OverlayContext, OverlayRoot } from "./overlay.js";
import type { OverlayRootProps } from "./overlay.js";
import { anchoredSurface } from "./anchored.js";
import type { AnchoredContentProps } from "./anchored.js";
import { classes, element, part, useStylesheet } from "./shared.js";
import { sheet } from "./generated/PopoverStyle.js";

export { DialogTrigger as PopoverTrigger, DialogClose as PopoverClose } from "./dialog.js";
export type { DialogTriggerProps as PopoverTriggerProps, DialogCloseProps as PopoverCloseProps } from "./dialog.js";
export type PopoverProps = OverlayRootProps;
export type PopoverHeaderProps = NativeElementProps;
export type PopoverTitleProps = NativeElementProps;
export type PopoverDescriptionProps = NativeElementProps;
export type PopoverContentProps = AnchoredContentProps;
export function Popover(props: PopoverProps = {}): NativeChild { return OverlayRoot(props); }
export const PopoverHeader = part("column", sheet, "zui-popover__header");
export function PopoverTitle(props: PopoverTitleProps = {}): NativeNode {
  useStylesheet(sheet);
  const context = useContext(OverlayContext);
  const [own, rest] = splitProps(props, ["ref"]);
  return element("box", mergeProps(rest, {
    ref(node: NativeNode) { context?.title.ref(node); if (typeof own.ref === "function") own.ref(node); },
    get class() { return classes("zui-popover__title", rest.class); },
  }));
}
export function PopoverDescription(props: PopoverDescriptionProps = {}): NativeNode {
  useStylesheet(sheet);
  const context = useContext(OverlayContext);
  const [own, rest] = splitProps(props, ["ref"]);
  return element("box", mergeProps(rest, {
    ref(node: NativeNode) { context?.description.ref(node); if (typeof own.ref === "function") own.ref(node); },
    get class() { return classes("zui-popover__description", rest.class); },
  }));
}
export function PopoverContent(props: PopoverContentProps = {}): NativeChild {
  useStylesheet(sheet);
  const context = useContext(OverlayContext);
  return anchoredSurface(context, mergeProps({
    get ariaLabelledBy() { return context?.title.node() ?? null; },
    get ariaDescribedBy() { return context?.description.node() ?? null; },
  }, props), { class: `${sheet.className} zui-popover`, role: "dialog", trap: true });
}
