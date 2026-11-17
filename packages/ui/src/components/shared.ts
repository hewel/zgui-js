import type { Context } from "#solid";
import { createElement, effect, insert, insertNode, mergeProps, spread } from "../renderer/index.js";
import type { NativeHost } from "../host.js";
import { installStylesheet } from "../nodes.js";
import type { NativeChild, NativeElementProps, NativeNode, NativeTag } from "../types.js";

declare const globalThis: NativeHost;

export interface Stylesheet {
  readonly name: string;
  readonly className: string;
  readonly css: string;
}

const installedSheets = new WeakMap<object, Set<string>>();

export function classes(...values: readonly (string | null | undefined | false)[]): string {
  return values.filter(Boolean).join(" ");
}

export function useStylesheet(sheet: Stylesheet): void {
  const host: object | undefined = globalThis.__zgui;
  const installed = host ? installedSheets.get(host) : undefined;
  if (installed?.has(sheet.name)) return;
  installStylesheet(sheet.name, `@layer zgui.components { ${sheet.css} }`);
  if (!host) return;
  if (installed) installed.add(sheet.name);
  else installedSheets.set(host, new Set([sheet.name]));
}

export function append(parent: NativeNode, child: NativeNode): void {
  insertNode(parent, child);
}

export function slot(parent: NativeNode, content: NativeChild, marker: NativeNode | null = null): void {
  insert(parent, content, marker);
}

export function provide<T>(
  context: Context<T>,
  value: T,
  source: { readonly children?: NativeChild } | (() => NativeChild),
): NativeChild {
  return context.Provider(
    typeof source === "function"
      ? { value, get children() { return source(); } }
      : { value, get children() { return source.children; } },
  );
}

export function element(tag: NativeTag, props: NativeElementProps): NativeNode {
  const node = createElement(tag);
  spread(node, props);
  return node;
}

export function part(tag: NativeTag, sheet: Stylesheet, className: string, defaults: NativeElementProps = {}) {
  return (props: NativeElementProps = {}): NativeNode => {
    useStylesheet(sheet);
    return element(tag, mergeProps(defaults, props, {
      get class() { return classes(className, props.class); },
    }));
  };
}

export function forbidChildren(name: string, props: object): void {
  effect(() => {
    if ("children" in props) throw new TypeError(`${name} does not accept children`);
  });
}
