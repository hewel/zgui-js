import { splitProps } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeNode } from "../types.js";
import { classes, element, useStylesheet } from "./shared.js";
import { sheet } from "./generated/MenuStyle.js";
import {
  MENU_VARIANT, MenuShortcutPart, buildMenuCheckboxItem, buildMenuGroup, buildMenuItem, buildMenuRadioGroup,
  buildMenuRadioItem, buildMenuSeparator, buildMenuTypeahead, renderMenuSurface, useMenuState,
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
  return buildMenuTypeahead(props);
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
  const [own, rest] = splitProps(props, ["inset"]);
  return element("label", mergeProps(rest, {
    get __zguiClass() { return classes("zui-menu__label", own.inset && "zui-menu__label--inset"); },
  }));
}

export function MenuGroup(props: MenuGroupProps = {}): NativeNode {
  return buildMenuGroup(props);
}

export function MenuSeparator(props: MenuSeparatorProps = {}): NativeNode {
  return buildMenuSeparator("MenuSeparator", sheet, "zui-menu__separator", props);
}

export const MenuShortcut = MenuShortcutPart;
