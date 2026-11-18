import { Show, createContext, createEffect, onCleanup, splitProps, useContext } from "#solid";
import { createElement, memo, mergeProps, spread } from "../renderer/index.js";
import { focus, getTextContent } from "../nodes.js";
import type {
  Accessor, NativeChild, NativeDismissReason, NativeElementProps, NativeEvent, NativeFocusEvent,
  NativeKeyEvent, NativeNode, NativePointerBoundaryEvent, NativePointerDownEvent, NativeRole,
} from "../types.js";
import { anchoredSurface, createOverlayState, OverlayContext, OverlayRoot } from "./overlay.js";
import type { AnchoredContentProps, OverlayRootProps, OverlayState } from "./overlay.js";
import { createRovingFocus } from "./roving.js";
import { createDelayed } from "./delay.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, element, forbidChildren, part, provide, slot, type Stylesheet, useStylesheet } from "./shared.js";
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

export interface MenuHierarchy {
  readonly state: OverlayState;
  dismissRoot(): void;
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
  readonly checkboxDefaultClose: boolean;
  readonly checkboxName: string;
  readonly radioName: string;
  readonly shortcut: (props: MenuShortcutProps) => NativeNode;
}

const MenuHierarchyContext = createContext<MenuHierarchy>();
const MenuSurfaceContext = createContext<MenuSurfaceContextValue>();
const MenuRadioContext = createContext<{ value: Accessor<string>; choose(value: string): void }>();

export const MenuShortcutPart = part("text", menuSheet, "zui-menu__shortcut", { ariaHidden: true });
export const MenubarShortcutPart = part("text", menubarSheet, "zui-menubar__shortcut", { ariaHidden: true });

export const MENU_VARIANT: MenuVariantConfig = {
  sheet: menuSheet,
  itemBaseClass: "zui-menu__item",
  checkClass: "zui-menu__item--check",
  indicatorClass: "zui-menu__indicator",
  indicatorDotClass: "zui-menu__indicator--dot",
  wrapLabel: false,
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
  checkboxDefaultClose: false,
  checkboxName: "MenubarCheckboxItem",
  radioName: "MenubarRadioItem",
  shortcut: MenubarShortcutPart,
};

export function useMenuHierarchy(): MenuHierarchy | undefined {
  return useContext(MenuHierarchyContext);
}

export function useMenuState(): OverlayState | undefined {
  return useContext(MenuHierarchyContext)?.state;
}

export function useMenuSub(): { readonly state: OverlayState; readonly closing: ReturnType<typeof createDelayed> } | undefined {
  return useContext(MenuHierarchyContext)?.sub;
}

export function provideMenuRoot(
  state: OverlayState,
  onDismiss: () => void,
  source: { readonly children?: NativeChild } | (() => NativeChild),
): NativeChild {
  let alive = true;
  onCleanup(() => { alive = false; });
  const dismiss = () => { if (alive) onDismiss(); };
  return provide(MenuHierarchyContext, { state, dismissRoot: dismiss, dismissCascade: dismiss }, source);
}

export function menuRoot(props: OverlayRootProps): NativeChild {
  return OverlayRoot(mergeProps(props, { get children() {
    const state = useContext(OverlayContext)!;
    return provideMenuRoot(state, () => state.setOpen(false), props);
  } }));
}

export function provideMenuSub(props: OverlayRootProps): NativeChild {
  const state = createOverlayState(props);
  const closing = createDelayed();
  const parent = useContext(MenuHierarchyContext);
  let alive = true;
  onCleanup(() => { alive = false; closing.cancel(); });
  const closeSelf = () => {
    if (!alive) return;
    closing.cancel();
    state.setOpen(false);
  };
  const node: MenuHierarchy = {
    state: parent?.state ?? state,
    dismissRoot() {
      if (parent) parent.dismissRoot();
      else closeSelf();
    },
    dismissCascade() {
      if (!alive) return;
      closeSelf();
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

export function buildMenuTypeahead(props: MenuTypeaheadProps = {}): NativeNode {
  useStylesheet(menuSheet);
  const group = useContext(MenuSurfaceContext)?.roving;
  const onTypeahead = createTypeaheadHandler(group);
  const [own, rest] = splitProps(props, ["children", "onKeyDown"]);
  const node = createElement("box");
  spread(node, mergeProps(rest, {
    get __zguiClass() { return classes("zui-menu__keys"); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, own.onKeyDown)) onTypeahead(event);
    },
  }), true);
  slot(node, () => own.children);
  return node;
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
  const [own, rest] = splitProps(props, ["children"]);
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
        createEffect(() => {
          if (focused) return;
          if (roving.focusFirst()) focused = true;
        });
      }
      return provide(MenuSurfaceContext, { node: state!.content.node, roving }, () => {
        const onTypeahead = createTypeaheadHandler(roving);
        const keys = createElement("box");
        spread(keys, {
          class: "zui-menu__keys",
          onKeyDown(event: NativeKeyEvent) {
            if (trap) onTypeahead(event);
            else if (callHandler(event, roving.onKeyDown)) onTypeahead(event);
          },
        }, true);
        if (options.onBarKeyDown) {
          const arrows = createElement("box");
          spread(arrows, { class: "zui-menubar__arrows", onKeyDown: options.onBarKeyDown }, true);
          append(keys, arrows);
          slot(arrows, () => own.children);
          return keys;
        }
        const list = createElement("box");
        spread(list, { class: "zui-menu__list", "data-orientation": "vertical", onKeyDown: roving.onKeyDown }, true);
        append(list, keys);
        slot(keys, () => own.children);
        return list;
      });
    },
  });
}

