import { splitProps } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, part, useStylesheet } from "./shared.js";
import { Button } from "./button.js";
import type { ButtonProps } from "./button.js";
import { Input, Textarea } from "./input.js";
import type { InputProps, TextareaProps } from "./input.js";
import { sheet } from "./generated/InputGroupStyle.js";
import { sheet as partSheet } from "./generated/InputGroupPartStyle.js";

export type InputGroupProps = NativeElementProps;
export type InputGroupAddonAlign = "inline-start" | "inline-end" | "block-start" | "block-end";
export interface InputGroupAddonProps extends NativeElementProps { align?: InputGroupAddonAlign; }
export type InputGroupTextProps = NativeElementProps;
export type InputGroupButtonSize = "xs" | "sm" | "icon-xs" | "icon-sm";
export interface InputGroupButtonProps extends Omit<ButtonProps, "size"> { size?: InputGroupButtonSize; }
export type InputGroupInputProps = InputProps;
export type InputGroupTextareaProps = TextareaProps;

export function InputGroup(props: InputGroupProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["disabled", "invalid"]);
  return element("box", mergeProps(
    {
      role: "group" as const,
      get "data-disabled"() { return String(own.disabled ?? false); },
      get "data-invalid"() { return String(own.invalid ?? false); },
    },
    rest,
    { get class() { return classes(sheet.className, "zui-input-group", rest.class); } },
  ));
}

export function InputGroupAddon(props: InputGroupAddonProps = {}): NativeNode {
  useStylesheet(partSheet);
  const [own, rest] = splitProps(props, ["align"]);
  return element("box", mergeProps(
    { get "data-align"() { return own.align ?? "inline-start"; } },
    rest,
    {
      get class() {
        return classes(
          "zui-input-group__addon",
          own.align && own.align !== "inline-start" && `zui-input-group__addon--${own.align}`,
          rest.class,
        );
      },
    },
  ));
}

export const InputGroupText = part("box", partSheet, "zui-input-group__text");

export function InputGroupButton(props: InputGroupButtonProps = {}): NativeNode {
  useStylesheet(partSheet);
  return Button(mergeProps(
    { variant: "ghost" as const, size: "xs" as const },
    props,
    { get class() { return classes("zui-input-group__button", props.class); } },
  ));
}

export function InputGroupInput(props: InputGroupInputProps = {}): NativeNode {
  useStylesheet(partSheet);
  return Input(mergeProps(props, {
    get class() { return classes("zui-input-group__field", props.class); },
  }));
}

export function InputGroupTextarea(props: InputGroupTextareaProps = {}): NativeNode {
  useStylesheet(partSheet);
  return Textarea(mergeProps(props, {
    get class() { return classes("zui-input-group__area", props.class); } },
  ));
}
