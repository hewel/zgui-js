import { createElement, mergeProps, spread } from "./renderer/index.js";

/** Native control defaults own pointer and non-repeating Enter/Space activation. */
export function Button(props = {}) {
  const control = createElement("control");
  spread(control, mergeProps({ tabIndex: 0 }, props, { role: "button" }));
  return control;
}
