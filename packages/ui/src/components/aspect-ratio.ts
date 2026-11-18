import { splitProps } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, useStylesheet } from "./shared.js";
import { sheet } from "./generated/AspectRatioStyle.js";

export interface AspectRatioProps extends NativeElementProps {
  ratio?: number;
}

export function AspectRatio(props: AspectRatioProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["ratio"]);
  return element("box", mergeProps(
    {
      get __zguiStyle() {
        const ratio = own.ratio ?? 1;
        if (!Number.isFinite(ratio) || ratio <= 0) {
          throw new TypeError(`AspectRatio ratio must be a positive finite number`);
        }
        return { "--zui-aspect-ratio": String(ratio) };
      },
    },
    rest,
    { get __zguiClass() { return classes(sheet.className); } },
  ));
}
