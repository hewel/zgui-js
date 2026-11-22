import { omit } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, part, useStylesheet, withDefaults } from "./shared.js";
import { Button } from "./button.js";
import type { ButtonProps } from "./button.js";
import { Input, Textarea } from "./input.js";
import type { InputProps, TextareaProps } from "./input.js";
import { sheet } from "./generated/InputGroupStyle.js";
import { sheet as partSheet } from "./generated/InputGroupPartStyle.js";

export type InputGroupProps = NativeElementProps;
export type InputGroupAddonAlign = "inline-start" | "inline-end" | "block-start" | "block-end";
export interface InputGroupAddonProps extends NativeElementProps {
  align?: InputGroupAddonAlign;
}
export type InputGroupTextProps = NativeElementProps;
export type InputGroupButtonSize = "xs" | "sm" | "icon-xs" | "icon-sm";
export interface InputGroupButtonProps extends Omit<ButtonProps, "size"> {
  size?: InputGroupButtonSize;
}
export type InputGroupInputProps = InputProps;
export type InputGroupTextareaProps = TextareaProps;

export function InputGroup(props: InputGroupProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "disabled", "invalid");
  return element(
    "box",
    mergeProps(
      rest,
      withDefaults(
        {
          role: "group" as const,
          get "data-disabled"() {
            return String(props.disabled ?? false);
          },
          get "data-invalid"() {
            return String(props.invalid ?? false);
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(sheet.className, "zui-input-group");
        },
      },
    ),
  );
}

export function InputGroupAddon(props: InputGroupAddonProps = {}): NativeNode {
  useStylesheet(partSheet);
  const rest = omit(props, "align");
  return element(
    "box",
    mergeProps(
      rest,
      withDefaults(
        {
          get "data-align"() {
            return props.align ?? "inline-start";
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(
            "zui-input-group__addon",
            props.align &&
              props.align !== "inline-start" &&
              `zui-input-group__addon--${props.align}`,
          );
        },
      },
    ),
  );
}

export const InputGroupText = part("box", partSheet, "zui-input-group__text");

export function InputGroupButton(props: InputGroupButtonProps = {}): NativeNode {
  useStylesheet(partSheet);
  return Button(
    mergeProps(props, withDefaults({ variant: "ghost" as const, size: "xs" as const }, props), {
      get __zguiClass() {
        return classes("zui-input-group__button");
      },
    }),
  );
}

export function InputGroupInput(props: InputGroupInputProps = {}): NativeNode {
  useStylesheet(partSheet);
  return Input(
    mergeProps(props, {
      get __zguiClass() {
        return classes("zui-input-group__field");
      },
    }),
  );
}

export function InputGroupTextarea(props: InputGroupTextareaProps = {}): NativeNode {
  useStylesheet(partSheet);
  return Textarea(
    mergeProps(props, {
      get __zguiClass() {
        return classes("zui-input-group__area");
      },
    }),
  );
}
