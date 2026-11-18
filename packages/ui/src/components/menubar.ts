import { Show, createContext, createEffect, createMemo, createSignal, onCleanup, splitProps, untrack, useContext } from "#solid";
import { createElement, memo, mergeProps, spread } from "../renderer/index.js";
import { focus } from "../nodes.js";
import type {
  Accessor, NativeChild, NativeElementProps, NativeEvent, NativeFocusEvent, NativeKeyEvent,
  NativeNode, NativePointerBoundaryEvent,
} from "../types.js";
import { anchoredSurface, createOverlayState } from "./overlay.js";
import {
  MenuContext, MenuGroup, MenuRadioContext, MenuRadioGroup, MenuSurfaceContext,
  createMenuCheckedState, createMenuRadioValue, createTypeaheadHandler, menuIndicator, menuRow,
} from "./menu.js";
import type {
  MenuCheckboxItemProps, MenuContentProps, MenuGroupProps, MenuItemProps, MenuRadioGroupProps,
  MenuRadioItemProps, MenuSeparatorProps, MenuShortcutProps,
} from "./menu.js";
import { createRovingFocus } from "./roving.js";
import { callHandler } from "./state.js";
import { append, classes, element, forbidChildren, part, provide, slot, useStylesheet } from "./shared.js";
import { sheet } from "./generated/MenubarStyle.js";
import { sheet as menuSheet } from "./generated/MenuStyle.js";

export { MenuGroup as MenubarGroup, MenuRadioGroup as MenubarRadioGroup };
export type { MenuGroupProps as MenubarGroupProps, MenuRadioGroupProps as MenubarRadioGroupProps };

export interface MenubarProps extends NativeElementProps {
  label?: string | null;
}
export interface MenubarMenuProps extends NativeElementProps {
  value: string;
}
export type MenubarTriggerProps = NativeElementProps;
export type MenubarContentProps = MenuContentProps;
export interface MenubarBarControl {
  step(direction: 1 | -1): boolean;
}
export interface MenubarArrowsProps extends NativeElementProps {
  bar?: MenubarBarControl;
}
export interface MenubarItemProps extends Omit<MenuItemProps, "destructive" | "inset"> {}
export type MenubarCheckboxItemProps = MenuCheckboxItemProps;
export type MenubarRadioItemProps = MenuRadioItemProps;
export type MenubarLabelProps = NativeElementProps;
export type MenubarSeparatorProps = MenuSeparatorProps;
export type MenubarShortcutProps = MenuShortcutProps;

interface BarContextValue {
  roving: ReturnType<typeof createRovingFocus>;
  anyOpen: Accessor<boolean>;
  isOpen(value: string): boolean;
  open(value: string): void;
  close(): void;
}

const BarContext = createContext<BarContextValue>();

export function Menubar(props: MenubarProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["children", "label", "onKeyDown"]);
  const [openValue, setOpenValue] = createSignal<string | null>(null);
  const roving = createRovingFocus(() => "horizontal", "target");
  let alive = true;
  onCleanup(() => { alive = false; });
  const bar: BarContextValue = {
    roving,
    anyOpen: () => openValue() !== null,
    isOpen: value => openValue() === value,
    open(value) { if (alive && openValue() !== value) setOpenValue(value); },
    close() { if (alive && openValue() !== null) setOpenValue(null); },
  };
  const node = createElement("box");
  spread(node, mergeProps({
    role: "menuBar",
    orientation: "horizontal",
    get ariaLabel() { return own.label ?? null; },
  }, rest, {
    get __zguiClass() { return classes("zui-menubar", sheet.className); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, own.onKeyDown) && alive) roving.onKeyDown(event);
    },
  }), true);
  slot(node, provide(BarContext, bar, own));
  return node;
}

export function MenubarMenu(props: MenubarMenuProps): NativeNode {
  useStylesheet(sheet);
  const bar = useContext(BarContext);
  const [own, rest] = splitProps(props, ["children", "value"]);
  const value = createMemo(() => {
    if (typeof own.value !== "string" || !own.value) {
      throw new TypeError("MenubarMenu value must be a non-empty string");
    }
    return own.value;
  });
  let alive = true;
  onCleanup(() => {
    alive = false;
    if (typeof own.value === "string" && own.value && bar?.isOpen(own.value)) bar.close();
  });
  const state = createOverlayState({
    get open() { return bar?.isOpen(value()) ?? false; },
    onOpenChange(next) {
      if (!alive || !bar) return;
      if (next) bar.open(value());
      else if (bar.isOpen(value())) bar.close();
    },
  });
  const dismiss = () => {
    if (!alive) return;
    state.setOpen(false);
    const trigger = untrack(state.trigger.node);
    if (trigger) focus(trigger);
  };
  const node = createElement("box");
  spread(node, mergeProps({
    get "data-state"() { return state.open() ? "open" : "closed"; },
  }, rest, {
    get __zguiClass() { return classes("zui-menubar__menu"); },
  }), true);
  slot(node, provide(MenuContext, { state, dismiss }, own));
  return node;
}

