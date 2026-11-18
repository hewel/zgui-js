import { $PROXY, omit } from "#solid";
import { createElement, mergeProps, setProp, spread } from "../renderer/index.js";
import type { NativeNode, VectorProps } from "../types.js";
import { classes, forbidChildren, useStylesheet, withDefaults } from "./shared.js";
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
  if (
    !($PROXY in props) && !("__zguiClass" in props)
    && Object.getOwnPropertyDescriptor(props, "icon")?.get === undefined
    && Object.getOwnPropertyDescriptor(props, "size")?.get === undefined
    && Object.getOwnPropertyDescriptor(props, "label")?.get === undefined
  ) {
    const node = createElement("vector");
    const size = props.size ?? "md";
    const side = props.icon.viewBox;
    setProp(node, "role", "image");
    setProp(node, "ariaHidden", (props.label ?? null) === null ? true : null);
    setProp(node, "ariaLabel", props.label ?? null);
    setProp(node, "d", props.icon.d);
    setProp(node, "viewBox", `0 0 ${side} ${side}`);
    setProp(node, "data-size", size);
    setProp(node, "data-icon", props.icon.name);
    setProp(node, "__zguiClass", classes("zui-icon", SIZE_CLASSES[size], sheet.className));
    const keys = Object.keys(props);
    if (keys.length > 0) {
      let extra: Record<string, unknown> | undefined;
      for (let i = 0; i < keys.length; i++) {
        const key = keys[i];
        if (key === "icon" || key === "size" || key === "label") continue;
        const desc = Object.getOwnPropertyDescriptor(props, key);
        if (desc) {
          extra ??= {};
          Object.defineProperty(extra, key, desc);
        }
      }
      if (extra) spread(node, extra as unknown as VectorProps, true);
    }
    return node;
  }
  const rest = omit(props, "icon", "size", "label");
  const node = createElement("vector");
  spread(node, mergeProps(
    rest,
    withDefaults({
      get role() { return "image" as const; },
      get ariaHidden() { return (props.label ?? null) === null ? true : null; },
      get ariaLabel() { return props.label ?? null; },
      get d() { return props.icon.d; },
      get viewBox() { const side = props.icon.viewBox; return `0 0 ${side} ${side}`; },
      get "data-size"() { return props.size ?? "md"; },
      get "data-icon"() { return props.icon.name; },
    }, rest),
    {
      get __zguiClass() {
        return classes("zui-icon", SIZE_CLASSES[props.size ?? "md"], sheet.className);
      },
    },
  ), true);
  return node;
}
