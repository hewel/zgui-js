import { createRenderer } from "@solidjs/universal";
import { $PROXY, flush as solidFlush, resetErrorHalt } from "#solid";
import type { Accessor, Component, ImageProps, NativeChild, NativeElementProps, NativeFieldProps, NativeNode, NativeTag, NativeTextControlProps, VectorProps } from "../types.js";
import type { NativeHost } from "../host.js";
import type { InternalProps } from "../internal-props.js";
import { invalidateTreeOrder } from "../tree-order.js";

declare const globalThis: NativeHost;

/** Native specialization of Solid Universal's DOM-independent runtime helpers. */
interface NativeRenderer {
  render(code: () => NativeChild, root: NativeNode): () => void;
  effect<T>(compute: (previous: T | undefined) => T, apply: (value: T, previous: T | undefined) => void | (() => void)): void;
  memo<T>(callback: () => T, equal?: boolean): Accessor<T>;
  createComponent<P>(component: Component<P>, props: P): NativeChild;
  createElement(tag: NativeTag): NativeNode;
  createTextNode(text: string): NativeNode;
  insertNode(parent: NativeNode, node: NativeNode, anchor?: NativeNode | null): void;
  insert(parent: NativeNode, value: NativeChild, marker?: NativeNode | null, initial?: NativeChild): void;
  spread(node: NativeNode, props: (NativeElementProps | NativeTextControlProps | NativeFieldProps | ImageProps | VectorProps) & InternalProps | Accessor<(NativeElementProps | NativeTextControlProps | NativeFieldProps | ImageProps | VectorProps) & InternalProps>, skipChildren?: boolean): void;
  setProp<T>(node: NativeNode, name: string, value: T, previous?: T): T;
  mergeProps<T extends readonly object[]>(...sources: T): MergeSources<T>;
  applyRef(callback: ((node: NativeNode) => void) | ((node: NativeNode) => void)[], node: NativeNode): void;
  ref(fn: () => ((node: NativeNode) => void) | ((node: NativeNode) => void)[], node: NativeNode): void;
}
type MergeSources<T extends readonly object[]> = T extends readonly [infer Head extends object, ...infer Tail extends object[]]
  ? Omit<Head, keyof MergeSources<Tail>> & MergeSources<Tail> : {};

// Upstream declarations restrict children to one NodeType and omit null navigation results.
const createNativeRenderer = createRenderer as (
  options: Omit<NativeHost["__zgui"], "createElement" | "replaceText" | "clearRoot" | "createPortal" | "createCanvasScene" | "registerDismissLayer" | "installStylesheet" | "removeStylesheet" | "focus" | "trapFocus" | "lockScroll" | "getWindowRect" | "getTextContent" | "isAnimating" | "precedes" | "observeGeometry" | "getReducedMotion"> & {
    createElement(tag: NativeTag, staticProps?: Record<string, unknown>): NativeNode;
    replaceText(node: NativeNode, text: string | number): void;
  },
) => NativeRenderer;

// Resolve the bridge at invocation, so module preflight never allocates native nodes.
const renderer = createNativeRenderer({
  createElement: (tag, staticProps) => {
    const node = globalThis.__zgui.createElement(tag);
    if (staticProps) {
      for (const key of Object.keys(staticProps)) {
        if (key !== "children") globalThis.__zgui.setProperty(node, key, staticProps[key]);
      }
    }
    return node;
  },
  createTextNode: (text: string | number) => globalThis.__zgui.createTextNode(typeof text === "number" ? `${text}` : text),
  replaceText: (node, text: string | number) => globalThis.__zgui.replaceText(node, typeof text === "number" ? `${text}` : text),
  isTextNode: node => globalThis.__zgui.isTextNode(node),
  insertNode: (parent, node, anchor) => {
    globalThis.__zgui.insertNode(parent, node, anchor);
    invalidateTreeOrder();
  },
  removeNode: (parent, node) => {
    globalThis.__zgui.removeNode(parent, node);
    invalidateTreeOrder();
  },
  getParentNode: node => globalThis.__zgui.getParentNode(node),
  getFirstChild: node => globalThis.__zgui.getFirstChild(node),
  getNextSibling: node => globalThis.__zgui.getNextSibling(node),
  setProperty: (node, name, value, previous) =>
    globalThis.__zgui.setProperty(node, name, value, previous),
});

export const {
  effect, memo, createComponent, createElement, createTextNode,
  insertNode, insert, setProp, applyRef, ref,
} = renderer;

