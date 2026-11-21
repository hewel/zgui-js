import {
  $PROXY,
  createContext,
  createEffect,
  createSignal,
  onCleanup,
  omit,
  untrack,
  useContext,
} from "#solid";
import { createElement, flush, memo, mergeProps, spread } from "../renderer/index.js";
import { focus, getTextContent } from "../nodes.js";
import type {
  Accessor,
  NativeChild,
  NativeDismissReason,
  NativeElementProps,
  NativeEvent,
  NativeFocusEvent,
  NativeKeyEvent,
  NativeNode,
  NativeOverlayLayer,
  NativePointerBoundaryEvent,
  NativePointerDownEvent,
  NativeRole,
} from "../types.js";
import { callHandler, createControllable } from "../components/state.js";
import { provide, slot, withDefaults } from "../components/shared.js";
import { createDelayed, type DelayedController } from "./delay.js";
import {
  createDismissLayer,
  createOverlayState,
  OverlayContext,
  type OverlayRootProps,
  type OverlayState,
} from "./overlay.js";
import { createRovingFocus, type GroupOrientation, type RovingFocusController } from "./roving.js";

export interface MenuHierarchy {
  readonly state: OverlayState;
  dismissCascade(): void;
  readonly sub?: { readonly state: OverlayState; readonly closing: DelayedController };
}

export interface MenuSurfaceContextValue {
  readonly node: Accessor<NativeNode | null>;
  readonly roving: RovingFocusController;
}

export interface MenuRadioContextValue {
  readonly value: Accessor<string>;
  choose(value: string): void;
}

export const MenuHierarchyContext = createContext<MenuHierarchy | null>(null);
export const MenuSurfaceContext = createContext<MenuSurfaceContextValue | null>(null);
export const MenuRadioContext = createContext<MenuRadioContextValue | null>(null);

export interface MenuRootPrimitiveProps extends OverlayRootProps {
  onDismiss?: (() => void) | undefined;
}

export interface MenuListPrimitiveProps extends Omit<NativeElementProps, "orientation"> {
  orientation?: GroupOrientation | undefined;
  autoFocus?: boolean | undefined;
  typeahead?: boolean | undefined;
  dismissOnEscape?: boolean | undefined;
  dismissOnOutsidePress?: boolean | undefined;
  dismissLayer?: NativeOverlayLayer | false | undefined;
  onDismiss?: ((reason: NativeDismissReason) => void) | undefined;
  label?: string | null | undefined;
}

export interface MenuItemPrimitiveProps extends NativeElementProps {
  closeOnSelect?: boolean | undefined;
  onSelect?: (() => void) | undefined;
}

export interface MenuCheckboxItemPrimitiveProps extends Omit<NativeElementProps, "checked"> {
  checked?: boolean | undefined;
  defaultChecked?: boolean | undefined;
  onCheckedChange?: ((checked: boolean) => void) | undefined;
  closeOnSelect?: boolean | undefined;
}

export interface MenuRadioGroupPrimitiveProps extends NativeElementProps {
  label?: string | null | undefined;
  value?: string | undefined;
  defaultValue?: string | undefined;
  onValueChange?: ((value: string) => void) | undefined;
}

export interface MenuRadioItemPrimitiveProps extends NativeElementProps {
  value: string;
  closeOnSelect?: boolean | undefined;
}

export type MenuSubPrimitiveProps = OverlayRootProps;

