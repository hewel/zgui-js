import { $PROXY, createContext, createEffect, onCleanup, omit, useContext } from "#solid";
import { createElement, flush, memo, mergeProps, setProp, spread } from "../renderer/index.js";
import { focus, getTextContent } from "../nodes.js";
import type {
  Accessor, NativeChild, NativeDismissReason, NativeElementProps, NativeEvent, NativeFocusEvent,
  NativeKeyEvent, NativeNode, NativePointerBoundaryEvent, NativePointerDownEvent, NativeRole,
} from "../types.js";
import { anchoredSurface, createOverlayState, OverlayContext } from "./overlay.js";
import type { AnchoredContentProps, OverlayRootProps, OverlayState } from "./overlay.js";
import { createRovingFocus } from "./roving.js";
import { createDelayed } from "./delay.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, element, forbidChildren, part, provide, slot, type Stylesheet, useStylesheet, withDefaults } from "./shared.js";
import { Icon } from "./icons.js";
import { CHECK, DOT } from "./generated/icons.js";
import { sheet as menuSheet } from "./generated/MenuStyle.js";
import { sheet as menubarSheet } from "./generated/MenubarStyle.js";

export type MenuContentProps = AnchoredContentProps;
export type MenuTypeaheadProps = NativeElementProps;
export type MenuShortcutProps = NativeElementProps;
export interface MenuLabelProps extends NativeElementProps { inset?: boolean; }
export interface MenuGroupProps extends NativeElementProps { label?: string | null; }
export interface MenuSeparatorProps extends Omit<NativeElementProps, "children"> { children?: never; }
export interface MenuItemProps extends NativeElementProps {
  destructive?: boolean; inset?: boolean; closeOnSelect?: boolean; onSelect?: () => void; shortcut?: string | null;
}
export interface MenuCheckboxItemProps extends Omit<NativeElementProps, "checked"> {
  checked?: boolean; defaultChecked?: boolean; onCheckedChange?: (checked: boolean) => void; closeOnSelect?: boolean;
}
export interface MenuRadioGroupProps extends MenuGroupProps {
  value?: string; defaultValue?: string; onValueChange?: (value: string) => void;
}
export interface MenuRadioItemProps extends NativeElementProps { value: string; closeOnSelect?: boolean; }

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

export interface MenuHierarchy {
  readonly state: OverlayState;
  dismissCascade(): void;
  readonly sub?: { readonly state: OverlayState; readonly closing: ReturnType<typeof createDelayed> };
}

