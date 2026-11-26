import { $PROXY, createContext, createEffect, omit, untrack, useContext } from "#solid";
import { createElement, flush, memo, mergeProps, setProp, spread } from "../renderer/index.js";
import type {
  Accessor,
  NativeChild,
  NativeDismissReason,
  NativeElementProps,
  NativeKeyEvent,
  NativeNode,
  NativePointerBoundaryEvent,
  NativeRole,
} from "../types.js";
import {
  closeDeepestMenuSub,
  createMenuRow,
  createMenuTypeahead,
  MenuHierarchyContext,
  MenuRadioContext,
  MenuRootPrimitive,
  MenuSubPrimitive,
  MenuSurfaceContext,
  provideMenuHierarchyRoot,
  type MenuHierarchy,
} from "../primitives/menu.js";
import { anchoredSurface } from "./overlay.js";
import type { AnchoredContentProps, OverlayRootProps, OverlayState } from "./overlay.js";
import { createRovingFocus } from "./roving.js";
import { createDelayed } from "./delay.js";
import { callHandler, createControllable } from "./state.js";
import {
  append,
  classes,
  element,
  forbidChildren,
  part,
  provide,
  slot,
  type Stylesheet,
  useStylesheet,
  withDefaults,
} from "./shared.js";
import { Icon } from "./icons.js";
import { CHECK, DOT } from "./generated/icons.js";
import { sheet as menuSheet } from "./generated/MenuStyle.js";
import { sheet as menubarSheet } from "./generated/MenubarStyle.js";

export type { MenuHierarchy };

export type MenuContentProps = AnchoredContentProps;
export type MenuTypeaheadProps = NativeElementProps;
export type MenuShortcutProps = NativeElementProps;
export interface MenuLabelProps extends NativeElementProps {
  inset?: boolean;
}
export interface MenuGroupProps extends NativeElementProps {
  label?: string | null;
}
export interface MenuSeparatorProps extends Omit<NativeElementProps, "children"> {
  children?: never;
}
export interface MenuItemProps extends NativeElementProps {
  destructive?: boolean;
  inset?: boolean;
  closeOnSelect?: boolean;
  onSelect?: () => void;
  shortcut?: string | null;
}
export interface MenuCheckboxItemProps extends Omit<NativeElementProps, "checked"> {
  checked?: boolean;
  defaultChecked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
  closeOnSelect?: boolean;
}
export interface MenuRadioGroupProps extends MenuGroupProps {
  value?: string;
  defaultValue?: string;
  onValueChange?: (value: string) => void;
}
export interface MenuRadioItemProps extends NativeElementProps {
  value: string;
  closeOnSelect?: boolean;
}

export interface ContextMenuPoint {
  readonly x: number;
  readonly y: number;
}
export interface ContextMenuPlace {
  openAt(position: ContextMenuPoint): void;
}
export interface ContextMenuRecord {
  readonly place: ContextMenuPlace;
  readonly point: Accessor<ContextMenuPoint>;
  setArea(node: NativeNode | null): void;
}

interface MenuVariantConfig {
  readonly sheet: Stylesheet;
  readonly itemBaseClass: string;
  readonly checkClass: string;
  readonly indicatorClass: string;
  readonly indicatorDotClass: string;
  readonly wrapLabel: boolean;
  readonly allowModifiers: boolean;
  readonly checkboxDefaultClose: boolean;
  readonly checkboxName: string;
  readonly radioName: string;
  readonly shortcut: (props: MenuShortcutProps) => NativeNode;
}

const ContextMenuContext = createContext<ContextMenuRecord | null>(null);

export const MenuShortcutPart = part("text", menuSheet, "zui-menu__shortcut", { ariaHidden: true });
export const MenubarShortcutPart = part("text", menubarSheet, "zui-menubar__shortcut", {
  ariaHidden: true,
});

