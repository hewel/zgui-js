import { omit } from "#solid";
import { createElement, mergeProps, setProp, spread } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, isStaticProps, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/BadgeStyle.js";

export type BadgeVariant =
  "default" | "secondary" | "destructive" | "success" | "warning" | "outline" | "ghost" | "link";
export interface BadgeProps extends NativeElementProps {
  variant?: BadgeVariant;
}

export function Badge(props: BadgeProps = {}): NativeNode {
  useStylesheet(sheet);
  if (isStaticProps(props) && !("__zguiClass" in props)) {
    const node = createElement("box");
    const variant = props.variant ?? "default";
    setProp(node, "data-variant", (props as Record<string, unknown>)["data-variant"] ?? variant);
    setProp(node, "__zguiClass", classes(sheet.className, "zui-badge", `zui-badge--${variant}`));
    if ("variant" in props) {
      const copy: Record<string, unknown> = {};
      for (const key of Object.keys(props)) {
        if (key === "variant" || key === "children") continue;
        copy[key] = (props as Record<string, unknown>)[key];
      }
      const childrenDesc = Object.getOwnPropertyDescriptor(props, "children");
      if (childrenDesc) Object.defineProperty(copy, "children", childrenDesc);
      spread(node, copy as NativeElementProps);
    } else {
      spread(node, props);
    }
    return node;
  }
  const rest = omit(props, "variant");
  return element("box", mergeProps(
    rest,
    withDefaults({ get "data-variant"() { return props.variant ?? "default"; } }, rest),
    {
      get __zguiClass() {
        return classes(sheet.className, "zui-badge", `zui-badge--${props.variant ?? "default"}`);
      },
    },
  ));
}
