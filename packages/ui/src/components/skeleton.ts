import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, forbidChildren, useStylesheet } from "./shared.js";
import { sheet } from "./generated/SkeletonStyle.js";

export interface SkeletonProps extends Omit<NativeElementProps, "children"> {
  children?: never;
}

export function Skeleton(props: SkeletonProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Skeleton", props);
  return element("box", mergeProps(
    { role: "genericContainer" as const, ariaBusy: true },
    props,
    { get class() { return classes(sheet.className, "zui-skeleton", props.class); } },
  ));
}
