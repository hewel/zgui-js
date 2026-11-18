import { omit } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { append, classes, element, forbidChildren, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/SpinnerStyle.js";

export interface SpinnerProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  label?: string | null;
}

export function Spinner(props: SpinnerProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Spinner", props);
  const rest = omit(props, "label");
  const node = element("box", mergeProps(
    rest,
    withDefaults({
      role: "status" as const,
      ariaBusy: true,
      get ariaLabel() { return props.label ?? "Loading"; },
    }, rest),
    { get __zguiClass() { return classes(sheet.className, "zui-spinner"); } },
  ));
  append(node, element("box", { class: "zui-spinner__mark" }));
  return node;
}
