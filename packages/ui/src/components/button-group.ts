import { splitProps } from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, forbidChildren, part, useStylesheet } from "./shared.js";
import { sheet } from "./generated/ButtonGroupStyle.js";

export type ButtonGroupOrientation = "horizontal" | "vertical";
export interface ButtonGroupProps extends Omit<NativeElementProps, "orientation"> {
  orientation?: ButtonGroupOrientation;
}
export type ButtonGroupTextProps = NativeElementProps;
export interface ButtonGroupSeparatorProps extends Omit<NativeElementProps, "children"> {
  children?: never;
}

export function ButtonGroup(props: ButtonGroupProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["orientation"]);
  return element("box", mergeProps(
    {
      role: "group" as const,
      get "data-orientation"() { return own.orientation ?? "horizontal"; },
    },
    rest,
    {
      get __zguiClass() {
        return classes(sheet.className, "zui-button-group",
        (own.orientation ?? "horizontal") === "vertical" && "zui-button-group--vertical", );
      },
    },
  ));
}

export const ButtonGroupText = part("box", sheet, "zui-button-group__text");

export function ButtonGroupSeparator(props: ButtonGroupSeparatorProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("ButtonGroupSeparator", props);
  const node = createElement("box");
  spread(node, mergeProps(
    { role: "genericContainer" as const, ariaHidden: true },
    props,
    { get __zguiClass() { return classes("zui-button-group__separator"); } },
  ), true);
  return node;
}
