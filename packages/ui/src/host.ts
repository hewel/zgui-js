import { createContext, createMemo, getOwner, runWithOwner, useContext } from "#solid";
import type { Owner } from "#solid";
import type {
  CanvasScene, LayerAnchor, LayerSurfacePlacement, Monitor, NativeDismissLayerOptions, NativeDismissReason,
  NativeGeometry, NativeGeometryKind, NativeNode, NativeOverlayLayer, NativeWindowRect,
  PopupSurfacePlacement, PopupSurfaceProps, SessionLockState, WindowDecorations,
} from "./types.js";

export type LayerPlacement = Required<Omit<LayerSurfacePlacement, "output" | "margin">> &
  Pick<LayerSurfacePlacement, "output"> & { readonly margin: Readonly<Record<LayerAnchor, number>>; };
export type PopupPlacement = Required<PopupSurfacePlacement> &
  Required<Pick<PopupSurfaceProps, "parent" | "offset" | "grab" | "width">> & {
    readonly height: number | "auto";
    readonly minHeight?: number;
    readonly maxHeight?: number;
    readonly anchorRect: NativeWindowRect;
  };
export interface WindowOptions {
  readonly title: string;
  readonly appId?: string;
  readonly width: number;
  readonly height: number;
  readonly minWidth?: number;
  readonly minHeight?: number;
  readonly maxWidth?: number;
  readonly maxHeight?: number;
  readonly resizable: boolean;
  readonly decorations: WindowDecorations;
  readonly transparent: boolean;
}
export type NativeInputRegion = "full" | "empty" | readonly NativeWindowRect[];

/** Installed by the host; resolving either bridge must remain lazy during preflight. */
export interface NativeHost {
  readonly __zgui: {
    createElement(tag: string): NativeNode;
    createPortal(layer: NativeOverlayLayer): NativeNode;
    createCanvasScene(): CanvasScene;
    registerDismissLayer(
      node: NativeNode, callback: (reason: NativeDismissReason) => void,
      options: Required<NativeDismissLayerOptions>,
    ): { update: (options: Required<NativeDismissLayerOptions>) => void; dispose: () => void; };
    createTextNode(text: string): NativeNode;
    replaceText(node: NativeNode, text: string): void;
    isTextNode(node: NativeNode): boolean;
    insertNode(parent: NativeNode, node: NativeNode, anchor?: NativeNode | null): void;
    removeNode(parent: NativeNode, node: NativeNode): void;
    getParentNode(node: NativeNode): NativeNode | null | undefined;
    getFirstChild(node: NativeNode): NativeNode | null | undefined;
    getNextSibling(node: NativeNode): NativeNode | null | undefined;
    setProperty(node: NativeNode, name: string, value: unknown, previous?: unknown): void;
    installStylesheet(name: string, css: string, target?: NativeNode): void;
    removeStylesheet(name: string): void;
    getReducedMotion(): boolean;
    focus(node: NativeNode, reveal?: boolean): void;
    trapFocus(node: NativeNode, wrap: boolean, autoFocus: boolean, restore: boolean): () => void;
    lockScroll(node: NativeNode): () => void;
    getWindowRect(node: NativeNode): NativeWindowRect | null;
    getTextContent(node: NativeNode): string;
    isAnimating(node: NativeNode): boolean;
    /** The optional attachment callback runs on registration and mount-tree changes, before layout. */
    observeGeometry<K extends NativeGeometryKind>(
      node: NativeNode,
      kind: K,
      callback: (value: NativeGeometry[K]) => void,
      onAttachment?: (attached: boolean) => void,
    ): () => void;
    precedes(first: NativeNode, second: NativeNode): boolean;
    clearRoot(root: NativeNode): void;
  };
  readonly __zguiShell: {
    getSessionGeneration(): number;
    installStylesheet(name: string, css: string): void;
    removeStylesheet(name: string): void;
    subscribeMonitors(callback: (monitors: readonly Monitor[]) => void): () => void;
    openLayer(name: string, placement: LayerPlacement, attach: (root: NativeNode, bridge: UiBridge) => () => void): number;
    updateLayer(id: number, placement: LayerPlacement): void;
    closeLayer(id: number): void;
    setInputRegion(id: number, region: NativeInputRegion): void;
    openPopup(name: string, placement: PopupPlacement, attach: (root: NativeNode, bridge: UiBridge) => () => void, dismiss: () => void): number;
    updatePopup(id: number, placement: PopupPlacement): void;
    closePopup(id: number): void;
    openWindow(name: string, options: WindowOptions, attach: (root: NativeNode, bridge: UiBridge) => () => void, onClose: () => void): number;
    updateWindow(id: number, options: WindowOptions): void;
    closeWindow(id: number): void;
    requestSessionLock(callback: (state: SessionLockState) => void): { readonly id: number; unlock(this: void): void; unsubscribe(this: void): void };
    openLock(name: string, output: string, attach: (root: NativeNode, bridge: UiBridge) => () => void): number;
    closeLock(id: number): void;
  };
}

declare const globalThis: NativeHost;

export type UiBridge = NativeHost["__zgui"];
const Region = createContext<UiBridge | null>(null);
const nodeBridges = new WeakMap<NativeNode, UiBridge>();
let callbackBridge: UiBridge | undefined;
let callbackOwner: Owner | null = null;

/** Resolves an explicit target without consulting the current owner or ambient bridge. */
export function bridgeForTarget(node: NativeNode): UiBridge {
  const bridge = nodeBridges.get(node);
  if (!bridge) throw new ReferenceError("Unknown native stylesheet target");
  return bridge;
}

export function bridgeFor(node?: NativeNode): UiBridge {
  const known = node && nodeBridges.get(node);
  if (known) return known;
  const owner = getOwner();
  const currentCallback = owner === callbackOwner ? callbackBridge : undefined;
  const bridge = currentCallback ?? (owner && useContext(Region)) ?? callbackBridge ?? globalThis.__zgui;
  if (!bridge) throw new ReferenceError("Native UI requires a mount region");
  return bridge;
}

export function rememberBridge(node: NativeNode, bridge: UiBridge): NativeNode {
  nodeBridges.set(node, bridge);
  return node;
}

export function provideRegion(bridge: UiBridge, mount: () => void): void {
  let owner: Owner | null = null;
  const children = Region({ value: bridge, get children() { owner = getOwner(); return null; } });
  // Evaluate the provider before mounting so native mount errors propagate directly.
  if (typeof children === "function") createMemo(children)();
  runWithOwner(owner, mount);
}

export function withCallbackBridge<T>(bridge: UiBridge, fn: () => T): T {
  const previousBridge = callbackBridge;
  const previousOwner = callbackOwner;
  callbackBridge = bridge;
  callbackOwner = getOwner();
  try { return fn(); }
  finally {
    callbackBridge = previousBridge;
    callbackOwner = previousOwner;
  }
}

export function shellBridge(): NativeHost["__zguiShell"] {
  const bridge = globalThis.__zguiShell;
  if (!bridge) throw new ReferenceError("Native shell operations require a session host");
  return bridge;
}
