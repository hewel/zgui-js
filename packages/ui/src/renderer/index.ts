import { createRenderer } from "@solidjs/universal";
import { $PROXY, createRoot, flush as solidFlush, resetErrorHalt } from "#solid";
import type { Accessor, Component, ImageProps, NativeChild, NativeElementProps, NativeFieldProps, NativeNode, NativeTag, NativeTextControlProps, VectorProps } from "../types.js";
import { attachReducedMotionBridge, bridgeFor, provideRegion, rememberBridge, tryBridgeFor, withCallbackBridge } from "../host.js";
import type { NativeHost, UiBridge } from "../host.js";
import type { InternalProps } from "../internal-props.js";
import { invalidateTreeOrder } from "../tree-order.js";

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
  options: Omit<NativeHost["__zgui"], "createElement" | "replaceText" | "clearRoot" | "createPortal" | "createCanvasScene" | "registerDismissLayer" | "installStylesheet" | "removeStylesheet" | "focus" | "trapFocus" | "lockScroll" | "scrollTo" | "scrollBy" | "scrollIntoView" | "getSelection" | "setSelection" | "selectAll" | "getDocumentSelection" | "setDocumentSelection" | "clearDocumentSelection" | "selectDocumentAll" | "setWindowTitle" | "setWindowFullscreen" | "setWindowMaximized" | "setWindowMinimized" | "setWindowDecorations" | "beginWindowMoveDrag" | "closeWindow" | "getWindowState" | "subscribeWindowState" | "onCloseRequest" | "getWindowRect" | "getTextContent" | "isAnimating" | "precedes" | "observeGeometry" | "getReducedMotion"> & {
    createElement(tag: NativeTag, staticProps?: Record<string, unknown>): NativeNode;
    replaceText(node: NativeNode, text: string | number): void;
  },
) => NativeRenderer;

function setNativeProperty(node: NativeNode, name: string, value: unknown, previous?: unknown): void {
  const bridge = bridgeFor(node);
  if (name.startsWith("on") && typeof value === "function") {
    const callback = value as (this: unknown, event: unknown) => unknown;
    value = function(this: unknown, event: unknown) {
      return withCallbackBridge(bridge, () => callback.call(this, event));
    };
  }
  bridge.setProperty(node, name, value, previous);
}

// Resolve the bridge at invocation, so module preflight never allocates native nodes.
const renderer = createNativeRenderer({
  createElement: (tag, staticProps) => {
    const bridge = bridgeFor();
    const node = rememberBridge(bridge.createElement(tag), bridge);
    if (staticProps) {
      for (const key of Object.keys(staticProps)) {
        if (key !== "children") setNativeProperty(node, key, staticProps[key]);
      }
    }
    return node;
  },
  createTextNode: (text: string | number) => {
    const bridge = bridgeFor();
    return rememberBridge(bridge.createTextNode(typeof text === "number" ? `${text}` : text), bridge);
  },
  replaceText: (node, text: string | number) => bridgeFor(node).replaceText(node, typeof text === "number" ? `${text}` : text),
  isTextNode: node => bridgeFor(node).isTextNode(node),
  insertNode: (parent, node, anchor) => {
    bridgeFor(parent).insertNode(parent, node, anchor);
    invalidateTreeOrder();
  },
  removeNode: (parent, node) => {
    bridgeFor(parent).removeNode(parent, node);
    invalidateTreeOrder();
  },
  getParentNode: node => bridgeFor(node).getParentNode(node),
  getFirstChild: node => bridgeFor(node).getFirstChild(node),
  getNextSibling: node => bridgeFor(node).getNextSibling(node),
  setProperty: setNativeProperty,
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
    for (const key of keys) {
      if (key !== "children" && Object.getOwnPropertyDescriptor(record, key)?.get !== undefined) {
        hasGetter = true;
        break;
      }
    }
    if (!hasGetter) {
      for (const key of keys) {
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
          setNativeProperty(node, key, value);
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
  let offset = 0;
  for (const source of sources) {
    const prior = mergedSources.get(source);
    if (prior) {
      inherited ??= sources.slice(0, offset);
      inherited.push(...prior);
    } else if (inherited) {
      inherited.push(source);
    }
    offset += 1;
  }
  const list = inherited ?? sources;
  let c0 = 0, c1 = 0, c2 = 0;
  for (const item of list) {
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
        for (const original of originals) {
          const value = Reflect.get(original, "__zguiClass");
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
          for (const original of originals) {
            const value = Reflect.get(original, key);
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
  const dispose = renderer.render(() => {
    const bridge = tryBridgeFor(root);
    if (bridge) attachReducedMotionBridge(bridge);
    return code();
  }, root);
  return () => {
    dispose();
    flush();
  };
}

export function renderRegion(code: () => NativeChild, root: NativeNode, bridge: UiBridge): () => void {
  rememberBridge(root, bridge);
  return mountOwned(() => provideRegion(bridge, () => {
    attachReducedMotionBridge(bridge);
    insert(root, code());
  }), () => bridge.clearRoot(root), "region");
}

export function mountOwned(mount: () => void, clear?: () => void, scope: "root" | "region" = "root"): () => void {
  let disposeRoot: (() => void) | undefined;
  let disposed = false;

  function dispose() {
    if (disposed) return;
    disposed = true;
    const errors: unknown[] = [];
    try {
      disposeRoot?.();
      // Region cleanup runs under the host disposal fence; leave other owners' jobs to the host.
      if (scope === "root") flush();
    } catch (error) {
      errors.push(error);
    } finally {
      resetErrorHalt();
    }
    try {
      clear?.();
    } catch (error) {
      errors.push(error);
    }
    if (errors.length === 1) throw errors[0];
    if (errors.length > 1) throw new AggregateError(errors, "Application cleanup failed");
  }

  try {
    // Own the root directly: Universal.render cannot return its disposer if mount throws.
    createRoot(dispose => {
      disposeRoot = dispose;
      mount();
    });
    flush();
  } catch (error) {
    try {
      dispose();
    } catch (cleanupError) {
      throw new AggregateError([error, cleanupError], "Application mount and cleanup failed");
    }
    throw error;
  }
  return dispose;
}
