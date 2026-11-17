import { splitProps } from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import type { NativeNode, VectorProps } from "../types.js";
import { classes, forbidChildren, useStylesheet } from "./shared.js";
import { sheet } from "./generated/IconStyle.js";

export {
  ARROW_UP, ARROW_DOWN, ARROW_LEFT, ARROW_RIGHT,
  CHEVRON_UP, CHEVRON_DOWN, CHEVRON_LEFT, CHEVRON_RIGHT,
  CHECK, MINUS, PLUS, CROSS, DISC, DOT,
  ALERT_CIRCLE, ALERT_TRIANGLE, INFO, CHECK_CIRCLE, CROSS_CIRCLE,
  SEARCH, SPINNER, ELLIPSIS, PANEL_LEFT,
} from "./generated/icons.js";

export interface IconData {
  readonly name: string;
  readonly viewBox: number;
  readonly d: string;
}
export type IconSize = "sm" | "md" | "lg" | "xl";
export interface IconProps extends Omit<VectorProps, "d" | "viewBox"> {
  icon: IconData;
  size?: IconSize;
  label?: string | null;
}

const SIZE_CLASSES: Record<IconSize, string> = {
  sm: "zui-icon--sm",
  md: "",
  lg: "zui-icon--lg",
  xl: "zui-icon--xl",
};

export function Icon(props: IconProps): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Icon", props);
  const [own, rest] = splitProps(props, ["icon", "size", "label"]);
  const node = createElement("vector");
  spread(node, mergeProps(
    {
      get role() { return "image" as const; },
      get ariaHidden() { return (own.label ?? null) === null ? true : null; },
      get ariaLabel() { return own.label ?? null; },
      get d() { return own.icon.d; },
      get viewBox() { const side = own.icon.viewBox; return `0 0 ${side} ${side}`; },
      get "data-size"() { return own.size ?? "md"; },
      get "data-icon"() { return own.icon.name; },
    },
    rest,
    {
      get class() {
        return classes("zui-icon", SIZE_CLASSES[own.size ?? "md"], sheet.className, rest.class);
      },
    },
  ), true);
  return node;
}
