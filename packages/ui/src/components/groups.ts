import { createContext, splitProps, useContext } from "#solid";
import { createElement, memo, mergeProps, spread } from "../renderer/index.js";
import type { Accessor, NativeElementProps, NativeEvent, NativeFocusEvent, NativeKeyEvent, NativeNode } from "../types.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, forbidChildren, provide, slot, useStylesheet } from "./shared.js";
import { createRovingFocus, groupOrientation } from "./roving.js";
import type { GroupOrientation } from "./roving.js";
import { Icon } from "./icons.js";
import { DISC } from "./generated/icons.js";
import { TOGGLE_SIZE_CLASSES } from "./choices.js";
import type { ToggleSize, ToggleVariant } from "./choices.js";
import { sheet as radioGroupSheet } from "./generated/RadioGroupStyle.js";
import { sheet as radioItemSheet } from "./generated/RadioItemStyle.js";
import { sheet as toggleGroupSheet } from "./generated/ToggleGroupStyle.js";
import { sheet as toggleSheet } from "./generated/ToggleStyle.js";

export type { GroupOrientation } from "./roving.js";
export type ToggleSelection = "single" | "multiple";

export interface RadioGroupProps extends Omit<NativeElementProps, "orientation"> {
  value?: string;
  defaultValue?: string;
  onValueChange?: (value: string) => void;
  orientation?: GroupOrientation;
  label?: string | null;
}
export interface RadioGroupItemProps extends Omit<NativeElementProps, "children" | "checked"> {
  children?: never;
  value: string;
  label?: string | null;
}
export interface ToggleGroupProps extends Omit<NativeElementProps, "orientation"> {
  value?: readonly string[];
  defaultValue?: readonly string[];
  onValueChange?: (value: readonly string[]) => void;
  selection?: ToggleSelection;
  variant?: ToggleVariant;
  size?: ToggleSize;
  spacing?: number;
  orientation?: GroupOrientation;
  label?: string | null;
}
export interface ToggleGroupItemProps extends Omit<NativeElementProps, "checked"> {
  value: string;
  variant?: ToggleVariant;
  size?: ToggleSize;
  label?: string | null;
}

type Roving = ReturnType<typeof createRovingFocus>;
interface RadioContextValue {
  node: NativeNode;
  disabled: Accessor<boolean>;
  value: Accessor<string>;
  choose: (value: string) => void;
  roving: Roving;
}
interface ToggleContextValue {
  disabled: Accessor<boolean>;
  value: Accessor<readonly string[]>;
  toggle: (value: string) => void;
  selection: Accessor<ToggleSelection>;
  variant: Accessor<ToggleVariant>;
  size: Accessor<ToggleSize>;
  roving: Roving;
}
const RadioContext = createContext<RadioContextValue>();
const ToggleContext = createContext<ToggleContextValue>();

function itemValue(value: string): string {
  if (typeof value !== "string" || value.length === 0) throw new TypeError("Group item value must be a non-empty string");
  return value;
}

export function RadioGroup(props: RadioGroupProps = {}): NativeNode {
  useStylesheet(radioGroupSheet);
  const [own, rest] = splitProps(props, ["value", "defaultValue", "onValueChange", "disabled", "orientation", "label", "children", "onKeyDown"]);
  const [value, choose] = createControllable<string>({
    value: () => own.value,
    defaultValue: () => own.defaultValue,
    onChange: () => own.onValueChange,
  }, "");
  const axis = memo(() => groupOrientation(own.orientation ?? "vertical"));
  const disabled = () => !!own.disabled;
  const node = createElement("box");
  const roving = createRovingFocus(axis);
  spread(node, mergeProps(
    {
      role: "radioGroup" as const,
      get disabled() { return disabled(); },
      get ariaLabel() { return own.label ?? null; },
      get orientation() { return axis() === "horizontal" ? "horizontal" as const : "vertical" as const; },
      get "data-orientation"() { return axis(); },
    },
    rest,
    {
      get __zguiClass() { return classes(radioGroupSheet.className, "zui-radio-group"); },
      get onKeyDown() {
        return (event: NativeKeyEvent) => {
          if (callHandler(event, own.onKeyDown) && !disabled()) roving.onKeyDown(event);
        };
      },
    },
  ));
  const context: RadioContextValue = { node, disabled, value, choose, roving };
  slot(node, provide(RadioContext, context, own));
  return node;
}

export function RadioGroupItem(props: RadioGroupItemProps): NativeNode {
  useStylesheet(radioItemSheet);
  forbidChildren("RadioGroupItem", props);
  const [own, rest] = splitProps(props, ["value", "disabled", "label", "onClick", "onFocus"]);
  const group = useContext(RadioContext);
  const value = memo(() => itemValue(own.value));
  const disabled = () => !!own.disabled || !!group?.disabled();
  const chosen = () => group?.value() === value();
  const node = createElement("control");
  const item = group?.roving.register(node, disabled, chosen);
  const choose = () => {
    if (disabled()) return;
    item?.activate();
    group?.choose(value());
  };
  spread(node, mergeProps(
    {
      role: "radioButton" as const,
      get tabIndex() { return item?.tabIndex() ?? 0; },
      get checked() { return chosen(); },
      get disabled() { return disabled(); },
      get ariaLabel() { return own.label ?? null; },
      ariaRadioGroup: group?.node ?? null,
      get "data-state"() { return chosen() ? "checked" : "unchecked"; },
      get "data-value"() { return value(); },
    },
    rest,
    {
      get __zguiClass() { return classes(radioItemSheet.className, "zui-radio-group__item"); },
      get onClick() {
        return (event: NativeEvent) => {
          if (callHandler(event, own.onClick)) choose();
        };
      },
      get onFocus() {
        return (event: NativeFocusEvent) => {
          if (callHandler(event, own.onFocus) && event.target === node) choose();
        };
      },
    },
  ));
  append(node, Icon({ icon: DISC }));
  return node;
}

