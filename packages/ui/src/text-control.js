import { createElement, effect, mergeProps, spread } from "./renderer/index.js";

function textControl(tag, role, name, props) {
  // Track spread shape changes without evaluating a lazy child getter.
  effect(() => {
    if ("children" in props) throw new TypeError(`${name} does not accept children; use value or defaultValue`);
  });
  const control = createElement(tag);
  spread(control, mergeProps({ tabIndex: 0 }, props, { role }), true);
  return control;
}

export function TextField(props = {}) {
  return textControl("field", "textInput", "TextField", props);
}

export function TextArea(props = {}) {
  return textControl("editor", "multilineTextInput", "TextArea", props);
}
