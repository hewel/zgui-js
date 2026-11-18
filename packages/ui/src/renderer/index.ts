import { createRenderer } from "solid-js/universal";
import { batch } from "#solid";
import type { Accessor, Component, ImageProps, NativeChild, NativeElementProps, NativeFieldProps, NativeNode, NativeTag, NativeTextControlProps, VectorProps } from "../types.js";
import type { NativeHost } from "../host.js";
import type { InternalProps } from "../internal-props.js";
import { invalidateTreeOrder } from "../tree-order.js";

declare const globalThis: NativeHost;

/** Native specialization of Solid Universal's DOM-independent runtime helpers. */
interface NativeRenderer {
  render(code: () => NativeChild, root: NativeNode): () => void;
  effect<T>(callback: (previous: T | undefined) => T, initial?: T): void;
  memo<T>(callback: () => T, equal?: boolean): Accessor<T>;
  createComponent<P>(component: Component<P>, props: P): NativeChild;
  createElement(tag: NativeTag): NativeNode;
  createTextNode(text: string): NativeNode;
  insertNode(parent: NativeNode, node: NativeNode, anchor?: NativeNode | null): void;
  insert(parent: NativeNode, value: NativeChild, marker?: NativeNode | null, initial?: NativeChild): void;
  spread(node: NativeNode, props: (NativeElementProps | NativeTextControlProps | NativeFieldProps | ImageProps | VectorProps) & InternalProps | Accessor<(NativeElementProps | NativeTextControlProps | NativeFieldProps | ImageProps | VectorProps) & InternalProps>, skipChildren?: boolean): void;
  setProp<T>(node: NativeNode, name: string, value: T, previous?: T): T;
  mergeProps<T extends readonly object[]>(...sources: T): MergeSources<T>;
  use<A, T>(callback: (node: NativeNode, argument: A) => T, node: NativeNode, argument: A): T;
}
type MergeSources<T extends readonly object[]> = T extends readonly [infer Head extends object, ...infer Tail extends object[]]
  ? Omit<Head, keyof MergeSources<Tail>> & MergeSources<Tail> : {};

// Upstream declarations restrict children to one NodeType, erase mergeProps keys,
// and omit null navigation results. Specialize that external boundary only;
// forwarding host values unchanged preserves the pinned reconciler's behavior.
const createNativeRenderer = createRenderer as (
  options: Omit<NativeHost["__zgui"], "clearRoot" | "createPortal" | "registerDismissLayer" | "installStylesheet" | "removeStylesheet" | "focus" | "trapFocus" | "lockScroll" | "getWindowRect" | "getTextContent" | "isAnimating" | "precedes" | "observeGeometry" | "getReducedMotion">,
) => NativeRenderer;

// Resolve the bridge at invocation, so module preflight never allocates native nodes.
const renderer = createNativeRenderer({
  createElement: tag => globalThis.__zgui.createElement(tag),
  createTextNode: (text: string | number) => globalThis.__zgui.createTextNode(typeof text === "number" ? `${text}` : text),
  replaceText: (node, text) => globalThis.__zgui.replaceText(node, text),
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
  insertNode, insert, spread, setProp, use,
} = renderer;

// Wrapper components contribute structural classes independently of application aliases.
const internalChannels = ["__zguiClass", "__zguiStyle", "__zguiInvariantStyle"];
const mergedSources = new WeakMap<object, readonly object[]>();
export function mergeProps<T extends readonly object[]>(...sources: T): MergeSources<T> {
  const inherited = sources.flatMap(source => mergedSources.get(source) ?? [source]);
  const merged = renderer.mergeProps(...inherited);
  const originals = inherited.filter(source => source !== merged);
  if (!internalChannels.some(key => originals.filter(source => key in source).length > 1)) return merged as unknown as MergeSources<T>;
  const result = new Proxy(merged, {
    get(target, key, receiver) {
      if (typeof key === "string" && internalChannels.includes(key)) {
        const values = originals.map(source => Reflect.get(source, key)).filter(Boolean);
        return key === "__zguiClass" ? values.join(" ") : Object.assign({}, ...values);
      }
      return Reflect.get(target, key, receiver);
    },
  });
  mergedSources.set(result, originals);
  return result as unknown as MergeSources<T>;
}

export function render(code: () => NativeChild, root: NativeNode): () => void {
  const dispose = batch(() => renderer.render(code, root));
  return () => batch(dispose);
}
