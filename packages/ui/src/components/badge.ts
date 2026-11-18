import { splitProps } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, useStylesheet } from "./shared.js";
import { sheet } from "./generated/BadgeStyle.js";

export type BadgeVariant =
  "default" | "secondary" | "destructive" | "success" | "warning" | "outline" | "ghost" | "link";
export interface BadgeProps extends NativeElementProps {
  variant?: BadgeVariant;
}

export function Badge(props: BadgeProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["variant"]);
  return element("box", mergeProps(
    { get "data-variant"() { return own.variant ?? "default"; } },
    rest,
    {
      get __zguiClass() {
        return classes(sheet.className, "zui-badge", `zui-badge--${own.variant ?? "default"}`);
      },
    },
  ));
}
