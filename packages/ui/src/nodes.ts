import { getOwner, onCleanup } from "#solid";
import { flush } from "./renderer/index.js";
import { bridgeFor, bridgeForTarget, getReducedMotion, setReducedMotion, withCallbackBridge } from "./host.js";
import type { CanvasScene, NativeDismissLayer, NativeDismissLayerOptions, NativeDismissReason, NativeFocusOptions, NativeFocusTrapOptions, NativeGeometry, NativeGeometryKind, NativeNode, NativeWindowRect } from "./types.js";

export { getReducedMotion, setReducedMotion };

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

export function observeGeometry<K extends NativeGeometryKind>(node: NativeNode, kind: K, callback: (value: NativeGeometry[K]) => void): () => void {
  const bridge = bridgeFor(node);
  const dispose = bridge.observeGeometry(node, kind, value => withCallbackBridge(bridge, () => {
    callback(value);
    flush();
  }));
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

export function trapFocus(node: NativeNode, options?: NativeFocusTrapOptions): () => void {
  const dispose = bridgeFor(node).trapFocus(node, options?.wrap ?? true, options?.autoFocus ?? true, options?.restore ?? true);
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

export function lockScroll(node: NativeNode): () => void {
  const dispose = bridgeFor(node).lockScroll(node);
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

function dismissOptions(options: NativeDismissLayerOptions = {}): Required<NativeDismissLayerOptions> {
  return { layer: options.layer ?? "popover", dismissOnOutsidePress: options.dismissOnOutsidePress ?? true,
    dismissOnEscape: options.dismissOnEscape ?? true, exclude: options.exclude ?? null, leaving: options.leaving ?? false };
}

export function registerDismissLayer(
  node: NativeNode, callback: (reason: NativeDismissReason) => void, options?: NativeDismissLayerOptions,
): NativeDismissLayer {
  const bridge = bridgeFor(node);
  const lease = bridge.registerDismissLayer(node, reason => withCallbackBridge(bridge, () => callback(reason)), dismissOptions(options));
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
