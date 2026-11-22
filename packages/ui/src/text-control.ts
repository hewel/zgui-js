import { createElement, effect, mergeProps, spread } from "./renderer/index.js";
import type { NativeNode, TextFieldProps, TextAreaProps } from "./types.js";

function textControl(
  tag: "field" | "editor",
  role: "textInput" | "multilineTextInput" | "passwordInput",
  name: string,
  props: TextFieldProps | TextAreaProps,
): NativeNode {
  // Track spread shape changes without evaluating a lazy child getter.
  effect(
    () => "children" in props,
    hasChildren => {
      if (hasChildren)
        throw new TypeError(`${name} does not accept children; use value or defaultValue`);
    },
  );
  const control = createElement(tag);
  spread(
    control,
    mergeProps(props, {
      get tabIndex() {
        return props.tabIndex ?? 0;
      },
      get role() {
        return tag === "field" && "secure" in props && props.secure ? "passwordInput" : role;
      },
    }),
    true,
  );
  return control;
}

export function TextField(props: TextFieldProps = {}): NativeNode {
  return textControl("field", "textInput", "TextField", props);
}

export function TextArea(props: TextAreaProps = {}): NativeNode {
  return textControl("editor", "multilineTextInput", "TextArea", props);
}
