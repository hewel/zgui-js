import { getOwner, onCleanup } from "#solid";
import type { NativeHost } from "./host.js";
import type { NativeDismissLayer, NativeDismissLayerOptions, NativeDismissReason, NativeFocusOptions, NativeFocusTrapOptions, NativeGeometry, NativeGeometryKind, NativeNode, NativeWindowRect } from "./types.js";

declare const globalThis: NativeHost;

export function focus(node: NativeNode, options?: NativeFocusOptions): void {
  globalThis.__zgui.focus(node, options?.reveal ?? false);
}

export function getWindowRect(node: NativeNode): NativeWindowRect | null {
  return globalThis.__zgui.getWindowRect(node);
}

export function getTextContent(node: NativeNode): string {
  return globalThis.__zgui.getTextContent(node);
}

export function isAnimating(node: NativeNode): boolean {
  return globalThis.__zgui.isAnimating(node);
}

export function precedes(first: NativeNode, second: NativeNode): boolean {
  return globalThis.__zgui.precedes(first, second);
}

export function observeGeometry<K extends NativeGeometryKind>(node: NativeNode, kind: K, callback: (value: NativeGeometry[K]) => void): () => void {
  const dispose = globalThis.__zgui.observeGeometry(node, kind, callback);
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

export function trapFocus(node: NativeNode, options?: NativeFocusTrapOptions): () => void {
  const dispose = globalThis.__zgui.trapFocus(node, options?.wrap ?? true, options?.autoFocus ?? true, options?.restore ?? true);
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

export function lockScroll(node: NativeNode): () => void {
  const dispose = globalThis.__zgui.lockScroll(node);
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
  const lease = globalThis.__zgui.registerDismissLayer(node, callback, dismissOptions(options));
  if (getOwner()) onCleanup(lease.dispose);
  return { dispose: lease.dispose, update: options => lease.update(dismissOptions(options)) };
}

export function installStylesheet(name: string, css: string): void {
  globalThis.__zgui.installStylesheet(name, css);
}

export function removeStylesheet(name: string): void {
  globalThis.__zgui.removeStylesheet(name);
}

export function mountStylesheet(name: string, css: string): () => void {
  globalThis.__zgui.installStylesheet(name, css);
  let removed = false;
  const dispose = () => {
    if (removed) return;
    removed = true;
    globalThis.__zgui.removeStylesheet(name);
  };
  if (getOwner()) onCleanup(dispose);
  return dispose;
}
