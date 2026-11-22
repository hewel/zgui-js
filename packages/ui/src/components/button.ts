import { omit } from "#solid";
import { createElement, mergeProps, setProp, spread } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, isStaticProps, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/ButtonStyle.js";

export type ButtonVariant = "default" | "secondary" | "destructive" | "outline" | "ghost" | "link";
export type ButtonSize = "xs" | "sm" | "md" | "lg" | "icon" | "icon-xs" | "icon-sm" | "icon-lg";
export interface ButtonProps extends NativeElementProps {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

const SIZE_CLASSES: Record<ButtonSize, string> = {
  xs: "zui-button--xs",
  sm: "zui-button--sm",
  md: "",
  lg: "zui-button--lg",
  icon: "zui-button--icon",
  "icon-xs": "zui-button--icon-xs",
  "icon-sm": "zui-button--icon-sm",
  "icon-lg": "zui-button--icon-lg",
};

export function Button(props: ButtonProps = {}): NativeNode {
  useStylesheet(sheet);
  if (isStaticProps(props) && !("__zguiClass" in props)) {
    const node = createElement("control");
    const variant = props.variant ?? "default";
    const size = props.size ?? "md";
    setProp(node, "tabIndex", props.tabIndex ?? 0);
    setProp(node, "role", props.role ?? "button");
    setProp(node, "data-variant", (props as Record<string, unknown>)["data-variant"] ?? variant);
    setProp(node, "data-size", (props as Record<string, unknown>)["data-size"] ?? size);
    setProp(
      node,
      "__zguiClass",
      classes(sheet.className, "zui-button", `zui-button--${variant}`, SIZE_CLASSES[size]),
    );
    if ("variant" in props || "size" in props) {
      const copy: Record<string, unknown> = {};
      for (const key of Object.keys(props)) {
        if (key === "variant" || key === "size" || key === "children") continue;
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
  const rest = omit(props, "variant", "size");
  return element(
    "control",
    mergeProps(
      rest,
      withDefaults(
        {
          tabIndex: 0,
          role: "button" as const,
          get "data-variant"() {
            return props.variant ?? "default";
          },
          get "data-size"() {
            return props.size ?? "md";
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(
            sheet.className,
            "zui-button",
            `zui-button--${props.variant ?? "default"}`,
            SIZE_CLASSES[props.size ?? "md"],
          );
        },
      },
    ),
  );
}
