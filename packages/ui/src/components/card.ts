import { createContext, createSignal, onCleanup, omit, useContext } from "#solid";
import { createElement, mergeProps, setProp, spread } from "../renderer/index.js";
import type { NativeChild, NativeElementProps, NativeNode } from "../types.js";
import { classes, element, isStaticProps, part, provide, slot, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/CardStyle.js";

export type CardProps = NativeElementProps;
export type CardHeaderProps = NativeElementProps;
export type CardTitleProps = NativeElementProps;
export type CardDescriptionProps = NativeElementProps;
export type CardActionProps = NativeElementProps;
export type CardContentProps = NativeElementProps;
export type CardFooterProps = NativeElementProps;

interface CardHeaderState {
  claim(): void;
}
const HeaderContext = createContext<CardHeaderState | null>(null);

export function Card(props: CardProps = {}): NativeNode {
  useStylesheet(sheet);
  if (isStaticProps(props) && !("__zguiClass" in props)) {
    const node = createElement("box");
    if (props.role === undefined) setProp(node, "role", "group");
    setProp(node, "__zguiClass", `${sheet.className} zui-card`);
    spread(node, props);
    return node;
  }
  return element("box", mergeProps(
    props,
    withDefaults({ role: "group" as const }, props),
    { get __zguiClass() { return classes(sheet.className, "zui-card"); } },
  ));
}

export function CardHeader(props: CardHeaderProps = {}): NativeChild {
  useStylesheet(sheet);
  const [count, setCount] = createSignal(0);
  const state: CardHeaderState = {
    claim() {
      setCount(value => value + 1);
      onCleanup(() => setCount(value => value - 1));
    },
  };
  const rest = omit(props, "children");
  const header = element("box", mergeProps(rest, {
    get __zguiClass() {
      return classes("zui-card__header", count() > 0 && "zui-card__header--action");
    },
  }));
  slot(header, provide(HeaderContext, state, props));
  return header;
}

export const CardTitle = part("label", sheet, "zui-card__title");
export const CardDescription = part("box", sheet, "zui-card__description");

export function CardAction(props: CardActionProps = {}): NativeNode {
  useStylesheet(sheet);
  useContext(HeaderContext)?.claim();
  return element("box", mergeProps(props, {
    get __zguiClass() { return classes("zui-card__action"); },
  }));
}

export const CardContent = part("box", sheet, "zui-card__content");
export const CardFooter = part("row", sheet, "zui-card__footer");
