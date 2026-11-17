import { splitProps } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { append, classes, element, forbidChildren, useStylesheet } from "./shared.js";
import { sheet } from "./generated/SpinnerStyle.js";

export interface SpinnerProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  label?: string | null;
}

export function Spinner(props: SpinnerProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Spinner", props);
  const [own, rest] = splitProps(props, ["label"]);
  const node = element("box", mergeProps(
    {
      role: "status" as const,
      ariaBusy: true,
      get ariaLabel() { return own.label ?? "Loading"; },
    },
    rest,
    { get class() { return classes(sheet.className, "zui-spinner", rest.class); } },
  ));
  append(node, element("box", { class: "zui-spinner__mark" }));
  return node;
}