export function ToggleGroup(props: ToggleGroupProps = {}): NativeNode {
  useStylesheet(toggleGroupSheet);
  const [own, rest] = splitProps(props, ["value", "defaultValue", "onValueChange", "selection", "variant", "size", "spacing", "disabled", "orientation", "label", "children", "onKeyDown"]);
  const [read, write] = createControllable<readonly string[]>({
    value: () => own.value,
    defaultValue: () => own.defaultValue,
    onChange: () => own.onValueChange,
  }, []);
  const values = memo(() => {
    const current = read();
    if (!Array.isArray(current) || !current.every(item => typeof item === "string")) {
      throw new TypeError("ToggleGroup value must be an array of strings");
    }
    return current;
  });
  const selection = memo(() => {
    const value = own.selection ?? "single";
    if (value !== "single" && value !== "multiple") throw new TypeError("ToggleGroup selection must be single or multiple");
    return value;
  });
  const spacing = memo(() => {
    const value = own.spacing ?? 0;
    if (!Number.isFinite(value) || value < 0) throw new TypeError("ToggleGroup spacing must be a finite non-negative number");
    return String(value);
  });
  const axis = memo(() => groupOrientation(own.orientation ?? "horizontal"));
  const disabled = () => !!own.disabled;
  const variant = () => own.variant ?? "default";
  const size = () => own.size ?? "md";
  const node = createElement("box");
  const roving = createRovingFocus(axis);
  const toggle = (value: string) => {
    if (disabled()) return;
    const next = new Set(values());
    if (next.has(value)) next.delete(value);
    else { if (selection() === "single") next.clear(); next.add(value); }
    write([...next].sort());
  };
  spread(node, mergeProps(
    {
      role: "toolbar" as const,
      get disabled() { return disabled(); },
      get ariaLabel() { return own.label ?? null; },
      get orientation() { return axis() === "vertical" ? "vertical" as const : "horizontal" as const; },
      get "data-orientation"() { return axis(); },
      get "data-variant"() { return variant(); },
      get "data-spacing"() { return spacing(); },
      get __zguiStyle() { return { "--zui-toggle-group-gap": spacing() }; },
    },
    rest,
    {
      get __zguiClass() { return classes(toggleGroupSheet.className, "zui-toggle-group"); },
      get onKeyDown() {
        return (event: NativeKeyEvent) => {
          if (callHandler(event, own.onKeyDown) && !disabled()) roving.onKeyDown(event);
        };
      },
    },
  ));
  const context: ToggleContextValue = { disabled, value: values, toggle, selection, variant, size, roving };
  slot(node, provide(ToggleContext, context, own));
  return node;
}

export function ToggleGroupItem(props: ToggleGroupItemProps): NativeNode {
  useStylesheet(toggleSheet);
  const [own, rest] = splitProps(props, ["value", "variant", "size", "disabled", "label", "onClick", "onFocus"]);
  const group = useContext(ToggleContext);
  const value = memo(() => itemValue(own.value));
  const disabled = () => !!own.disabled || !!group?.disabled();
  const variant = () => own.variant ?? group?.variant() ?? "default";
  const size = () => own.size ?? group?.size() ?? "md";
  const on = () => group?.value().includes(value()) ?? false;
  const node = createElement("control");
  const item = group?.roving.register(node, disabled);
  spread(node, mergeProps(
    {
      get role() { return group?.selection() === "single" ? "radioButton" as const : "button" as const; },
      get tabIndex() { return item?.tabIndex() ?? 0; },
      get checked() { return on(); },
      get disabled() { return disabled(); },
      get ariaLabel() { return own.label ?? null; },
      get "data-state"() { return on() ? "on" : "off"; },
      get "data-value"() { return value(); },
      get "data-variant"() { return variant(); },
      get "data-size"() { return size(); },
    },
    rest,
    {
      get __zguiClass() {
        return classes(toggleSheet.className, "zui-toggle", "zui-toggle-group__item",
        variant() === "outline" && "zui-toggle--outline",
        TOGGLE_SIZE_CLASSES[size()], );
      },
      get onClick() {
        return (event: NativeEvent) => {
          if (callHandler(event, own.onClick) && !disabled()) group?.toggle(value());
        };
      },
      get onFocus() {
        return (event: NativeFocusEvent) => {
          if (callHandler(event, own.onFocus) && event.target === node && !disabled()) item?.activate();
        };
      },
    },
  ));
  return node;
}