export function menuRow(props: NativeElementProps, options: {
  role: NativeRole;
  class: () => string;
  select(): void;
  sheet?: Stylesheet;
  baseClass?: string;
  activateOnPress?: boolean;
  enter?: (activate: () => void) => void;
  leave?: (event: NativePointerBoundaryEvent, defocus: () => void) => void;
}): NativeNode {
  useStylesheet(options.sheet ?? menuSheet);
  const surface = useContext(MenuSurfaceContext);
  const [own, rest] = splitProps(props, ["children", "ref", "onClick", "onPointerDown", "onPointerEnter", "onPointerLeave", "onFocus", "onBlur"]);
  const node = createElement("control");
  const disabled = () => !!props.disabled;
  const item = surface?.roving.register(node, disabled);
  let within = false, alive = true;
  onCleanup(() => { alive = false; within = false; });
  const activate = () => { item?.activate(); focus(node); };
  const defocus = () => {
    if (!within) return;
    const target = surface?.node();
    if (target) focus(target);
  };
  spread(node, mergeProps({ role: options.role, get tabIndex() { return item?.tabIndex() ?? (disabled() ? -1 : 0); } }, rest, {
    get __zguiClass() { return classes(options.baseClass ?? "zui-menu__item", options.class()); },
    ref(value: NativeNode) { if (typeof own.ref === "function") own.ref(value); },
    onClick(event: NativeEvent) { if (callHandler(event, own.onClick) && alive && !disabled()) options.select(); },
    onPointerDown(event: NativePointerDownEvent) {
      if (callHandler(event, own.onPointerDown) && alive && !disabled() && event.button === "primary" && (options.activateOnPress ?? true)) event.activateNow();
    },
    onPointerEnter(event: NativePointerBoundaryEvent) {
      if (callHandler(event, own.onPointerEnter) && alive && !disabled()) {
        if (options.enter) options.enter(activate); else activate();
      }
    },
    onPointerLeave(event: NativePointerBoundaryEvent) {
      if (!callHandler(event, own.onPointerLeave) || !alive) return;
      if (options.leave) options.leave(event, defocus); else defocus();
    },
    onFocus(event: NativeFocusEvent) {
      within = true;
      if (callHandler(event, own.onFocus) && alive && !disabled()) item?.activate();
    },
    onBlur(event: NativeFocusEvent) { within = false; own.onBlur?.(event); },
  }), true);
  return node;
}

function attachItemContent(node: NativeNode, variant: MenuVariantConfig, children: () => NativeChild): void {
  if (!variant.wrapLabel) {
    slot(node, memo(children));
    return;
  }
  const box = createElement("box");
  spread(box, { class: "zui-menubar__item-label" }, true);
  slot(box, memo(children));
  append(node, box);
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
  const [own, rest] = splitProps(props, ["children", "destructive", "inset", "closeOnSelect", "onSelect", "shortcut"]);
  const allowModifiers = !variant.wrapLabel;
  const modifiers = allowModifiers ? {
    get "data-destructive"() { return own.destructive ? "true" : null; },
    get "data-inset"() { return own.inset ? "true" : null; },
  } : {};
  const node = menuRow(mergeProps(modifiers, {
    get ariaKeyShortcuts() { return own.shortcut ?? null; },
  }, rest), {
    role: "menuItem",
    sheet: variant.sheet,
    baseClass: variant.itemBaseClass,
    class: () => allowModifiers
      ? classes(own.destructive && "zui-menu__item--destructive", own.inset && "zui-menu__item--inset")
      : "",
    select() { own.onSelect?.(); if (own.closeOnSelect ?? true) menu?.dismissCascade(); },
  });
  attachItemContent(node, variant, () => own.children);
  slot(node, Show({ get when() { return own.shortcut != null; }, get children() {
    return variant.shortcut({ get children() { return own.shortcut; } });
  } }));
  return node;
}