export const MENU_VARIANT: MenuVariantConfig = {
  sheet: menuSheet,
  itemBaseClass: "zui-menu__item",
  checkClass: "zui-menu__item--check",
  indicatorClass: "zui-menu__indicator",
  indicatorDotClass: "zui-menu__indicator--dot",
  wrapLabel: false,
  allowModifiers: true,
  checkboxDefaultClose: true,
  checkboxName: "MenuCheckboxItem",
  radioName: "MenuRadioItem",
  shortcut: MenuShortcutPart,
};

export const MENUBAR_VARIANT: MenuVariantConfig = {
  sheet: menubarSheet,
  itemBaseClass: "zui-menubar__item",
  checkClass: "zui-menubar__item--check",
  indicatorClass: "zui-menubar__indicator",
  indicatorDotClass: "zui-menubar__indicator--dot",
  wrapLabel: true,
  allowModifiers: false,
  checkboxDefaultClose: false,
  checkboxName: "MenubarCheckboxItem",
  radioName: "MenubarRadioItem",
  shortcut: MenubarShortcutPart,
};

export function useMenuHierarchy(): MenuHierarchy | undefined {
  return useContext(MenuHierarchyContext) ?? undefined;
}

export function useMenuState(): OverlayState | undefined {
  return useContext(MenuHierarchyContext)?.state;
}

export function useMenuSub():
  | { readonly state: OverlayState; readonly closing: ReturnType<typeof createDelayed> }
  | undefined {
  return useContext(MenuHierarchyContext)?.sub;
}

export function provideContextMenu(
  record: ContextMenuRecord,
  source: { readonly children?: NativeChild } | (() => NativeChild),
): NativeChild {
  return provide(ContextMenuContext, record, source);
}

export function useContextMenuRecord(): ContextMenuRecord | undefined {
  return useContext(ContextMenuContext) ?? undefined;
}

export function provideMenuRoot(
  state: OverlayState,
  onDismiss: () => void,
  source: { readonly children?: NativeChild } | (() => NativeChild),
): NativeChild {
  return provideMenuHierarchyRoot(state, onDismiss, source);
}

export function menuRoot(props: OverlayRootProps): NativeChild {
  return MenuRootPrimitive(props);
}

export function provideMenuSub(props: OverlayRootProps): NativeChild {
  return MenuSubPrimitive(props);
}

export function useMenuTypeahead(): (event: NativeKeyEvent) => void {
  return createMenuTypeahead(useContext(MenuSurfaceContext)?.roving);
}

export function renderMenuSurface(
  state: OverlayState | undefined,
  props: MenuContentProps,
  options: {
    trap?: boolean;
    class?: string;
    surfaceClass?: boolean;
    onDismiss?: (reason: NativeDismissReason) => void;
    onBarKeyDown?: (event: NativeKeyEvent) => void;
    extraSheet?: Stylesheet;
  } = {},
): NativeChild {
  if (options.extraSheet) useStylesheet(options.extraSheet);
  useStylesheet(menuSheet);
  const hierarchy = useContext(MenuHierarchyContext);
  const trap = options.trap ?? true;
  const rest = omit(props, "children");
  return anchoredSurface(state, rest, {
    class: options.class ?? `${menuSheet.className} zui-menu`,
    role: "menu",
    trap,
    surfaceClass: options.surfaceClass,
    onDismiss(reason) {
      if (reason === "escape" && closeDeepestMenuSub(hierarchy, "escape")) {
        return;
      }
      if (options.onDismiss) options.onDismiss(reason);
      else state?.setOpen(false);
    },
    children() {
      const roving = createRovingFocus(() => "vertical", "active");
      if (!trap) {
        let focused = false;
        createEffect(
          () => (focused ? undefined : roving.firstEnabled()),
          first => {
            if (!focused && first && roving.focusFirst()) focused = true;
          },
        );
      }
      const onTypeahead = createMenuTypeahead(roving);
      const keys = createElement("box");
      spread(
        keys,
        {
          class: "zui-menu__keys",
          onKeyDown(event: NativeKeyEvent) {
            if (trap) onTypeahead(event);
            else if (callHandler(event, roving.onKeyDown)) onTypeahead(event);
          },
        },
        true,
      );
      const surfaceContent = provide(
        MenuSurfaceContext,
        { node: state!.content.node, roving },
        props,
      );
      if (options.onBarKeyDown) {
        const arrows = createElement("box");
        spread(arrows, { class: "zui-menubar__arrows", onKeyDown: options.onBarKeyDown }, true);
        append(keys, arrows);
        slot(arrows, surfaceContent);
        return keys;
      }
      const list = createElement("box");
      spread(
        list,
        { class: "zui-menu__list", "data-orientation": "vertical", onKeyDown: roving.onKeyDown },
        true,
      );
      append(list, keys);
      slot(keys, surfaceContent);
      return list;
    },
  });
}