export function MenubarTrigger(props: MenubarTriggerProps = {}): NativeNode {
  useStylesheet(sheet);
  const menu = useContext(MenuContext);
  const bar = useContext(BarContext);
  const state = menu?.state;
  const [own, rest] = splitProps(props, ["children", "ref", "onClick", "onPointerEnter", "onFocus", "onKeyDown"]);
  const node = createElement("control");
  const disabled = () => !!props.disabled;
  const item = bar?.roving.register(node, disabled);
  state?.trigger.ref(node);
  let alive = true;
  onCleanup(() => { alive = false; });
  spread(node, mergeProps({
    role: "menuItem",
    ariaHasPopup: "menu",
    get expanded() { return state?.open() ?? false; },
    get ariaControls() { return state?.content.node() ?? null; },
    get tabIndex() { return item?.tabIndex() ?? (disabled() ? -1 : 0); },
    get "data-state"() { return state?.open() ? "open" : "closed"; },
  }, rest, {
    get __zguiClass() { return classes("zui-menubar__trigger"); },
    onFocus(event: NativeFocusEvent) {
      if (!callHandler(event, own.onFocus) || !alive || disabled()) return;
      item?.activate();
      if (bar?.anyOpen()) state?.setOpen(true);
    },
    onPointerEnter(event: NativePointerBoundaryEvent) {
      if (!callHandler(event, own.onPointerEnter) || !alive || disabled()) return;
      if (bar?.anyOpen() && !state?.open()) {
        item?.activate();
        state?.setOpen(true);
      }
    },
    onClick(event: NativeEvent) {
      if (!callHandler(event, own.onClick) || !alive || disabled() || !state) return;
      item?.activate();
      state.setOpen(!state.open());
    },
    onKeyDown(event: NativeKeyEvent) {
      if (!callHandler(event, own.onKeyDown) || !alive || disabled() || !state) return;
      if (event.character == null && event.key === "ArrowDown") {
        item?.activate();
        state.setOpen(true);
        event.preventDefault();
        event.stopPropagation();
      }
    },
  }), true);
  if (typeof own.ref === "function") own.ref(node);
  slot(node, memo(() => own.children));
  return node;
}

export function MenubarArrows(props: MenubarArrowsProps = {}): NativeNode {
  useStylesheet(sheet);
  const fallback = useContext(BarContext)?.roving;
  const [own, rest] = splitProps(props, ["children", "bar", "onKeyDown"]);
  const node = createElement("box");
  spread(node, mergeProps(rest, {
    get __zguiClass() { return classes("zui-menubar__arrows"); },
    onKeyDown(event: NativeKeyEvent) {
      if (!callHandler(event, own.onKeyDown) || event.character != null) return;
      const direction = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
      if (!direction) return;
      const target = own.bar ?? fallback;
      if (!target || !target.step(direction)) return;
      event.preventDefault();
      event.stopPropagation();
    },
  }), true);
  slot(node, () => own.children);
  return node;
}

