import { omit } from "#solid";
import { mergeProps } from "../renderer/index.js";
import { focus } from "../nodes.js";
import type { Accessor, NativeElementProps, NativeEvent, NativeNode } from "../types.js";
import { callHandler } from "./state.js";
import { classes, element, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/LabelStyle.js";

export interface LabelProps extends NativeElementProps {
  control?: NativeNode | Accessor<NativeNode | null | undefined> | null;
}

export function Label(props: LabelProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "control", "onClick");
  return element("label", mergeProps(
    rest,
    withDefaults({ role: "label" as const }, rest),
    {
      get __zguiClass() { return classes(sheet.className, "zui-label"); },
      get onClick() {
        return (event: NativeEvent) => {
          if (!callHandler(event, props.onClick) || rest.disabled) return;
          const control = props.control;
          const node = typeof control === "function" ? control() : control;
          if (node) focus(node);
        };
      },
    },
  ));
}