export function createMenuTypeahead(
  group: RovingFocusController | undefined,
): (event: NativeKeyEvent) => void {
  const reset = createDelayed();
  let buffer = "";
  let alive = true;
  onCleanup(() => {
    alive = false;
    reset.cancel();
    buffer = "";
  });
  return (event: NativeKeyEvent) => {
    if (!alive || !group || event.character == null) return;
    if (event.character === " " && buffer === "") return;
    buffer += event.character.toLowerCase();
    reset.after(1000, () => {
      buffer = "";
    });
    const characters = Array.from(buffer);
    const search =
      characters.length > 1 && characters.every(character => character === characters[0])
        ? characters[0]
        : buffer;
    if (
      search &&
      group.focusNext(node =>
        getTextContent(node)
          .replace(/^\p{White_Space}+/u, "")
          .toLowerCase()
          .startsWith(search),
      )
    ) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
}

export interface MenuRowOptions {
  role: NativeRole;
  readonly select: () => void;
  disabled?: (() => boolean) | undefined;
  extra?: NativeElementProps | undefined;
  overrides?: NativeElementProps | undefined;
  activateOnPress?: boolean | undefined;
  emitDataHooks?: boolean | undefined;
  enter?: ((activate: () => void) => void) | undefined;
  leave?: ((event: NativePointerBoundaryEvent, defocus: () => void) => void) | undefined;
  classGetter?: (() => string) | undefined;
}

export function createMenuRow(props: NativeElementProps, options: MenuRowOptions): NativeNode {
  const surface = useContext(MenuSurfaceContext);
  const rest = omit(
    props,
    "children",
    "ref",
    "onClick",
    "onPointerDown",
    "onPointerEnter",
    "onPointerLeave",
    "onFocus",
    "onBlur",
  );
  const node = createElement("control");
  const disabled = options.disabled ?? (() => !!props.disabled);
  const emitDataHooks = options.emitDataHooks ?? true;
  const item = surface?.roving.register(node, disabled);
  const [highlighted, setHighlighted] = createSignal(false);
  let within = false;
  let alive = true;
  onCleanup(() => {
    alive = false;
    within = false;
  });
  const activate = () => {
    item?.activate();
    focus(node);
  };
  const defocus = () => {
    if (!within) return;
    const target = surface?.node();
    if (target) focus(target);
  };
  const classLayer = options.classGetter
    ? [
        {
          get __zguiClass() {
            return options.classGetter!();
          },
        },
      ]
    : [];
  const dataHooksLayer = emitDataHooks
    ? [
        withDefaults(
          {
            get "data-highlighted"() {
              return highlighted() && !disabled() ? "true" : null;
            },
            get "data-disabled"() {
              return disabled() ? "true" : null;
            },
          },
          rest,
        ),
      ]
    : [];
  spread(
    node,
    mergeProps(
      rest,
      ...(options.extra ? [withDefaults(options.extra, rest)] : []),
      withDefaults(
        {
          role: options.role,
          get tabIndex() {
            return item?.tabIndex() ?? (disabled() ? -1 : 0);
          },
        },
        rest,
      ),
      ...dataHooksLayer,
      ...(options.overrides ? [options.overrides] : []),
      ...classLayer,
      {
        ref(value: NativeNode) {
          if (typeof props.ref === "function") props.ref(value);
        },
        onClick(event: NativeEvent) {
          if (callHandler(event, props.onClick) && alive && !disabled()) options.select();
        },
        onPointerDown(event: NativePointerDownEvent) {
          if (
            callHandler(event, props.onPointerDown) &&
            alive &&
            !disabled() &&
            event.button === "primary" &&
            (options.activateOnPress ?? true)
          ) {
            event.activateNow();
          }
        },
        onPointerEnter(event: NativePointerBoundaryEvent) {
          if (callHandler(event, props.onPointerEnter) && alive && !disabled()) {
            if (options.enter) options.enter(activate);
            else activate();
          }
        },
        onPointerLeave(event: NativePointerBoundaryEvent) {
          if (!callHandler(event, props.onPointerLeave) || !alive) return;
          if (options.leave) options.leave(event, defocus);
          else defocus();
        },
        onFocus(event: NativeFocusEvent) {
          within = true;
          if (emitDataHooks && alive && !disabled()) setHighlighted(true);
          if (callHandler(event, props.onFocus) && alive && !disabled()) item?.activate();
        },
        onBlur(event: NativeFocusEvent) {
          within = false;
          if (emitDataHooks) setHighlighted(false);
          props.onBlur?.(event);
        },
      },
    ),
    true,
  );
  return node;
}

export function provideMenuHierarchyRoot(
  state: OverlayState,
  onDismiss: () => void,
  source: { readonly children?: NativeChild } | (() => NativeChild),
): NativeChild {
  let alive = true;
  onCleanup(() => {
    alive = false;
  });
  const dismiss = () => {
    if (alive) onDismiss();
  };
  return provide(MenuHierarchyContext, { state, dismissCascade: dismiss }, source);
}

/**
 * Headless menu root providing OverlayContext and MenuHierarchyContext with zero Portal,
 * Popper, or stylesheet coupling. Suitable for WindowSurface, LayerSurface, and PopupSurface.
 */
export function MenuRootPrimitive(props: MenuRootPrimitiveProps = {}): NativeChild {
  const state = createOverlayState(props);
  return provide(OverlayContext, state, () =>
    provideMenuHierarchyRoot(
      state,
      () => {
        state.setOpen(false);
        props.onDismiss?.();
      },
      props,
    ),
  );
}

/**
 * Headless nested submenu root providing cascading dismissal and hover-intent closing timer.
 */
export function MenuSubPrimitive(props: MenuSubPrimitiveProps = {}): NativeChild {
  const state = createOverlayState(props);
  const closing = createDelayed();
  const parent = useContext(MenuHierarchyContext);
  let alive = true;
  onCleanup(() => {
    alive = false;
    closing.cancel();
  });
  const node: MenuHierarchy = {
    state: parent?.state ?? state,
    dismissCascade() {
      if (!alive) return;
      closing.cancel();
      state.setOpen(false);
      parent?.dismissCascade();
    },
    sub: { state, closing },
  };
  return provide(OverlayContext, state, () => provide(MenuHierarchyContext, node, props));
}

function attachPrimitiveChildren(
  node: NativeNode,
  props: { readonly children?: NativeChild | undefined },
): void {
  if (!($PROXY in props) && Object.getOwnPropertyDescriptor(props, "children")?.get === undefined) {
    slot(node, props.children);
    return;
  }
  slot(
    node,
    memo(() => props.children),
  );
}

/**
 * Headless menu list container (`role="menu"`) with roving keyboard navigation, typeahead,
 * optional Escape/dismiss handling, and zero built-in styling.
 */
export function MenuListPrimitive(props: MenuListPrimitiveProps = {}): NativeNode {
  const hierarchy = useContext(MenuHierarchyContext);
  const state = hierarchy?.sub?.state ?? hierarchy?.state ?? useContext(OverlayContext);
  const orientation = () => props.orientation ?? "vertical";
  const roving = createRovingFocus(orientation, "active");
  const onTypeahead = createMenuTypeahead(roving);
  const rest = omit(
    props,
    "children",
    "ref",
    "orientation",
    "autoFocus",
    "typeahead",
    "dismissOnEscape",
    "dismissOnOutsidePress",
    "dismissLayer",
    "onDismiss",
    "label",
    "onKeyDown",
  );
  const node = createElement("box");
  state?.content.ref(node);

  const dismiss = (reason: NativeDismissReason) => {
    if (props.onDismiss) props.onDismiss(reason);
    else hierarchy?.dismissCascade();
  };

  if (props.dismissLayer) {
    const layer = props.dismissLayer;
    createDismissLayer(node, dismiss, () => ({
      layer,
      dismissOnOutsidePress: props.dismissOnOutsidePress ?? true,
      dismissOnEscape: props.dismissOnEscape ?? true,
      exclude: state?.trigger.node() ?? null,
      leaving: state ? !state.open() : false,
    }));
  }

  let focused = false;
  createEffect(
    () => ((props.autoFocus ?? true) && !focused ? roving.firstEnabled() : undefined),
    first => {
      if ((props.autoFocus ?? true) && !focused && first && roving.focusFirst()) {
        focused = true;
      }
    },
  );

  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "menu" as const,
          tabIndex: -1,
          get ariaLabel() {
            return props.label ?? null;
          },
          get "data-orientation"() {
            return orientation();
          },
          get "data-state"() {
            return state ? (state.open() ? "open" : "closed") : "open";
          },
        },
        rest,
      ),
      {
        ref(value: NativeNode) {
          if (typeof props.ref === "function") props.ref(value);
        },
        onKeyDown(event: NativeKeyEvent) {
          if (!callHandler(event, props.onKeyDown)) return;
          if (
            (props.dismissOnEscape ?? true) &&
            event.character == null &&
            event.key === "Escape"
          ) {
            event.preventDefault();
            event.stopPropagation();
            dismiss("escape");
            return;
          }
          if (callHandler(event, roving.onKeyDown) && (props.typeahead ?? true)) {
            onTypeahead(event);
          }
        },
      },
    ),
    true,
  );

  const surfaceValue: MenuSurfaceContextValue = {
    node: () => node,
    roving,
  };
  slot(node, provide(MenuSurfaceContext, surfaceValue, props));
  return node;
}

