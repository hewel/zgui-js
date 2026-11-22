import { omit } from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import type {
  NativeFieldProps,
  NativeNode,
  NativeRole,
  NativeTextControlProps,
  NativeValueEvent,
} from "../types.js";
import { classes, forbidChildren, useStylesheet, withDefaults } from "./shared.js";
import type { Stylesheet } from "./shared.js";
import { sheet as inputSheet } from "./generated/InputStyle.js";
import { sheet as textareaSheet } from "./generated/TextareaStyle.js";

export interface InputProps extends NativeFieldProps {
  label?: string | null;
  onValueChange?: (value: string) => void;
}

export interface TextareaProps extends NativeTextControlProps {
  label?: string | null;
  onValueChange?: (value: string) => void;
}

function quoted(text: string): string {
  let out = '"';
  for (let index = 0; index < text.length; index++) {
    const character = text[index];
    if (character === "\\" || character === '"') out += `\\${character}`;
    else if (character === "\r") {
      out += "\\A ";
      if (text[index + 1] === "\n") index++;
    } else if (character === "\n") out += "\\A ";
    else if (character === "\f") out += "\\C ";
    else out += character;
  }
  return `${out}"`;
}

function editing(
  name: string,
  tag: "field" | "editor",
  sheet: Stylesheet,
  role: NativeRole,
  className: () => string,
  props: InputProps | TextareaProps,
): NativeNode {
  useStylesheet(sheet);
  forbidChildren(name, props);
  const rest = omit(props, "label", "onValueChange", "onInput");
  const node = createElement(tag);
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          tabIndex: 0,
          get role() {
            return tag === "field" && (rest as { secure?: boolean | null }).secure
              ? "passwordInput"
              : role;
          },
          get ariaLabel() {
            return props.label ?? null;
          },
          get __zguiStyle() {
            return {
              "--zui-field-placeholder": rest.placeholder == null ? null : quoted(rest.placeholder),
            };
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return className();
        },
        get onInput() {
          const input = props.onInput;
          const change = props.onValueChange;
          if (!input && !change) return null;
          return (event: NativeValueEvent) => {
            input?.(event);
            change?.(event.value);
          };
        },
      },
    ),
    true,
  );
  return node;
}

export function Input(props: InputProps = {}): NativeNode {
  return editing(
    "Input",
    "field",
    inputSheet,
    "textInput",
    () => classes(inputSheet.className, "zui-field"),
    props,
  );
}

export function Textarea(props: TextareaProps = {}): NativeNode {
  return editing(
    "Textarea",
    "editor",
    textareaSheet,
    "multilineTextInput",
    () => classes(textareaSheet.className, "zui-field", "zui-textarea"),
    props,
  );
}
