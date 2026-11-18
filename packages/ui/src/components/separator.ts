import { splitProps } from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, forbidChildren, useStylesheet } from "./shared.js";
import { sheet } from "./generated/SeparatorStyle.js";

export type SeparatorOrientation = "horizontal" | "vertical";
export interface SeparatorProps extends Omit<NativeElementProps, "children" | "orientation"> {
  children?: never;
  orientation?: SeparatorOrientation;
  decorative?: boolean;
}

export function Separator(props: SeparatorProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Separator", props);
  const [own, rest] = splitProps(props, ["orientation", "decorative"]);
  const node = createElement("box");
  spread(node, mergeProps(
    {
      get role() { return (own.decorative ?? true) ? "genericContainer" as const : "splitter" as const; },
      get ariaHidden() { return (own.decorative ?? true) || null; },
      get orientation() { return (own.decorative ?? true) ? null : (own.orientation ?? "horizontal"); },
      get "data-orientation"() { return own.orientation ?? "horizontal"; },
    },
    rest,
    {
      get __zguiClass() {
        return classes(sheet.className, "zui-separator", `zui-separator--${own.orientation ?? "horizontal"}`);
      },
    },
  ), true);
  return node;
}
