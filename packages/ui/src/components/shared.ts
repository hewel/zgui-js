import { createElement, effect, mergeProps, spread } from "../renderer/index.js";
import type { NativeHost } from "../host.js";
import type { NativeElementProps, NativeNode, NativeTag } from "../types.js";

declare const globalThis: NativeHost;

export interface Stylesheet {
  readonly name: string;
  readonly className: string;
  readonly css: string;
}

export function classes(...values: readonly (string | null | undefined | false)[]): string {
  return values.filter(Boolean).join(" ");
}

export function useStylesheet(sheet: Stylesheet): void {
  globalThis.__zgui.installStylesheet(sheet.name, `@layer zgui.components { ${sheet.css} }`);
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
