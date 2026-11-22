import { omit } from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, forbidChildren, useStylesheet, withDefaults } from "./shared.js";
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
  const rest = omit(props, "orientation", "decorative");
  const node = createElement("box");
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          get role() {
            return (props.decorative ?? true)
              ? ("genericContainer" as const)
              : ("splitter" as const);
          },
          get ariaHidden() {
            return (props.decorative ?? true) || null;
          },
          get orientation() {
            return (props.decorative ?? true) ? null : (props.orientation ?? "horizontal");
          },
          get "data-orientation"() {
            return props.orientation ?? "horizontal";
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(
            sheet.className,
            "zui-separator",
            `zui-separator--${props.orientation ?? "horizontal"}`,
          );
        },
      },
    ),
    true,
  );
  return node;
}
