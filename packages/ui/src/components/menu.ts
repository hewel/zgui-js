import { Show, createContext, onCleanup, splitProps, useContext } from "#solid";
import { createElement, insert, insertNode, memo, mergeProps, spread } from "../renderer/index.js";
import { focus, getTextContent } from "../nodes.js";
import type { Accessor, NativeChild, NativeElementProps, NativeEvent, NativeFocusEvent, NativeKeyEvent, NativeNode, NativePointerBoundaryEvent, NativePointerDownEvent, NativeRole } from "../types.js";
import { anchoredSurface } from "./anchored.js";
import type { AnchoredContentProps } from "./anchored.js";
import { OverlayContext, OverlayRoot } from "./overlay.js";
import type { OverlayRootProps, OverlayState } from "./overlay.js";
import { createRovingFocus } from "./roving.js";
import { createDelayed } from "./delay.js";
import { callHandler, createControllable } from "./state.js";
import { classes, element, forbidChildren, part, type Stylesheet, useStylesheet } from "./shared.js";
import { Icon } from "./icons.js";
import { CHECK, DOT } from "./generated/icons.js";
import { sheet } from "./generated/MenuStyle.js";

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
interface MenuContextValue { state: OverlayState; dismiss(): void; }
export const MenuContext = createContext<MenuContextValue>();
export const MenuSurfaceContext = createContext<{ node: Accessor<NativeNode | null>; roving: ReturnType<typeof createRovingFocus> }>();
export const MenuRadioContext = createContext<{ value: Accessor<string>; choose(value: string): void }>();

export function menuRoot(props: OverlayRootProps): NativeChild {
  return OverlayRoot(mergeProps(props, { get children() {
    const state = useContext(OverlayContext)!;
    let alive = true;
    onCleanup(() => { alive = false; });
    const context = { state, dismiss() { if (alive) state.setOpen(false); } };
    return MenuContext.Provider({ value: context, get children() { return props.children; } });
  } }));
}

export function createTypeaheadHandler(group: ReturnType<typeof createRovingFocus> | undefined) {
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

export function menuSurface(state: OverlayState | undefined, props: MenuContentProps): NativeChild {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["children"]);
  return anchoredSurface(state, rest, {
    class: `${sheet.className} zui-menu`,
    role: "menu",
    trap: true,
    children() {
      const roving = createRovingFocus(() => "vertical", "active");
      return MenuSurfaceContext.Provider({
        value: { node: state!.content.node, roving },
        get children() {
          const list = createElement("box");
          spread(list, { class: "zui-menu__list", "data-orientation": "vertical", onKeyDown: roving.onKeyDown }, true);
          const keys = createElement("box");
          const onTypeahead = createTypeaheadHandler(roving);
          spread(keys, { class: "zui-menu__keys", onKeyDown: onTypeahead }, true);
          insertNode(list, keys);
          insert(keys, () => own.children);
          return list;
        },
      });
    },
  });
}

export function MenuContent(props: MenuContentProps = {}): NativeChild {
  return menuSurface(useContext(MenuContext)?.state, props);
}

export function MenuTypeahead(props: MenuTypeaheadProps = {}): NativeNode {
  useStylesheet(sheet);
  const group = useContext(MenuSurfaceContext)?.roving;
  const onTypeahead = createTypeaheadHandler(group);
  const [own, rest] = splitProps(props, ["children", "onKeyDown"]);
  const node = createElement("box");
  spread(node, mergeProps(rest, {
    get class() { return classes("zui-menu__keys", rest.class); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, own.onKeyDown)) onTypeahead(event);
    },
  }), true);
  insert(node, () => own.children);
  return node;
}

