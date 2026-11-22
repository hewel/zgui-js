import { createEffect, getOwner, onCleanup } from "#solid";
import { flush } from "./renderer/index.js";
import {
  bridgeFor,
  bridgeForTarget,
  getBackgroundBlurSupported,
  getReducedMotion,
  setReducedMotion,
  withCallbackBridge,
} from "./host.js";
import type {
  Accessor,
  CanvasScene,
  DocumentSelection,
  DocumentSelectionPoint,
  NativeDismissLayer,
  NativeDismissLayerOptions,
  NativeDismissReason,
  NativeFocusOptions,
  NativeFocusTrapOptions,
  NativeGeometry,
  NativeGeometryKind,
  NativeNode,
  NativeNodeTarget,
  NativeWindowRect,
  ScrollBehavior,
  ScrollIntoViewOptions,
  ScrollToOptions,
  TextSelectionRange,
} from "./types.js";

export { getBackgroundBlurSupported, getReducedMotion, setReducedMotion };

export function focus(node: NativeNode, options?: NativeFocusOptions): void {
  bridgeFor(node).focus(node, options?.reveal ?? false);
}

export function getWindowRect(node: NativeNode): NativeWindowRect | null {
  return bridgeFor(node).getWindowRect(node);
}

export function getTextContent(node: NativeNode): string {
  return bridgeFor(node).getTextContent(node);
}

export function isAnimating(node: NativeNode): boolean {
  return bridgeFor(node).isAnimating(node);
}

export function precedes(first: NativeNode, second: NativeNode): boolean {
  return bridgeFor(first).precedes(first, second);
}

/** Creates a retained drawing, disposed automatically under a Solid owner. */
export function createCanvasScene(): CanvasScene {
  const scene = bridgeFor().createCanvasScene();
  if (getOwner()) onCleanup(() => scene.dispose());
  return scene;
}

function bindNodeLease(
  target: Accessor<NativeNode | null | undefined>,
  attach: (node: NativeNode) => () => void,
): () => void {
  let active: (() => void) | undefined;
  let disposed = false;
  const stop = () => {
    if (disposed) return;
    disposed = true;
    active?.();
    active = undefined;
  };
  createEffect(
    () => (disposed ? null : target()),
    node => {
      active?.();
      active = undefined;
      if (!node || disposed) return;
      const lease = attach(node);
      active = lease;
      return () => {
        lease();
        if (active === lease) active = undefined;
      };
    },
  );
  if (getOwner()) onCleanup(stop);
  return stop;
}

