import { createElement, mergeProps, spread } from "./renderer/index.js";
import type { ButtonProps, NativeNode } from "./types.js";

/** Native control defaults own pointer and non-repeating Enter/Space activation. */
export function Button(props: ButtonProps = {}): NativeNode {
  const control = createElement("control");
  spread(control, mergeProps({ tabIndex: 0 }, props, { role: "button" as const }));
  return control;
}