/**
 * Headless menu item (`role="menuItem"`) with roving focus, hover highlight,
 * `data-highlighted` / `data-disabled` styling hooks, and zero stylesheets.
 */
export function MenuItemPrimitive(props: MenuItemPrimitiveProps = {}): NativeNode {
  const menu = useContext(MenuHierarchyContext);
  const rest = omit(props, "children", "closeOnSelect", "onSelect");
  const node = createMenuRow(rest, {
    role: "menuItem",
    disabled: () => !!props.disabled,
    select() {
      props.onSelect?.();
      if (props.closeOnSelect ?? true) {
        flush();
        menu?.dismissCascade();
      }
    },
  });
  attachPrimitiveChildren(node, props);
  return node;
}

/**
 * Headless checkbox menu item (`role="menuItemCheckBox"`) exposing `checked`,
 * `data-state="checked" | "unchecked"`, `data-highlighted`, and `data-disabled`.
 */
export function MenuCheckboxItemPrimitive(props: MenuCheckboxItemPrimitiveProps = {}): NativeNode {
  const menu = useContext(MenuHierarchyContext);
  const rest = omit(
    props,
    "children",
    "checked",
    "defaultChecked",
    "onCheckedChange",
    "closeOnSelect",
  );
  const [read, write] = createControllable<boolean>(
    {
      value: () => props.checked,
      defaultValue: () => props.defaultChecked,
      onChange: () => props.onCheckedChange,
    },
    false,
  );
  const validate = (value: boolean): boolean => {
    if (typeof value !== "boolean") {
      throw new TypeError("MenuCheckboxItemPrimitive checked must be a boolean");
    }
    return value;
  };
  validate(untrack(read));
  const checked = memo(() => validate(read()));
  const node = createMenuRow(rest, {
    role: "menuItemCheckBox",
    disabled: () => !!props.disabled,
    overrides: {
      get checked() {
        return checked();
      },
      get "data-state"() {
        return checked() ? "checked" : "unchecked";
      },
    },
    select() {
      write(!checked());
      if (props.closeOnSelect ?? true) {
        flush();
        menu?.dismissCascade();
      }
    },
  });
  attachPrimitiveChildren(node, props);
  return node;
}