export function menuRow(props: NativeElementProps, options: {
  role: NativeRole; class: () => string; select(): void;
  sheet?: Stylesheet;
  baseClass?: string;
  activateOnPress?: boolean;
  enter?: (activate: () => void) => void;
  leave?: (event: NativePointerBoundaryEvent, defocus: () => void) => void;
}): NativeNode {
  useStylesheet(options.sheet ?? sheet);
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
    get class() { return classes(options.baseClass ?? "zui-menu__item", options.class(), rest.class); },
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

export function MenuItem(props: MenuItemProps = {}): NativeNode {
  const menu = useContext(MenuContext);
  const [own, rest] = splitProps(props, ["children", "destructive", "inset", "closeOnSelect", "onSelect", "shortcut"]);
  const node = menuRow(mergeProps({
    get "data-destructive"() { return own.destructive ? "true" : null; },
    get "data-inset"() { return own.inset ? "true" : null; },
    get ariaKeyShortcuts() { return own.shortcut ?? null; },
  }, rest), {
    role: "menuItem",
    class: () => classes(own.destructive && "zui-menu__item--destructive", own.inset && "zui-menu__item--inset"),
    select() { own.onSelect?.(); if (own.closeOnSelect ?? true) menu?.dismiss(); },
  });
  insert(node, memo(() => own.children), null);
  insert(node, Show({ get when() { return own.shortcut != null; }, get children() {
    return MenuShortcut({ get children() { return own.shortcut; } });
  } }), null);
  return node;
}

function indicator(checked: Accessor<boolean>, dot: boolean): NativeNode {
  const box = createElement("box");
  spread(box, { class: classes("zui-menu__indicator", dot && "zui-menu__indicator--dot") }, true);
  const show = memo(checked);
  insert(box, () => show() ? Icon({ icon: dot ? DOT : CHECK }) : null);
  return box;
}

export function MenuCheckboxItem(props: MenuCheckboxItemProps = {}): NativeNode {
  const menu = useContext(MenuContext);
  const [own, rest] = splitProps(props, ["children", "checked", "defaultChecked", "onCheckedChange", "closeOnSelect"]);
  const [read, write] = createControllable<boolean>({ value: () => own.checked, defaultValue: () => own.defaultChecked, onChange: () => own.onCheckedChange }, false);
  const checked = memo(() => { const value = read(); if (typeof value !== "boolean") throw new TypeError("MenuCheckboxItem checked must be a boolean"); return value; });
  const node = menuRow(mergeProps(rest, {
    get checked() { return checked(); }, get "data-state"() { return checked() ? "checked" : "unchecked"; },
  }), { role: "menuItemCheckBox", class: () => "zui-menu__item--check",
    select() { write(!checked()); if (own.closeOnSelect ?? true) menu?.dismiss(); },
  });
  insert(node, indicator(checked, false));
  insert(node, memo(() => own.children), null);
  return node;
}

export function MenuRadioGroup(props: MenuRadioGroupProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["children", "value", "defaultValue", "onValueChange", "label"]);
  const [read, choose] = createControllable<string>({ value: () => own.value, defaultValue: () => own.defaultValue, onChange: () => own.onValueChange }, "");
  const value = memo(() => { const current = read(); if (typeof current !== "string") throw new TypeError("MenuRadioGroup value must be a string"); return current; });
  const node = createElement("box");
  spread(node, mergeProps({ role: "group" as const, get ariaLabel() { return own.label ?? null; } }, rest, {
    get class() { return classes("zui-menu__group", rest.class); },
  }), true);
  MenuRadioContext.Provider({
    value: { value, choose },
    get children() {
      insert(node, () => own.children);
      return null;
    },
  });
  return node;
}

export function MenuRadioItem(props: MenuRadioItemProps): NativeNode {
  const menu = useContext(MenuContext), radio = useContext(MenuRadioContext);
  const [own, rest] = splitProps(props, ["children", "value", "closeOnSelect"]);
  const value = memo(() => { const value = own.value; if (typeof value !== "string" || !value) throw new TypeError("MenuRadioItem value must be a non-empty string"); return value; });
  const checked = () => radio?.value() === value();
  const node = menuRow(mergeProps(rest, {
    get checked() { return checked(); }, get "data-state"() { return checked() ? "checked" : "unchecked"; }, get "data-value"() { return value(); },
  }), { role: "menuItemRadio", class: () => "zui-menu__item--check",
    select() { radio?.choose(value()); if (own.closeOnSelect ?? true) menu?.dismiss(); },
  });
  insert(node, indicator(checked, true));
  insert(node, memo(() => own.children), null);
  return node;
}

export function MenuLabel(props: MenuLabelProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["inset"]);
  return element("label", mergeProps(rest, { get class() { return classes("zui-menu__label", own.inset && "zui-menu__label--inset", rest.class); } }));
}
export function MenuGroup(props: MenuGroupProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["children", "label"]);
  const node = createElement("box");
  spread(node, mergeProps({ role: "group" as const, get ariaLabel() { return own.label ?? null; } }, rest, {
    get class() { return classes("zui-menu__group", rest.class); },
  }), true);
  insert(node, () => own.children);
  return node;
}
export function MenuSeparator(props: MenuSeparatorProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("MenuSeparator", props);
  return element("box", mergeProps({ ariaHidden: true }, props, { get class() { return classes("zui-menu__separator", props.class); } }));
}
export const MenuShortcut = part("text", sheet, "zui-menu__shortcut", { ariaHidden: true });
