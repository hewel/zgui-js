import { splitProps } from "#solid";
import { mergeProps } from "../renderer/index.js";
import { focus } from "../nodes.js";
import type { Accessor, NativeElementProps, NativeEvent, NativeNode } from "../types.js";
import { callHandler } from "./state.js";
import { classes, element, useStylesheet } from "./shared.js";
import { sheet } from "./generated/LabelStyle.js";

export interface LabelProps extends NativeElementProps {
  control?: NativeNode | Accessor<NativeNode | null | undefined> | null;
}

export function Label(props: LabelProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["control", "onClick"]);
  return element("label", mergeProps(
    { role: "label" as const },
    rest,
    {
      get class() { return classes(sheet.className, "zui-label", rest.class); },
      get onClick() {
        return (event: NativeEvent) => {
          if (!callHandler(event, own.onClick) || rest.disabled) return;
          const control = own.control;
          const node = typeof control === "function" ? control() : control;
          if (node) focus(node);
        };
      },
    },
  ));
}
