import { omit } from "#solid";
import { createElement, mergeProps, setProp } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { append, classes, element, forbidChildren, isStaticProps, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/SpinnerStyle.js";

export interface SpinnerProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  label?: string | null;
}

export function Spinner(props: SpinnerProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Spinner", props);
  if (isStaticProps(props) && !("__zguiClass" in props)) {
    const node = createElement("box");
    setProp(node, "role", "status");
    setProp(node, "ariaBusy", true);
    setProp(node, "ariaLabel", props.label ?? "Loading");
    setProp(node, "__zguiClass", classes(sheet.className, "zui-spinner"));
    const record = props as unknown as Record<string, unknown>;
    for (const key of Object.keys(record)) {
      if (key === "label") continue;
      const value = record[key];
      if (value !== undefined) setProp(node, key, value);
    }
    const mark = createElement("box");
    setProp(mark, "class", "zui-spinner__mark");
    append(node, mark);
    return node;
  }
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
