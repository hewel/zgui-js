import type { NativeDismissLayerOptions, NativeDismissReason, NativeGeometry, NativeGeometryKind, NativeNode, NativeOverlayLayer, NativeWindowRect } from "./types.js";

/** Installed by UiBridge::install; resolving it must remain lazy during preflight. */
export interface NativeHost {
  readonly __zgui: {
    createElement(tag: string): NativeNode;
    createPortal(layer: NativeOverlayLayer): NativeNode;
    registerDismissLayer(
      node: NativeNode, callback: (reason: NativeDismissReason) => void,
      options: Required<NativeDismissLayerOptions>,
    ): { update(options: Required<NativeDismissLayerOptions>): void; dispose(): void; };
    createTextNode(text: string): NativeNode;
    replaceText(node: NativeNode, text: string): void;
    isTextNode(node: NativeNode): boolean;
    insertNode(parent: NativeNode, node: NativeNode, anchor?: NativeNode | null): void;
    removeNode(parent: NativeNode, node: NativeNode): void;
    getParentNode(node: NativeNode): NativeNode | null | undefined;
    getFirstChild(node: NativeNode): NativeNode | null | undefined;
    getNextSibling(node: NativeNode): NativeNode | null | undefined;
    setProperty(node: NativeNode, name: string, value: unknown, previous?: unknown): void;
    installStylesheet(name: string, css: string): void;
    removeStylesheet(name: string): void;
    getReducedMotion(): boolean;
    focus(node: NativeNode, reveal?: boolean): void;
    trapFocus(node: NativeNode, wrap: boolean, autoFocus: boolean, restore: boolean): () => void;
    lockScroll(node: NativeNode): () => void;
    getWindowRect(node: NativeNode): NativeWindowRect | null;
    getTextContent(node: NativeNode): string;
    isAnimating(node: NativeNode): boolean;
    observeGeometry<K extends NativeGeometryKind>(
      node: NativeNode,
      kind: K,
      callback: (value: NativeGeometry[K]) => void,
    ): () => void;
    precedes(first: NativeNode, second: NativeNode): boolean;
    clearRoot(root: NativeNode): void;
  };
}
