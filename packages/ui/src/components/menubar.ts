import {
  createContext,
  createMemo,
  createSignal,
  onCleanup,
  omit,
  untrack,
  useContext,
} from "#solid";
import { createElement, memo, mergeProps, spread } from "../renderer/index.js";
import { focus } from "../nodes.js";
import type {
  Accessor, NativeChild, NativeElementProps, NativeEvent, NativeFocusEvent, NativeKeyEvent,
  NativeNode, NativePointerBoundaryEvent,
} from "../types.js";
import { createOverlayState } from "./overlay.js";
import {
  MenuGroup, MenuRadioGroup, type MenuCheckboxItemProps, type MenuContentProps, type MenuGroupProps,
  type MenuItemProps, type MenuRadioGroupProps, type MenuRadioItemProps, type MenuSeparatorProps, type MenuShortcutProps,
} from "./menu.js";
import {
  MENUBAR_VARIANT, MenubarShortcutPart, buildMenuCheckboxItem, buildMenuItem, buildMenuRadioItem,
  buildMenuSeparator, provideMenuRoot, renderMenuSurface, useMenuHierarchy, useMenuState,
} from "./menu-core.js";
import { createRovingFocus } from "./roving.js";
import { callHandler } from "./state.js";
import { classes, part, provide, slot, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/MenubarStyle.js";

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

const BarContext = createContext<BarContextValue | null>(null);

function handleBarArrows(target: MenubarBarControl | undefined, event: NativeKeyEvent): void {
  if (event.character != null) return;
  const direction = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
  if (!direction || !target || !target.step(direction)) return;
  event.preventDefault();
  event.stopPropagation();
}

export function Menubar(props: MenubarProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "children", "label", "onKeyDown");
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
  spread(node, mergeProps(rest, withDefaults({
    role: "menuBar" as const,
    orientation: "horizontal" as const,
    get ariaLabel() { return props.label ?? null; },
  }, rest), {
    get __zguiClass() { return classes("zui-menubar", sheet.className); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, props.onKeyDown) && alive) roving.onKeyDown(event);
    },
  }), true);
  slot(node, provide(BarContext, bar, props));
  return node;
}

function menubarValue(value: string): string {
  if (typeof value !== "string" || !value) {
    throw new TypeError("MenubarMenu value must be a non-empty string");
  }
  return value;
}

export function MenubarMenu(props: MenubarMenuProps): NativeNode {
  useStylesheet(sheet);
  const bar = useContext(BarContext);
  const rest = omit(props, "children", "value");
  menubarValue(props.value);
  const value = createMemo(() => menubarValue(props.value));
  let alive = true;
  onCleanup(() => {
    alive = false;
    if (typeof props.value === "string" && props.value && bar?.isOpen(props.value)) bar.close();
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
    state.setOpen(false);
    const trigger = untrack(state.trigger.node);
    if (trigger) focus(trigger);
  };
  const node = createElement("box");
  spread(node, mergeProps(rest, withDefaults({
    get "data-state"() { return state.open() ? "open" : "closed"; },
  }, rest), {
    get __zguiClass() { return classes("zui-menubar__menu"); },
  }), true);
  slot(node, provideMenuRoot(state, dismiss, props));
  return node;
}

export function MenubarTrigger(props: MenubarTriggerProps = {}): NativeNode {
  useStylesheet(sheet);
  const bar = useContext(BarContext);
  const state = useMenuState();
  const rest = omit(props, "children", "ref", "onClick", "onPointerEnter", "onFocus", "onKeyDown");
  const node = createElement("control");
  const disabled = () => !!props.disabled;
  const item = bar?.roving.register(node, disabled);
  state?.trigger.ref(node);
  let alive = true;
  onCleanup(() => { alive = false; });
  spread(node, mergeProps(rest, withDefaults({
    role: "menuItem" as const,
    ariaHasPopup: "menu" as const,
    get expanded() { return state?.open() ?? false; },
    get ariaControls() { return state?.content.node() ?? null; },
    get tabIndex() { return item?.tabIndex() ?? (disabled() ? -1 : 0); },
    get "data-state"() { return state?.open() ? "open" : "closed"; },
  }, rest), {
    get __zguiClass() { return classes("zui-menubar__trigger"); },
    onFocus(event: NativeFocusEvent) {
      if (!callHandler(event, props.onFocus) || !alive || disabled()) return;
      item?.activate();
      if (bar?.anyOpen()) state?.setOpen(true);
    },
    onPointerEnter(event: NativePointerBoundaryEvent) {
      if (!callHandler(event, props.onPointerEnter) || !alive || disabled()) return;
      if (bar?.anyOpen() && !state?.open()) {
        item?.activate();
        state?.setOpen(true);
      }
    },
    onClick(event: NativeEvent) {
      if (!callHandler(event, props.onClick) || !alive || disabled() || !state) return;
      item?.activate();
      state.setOpen(!state.open());
    },
    onKeyDown(event: NativeKeyEvent) {
      if (!callHandler(event, props.onKeyDown) || !alive || disabled() || !state) return;
      if (event.character == null && event.key === "ArrowDown") {
        item?.activate();
        state.setOpen(true);
        event.preventDefault();
        event.stopPropagation();
      }
    },
  }), true);
  if (typeof props.ref === "function") props.ref(node);
  slot(node, memo(() => props.children));
  return node;
}

export function MenubarArrows(props: MenubarArrowsProps = {}): NativeNode {
  useStylesheet(sheet);
  const fallback = useContext(BarContext)?.roving;
  const rest = omit(props, "children", "bar", "onKeyDown");
  const node = createElement("box");
  spread(node, mergeProps(rest, {
    get __zguiClass() { return classes("zui-menubar__arrows"); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, props.onKeyDown)) handleBarArrows(props.bar ?? fallback, event);
    },
  }), true);
  slot(node, () => props.children);
  return node;
}

export function MenubarContent(props: MenubarContentProps = {}): NativeChild {
  const menu = useMenuHierarchy();
  const bar = useContext(BarContext)?.roving;
  const rest = omit(props, "placement");
  return renderMenuSurface(menu?.state, mergeProps(rest, withDefaults({
    get placement() { return props.placement ?? ({ side: "bottom", align: "start" } as const); },
  }, rest)), {
    extraSheet: sheet,
    class: "zui-menubar__content",
    trap: false,
    surfaceClass: false,
    onDismiss(reason) {
      if (reason === "escape") menu?.dismissCascade();
      else menu?.state.setOpen(false);
    },
    onBarKeyDown(event) { handleBarArrows(bar, event); },
  });
}

export function MenubarItem(props: MenubarItemProps = {}): NativeNode {
  return buildMenuItem(MENUBAR_VARIANT, props);
}

export function MenubarCheckboxItem(props: MenubarCheckboxItemProps = {}): NativeNode {
  return buildMenuCheckboxItem(MENUBAR_VARIANT, props);
}

export function MenubarRadioItem(props: MenubarRadioItemProps): NativeNode {
  return buildMenuRadioItem(MENUBAR_VARIANT, props);
}

export const MenubarLabel = part("label", sheet, "zui-menubar__label");

export function MenubarSeparator(props: MenubarSeparatorProps = {}): NativeNode {
  return buildMenuSeparator("MenubarSeparator", sheet, "zui-menubar__separator", props);
}

export const MenubarShortcut = MenubarShortcutPart;
