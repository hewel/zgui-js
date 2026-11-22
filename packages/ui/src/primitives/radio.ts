import { createContext, omit, useContext } from "#solid";
import { createElement, memo, mergeProps, spread } from "../renderer/index.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeEvent,
  NativeFocusEvent,
  NativeKeyEvent,
  NativeNode,
} from "../types.js";
import { callHandler, createControllable } from "../components/state.js";
import { provide, slot, withDefaults } from "../components/shared.js";
import { createRovingFocus, groupOrientation } from "./roving.js";
import type { GroupOrientation, RovingFocusController } from "./roving.js";

export interface RadioGroupPrimitiveProps extends Omit<NativeElementProps, "orientation"> {
  readonly value?: string | undefined;
  readonly defaultValue?: string | undefined;
  readonly onValueChange?: ((value: string) => void) | undefined;
  readonly orientation?: GroupOrientation | undefined;
  readonly children?: NativeChild;
}

export interface RadioItemPrimitiveProps extends Omit<NativeElementProps, "checked"> {
  readonly value: string;
  readonly children?: NativeChild;
}

interface RadioPrimitiveContextValue {
  readonly node: NativeNode;
  readonly disabled: Accessor<boolean>;
  readonly value: Accessor<string>;
  readonly choose: (value: string) => void;
  readonly roving: RovingFocusController;
}

const RadioPrimitiveContext = createContext<RadioPrimitiveContextValue | null>(null);

function validateRadioValue(value: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new TypeError("Group item value must be a non-empty string");
  }
  return value;
}

/**
 * Unstyled headless radio group (`role="radioGroup"`) with roving tabindex and zero built-in styles.
 */
export function RadioGroupPrimitive(props: RadioGroupPrimitiveProps = {}): NativeNode {
  const rest = omit(
    props,
    "value",
    "defaultValue",
    "onValueChange",
    "disabled",
    "orientation",
    "children",
    "onKeyDown",
  );
  const [value, choose] = createControllable<string>(
    {
      value: () => props.value,
      defaultValue: () => props.defaultValue,
      onChange: () => props.onValueChange,
    },
    "",
  );
  const axis = memo(() => groupOrientation(props.orientation ?? "vertical"));
  const disabled = () => !!props.disabled;
  const node = createElement("box");
  const roving = createRovingFocus(axis);
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "radioGroup" as const,
          get disabled() {
            return disabled();
          },
          get orientation() {
            return axis() === "horizontal" ? ("horizontal" as const) : ("vertical" as const);
          },
          get "data-orientation"() {
            return axis();
          },
        },
        rest,
      ),
      {
        get onKeyDown() {
          return (event: NativeKeyEvent) => {
            if (callHandler(event, props.onKeyDown) && !disabled()) roving.onKeyDown(event);
          };
        },
      },
    ),
  );
  const context: RadioPrimitiveContextValue = { node, disabled, value, choose, roving };
  slot(node, provide(RadioPrimitiveContext, context, props));
  return node;
}

/**
 * Unstyled headless radio item (`role="radioButton"`) accepting arbitrary children with zero built-in styles.
 */
export function RadioItemPrimitive(props: RadioItemPrimitiveProps): NativeNode {
  const rest = omit(props, "value", "disabled", "children", "onClick", "onFocus");
  const group = useContext(RadioPrimitiveContext);
  validateRadioValue(props.value);
  const value = memo(() => validateRadioValue(props.value));
  const disabled = () => !!props.disabled || !!group?.disabled();
  const chosen = () => group?.value() === value();
  const node = createElement("control");
  const item = group?.roving.register(node, disabled, chosen);
  const choose = () => {
    if (disabled()) return;
    item?.activate();
    group?.choose(value());
  };
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "radioButton" as const,
          get tabIndex() {
            return item?.tabIndex() ?? 0;
          },
          get checked() {
            return chosen();
          },
          get disabled() {
            return disabled();
          },
          ariaRadioGroup: group?.node ?? null,
          get "data-state"() {
            return chosen() ? "checked" : "unchecked";
          },
          get "data-value"() {
            return value();
          },
        },
        rest,
      ),
      {
        get onClick() {
          return (event: NativeEvent) => {
            if (callHandler(event, props.onClick)) choose();
          };
        },
        get onFocus() {
          return (event: NativeFocusEvent) => {
            if (callHandler(event, props.onFocus) && event.target === node) choose();
          };
        },
      },
    ),
  );
  if ("children" in props) {
    slot(node, () => props.children);
  }
  return node;
}
