import { createContext, createSignal, onCleanup, splitProps, useContext } from "#solid";
import { insert, mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeElementProps, NativeNode } from "../types.js";
import { classes, element, part, useStylesheet } from "./shared.js";
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
const HeaderContext = createContext<CardHeaderState>();

export function Card(props: CardProps = {}): NativeNode {
  useStylesheet(sheet);
  return element("box", mergeProps(
    { role: "group" as const },
    props,
    { get class() { return classes(sheet.className, "zui-card", props.class); } },
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
  const [own, rest] = splitProps(props, ["children"]);
  const header = element("box", mergeProps(rest, {
    get class() {
      return classes("zui-card__header", count() > 0 && "zui-card__header--action", rest.class);
    },
  }));
  insert(header, HeaderContext.Provider({ value: state, get children() { return own.children; } }));
  return header;
}

export const CardTitle = part("label", sheet, "zui-card__title");
export const CardDescription = part("box", sheet, "zui-card__description");

export function CardAction(props: CardActionProps = {}): NativeNode {
  useStylesheet(sheet);
  useContext(HeaderContext)?.claim();
  return element("box", mergeProps(props, {
    get class() { return classes("zui-card__action", props.class); },
  }));
}

export const CardContent = part("box", sheet, "zui-card__content");
export const CardFooter = part("row", sheet, "zui-card__footer");