export function menuRow(
  props: NativeElementProps,
  options: {
    role: NativeRole;
    class: () => string;
    readonly select: () => void;
    disabled?: () => boolean;
    extra?: NativeElementProps;
    overrides?: NativeElementProps;
    sheet?: Stylesheet;
    baseClass?: string;
    activateOnPress?: boolean;
    enter?: (activate: () => void) => void;
    leave?: (event: NativePointerBoundaryEvent, defocus: () => void) => void;
  },
): NativeNode {
  useStylesheet(options.sheet ?? menuSheet);
  return createMenuRow(props, {
    role: options.role,
    select: options.select,
    disabled: options.disabled,
    extra: options.extra,
    overrides: options.overrides,
    activateOnPress: options.activateOnPress,
    emitDataHooks: false,
    enter: options.enter,
    leave: options.leave,
    classGetter: () => classes(options.baseClass ?? "zui-menu__item", options.class()),
  });
}

function attachItemContent(
  node: NativeNode,
  variant: MenuVariantConfig,
  props: { readonly children?: NativeChild },
): void {
  let target = node;
  if (variant.wrapLabel) {
    target = createElement("box");
    setProp(target, "class", "zui-menubar__item-label");
    append(node, target);
  }
  if (!($PROXY in props) && Object.getOwnPropertyDescriptor(props, "children")?.get === undefined) {
    slot(target, props.children);
    return;
  }
  slot(
    target,
    memo(() => props.children),
  );
}

function menuIndicator(
  baseClass: string,
  dotClass: string,
  checked: Accessor<boolean>,
  dot: boolean,
): NativeNode {
  const box = createElement("box");
  spread(box, { class: classes(baseClass, dot && dotClass) }, true);
  const show = memo(checked);
  slot(box, () => (show() ? Icon({ icon: dot ? DOT : CHECK }) : null));
  return box;
}

export function buildMenuItem(variant: MenuVariantConfig, props: MenuItemProps = {}): NativeNode {
  const menu = useMenuHierarchy();
  if (!variant.allowModifiers) useStylesheet(menuSheet);
  const rest = omit(
    props,
    "children",
    "destructive",
    "inset",
    "closeOnSelect",
    "onSelect",
    "shortcut",
  );
  const extra: NativeElementProps = variant.allowModifiers
    ? {
        get "data-destructive"() {
          return props.destructive ? "true" : null;
        },
        get "data-inset"() {
          return props.inset ? "true" : null;
        },
        get ariaKeyShortcuts() {
          return props.shortcut ?? null;
        },
      }
    : {
        get "data-destructive"() {
          return props.destructive ? "true" : null;
        },
        get ariaKeyShortcuts() {
          return props.shortcut ?? null;
        },
      };
  const node = menuRow(rest, {
    role: "menuItem",
    disabled: () => !!props.disabled,
    extra,
    sheet: variant.sheet,
    baseClass: variant.itemBaseClass,
    class: () =>
      variant.allowModifiers
        ? classes(
            props.destructive && "zui-menu__item--destructive",
            props.inset && "zui-menu__item--inset",
          )
        : classes(props.destructive && "zui-menu__item--destructive"),
    select() {
      props.onSelect?.();
      if (props.closeOnSelect ?? true) {
        flush();
        menu?.dismissCascade();
      }
    },
  });
  attachItemContent(node, variant, props);
  if (!($PROXY in props) && Object.getOwnPropertyDescriptor(props, "shortcut")?.get === undefined) {
    if (props.shortcut != null) append(node, variant.shortcut({ children: props.shortcut }));
  } else {
    const hasShortcut = memo(() => props.shortcut != null);
    slot(node, () =>
      hasShortcut()
        ? variant.shortcut({
            get children() {
              return props.shortcut;
            },
          })
        : null,
    );
  }
  return node;
}

