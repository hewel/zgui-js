import { createContext, omit, useContext } from "#solid";
import { createElement, memo, mergeProps, spread } from "../renderer/index.js";
import type {
  Accessor,
  NativeElementProps,
  NativeEvent,
  NativeFocusEvent,
  NativeKeyEvent,
  NativeNode,
} from "../types.js";
import { callHandler, createControllable } from "./state.js";
import { classes, forbidChildren, provide, slot, useStylesheet, withDefaults } from "./shared.js";
import { createRovingFocus, groupOrientation } from "./roving.js";
import type { GroupOrientation } from "./roving.js";
import { RadioGroupPrimitive, RadioItemPrimitive } from "../primitives/radio.js";
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
interface ToggleContextValue {
  disabled: Accessor<boolean>;
  value: Accessor<readonly string[]>;
  toggle: (value: string) => void;
  selection: Accessor<ToggleSelection>;
  variant: Accessor<ToggleVariant>;
  size: Accessor<ToggleSize>;
  roving: Roving;
}
const ToggleContext = createContext<ToggleContextValue | null>(null);

function itemValue(value: string): string {
  if (typeof value !== "string" || value.length === 0)
    throw new TypeError("Group item value must be a non-empty string");
  return value;
}

export function RadioGroup(props: RadioGroupProps = {}): NativeNode {
  useStylesheet(radioGroupSheet);
  const rest = omit(props, "label");
  return RadioGroupPrimitive(
    mergeProps(
      rest,
      withDefaults(
        {
          get ariaLabel() {
            return props.label ?? null;
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(radioGroupSheet.className, "zui-radio-group");
        },
      },
    ),
  );
}

export function RadioGroupItem(props: RadioGroupItemProps): NativeNode {
  useStylesheet(radioItemSheet);
  forbidChildren("RadioGroupItem", props);
  const rest = omit(props, "label", "children");
  return RadioItemPrimitive(
    mergeProps(
      rest,
      withDefaults(
        {
          get ariaLabel() {
            return props.label ?? null;
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(radioItemSheet.className, "zui-radio-group__item");
        },
        get children() {
          return Icon({ icon: DISC });
        },
      },
    ),
  );
}

function toggleValues(current: readonly string[]): readonly string[] {
  if (!Array.isArray(current) || !current.every(item => typeof item === "string")) {
    throw new TypeError("ToggleGroup value must be an array of strings");
  }
  return current;
}
function toggleSelection(value: ToggleSelection | undefined): ToggleSelection {
  const resolved = value ?? "single";
  if (resolved !== "single" && resolved !== "multiple")
    throw new TypeError("ToggleGroup selection must be single or multiple");
  return resolved;
}
function toggleSpacing(value: number | undefined): string {
  const resolved = value ?? 0;
  if (!Number.isFinite(resolved) || resolved < 0)
    throw new TypeError("ToggleGroup spacing must be a finite non-negative number");
  return String(resolved);
}

export function ToggleGroup(props: ToggleGroupProps = {}): NativeNode {
  useStylesheet(toggleGroupSheet);
  const rest = omit(
    props,
    "value",
    "defaultValue",
    "onValueChange",
    "selection",
    "variant",
    "size",
    "spacing",
    "disabled",
    "orientation",
    "label",
    "children",
    "onKeyDown",
  );
  const [read, write] = createControllable<readonly string[]>(
    {
      value: () => props.value,
      defaultValue: () => props.defaultValue,
      onChange: () => props.onValueChange,
    },
    [],
  );
  toggleValues(read());
  toggleSelection(props.selection);
  toggleSpacing(props.spacing);
  groupOrientation(props.orientation ?? "horizontal");
  const values = memo(() => toggleValues(read()));
  const selection = memo(() => toggleSelection(props.selection));
  const spacing = memo(() => toggleSpacing(props.spacing));
  const axis = memo(() => groupOrientation(props.orientation ?? "horizontal"));
  const disabled = () => !!props.disabled;
  const variant = () => props.variant ?? "default";
  const size = () => props.size ?? "md";
  const node = createElement("box");
  const roving = createRovingFocus(axis);
  const toggle = (value: string) => {
    if (disabled()) return;
    const next = new Set(values());
    if (next.has(value)) next.delete(value);
    else {
      if (selection() === "single") next.clear();
      next.add(value);
    }
    write([...next].sort());
  };
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "toolbar" as const,
          get disabled() {
            return disabled();
          },
          get ariaLabel() {
            return props.label ?? null;
          },
          get orientation() {
            return axis() === "vertical" ? ("vertical" as const) : ("horizontal" as const);
          },
          get "data-orientation"() {
            return axis();
          },
          get "data-variant"() {
            return variant();
          },
          get "data-spacing"() {
            return spacing();
          },
          get __zguiStyle() {
            return { "--zui-toggle-group-gap": spacing() };
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(toggleGroupSheet.className, "zui-toggle-group");
        },
        get onKeyDown() {
          return (event: NativeKeyEvent) => {
            if (callHandler(event, props.onKeyDown) && !disabled()) roving.onKeyDown(event);
          };
        },
      },
    ),
  );
  const context: ToggleContextValue = {
    disabled,
    value: values,
    toggle,
    selection,
    variant,
    size,
    roving,
  };
  slot(node, provide(ToggleContext, context, props));
  return node;
}

export function ToggleGroupItem(props: ToggleGroupItemProps): NativeNode {
  useStylesheet(toggleSheet);
  const rest = omit(props, "value", "variant", "size", "disabled", "label", "onClick", "onFocus");
  const group = useContext(ToggleContext);
  itemValue(props.value);
  const value = memo(() => itemValue(props.value));
  const disabled = () => !!props.disabled || !!group?.disabled();
  const variant = () => props.variant ?? group?.variant() ?? "default";
  const size = () => props.size ?? group?.size() ?? "md";
  const on = () => group?.value().includes(value()) ?? false;
  const node = createElement("control");
  const item = group?.roving.register(node, disabled);
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          get role() {
            return group?.selection() === "single" ? ("radioButton" as const) : ("button" as const);
          },
          get tabIndex() {
            return item?.tabIndex() ?? 0;
          },
          get checked() {
            return on();
          },
          get disabled() {
            return disabled();
          },
          get ariaLabel() {
            return props.label ?? null;
          },
          get "data-state"() {
            return on() ? "on" : "off";
          },
          get "data-value"() {
            return value();
          },
          get "data-variant"() {
            return variant();
          },
          get "data-size"() {
            return size();
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(
            toggleSheet.className,
            "zui-toggle",
            "zui-toggle-group__item",
            variant() === "outline" && "zui-toggle--outline",
            TOGGLE_SIZE_CLASSES[size()],
          );
        },
        get onClick() {
          return (event: NativeEvent) => {
            if (callHandler(event, props.onClick) && !disabled()) group?.toggle(value());
          };
        },
        get onFocus() {
          return (event: NativeFocusEvent) => {
            if (callHandler(event, props.onFocus) && event.target === node && !disabled())
              item?.activate();
          };
        },
      },
    ),
  );
  return node;
}
