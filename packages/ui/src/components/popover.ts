import { useContext } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeElementProps, NativeNode } from "../types.js";
import type { ButtonProps } from "./button.js";
import {
  anchoredSurface,
  type AnchoredContentProps,
  OverlayContext,
  OverlayRoot,
  type OverlayRootProps,
  overlayClose,
  overlayDescription,
  overlayTitle,
  overlayTrigger,
} from "./overlay.js";
import { part, useStylesheet } from "./shared.js";
import { sheet } from "./generated/PopoverStyle.js";

export type PopoverProps = OverlayRootProps;
export type PopoverTriggerProps = ButtonProps;
export type PopoverCloseProps = ButtonProps;
export type PopoverHeaderProps = NativeElementProps;
export type PopoverTitleProps = NativeElementProps;
export type PopoverDescriptionProps = NativeElementProps;
export type PopoverContentProps = AnchoredContentProps;

export function Popover(props: PopoverProps = {}): NativeChild {
  return OverlayRoot(props);
}

export function PopoverTrigger(props: PopoverTriggerProps = {}): NativeNode {
  return overlayTrigger("dialog", props);
}

export function PopoverClose(props: PopoverCloseProps = {}): NativeNode {
  return overlayClose(props);
}

export const PopoverHeader = part("column", sheet, "zui-popover__header");

export function PopoverTitle(props: PopoverTitleProps = {}): NativeNode {
  return overlayTitle("box", sheet, "zui-popover__title", props);
}

export function PopoverDescription(props: PopoverDescriptionProps = {}): NativeNode {
  return overlayDescription(sheet, "zui-popover__description", props);
}

export function PopoverContent(props: PopoverContentProps = {}): NativeChild {
  useStylesheet(sheet);
  const context = useContext(OverlayContext);
  return anchoredSurface(
    context,
    mergeProps(
      {
        get ariaLabelledBy() {
          return context?.title.node() ?? null;
        },
        get ariaDescribedBy() {
          return context?.description.node() ?? null;
        },
      },
      props,
    ),
    { class: `${sheet.className} zui-popover`, role: "dialog", trap: true },
  );
}