interface MenuSurfaceContextValue {
  readonly node: Accessor<NativeNode | null>;
  readonly roving: ReturnType<typeof createRovingFocus>;
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

const MenuHierarchyContext = createContext<MenuHierarchy | null>(null);
const MenuSurfaceContext = createContext<MenuSurfaceContextValue | null>(null);
const MenuRadioContext = createContext<{ value: Accessor<string>; choose(value: string): void } | null>(null);
const ContextMenuContext = createContext<ContextMenuRecord | null>(null);

export const MenuShortcutPart = part("text", menuSheet, "zui-menu__shortcut", { ariaHidden: true });
export const MenubarShortcutPart = part("text", menubarSheet, "zui-menubar__shortcut", { ariaHidden: true });

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

export function useMenuSub(): { readonly state: OverlayState; readonly closing: ReturnType<typeof createDelayed> } | undefined {
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
  let alive = true;
  onCleanup(() => { alive = false; });
  const dismiss = () => { if (alive) onDismiss(); };
  return provide(MenuHierarchyContext, { state, dismissCascade: dismiss }, source);
}

export function menuRoot(props: OverlayRootProps): NativeChild {
  const state = createOverlayState(props);
  return provide(OverlayContext, state, () => provideMenuRoot(state, () => state.setOpen(false), props));
}

export function provideMenuSub(props: OverlayRootProps): NativeChild {
  const state = createOverlayState(props);
  const closing = createDelayed();
  const parent = useContext(MenuHierarchyContext);
  let alive = true;
  onCleanup(() => { alive = false; closing.cancel(); });
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

function createTypeaheadHandler(group: ReturnType<typeof createRovingFocus> | undefined) {
  const reset = createDelayed();
  let buffer = "", alive = true;
  onCleanup(() => { alive = false; reset.cancel(); buffer = ""; });
  return (event: NativeKeyEvent) => {
    if (!alive || !group || event.character == null) return;
    if (event.character === " " && buffer === "") return;
    buffer += event.character.toLowerCase();
    reset.after(1000, () => { buffer = ""; });
    const characters = Array.from(buffer);
    const search = characters.length > 1 && characters.every(character => character === characters[0]) ? characters[0] : buffer;
    if (search && group.focusNext(node => getTextContent(node).replace(/^\p{White_Space}+/u, "").toLowerCase().startsWith(search))) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
}

export function useMenuTypeahead(): (event: NativeKeyEvent) => void {
  return createTypeaheadHandler(useContext(MenuSurfaceContext)?.roving);
}

export function renderMenuSurface(state: OverlayState | undefined, props: MenuContentProps, options: {
  trap?: boolean;
  class?: string;
  surfaceClass?: boolean;
  onDismiss?: (reason: NativeDismissReason) => void;
  onBarKeyDown?: (event: NativeKeyEvent) => void;
  extraSheet?: Stylesheet;
} = {}): NativeChild {
  if (options.extraSheet) useStylesheet(options.extraSheet);
  useStylesheet(menuSheet);
  const trap = options.trap ?? true;
  const rest = omit(props, "children");
  return anchoredSurface(state, rest, {
    class: options.class ?? `${menuSheet.className} zui-menu`,
    role: "menu",
    trap,
    surfaceClass: options.surfaceClass,
    onDismiss: options.onDismiss,
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
      const onTypeahead = createTypeaheadHandler(roving);
      const keys = createElement("box");
      spread(keys, {
        class: "zui-menu__keys",
        onKeyDown(event: NativeKeyEvent) {
          if (trap) onTypeahead(event);
          else if (callHandler(event, roving.onKeyDown)) onTypeahead(event);
        },
      }, true);
      const surfaceContent = provide(MenuSurfaceContext, { node: state!.content.node, roving }, props);
      if (options.onBarKeyDown) {
        const arrows = createElement("box");
        spread(arrows, { class: "zui-menubar__arrows", onKeyDown: options.onBarKeyDown }, true);
        append(keys, arrows);
        slot(arrows, surfaceContent);
        return keys;
      }
      const list = createElement("box");
      spread(list, { class: "zui-menu__list", "data-orientation": "vertical", onKeyDown: roving.onKeyDown }, true);
      append(list, keys);
      slot(keys, surfaceContent);
      return list;
    },
  });
}

export function menuRow(props: NativeElementProps, options: {
  role: NativeRole;
  class: () => string;
  select(): void;
  disabled?: () => boolean;
  extra?: NativeElementProps;
  overrides?: NativeElementProps;
  sheet?: Stylesheet;
  baseClass?: string;
  activateOnPress?: boolean;
  enter?: (activate: () => void) => void;
  leave?: (event: NativePointerBoundaryEvent, defocus: () => void) => void;
}): NativeNode {
  useStylesheet(options.sheet ?? menuSheet);
  const surface = useContext(MenuSurfaceContext);
  const rest = omit(props, "children", "ref", "onClick", "onPointerDown", "onPointerEnter", "onPointerLeave", "onFocus", "onBlur");
  const node = createElement("control");
  const disabled = options.disabled ?? (() => !!props.disabled);
  const item = surface?.roving.register(node, disabled);
  let within = false, alive = true;
  onCleanup(() => { alive = false; within = false; });
  const activate = () => { item?.activate(); focus(node); };
  const defocus = () => {
    if (!within) return;
    const target = surface?.node();
    if (target) focus(target);
  };
  spread(node, mergeProps(
    rest,
    ...(options.extra ? [withDefaults(options.extra, rest)] : []),
    withDefaults({
      role: options.role,
      get tabIndex() { return item?.tabIndex() ?? (disabled() ? -1 : 0); },
    }, rest),
    ...(options.overrides ? [options.overrides] : []),
    {
      get __zguiClass() { return classes(options.baseClass ?? "zui-menu__item", options.class()); },
      ref(value: NativeNode) { if (typeof props.ref === "function") props.ref(value); },
      onClick(event: NativeEvent) { if (callHandler(event, props.onClick) && alive && !disabled()) options.select(); },
      onPointerDown(event: NativePointerDownEvent) {
        if (callHandler(event, props.onPointerDown) && alive && !disabled() && event.button === "primary" && (options.activateOnPress ?? true)) event.activateNow();
      },
      onPointerEnter(event: NativePointerBoundaryEvent) {
        if (callHandler(event, props.onPointerEnter) && alive && !disabled()) {
          if (options.enter) options.enter(activate); else activate();
        }
      },
      onPointerLeave(event: NativePointerBoundaryEvent) {
        if (!callHandler(event, props.onPointerLeave) || !alive) return;
        if (options.leave) options.leave(event, defocus); else defocus();
      },
      onFocus(event: NativeFocusEvent) {
        within = true;
        if (callHandler(event, props.onFocus) && alive && !disabled()) item?.activate();
      },
      onBlur(event: NativeFocusEvent) { within = false; props.onBlur?.(event); },
    },
  ), true);
  return node;
}

function attachItemContent(node: NativeNode, variant: MenuVariantConfig, props: { readonly children?: NativeChild }): void {
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
  slot(target, memo(() => props.children));
}

function menuIndicator(baseClass: string, dotClass: string, checked: Accessor<boolean>, dot: boolean): NativeNode {
  const box = createElement("box");
  spread(box, { class: classes(baseClass, dot && dotClass) }, true);
  const show = memo(checked);
  slot(box, () => show() ? Icon({ icon: dot ? DOT : CHECK }) : null);
  return box;
}

export function buildMenuItem(variant: MenuVariantConfig, props: MenuItemProps = {}): NativeNode {
  const menu = useMenuHierarchy();
  if (!variant.allowModifiers) useStylesheet(menuSheet);
  const rest = omit(props, "children", "destructive", "inset", "closeOnSelect", "onSelect", "shortcut");
  const extra: NativeElementProps = variant.allowModifiers
    ? {
        get "data-destructive"() { return props.destructive ? "true" : null; },
        get "data-inset"() { return props.inset ? "true" : null; },
        get ariaKeyShortcuts() { return props.shortcut ?? null; },
      }
    : {
        get "data-destructive"() { return props.destructive ? "true" : null; },
        get ariaKeyShortcuts() { return props.shortcut ?? null; },
      };
  const node = menuRow(rest, {
    role: "menuItem",
    disabled: () => !!props.disabled,
    extra,
    sheet: variant.sheet,
    baseClass: variant.itemBaseClass,
    class: () => variant.allowModifiers
      ? classes(props.destructive && "zui-menu__item--destructive", props.inset && "zui-menu__item--inset")
      : classes(props.destructive && "zui-menu__item--destructive"),
    select() { props.onSelect?.(); if (props.closeOnSelect ?? true) { flush(); menu?.dismissCascade(); } },
  });
  attachItemContent(node, variant, props);
  if (!($PROXY in props) && Object.getOwnPropertyDescriptor(props, "shortcut")?.get === undefined) {
    if (props.shortcut != null) append(node, variant.shortcut({ children: props.shortcut }));
  } else {
    const hasShortcut = memo(() => props.shortcut != null);
    slot(node, () => hasShortcut() ? variant.shortcut({ get children() { return props.shortcut; } }) : null);
  }
  return node;
}

function validateMenuChecked(variant: MenuVariantConfig, value: boolean): boolean {
  if (typeof value !== "boolean") throw new TypeError(`${variant.checkboxName} checked must be a boolean`);
  return value;
}
function validateMenuRadioGroupValue(current: string): string {
  if (typeof current !== "string") throw new TypeError("MenuRadioGroup value must be a string");
  return current;
}
function validateMenuRadioValue(variant: MenuVariantConfig, current: string): string {
  if (typeof current !== "string" || !current) throw new TypeError(`${variant.radioName} value must be a non-empty string`);
  return current;
}

export function buildMenuCheckboxItem(variant: MenuVariantConfig, props: MenuCheckboxItemProps = {}): NativeNode {
  const menu = useMenuHierarchy();
  const rest = omit(props, "children", "checked", "defaultChecked", "onCheckedChange", "closeOnSelect");
  const [read, write] = createControllable<boolean>({
    value: () => props.checked, defaultValue: () => props.defaultChecked, onChange: () => props.onCheckedChange,
  }, false);
  validateMenuChecked(variant, read());
  const checked = memo(() => validateMenuChecked(variant, read()));
  const node = menuRow(rest, {
    role: "menuItemCheckBox",
    disabled: () => !!props.disabled,
    overrides: {
      get checked() { return checked(); },
      get "data-state"() { return checked() ? "checked" : "unchecked"; },
    },
    sheet: variant.sheet,
    baseClass: variant.itemBaseClass,
    class: () => variant.checkClass,
    select() { write(!checked()); if (props.closeOnSelect ?? variant.checkboxDefaultClose) { flush(); menu?.dismissCascade(); } },
  });
  append(node, menuIndicator(variant.indicatorClass, variant.indicatorDotClass, checked, false));
  attachItemContent(node, variant, props);
  return node;
}

export function buildMenuRadioGroup(props: MenuRadioGroupProps = {}): NativeNode {
  useStylesheet(menuSheet);
  const rest = omit(props, "children", "value", "defaultValue", "onValueChange", "label");
  const [read, choose] = createControllable<string>({
    value: () => props.value, defaultValue: () => props.defaultValue, onChange: () => props.onValueChange,
  }, "");
  validateMenuRadioGroupValue(read());
  const value = memo(() => validateMenuRadioGroupValue(read()));
  const node = createElement("box");
  spread(node, mergeProps(rest, withDefaults({ role: "group" as const, get ariaLabel() { return props.label ?? null; } }, rest), {
    get __zguiClass() { return classes("zui-menu__group"); },
  }), true);
  slot(node, provide(MenuRadioContext, { value, choose }, props));
  return node;
}

export function buildMenuRadioItem(variant: MenuVariantConfig, props: MenuRadioItemProps): NativeNode {
  const menu = useMenuHierarchy(), radio = useContext(MenuRadioContext);
  const rest = omit(props, "children", "value", "closeOnSelect");
  validateMenuRadioValue(variant, props.value);
  const value = memo(() => validateMenuRadioValue(variant, props.value));
  const checked = () => radio?.value() === value();
  const node = menuRow(rest, {
    role: "menuItemRadio",
    disabled: () => !!props.disabled,
    overrides: {
      get checked() { return checked(); },
      get "data-state"() { return checked() ? "checked" : "unchecked"; },
      get "data-value"() { return value(); },
    },
    sheet: variant.sheet,
    baseClass: variant.itemBaseClass,
    class: () => variant.checkClass,
    select() { radio?.choose(value()); if (props.closeOnSelect ?? true) { flush(); menu?.dismissCascade(); } },
  });
  append(node, menuIndicator(variant.indicatorClass, variant.indicatorDotClass, checked, true));
  attachItemContent(node, variant, props);
  return node;
}

export function buildMenuSeparator(name: string, sheet: Stylesheet, className: string, props: MenuSeparatorProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren(name, props);
  return element("box", mergeProps(props, withDefaults({ ariaHidden: true }, props), { get __zguiClass() { return classes(className); } }));
}
