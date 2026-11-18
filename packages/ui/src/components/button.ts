import { splitProps } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, useStylesheet } from "./shared.js";
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
  const [own, rest] = splitProps(props, ["variant", "size"]);
  return element("control", mergeProps(
    {
      tabIndex: 0,
      role: "button" as const,
      get "data-variant"() { return own.variant ?? "default"; },
      get "data-size"() { return own.size ?? "md"; },
    },
    rest,
    {
      get __zguiClass() {
        return classes(sheet.className, "zui-button",
        `zui-button--${own.variant ?? "default"}`,
        SIZE_CLASSES[own.size ?? "md"], );
      },
    },
  ));
}
