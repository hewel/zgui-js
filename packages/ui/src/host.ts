import { createContext, createMemo, createSignal, flush as solidFlush, getOwner, onCleanup, resetErrorHalt, runWithOwner, untrack, useContext } from "#solid";
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
    subscribeReducedMotion?(callback: (value: boolean) => void): () => void;
    requestAnimationFrame?(callback: (time: number) => void): number;
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
  const bridge = tryBridgeFor(node);
  if (!bridge) throw new ReferenceError("Native UI requires a mount region");
  return bridge;
}

/** Returns the active region bridge when available without throwing in unscoped contexts. */
export function tryBridgeFor(node?: NativeNode): UiBridge | undefined {
  const known = node && nodeBridges.get(node);
  if (known) return known;
  const owner = getOwner();
  const currentCallback = owner === callbackOwner ? callbackBridge : undefined;
  return currentCallback ?? (owner && useContext(Region)) ?? callbackBridge ?? globalThis.__zgui;
}

/** Stable per-session scope key shared across all surfaces in a shell or one window. */
export function sessionScopeKey(bridge?: UiBridge): object | undefined {
  return globalThis.__zguiShell ?? bridge ?? tryBridgeFor();
}

interface MotionState {
  hostValue: () => boolean;
  setHostValue: (next: boolean) => void;
  override: () => boolean | null;
  setOverride: (next: boolean | null) => void;
  subscribedBridges: WeakSet<UiBridge>;
}

const motionStates = new WeakMap<object, MotionState>();

function flushMotion(): void {
  try {
    solidFlush();
  } catch (error) {
    resetErrorHalt();
    throw error;
  }
}

function ensureMotionState(bridge?: UiBridge): MotionState {
  const key = sessionScopeKey(bridge);
  if (!key) throw new ReferenceError("Native UI requires a mount region or shell session");
  let state = motionStates.get(key);
  if (!state) {
    const initial = bridge ? bridge.getReducedMotion() : false;
    const [hostValue, setHostValue] = createSignal(initial);
    const [override, setOverride] = createSignal<boolean | null>(null);
    state = { hostValue, setHostValue, override, setOverride, subscribedBridges: new WeakSet() };
    motionStates.set(key, state);
  }
  return state;
}

function subscribeMotionBridge(state: MotionState, bridge: UiBridge, allowReadOnlySkip: boolean): (() => void) | undefined {
  if (state.subscribedBridges.has(bridge)) return undefined;
  const current = bridge.getReducedMotion();
  if (current !== untrack(state.hostValue)) state.setHostValue(current);
  let unsubscribe: (() => void) | undefined;
  if (bridge.subscribeReducedMotion) {
    try {
      unsubscribe = bridge.subscribeReducedMotion(next => {
        state.setHostValue(next);
        flushMotion();
      });
    } catch (error) {
      if (allowReadOnlySkip) return undefined;
      throw error;
    }
  }
  state.subscribedBridges.add(bridge);
  return unsubscribe
    ? () => {
        state.subscribedBridges.delete(bridge);
        unsubscribe();
      }
    : undefined;
}

/** Eagerly subscribe a mounted region bridge so unscoped shell-root readers stay synchronized. */
export function attachReducedMotionBridge(bridge: UiBridge): void {
  if (typeof bridge.getReducedMotion !== "function") return;
  const state = ensureMotionState(bridge);
  const dispose = subscribeMotionBridge(state, bridge, false);
  if (dispose && getOwner()) onCleanup(dispose);
}

/** Reactive reduced-motion preference (runtime override when set, otherwise host/desktop preference). */
export function getReducedMotion(): boolean {
  const bridge = tryBridgeFor();
  const state = ensureMotionState(bridge);
  if (bridge) subscribeMotionBridge(state, bridge, true);
  return state.override() ?? state.hostValue();
}

/** Override reduced motion dynamically for this session, or pass `null` to follow the host/desktop setting. */
export function setReducedMotion(value: boolean | null): void {
  if (value !== null && typeof value !== "boolean") {
    throw new TypeError("setReducedMotion requires a boolean or null");
  }
  const bridge = tryBridgeFor();
  const state = ensureMotionState(bridge);
  if (bridge) subscribeMotionBridge(state, bridge, true);
  state.setOverride(value);
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