function validateMenuChecked(variant: MenuVariantConfig, value: boolean): boolean {
  if (typeof value !== "boolean")
    throw new TypeError(`${variant.checkboxName} checked must be a boolean`);
  return value;
}
function validateMenuRadioGroupValue(current: string): string {
  if (typeof current !== "string") throw new TypeError("MenuRadioGroup value must be a string");
  return current;
}
function validateMenuRadioValue(variant: MenuVariantConfig, current: string): string {
  if (typeof current !== "string" || !current)
    throw new TypeError(`${variant.radioName} value must be a non-empty string`);
  return current;
}

export function buildMenuCheckboxItem(
  variant: MenuVariantConfig,
  props: MenuCheckboxItemProps = {},
): NativeNode {
  const menu = useMenuHierarchy();
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
  validateMenuChecked(variant, untrack(read));
  const checked = memo(() => validateMenuChecked(variant, read()));
  const node = menuRow(rest, {
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
    sheet: variant.sheet,
    baseClass: variant.itemBaseClass,
    class: () => variant.checkClass,
    select() {
      write(!checked());
      if (props.closeOnSelect ?? variant.checkboxDefaultClose) {
        flush();
        menu?.dismissCascade();
      }
    },
  });
  append(node, menuIndicator(variant.indicatorClass, variant.indicatorDotClass, checked, false));
  attachItemContent(node, variant, props);
  return node;
}

export function buildMenuRadioGroup(props: MenuRadioGroupProps = {}): NativeNode {
  useStylesheet(menuSheet);
  const rest = omit(props, "children", "value", "defaultValue", "onValueChange", "label");
  const [read, choose] = createControllable<string>(
    {
      value: () => props.value,
      defaultValue: () => props.defaultValue,
      onChange: () => props.onValueChange,
    },
    "",
  );
  validateMenuRadioGroupValue(untrack(read));
  const value = memo(() => validateMenuRadioGroupValue(read()));
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
      {
        get __zguiClass() {
          return classes("zui-menu__group");
        },
      },
    ),
    true,
  );
  slot(node, provide(MenuRadioContext, { value, choose }, props));
  return node;
}

export function buildMenuRadioItem(
  variant: MenuVariantConfig,
  props: MenuRadioItemProps,
): NativeNode {
  const menu = useMenuHierarchy(),
    radio = useContext(MenuRadioContext);
  const rest = omit(props, "children", "value", "closeOnSelect");
  validateMenuRadioValue(
    variant,
    untrack(() => props.value),
  );
  const value = memo(() => validateMenuRadioValue(variant, props.value));
  const checked = () => radio?.value() === value();
  const node = menuRow(rest, {
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
    sheet: variant.sheet,
    baseClass: variant.itemBaseClass,
    class: () => variant.checkClass,
    select() {
      radio?.choose(value());
      if (props.closeOnSelect ?? true) {
        flush();
        menu?.dismissCascade();
      }
    },
  });
  append(node, menuIndicator(variant.indicatorClass, variant.indicatorDotClass, checked, true));
  attachItemContent(node, variant, props);
  return node;
}

export function buildMenuSeparator(
  name: string,
  sheet: Stylesheet,
  className: string,
  props: MenuSeparatorProps = {},
): NativeNode {
  useStylesheet(sheet);
  forbidChildren(name, props);
  return element(
    "box",
    mergeProps(props, withDefaults({ ariaHidden: true }, props), {
      get __zguiClass() {
        return classes(className);
      },
    }),
  );
}
