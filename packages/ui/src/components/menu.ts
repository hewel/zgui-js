import { omit } from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import type { NativeChild, NativeKeyEvent, NativeNode } from "../types.js";
import { classes, element, slot, useStylesheet, withDefaults } from "./shared.js";
import { callHandler } from "./state.js";
import { sheet } from "./generated/MenuStyle.js";
import {
  MENU_VARIANT, MenuShortcutPart, buildMenuCheckboxItem, buildMenuItem, buildMenuRadioGroup,
  buildMenuRadioItem, buildMenuSeparator, renderMenuSurface, useMenuState, useMenuTypeahead,
  type MenuCheckboxItemProps, type MenuContentProps, type MenuGroupProps, type MenuItemProps,
  type MenuLabelProps, type MenuRadioGroupProps, type MenuRadioItemProps, type MenuSeparatorProps,
  type MenuShortcutProps, type MenuTypeaheadProps,
} from "./menu-core.js";

export type {
  MenuContentProps, MenuTypeaheadProps, MenuShortcutProps, MenuLabelProps, MenuGroupProps,
  MenuSeparatorProps, MenuItemProps, MenuCheckboxItemProps, MenuRadioGroupProps, MenuRadioItemProps,
};

export function MenuContent(props: MenuContentProps = {}): NativeChild {
  return renderMenuSurface(useMenuState(), props);
}

export function MenuTypeahead(props: MenuTypeaheadProps = {}): NativeNode {
  useStylesheet(sheet);
  const onTypeahead = useMenuTypeahead();
  const rest = omit(props, "children", "onKeyDown");
  const node = createElement("box");
  spread(node, mergeProps(rest, {
    get __zguiClass() { return classes("zui-menu__keys"); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, props.onKeyDown)) onTypeahead(event);
    },
  }), true);
  slot(node, () => props.children);
  return node;
}

export function MenuItem(props: MenuItemProps = {}): NativeNode {
  return buildMenuItem(MENU_VARIANT, props);
}

export function MenuCheckboxItem(props: MenuCheckboxItemProps = {}): NativeNode {
  return buildMenuCheckboxItem(MENU_VARIANT, props);
}

export function MenuRadioGroup(props: MenuRadioGroupProps = {}): NativeNode {
  return buildMenuRadioGroup(props);
}

export function MenuRadioItem(props: MenuRadioItemProps): NativeNode {
  return buildMenuRadioItem(MENU_VARIANT, props);
}

export function MenuLabel(props: MenuLabelProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "inset");
  return element("label", mergeProps(rest, {
    get __zguiClass() { return classes("zui-menu__label", props.inset && "zui-menu__label--inset"); },
  }));
}

export function MenuGroup(props: MenuGroupProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "children", "label");
  const node = createElement("box");
  spread(node, mergeProps(rest, withDefaults({ role: "group" as const, get ariaLabel() { return props.label ?? null; } }, rest), {
    get __zguiClass() { return classes("zui-menu__group"); },
  }), true);
  slot(node, () => props.children);
  return node;
}

export function MenuSeparator(props: MenuSeparatorProps = {}): NativeNode {
  return buildMenuSeparator("MenuSeparator", sheet, "zui-menu__separator", props);
}

export const MenuShortcut = MenuShortcutPart;