export function MenubarContent(props: MenubarContentProps = {}): NativeChild {
  useStylesheet(sheet);
  useStylesheet(menuSheet);
  const menu = useContext(MenuContext);
  const bar = useContext(BarContext)?.roving;
  const [own, rest] = splitProps(props, ["children", "placement", "offset"]);
  return anchoredSurface(menu?.state, mergeProps({
    placement: own.placement ?? ({ side: "bottom", align: "start" } as const),
    get offset() { return own.offset; },
  }, rest), {
    class: "zui-menubar__content",
    role: "menu",
    trap: false,
    surfaceClass: false,
    onDismiss(reason) {
      if (reason === "escape") menu?.dismiss();
      else menu?.state.setOpen(false);
    },
    children() {
      const roving = createRovingFocus(() => "vertical", "active");
      let focused = false;
      createEffect(() => {
        if (focused) return;
        if (roving.focusFirst()) focused = true;
      });
      return provide(MenuSurfaceContext, { node: menu!.state.content.node, roving }, () => {
        const keys = createElement("box");
        const onTypeahead = createTypeaheadHandler(roving);
        spread(keys, {
          class: "zui-menu__keys",
          onKeyDown(event: NativeKeyEvent) {
            if (callHandler(event, roving.onKeyDown)) onTypeahead(event);
          },
        }, true);
        const arrows = createElement("box");
        spread(arrows, {
          class: "zui-menubar__arrows",
          onKeyDown(event: NativeKeyEvent) {
            if (event.character != null) return;
            const direction = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
            if (!direction || !bar || !bar.step(direction)) return;
            event.preventDefault();
            event.stopPropagation();
          },
        }, true);
        append(keys, arrows);
        slot(arrows, () => own.children);
        return keys;
      });
    },
  });
}

function itemLabel(children: Accessor<NativeChild>): NativeNode {
  const box = createElement("box");
  spread(box, { class: "zui-menubar__item-label" }, true);
  slot(box, memo(children));
  return box;
}

export function MenubarItem(props: MenubarItemProps = {}): NativeNode {
  const menu = useContext(MenuContext);
  const [own, rest] = splitProps(props, ["children", "closeOnSelect", "onSelect", "shortcut"]);
  const node = menuRow(mergeProps({
    get ariaKeyShortcuts() { return own.shortcut ?? null; },
  }, rest), {
    role: "menuItem",
    sheet,
    baseClass: "zui-menubar__item",
    class: () => "",
    select() {
      own.onSelect?.();
      if (own.closeOnSelect ?? true) menu?.dismiss();
    },
  });
  append(node, itemLabel(() => own.children));
  slot(node, Show({
    get when() { return own.shortcut != null; },
    get children() { return MenubarShortcut({ get children() { return own.shortcut; } }); },
  }));
  return node;
}

export function MenubarCheckboxItem(props: MenubarCheckboxItemProps = {}): NativeNode {
  const menu = useContext(MenuContext);
  const [own, rest] = splitProps(props, ["children", "checked", "defaultChecked", "onCheckedChange", "closeOnSelect"]);
  const [checked, write] = createMenuCheckedState("MenubarCheckboxItem", own);
  const node = menuRow(mergeProps(rest, {
    get checked() { return checked(); },
    get "data-state"() { return checked() ? "checked" : "unchecked"; },
  }), {
    role: "menuItemCheckBox",
    sheet,
    baseClass: "zui-menubar__item",
    class: () => "zui-menubar__item--check",
    select() {
      write(!checked());
      if (own.closeOnSelect ?? false) menu?.dismiss();
    },
  });
  append(node, menuIndicator("zui-menubar__indicator", "zui-menubar__indicator--dot", checked, false));
  append(node, itemLabel(() => own.children));
  return node;
}

export function MenubarRadioItem(props: MenubarRadioItemProps): NativeNode {
  const menu = useContext(MenuContext);
  const radio = useContext(MenuRadioContext);
  const [own, rest] = splitProps(props, ["children", "value", "closeOnSelect"]);
  const value = createMenuRadioValue("MenubarRadioItem", () => own.value);
  const checked = () => radio?.value() === value();
  const node = menuRow(mergeProps(rest, {
    get checked() { return checked(); },
    get "data-state"() { return checked() ? "checked" : "unchecked"; },
    get "data-value"() { return value(); },
  }), {
    role: "menuItemRadio",
    sheet,
    baseClass: "zui-menubar__item",
    class: () => "zui-menubar__item--check",
    select() {
      radio?.choose(value());
      if (own.closeOnSelect ?? true) menu?.dismiss();
    },
  });
  append(node, menuIndicator("zui-menubar__indicator", "zui-menubar__indicator--dot", checked, true));
  append(node, itemLabel(() => own.children));
  return node;
}

export const MenubarLabel = part("label", sheet, "zui-menubar__label");

export function MenubarSeparator(props: MenubarSeparatorProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("MenubarSeparator", props);
  return element("box", mergeProps({ ariaHidden: true }, props, {
    get __zguiClass() { return classes("zui-menubar__separator"); },
  }));
}

export const MenubarShortcut = part("text", sheet, "zui-menubar__shortcut", { ariaHidden: true });
