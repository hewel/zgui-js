import { omit } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeEvent, NativeNode, NativePointerDownEvent } from "../types.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, element, forbidChildren, useStylesheet, withDefaults } from "./shared.js";
import { Icon } from "./icons.js";
import { CHECK, MINUS } from "./generated/icons.js";
import { sheet as checkboxSheet } from "./generated/CheckboxStyle.js";
import { sheet as switchSheet } from "./generated/SwitchStyle.js";
import { sheet as toggleSheet } from "./generated/ToggleStyle.js";

export type CheckedState = boolean | "mixed";

export interface CheckboxProps extends Omit<NativeElementProps, "checked" | "children"> {
  children?: never;
  checked?: CheckedState;
  defaultChecked?: CheckedState;
  onCheckedChange?: (value: CheckedState) => void;
}

export interface SwitchProps extends Omit<NativeElementProps, "checked" | "children"> {
  children?: never;
  checked?: boolean;
  defaultChecked?: boolean;
  onCheckedChange?: (value: boolean) => void;
  size?: "sm" | "md";
}

export type ToggleVariant = "default" | "outline";
export type ToggleSize = "sm" | "md" | "lg";

export interface ToggleProps extends Omit<NativeElementProps, "checked"> {
  pressed?: boolean;
  defaultPressed?: boolean;
  onPressedChange?: (value: boolean) => void;
  variant?: ToggleVariant;
  size?: ToggleSize;
  label?: string | null;
}

export function Checkbox(props: CheckboxProps = {}): NativeNode {
  useStylesheet(checkboxSheet);
  forbidChildren("Checkbox", props);
  const rest = omit(props, "checked", "defaultChecked", "onCheckedChange", "onClick", "onPointerDown");
  const [checked, write] = createControllable<CheckedState>({
    value: () => props.checked,
    defaultValue: () => props.defaultChecked,
    onChange: () => props.onCheckedChange,
  }, false);
  const node = element("control", mergeProps(
    rest,
    withDefaults({
      tabIndex: 0,
      role: "checkBox" as const,
      get checked() { return checked(); },
      get "data-state"() {
        const value = checked();
        return value === "mixed" ? "indeterminate" : value ? "checked" : "unchecked";
      },
    }, rest),
    {
      get __zguiClass() { return classes(checkboxSheet.className, "zui-checkbox"); },
      get onClick() {
        return (event: NativeEvent) => {
          if (callHandler(event, props.onClick) && !rest.disabled) {
            write(checked() !== true);
          }
        };
      },
      get onPointerDown() {
        return (event: NativePointerDownEvent) => {
          if (callHandler(event, props.onPointerDown) && !rest.disabled && event.button === "primary") {
            event.activateNow();
          }
        };
      },
    },
  ));
  append(node, Icon({ icon: CHECK, class: "zui-checkbox__tick" }));
  append(node, Icon({ icon: MINUS, class: "zui-checkbox__dash" }));
  return node;
}

export function Switch(props: SwitchProps = {}): NativeNode {
  useStylesheet(switchSheet);
  forbidChildren("Switch", props);
  const rest = omit(props, "checked", "defaultChecked", "onCheckedChange", "onClick", "onPointerDown", "size");
  const [checked, write] = createControllable<boolean>({
    value: () => props.checked,
    defaultValue: () => props.defaultChecked,
    onChange: () => props.onCheckedChange,
  }, false);
  const node = element("control", mergeProps(
    rest,
    withDefaults({
      tabIndex: 0,
      role: "switch" as const,
      get checked() { return checked(); },
      get "data-state"() { return checked() ? "checked" : "unchecked"; },
      get "data-size"() { return props.size ?? "md"; },
    }, rest),
    {
      get __zguiClass() {
        return classes(switchSheet.className, "zui-switch",
        (props.size ?? "md") === "sm" && "zui-switch--sm", );
      },
      get onClick() {
        return (event: NativeEvent) => {
          if (callHandler(event, props.onClick) && !rest.disabled) {
            write(!checked());
          }
        };
      },
      get onPointerDown() {
        return (event: NativePointerDownEvent) => {
          if (callHandler(event, props.onPointerDown) && !rest.disabled && event.button === "primary") {
            event.activateNow();
          }
        };
      },
    },
  ));
  append(node, element("box", { class: "zui-switch__thumb" }));
  return node;
}

export const TOGGLE_SIZE_CLASSES: Record<ToggleSize, string> = { sm: "zui-toggle--sm", md: "", lg: "zui-toggle--lg" };

export function Toggle(props: ToggleProps = {}): NativeNode {
  useStylesheet(toggleSheet);
  const rest = omit(props, "pressed", "defaultPressed", "onPressedChange", "variant", "size", "label", "onClick");
  const [pressed, write] = createControllable<boolean>({
    value: () => props.pressed,
    defaultValue: () => props.defaultPressed,
    onChange: () => props.onPressedChange,
  }, false);
  return element("control", mergeProps(
    rest,
    withDefaults({
      tabIndex: 0,
      role: "button" as const,
      get checked() { return pressed(); },
      get ariaLabel() { return props.label ?? null; },
      get "data-state"() { return pressed() ? "on" : "off"; },
      get "data-variant"() { return props.variant ?? "default"; },
      get "data-size"() { return props.size ?? "md"; },
    }, rest),
    {
      get __zguiClass() {
        return classes(toggleSheet.className, "zui-toggle",
        (props.variant ?? "default") === "outline" && "zui-toggle--outline",
        TOGGLE_SIZE_CLASSES[props.size ?? "md"], );
      },
      get onClick() {
        return (event: NativeEvent) => {
          if (callHandler(event, props.onClick) && !rest.disabled) {
            write(!pressed());
          }
        };
      },
    },
  ));
}