export function buildMenuCheckboxItem(variant: MenuVariantConfig, props: MenuCheckboxItemProps = {}): NativeNode {
  const menu = useMenuHierarchy();
  const [own, rest] = splitProps(props, ["children", "checked", "defaultChecked", "onCheckedChange", "closeOnSelect"]);
  const [read, write] = createControllable<boolean>({
    value: () => own.checked, defaultValue: () => own.defaultChecked, onChange: () => own.onCheckedChange,
  }, false);
  const checked = memo(() => {
    const value = read();
    if (typeof value !== "boolean") throw new TypeError(`${variant.checkboxName} checked must be a boolean`);
    return value;
  });
  const node = menuRow(mergeProps(rest, {
    get checked() { return checked(); },
    get "data-state"() { return checked() ? "checked" : "unchecked"; },
  }), {
    role: "menuItemCheckBox",
    sheet: variant.sheet,
    baseClass: variant.itemBaseClass,
    class: () => variant.checkClass,
    select() { write(!checked()); if (own.closeOnSelect ?? variant.checkboxDefaultClose) menu?.dismissCascade(); },
  });
  append(node, menuIndicator(variant.indicatorClass, variant.indicatorDotClass, checked, false));
  attachItemContent(node, variant, () => own.children);
  return node;
}

export function buildMenuRadioGroup(props: MenuRadioGroupProps = {}): NativeNode {
  useStylesheet(menuSheet);
  const [own, rest] = splitProps(props, ["children", "value", "defaultValue", "onValueChange", "label"]);
  const [read, choose] = createControllable<string>({
    value: () => own.value, defaultValue: () => own.defaultValue, onChange: () => own.onValueChange,
  }, "");
  const value = memo(() => {
    const current = read();
    if (typeof current !== "string") throw new TypeError("MenuRadioGroup value must be a string");
    return current;
  });
  const node = createElement("box");
  spread(node, mergeProps({ role: "group" as const, get ariaLabel() { return own.label ?? null; } }, rest, {
    get __zguiClass() { return classes("zui-menu__group"); },
  }), true);
  provide(MenuRadioContext, { value, choose }, () => {
    slot(node, () => own.children);
    return null;
  });
  return node;
}

export function buildMenuRadioItem(variant: MenuVariantConfig, props: MenuRadioItemProps): NativeNode {
  const menu = useMenuHierarchy(), radio = useContext(MenuRadioContext);
  const [own, rest] = splitProps(props, ["children", "value", "closeOnSelect"]);
  const value = memo(() => {
    const current = own.value;
    if (typeof current !== "string" || !current) throw new TypeError(`${variant.radioName} value must be a non-empty string`);
    return current;
  });
  const checked = () => radio?.value() === value();
  const node = menuRow(mergeProps(rest, {
    get checked() { return checked(); },
    get "data-state"() { return checked() ? "checked" : "unchecked"; },
    get "data-value"() { return value(); },
  }), {
    role: "menuItemRadio",
    sheet: variant.sheet,
    baseClass: variant.itemBaseClass,
    class: () => variant.checkClass,
    select() { radio?.choose(value()); if (own.closeOnSelect ?? true) menu?.dismissCascade(); },
  });
  append(node, menuIndicator(variant.indicatorClass, variant.indicatorDotClass, checked, true));
  attachItemContent(node, variant, () => own.children);
  return node;
}

export function buildMenuGroup(props: MenuGroupProps = {}): NativeNode {
  useStylesheet(menuSheet);
  const [own, rest] = splitProps(props, ["children", "label"]);
  const node = createElement("box");
  spread(node, mergeProps({ role: "group" as const, get ariaLabel() { return own.label ?? null; } }, rest, {
    get __zguiClass() { return classes("zui-menu__group"); },
  }), true);
  slot(node, () => own.children);
  return node;
}

export function buildMenuSeparator(name: string, sheet: Stylesheet, className: string, props: MenuSeparatorProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren(name, props);
  return element("box", mergeProps({ ariaHidden: true }, props, { get __zguiClass() { return classes(className); } }));
}
