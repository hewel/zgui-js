import { $PROXY } from "#solid";
import type { Context } from "#solid";
import { createElement, effect, insert, insertNode, mergeProps, setProp, spread } from "../renderer/index.js";
import { bridgeFor } from "../host.js";
import type { NativeChild, NativeElementProps, NativeNode, NativeTag } from "../types.js";
import type { InternalProps } from "../internal-props.js";

export interface Stylesheet {
  readonly name: string;
  readonly className: string;
  readonly css: string;
}

const installedSheets = new WeakMap<object, Set<string>>();

export function classes(...values: readonly (string | null | undefined | false)[]): string {
  return values.filter(Boolean).join(" ");
}

export function withDefaults<D extends object, P extends object>(
  defaults: D,
  props: P,
): D {
  const defaultRecord = defaults as Record<string, unknown>;
  const propRecord = props as Record<string, unknown>;
  const defaultKeys = Object.keys(defaultRecord);
  let hasOverlap = false;
  for (const key of defaultKeys) {
    if (key in propRecord) {
      hasOverlap = true;
      break;
    }
  }
  if (!hasOverlap) return defaults;
  const resolved: Record<string, unknown> = {};
  for (const key of defaultKeys) {
    if (key in propRecord) {
      Object.defineProperty(resolved, key, {
        enumerable: true,
        configurable: true,
        get() {
          const value = propRecord[key];
          return value === undefined ? defaultRecord[key] : value;
        },
      });
    } else {
      const desc = Object.getOwnPropertyDescriptor(defaultRecord, key);
      if (desc) Object.defineProperty(resolved, key, desc);
    }
  }
  return resolved as D;
}

export function useStylesheet(sheet: Stylesheet): void {
  if (sheet.css === "") return;
  const host = bridgeFor();
  const installed = installedSheets.get(host);
  if (installed?.has(sheet.name)) return;
  host.installStylesheet(sheet.name, `@layer zgui.components { ${sheet.css} }`);
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
  if (typeof source === "function") {
    return context({ value, get children() { return source(); } });
  }
  if (!($PROXY in source)) {
    const desc = Object.getOwnPropertyDescriptor(source, "children");
    if (desc) {
      const next: { value: T; children?: NativeChild } = { value };
      Object.defineProperty(next, "children", desc);
      return context(next);
    }
  }
  return context({ value, get children() { return source.children; } });
}

export function isStaticProps(props: object): boolean {
  if ($PROXY in props) return false;
  for (const key of Object.keys(props)) {
    if (key !== "children" && Object.getOwnPropertyDescriptor(props, key)?.get !== undefined) {
      return false;
    }
  }
  return true;
}

export function element(tag: NativeTag, props: NativeElementProps & InternalProps): NativeNode {
  const node = createElement(tag);
  spread(node, props);
  return node;
}

export function part(tag: NativeTag, sheet: Stylesheet, className: string, defaults: NativeElementProps = {}) {
  const hasDefaults = Object.keys(defaults).length > 0;
  return (props: NativeElementProps = {}): NativeNode => {
    useStylesheet(sheet);
    if (!hasDefaults && !($PROXY in props) && !("__zguiClass" in props)) {
      const node = createElement(tag);
      setProp(node, "__zguiClass", className);
      spread(node, props);
      return node;
    }
    return element(tag, mergeProps(props, withDefaults(defaults, props), { __zguiClass: className }));
  };
}

export function forbidChildren(name: string, props: object): void {
  if (!($PROXY in props)) {
    if ("children" in props) throw new TypeError(`${name} does not accept children`);
    return;
  }
  effect(() => "children" in props, hasChildren => {
    if (hasChildren) throw new TypeError(`${name} does not accept children`);
  });
}
