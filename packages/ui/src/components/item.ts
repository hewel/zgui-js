import { createContext, createSignal, onCleanup, splitProps, useContext } from "#solid";
import { createElement, insert, mergeProps, spread } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, forbidChildren, part, useStylesheet } from "./shared.js";
import { sheet } from "./generated/ItemStyle.js";
import { sheet as partSheet } from "./generated/ItemPartStyle.js";
import { sheet as groupSheet } from "./generated/ItemGroupStyle.js";

export type ItemVariant = "default" | "outline" | "muted";
export type ItemSize = "md" | "sm";
export type ItemMediaVariant = "default" | "icon" | "image";
export interface ItemProps extends NativeElementProps { variant?: ItemVariant; size?: ItemSize; }
export interface ItemMediaProps extends NativeElementProps { variant?: ItemMediaVariant; }
export interface ItemSeparatorProps extends Omit<NativeElementProps, "children"> { children?: never; }
export type ItemGroupProps = NativeElementProps;
export type ItemContentProps = NativeElementProps;
export type ItemTitleProps = NativeElementProps;
export type ItemDescriptionProps = NativeElementProps;
export type ItemActionsProps = NativeElementProps;
export type ItemHeaderProps = NativeElementProps;
export type ItemFooterProps = NativeElementProps;

const ItemContext = createContext<() => void>();

export const ItemContent = part("box", partSheet, "zui-item__content");
export const ItemTitle = part("label", partSheet, "zui-item__title");
export const ItemActions = part("box", partSheet, "zui-item__actions");
export const ItemHeader = part("box", partSheet, "zui-item__header");
export const ItemFooter = part("box", partSheet, "zui-item__footer");

export function Item(props: ItemProps = {}): NativeNode {
  useStylesheet(sheet);
  useStylesheet(partSheet);
  const [own, rest] = splitProps(props, ["variant", "size"]);
  const [descriptions, setDescriptions] = createSignal(0);
  const node = createElement("box");
  spread(node, mergeProps(
    {
      get "data-variant"() { return own.variant ?? "default"; },
      get "data-size"() { return own.size ?? "md"; },
    },
    rest,
    {
      get class() {
        return classes(
          sheet.className, "zui-item",
          own.variant === "outline" && "zui-item--outline",
          own.variant === "muted" && "zui-item--muted",
          own.size === "sm" && "zui-item--sm",
          descriptions() > 0 && "zui-item--described",
          rest.class,
        );
      },
    },
  ), true);
  insert(node, ItemContext.Provider({
    value: () => {
      setDescriptions(count => count + 1);
      onCleanup(() => setDescriptions(count => count - 1));
    },
    get children() { return rest.children; },
  }));
  return node;
}

export function ItemMedia(props: ItemMediaProps = {}): NativeNode {
  useStylesheet(partSheet);
  const [own, rest] = splitProps(props, ["variant"]);
  return element("box", mergeProps(
    { get "data-variant"() { return own.variant ?? "default"; } },
    rest,
    {
      get class() {
        return classes(
          "zui-item__media",
          own.variant === "icon" && "zui-item__media--icon",
          own.variant === "image" && "zui-item__media--image",
          rest.class,
        );
      },
    },
  ));
}

const descriptionPart = part("box", partSheet, "zui-item__description");
export function ItemDescription(props: ItemDescriptionProps = {}): NativeNode {
  useContext(ItemContext)?.();
  return descriptionPart(props);
}

export const ItemGroup = part("box", groupSheet, groupSheet.className, { role: "list" });

export function ItemSeparator(props: ItemSeparatorProps = {}): NativeNode {
  useStylesheet(groupSheet);
  forbidChildren("ItemSeparator", props);
  return element("box", mergeProps(
    { role: "genericContainer" as const, ariaHidden: true },
    props,
    { get class() { return classes("zui-item__separator", props.class); } },
  ));
}