export function spread(
  node: NativeNode,
  props: (NativeElementProps | NativeTextControlProps | NativeFieldProps | ImageProps | VectorProps) & InternalProps | Accessor<(NativeElementProps | NativeTextControlProps | NativeFieldProps | ImageProps | VectorProps) & InternalProps>,
  skipChildren?: boolean,
): void {
  if (props != null && typeof props === "object" && !Array.isArray(props)) {
    // QuickJS-NG drops getter descriptors from Proxy getOwnPropertyDescriptor traps.
    if ($PROXY in props) {
      if (!skipChildren) {
        renderer.spread(node, props, true);
        if ("children" in props) {
          renderer.insert(node, () => (props as { children?: NativeChild }).children);
        }
        return;
      }
      renderer.spread(node, props, true);
      return;
    }
    const record = props as Record<string, unknown>;
    const keys = Object.keys(record);
    let hasGetter = false;
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (key !== "children" && Object.getOwnPropertyDescriptor(record, key)?.get !== undefined) {
        hasGetter = true;
        break;
      }
    }
    if (!hasGetter) {
      for (let i = 0; i < keys.length; i++) {
        const key = keys[i];
        if (key === "children") {
          if (!skipChildren) {
            const desc = Object.getOwnPropertyDescriptor(record, "children");
            if (desc?.get !== undefined) renderer.insert(node, () => record.children as NativeChild);
            else if (desc?.value !== undefined) renderer.insert(node, desc.value as NativeChild);
          }
          continue;
        }
        const value = record[key];
        if (key === "ref") {
          if (typeof value === "function" || Array.isArray(value)) {
            renderer.ref(() => value as ((node: NativeNode) => void) | ((node: NativeNode) => void)[], node);
          }
        } else if (value !== undefined) {
          globalThis.__zgui.setProperty(node, key, value);
        }
      }
      return;
    }
  }
  renderer.spread(node, props, skipChildren);
}

export function flush(): void;
export function flush<T>(fn: () => T): T;
export function flush<T>(fn?: () => T): T | void {
  try {
    return fn ? solidFlush(fn) : solidFlush();
  } catch (error) {
    resetErrorHalt();
    throw error;
  }
}

// Append a trailing channel source inside MergeView so viewOf() sees merged channels directly.
const mergedSources = new WeakMap<object, readonly object[]>();
export function mergeProps<T extends readonly object[]>(...sources: T): MergeSources<T> {
  let inherited: object[] | null = null;
  for (let i = 0; i < sources.length; i++) {
    const prior = mergedSources.get(sources[i]);
    if (prior) {
      inherited ??= sources.slice(0, i);
      inherited.push(...prior);
    } else if (inherited) {
      inherited.push(sources[i]);
    }
  }
  const list = inherited ?? sources;
  let c0 = 0, c1 = 0, c2 = 0;
  for (let i = 0; i < list.length; i++) {
    const item = list[i];
    if ("__zguiClass" in item) c0 += 1;
    if ("__zguiStyle" in item) c1 += 1;
    if ("__zguiInvariantStyle" in item) c2 += 1;
  }
  if (c0 <= 1 && c1 <= 1 && c2 <= 1) return renderer.mergeProps(...list) as unknown as MergeSources<T>;
  const originals = [...list];
  const channelOverrides: Record<string, unknown> = {};
  if (c0 > 1) {
    Object.defineProperty(channelOverrides, "__zguiClass", {
      enumerable: true,
      configurable: true,
      get() {
        let joined = "";
        for (let i = 0; i < originals.length; i++) {
          const value = Reflect.get(originals[i], "__zguiClass");
          if (value) joined = joined ? `${joined} ${String(value)}` : String(value);
        }
        return joined;
      },
    });
  }
  for (const [count, key] of [[c1, "__zguiStyle"], [c2, "__zguiInvariantStyle"]] as const) {
    if (count > 1) {
      Object.defineProperty(channelOverrides, key, {
        enumerable: true,
        configurable: true,
        get() {
          const combined: Record<string, unknown> = {};
          for (let i = 0; i < originals.length; i++) {
            const value = Reflect.get(originals[i], key);
            if (value) Object.assign(combined, value);
          }
          return combined;
        },
      });
    }
  }
  const result = renderer.mergeProps(...list, channelOverrides);
  mergedSources.set(result, originals);
  return result as unknown as MergeSources<T>;
}

export function render(code: () => NativeChild, root: NativeNode): () => void {
  const dispose = renderer.render(code, root);
  return () => {
    dispose();
    flush();
  };
}