export function observeGeometry<K extends NativeGeometryKind>(
  node: NativeNodeTarget,
  kind: K,
  callback: (value: NativeGeometry[K]) => void,
): () => void {
  const attach = (target: NativeNode): (() => void) => {
    const bridge = bridgeFor(target);
    return bridge.observeGeometry(target, kind, value =>
      withCallbackBridge(bridge, () => {
        callback(value);
        flush();
      }),
    );
  };
  if (typeof node === "function") return bindNodeLease(node, attach);
  const dispose = attach(node);
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

export function trapFocus(node: NativeNodeTarget, options?: NativeFocusTrapOptions): () => void {
  const wrap = options?.wrap ?? true;
  const autoFocus = options?.autoFocus ?? true;
  const restore = options?.restore ?? true;
  const attach = (target: NativeNode): (() => void) =>
    bridgeFor(target).trapFocus(target, wrap, autoFocus, restore);
  if (typeof node === "function") return bindNodeLease(node, attach);
  const dispose = attach(node);
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

function resolveNode(target: NativeNodeTarget): NativeNode | null {
  return typeof target === "function" ? (target() ?? null) : target;
}

function resolveScrollBehavior(behavior?: ScrollBehavior): ScrollBehavior {
  const requested = behavior ?? "instant";
  if (requested !== "instant" && requested !== "smooth") {
    throw new TypeError(`unsupported scroll behavior: ${String(requested)}`);
  }
  return requested === "smooth" && getReducedMotion() ? "instant" : requested;
}

function validateScrollOffset(value: number | undefined, name: string): number | null {
  if (value === undefined) return null;
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new RangeError(`scroll ${name} offset must be a finite number`);
  }
  return value;
}

/** Scroll a scroll container to absolute CSS pixel offsets. Downgrades `smooth` to `instant` under reduced motion. */
export function scrollTo(target: NativeNodeTarget, options: ScrollToOptions = {}): void {
  const node = resolveNode(target);
  if (!node) return;
  const left = validateScrollOffset(options.left, "left");
  const top = validateScrollOffset(options.top, "top");
  const behavior = resolveScrollBehavior(options.behavior);
  bridgeFor(node).scrollTo(node, left, top, behavior);
}

/** Scroll a scroll container by relative CSS pixel deltas. Downgrades `smooth` to `instant` under reduced motion. */
export function scrollBy(target: NativeNodeTarget, options: ScrollToOptions = {}): void {
  const node = resolveNode(target);
  if (!node) return;
  const left = validateScrollOffset(options.left, "left") ?? 0;
  const top = validateScrollOffset(options.top, "top") ?? 0;
  const behavior = resolveScrollBehavior(options.behavior);
  bridgeFor(node).scrollBy(node, left, top, behavior);
}

/** Bring a node into view inside its nearest scroll container. Downgrades `smooth` to `instant` under reduced motion. */
export function scrollIntoView(
  target: NativeNodeTarget,
  options: ScrollIntoViewOptions = {},
): void {
  const node = resolveNode(target);
  if (!node) return;
  const block = options.block ?? "nearest";
  if (block !== "nearest" && block !== "start" && block !== "center" && block !== "end") {
    throw new TypeError(`unsupported scrollIntoView block: ${String(block)}`);
  }
  const behavior = resolveScrollBehavior(options.behavior);
  bridgeFor(node).scrollIntoView(node, block, behavior);
}

/** Read the current selection range in UTF-16 code units from an editable field/editor, or `null` when unset. */
export function getSelection(target: NativeNodeTarget): TextSelectionRange | null {
  const node = resolveNode(target);
  if (!node) return null;
  return bridgeFor(node).getSelection(node);
}

/** Set the caret or selection range in UTF-16 code units on an editable field/editor. Defaults `end` to `start`. */
export function setSelection(target: NativeNodeTarget, start: number, end: number = start): void {
  const node = resolveNode(target);
  if (!node) return;
  if (typeof start !== "number" || !Number.isFinite(start)) {
    throw new RangeError("selection start offset must be a finite number");
  }
  if (typeof end !== "number" || !Number.isFinite(end)) {
    throw new RangeError("selection end offset must be a finite number");
  }
  bridgeFor(node).setSelection(node, start, end);
}

/** Select the entire text value of an editable field/editor. */
export function selectAll(target: NativeNodeTarget): void {
  const node = resolveNode(target);
  if (!node) return;
  bridgeFor(node).selectAll(node);
}

/** Read the active opt-in read-only cross-block document selection (UTF-16 offsets), or `null`. */
export function getDocumentSelection(regionNode?: NativeNodeTarget): DocumentSelection | null {
  if (regionNode !== undefined) {
    const node = resolveNode(regionNode);
    if (!node) return null;
    return bridgeFor(node).getDocumentSelection();
  }
  return bridgeFor().getDocumentSelection();
}

/** Programmatically set read-only cross-block document selection inside an opt-in container (UTF-16 offsets). */
export function setDocumentSelection(
  containerTarget: NativeNodeTarget,
  anchor: DocumentSelectionPoint,
  focus: DocumentSelectionPoint = anchor,
): void {
  const container = resolveNode(containerTarget);
  if (!container) return;
  if (typeof anchor.offset !== "number" || !Number.isFinite(anchor.offset)) {
    throw new RangeError("document selection anchor.offset must be a finite number");
  }
  if (typeof focus.offset !== "number" || !Number.isFinite(focus.offset)) {
    throw new RangeError("document selection focus.offset must be a finite number");
  }
  bridgeFor(container).setDocumentSelection(
    container,
    anchor.node,
    anchor.offset,
    focus.node,
    focus.offset,
  );
}

/** Clear the active read-only cross-block document selection in the current window/region. */
export function clearDocumentSelection(regionNode?: NativeNodeTarget): void {
  if (regionNode !== undefined) {
    const node = resolveNode(regionNode);
    if (!node) return;
    bridgeFor(node).clearDocumentSelection();
    return;
  }
  bridgeFor().clearDocumentSelection();
}

/** Select all selectable read-only text inside an opt-in `documentSelection` container. */
export function selectDocumentAll(containerTarget: NativeNodeTarget): void {
  const container = resolveNode(containerTarget);
  if (!container) return;
  bridgeFor(container).selectDocumentAll(container);
}

export function lockScroll(node: NativeNodeTarget): () => void {
  const attach = (target: NativeNode): (() => void) => bridgeFor(target).lockScroll(target);
  if (typeof node === "function") return bindNodeLease(node, attach);
  const dispose = attach(node);
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

function dismissOptions(
  options: NativeDismissLayerOptions = {},
): Required<NativeDismissLayerOptions> {
  return {
    layer: options.layer ?? "popover",
    dismissOnOutsidePress: options.dismissOnOutsidePress ?? true,
    dismissOnEscape: options.dismissOnEscape ?? true,
    exclude: options.exclude ?? null,
    leaving: options.leaving ?? false,
  };
}

export function registerDismissLayer(
  node: NativeNodeTarget,
  callback: (reason: NativeDismissReason) => void,
  options?: NativeDismissLayerOptions,
): NativeDismissLayer {
  if (typeof node === "function") {
    let currentOptions = dismissOptions(options);
    let active: NativeDismissLayer | undefined;
    let disposed = false;
    const dispose = () => {
      if (disposed) return;
      disposed = true;
      active?.dispose();
      active = undefined;
    };
    createEffect(
      () => (disposed ? null : node()),
      target => {
        active?.dispose();
        active = undefined;
        if (!target || disposed) return;
        const bridge = bridgeFor(target);
        const lease = bridge.registerDismissLayer(
          target,
          reason => withCallbackBridge(bridge, () => callback(reason)),
          currentOptions,
        );
        active = lease;
        return () => {
          lease.dispose();
          if (active === lease) active = undefined;
        };
      },
    );
    if (getOwner()) onCleanup(dispose);
    return {
      dispose,
      update(next) {
        currentOptions = dismissOptions(next);
        if (!disposed) active?.update(currentOptions);
      },
    };
  }
  const bridge = bridgeFor(node);
  const lease = bridge.registerDismissLayer(
    node,
    reason => withCallbackBridge(bridge, () => callback(reason)),
    dismissOptions(options),
  );
  if (getOwner()) onCleanup(lease.dispose);
  return { dispose: lease.dispose, update: options => lease.update(dismissOptions(options)) };
}

/** Mount a stylesheet in the current region, or in the region of a live, attached target. */
export function mountStylesheet(name: string, css: string, target?: NativeNode): () => void {
  const bridge = target === undefined ? bridgeFor() : bridgeForTarget(target);
  bridge.installStylesheet(name, css, target);
  let removed = false;
  const dispose = () => {
    if (removed) return;
    removed = true;
    bridge.removeStylesheet(name);
  };
  if (getOwner()) onCleanup(dispose);
  return dispose;
}