/**
 * Headless radio group (`role="group"`) for `MenuRadioItemPrimitive`.
 */
export function MenuRadioGroupPrimitive(props: MenuRadioGroupPrimitiveProps = {}): NativeNode {
  const rest = omit(props, "children", "value", "defaultValue", "onValueChange", "label");
  const [read, choose] = createControllable<string>(
    {
      value: () => props.value,
      defaultValue: () => props.defaultValue,
      onChange: () => props.onValueChange,
    },
    "",
  );
  const validate = (current: string): string => {
    if (typeof current !== "string") {
      throw new TypeError("MenuRadioGroupPrimitive value must be a string");
    }
    return current;
  };
  validate(untrack(read));
  const value = memo(() => validate(read()));
  const node = createElement("box");
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "group" as const,
          get ariaLabel() {
            return props.label ?? null;
          },
        },
        rest,
      ),
    ),
    true,
  );
  slot(node, provide(MenuRadioContext, { value, choose }, props));
  return node;
}

/**
 * Headless radio menu item (`role="menuItemRadio"`) exposing `checked`,
 * `data-state="checked" | "unchecked"`, `data-value`, `data-highlighted`, and `data-disabled`.
 */
export function MenuRadioItemPrimitive(props: MenuRadioItemPrimitiveProps): NativeNode {
  const menu = useContext(MenuHierarchyContext);
  const radio = useContext(MenuRadioContext);
  const rest = omit(props, "children", "value", "closeOnSelect");
  const validate = (current: string): string => {
    if (typeof current !== "string" || !current) {
      throw new TypeError("MenuRadioItemPrimitive value must be a non-empty string");
    }
    return current;
  };
  validate(props.value);
  const value = memo(() => validate(props.value));
  const checked = () => radio?.value() === value();
  const node = createMenuRow(rest, {
    role: "menuItemRadio",
    disabled: () => !!props.disabled,
    overrides: {
      get checked() {
        return checked();
      },
      get "data-state"() {
        return checked() ? "checked" : "unchecked";
      },
      get "data-value"() {
        return value();
      },
    },
    select() {
      radio?.choose(value());
      if (props.closeOnSelect ?? true) {
        flush();
        menu?.dismissCascade();
      }
    },
  });
  attachPrimitiveChildren(node, props);
  return node;
}
