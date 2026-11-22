import {
  createContext,
  createMemo,
  createSignal,
  flush as solidFlush,
  getOwner,
  onCleanup,
  resetErrorHalt,
  runWithOwner,
  useContext,
} from "#solid";
import type { Owner } from "#solid";
import type {
  CanvasScene,
  CloseRequestEvent,
  DocumentSelection,
  LayerAnchor,
  LayerSurfacePlacement,
  Monitor,
  NativeDismissLayerOptions,
  NativeDismissReason,
  NativeGeometry,
  NativeGeometryKind,
  NativeNode,
  NativeOverlayLayer,
  NativeWindowRect,
  PopupSurfacePlacement,
  PopupSurfaceProps,
  SessionLockState,
  WindowDecorations,
  WindowState,
} from "./types.js";

export type LayerPlacement = Required<Omit<LayerSurfacePlacement, "output" | "margin">> &
  Pick<LayerSurfacePlacement, "output"> & {
    readonly margin: Readonly<Record<LayerAnchor, number>>;
  };
export type PopupPlacement = Required<PopupSurfacePlacement> &
  Required<Pick<PopupSurfaceProps, "parent" | "offset" | "grab" | "width">> & {
    readonly height: number | "auto";
    readonly minHeight?: number;
    readonly maxHeight?: number;
    readonly anchorRect: NativeWindowRect;
    readonly geometryInsets: Readonly<Record<LayerAnchor, number>>;
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
export type NativeBlurRegion = "none" | "full" | readonly NativeWindowRect[];

/** Installed by the host; resolving either bridge must remain lazy during preflight. */
export interface NativeHost {
  readonly __zgui: {
    createElement(tag: string): NativeNode;
    createPortal(layer: NativeOverlayLayer): NativeNode;
    createCanvasScene(): CanvasScene;
    registerDismissLayer(
      node: NativeNode,
      callback: (reason: NativeDismissReason) => void,
      options: Required<NativeDismissLayerOptions>,
    ): { update: (options: Required<NativeDismissLayerOptions>) => void; dispose: () => void };
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
    getBackgroundBlurSupported?(): boolean;
    subscribeBackgroundBlurSupported?(callback: (value: boolean) => void): () => void;
    requestAnimationFrame?(callback: (time: number) => void): number;
    focus(node: NativeNode, reveal?: boolean): void;
    trapFocus(node: NativeNode, wrap: boolean, autoFocus: boolean, restore: boolean): () => void;
    lockScroll(node: NativeNode): () => void;
    scrollTo(
      node: NativeNode,
      left: number | null,
      top: number | null,
      behavior?: "instant" | "smooth",
    ): void;
    scrollBy(
      node: NativeNode,
      left: number | null,
      top: number | null,
      behavior?: "instant" | "smooth",
    ): void;
    scrollIntoView(
      node: NativeNode,
      block?: "nearest" | "start" | "center" | "end",
      behavior?: "instant" | "smooth",
    ): void;
    getSelection(node: NativeNode): { readonly start: number; readonly end: number } | null;
    setSelection(node: NativeNode, start: number, end?: number): void;
    selectAll(node: NativeNode): void;
    getDocumentSelection(): DocumentSelection | null;
    setDocumentSelection(
      container: NativeNode,
      anchorNode: NativeNode,
      anchorOffset: number,
      focusNode?: NativeNode,
      focusOffset?: number,
    ): void;
    clearDocumentSelection(): void;
    selectDocumentAll(container: NativeNode): void;
    setWindowTitle(title: string): void;
    setWindowFullscreen(fullscreen: boolean): void;
    setWindowMaximized(maximized: boolean): void;
    setWindowMinimized(minimized: boolean): void;
    setWindowDecorations(decorations: WindowDecorations): void;
    beginWindowMoveDrag(): void;
    closeWindow(force?: boolean): void;
    getWindowState(): WindowState;
    subscribeWindowState(callback: (state: WindowState) => void): () => void;
    onCloseRequest(callback: (event: CloseRequestEvent) => void): () => void;
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
    getBackgroundBlurSupported?(): boolean;
    subscribeBackgroundBlurSupported?(callback: (value: boolean) => void): () => void;
    openLayer(
      name: string,
      placement: LayerPlacement,
      attach: (root: NativeNode, bridge: UiBridge) => () => void,
    ): number;
    updateLayer(id: number, placement: LayerPlacement): void;
    closeLayer(id: number): void;
    setInputRegion(id: number, region: NativeInputRegion): void;
    setBlurRegion(id: number, region: NativeBlurRegion): void;
    openPopup(
      name: string,
      placement: PopupPlacement,
      attach: (root: NativeNode, bridge: UiBridge) => () => void,
      dismiss: () => void,
    ): number;
    updatePopup(id: number, placement: PopupPlacement): void;
    closePopup(id: number): void;
    openWindow(
      name: string,
      options: WindowOptions,
      attach: (root: NativeNode, bridge: UiBridge) => () => void,
      onClose: () => void,
    ): number;
    updateWindow(id: number, options: WindowOptions): void;
    closeWindow(id: number): void;
    requestSessionLock(callback: (state: SessionLockState) => void): {
      readonly id: number;
      unlock(this: void): void;
      unsubscribe(this: void): void;
    };
    openLock(
      name: string,
      output: string,
      attach: (root: NativeNode, bridge: UiBridge) => () => void,
    ): number;
    closeLock(id: number): void;
  };
}

declare const globalThis: NativeHost;

export type UiBridge = NativeHost["__zgui"];
const Region = createContext<UiBridge | null>(null);
const nodeBridges = new WeakMap<NativeNode, UiBridge>();
let regionProviders = 0;
const regionPresenceListeners = new Set<(hasRegions: boolean) => void>();
let callbackBridge: UiBridge | undefined;
let callbackOwner: Owner | null = null;

function notifyRegionPresence(hasRegions: boolean): void {
  for (const listener of [...regionPresenceListeners]) {
    listener(hasRegions);
  }
}

/** Subscribe to transitions between zero and one-or-more mounted SDK regions. */
export function onRegionPresenceChange(listener: (hasRegions: boolean) => void): () => void {
  regionPresenceListeners.add(listener);
  return () => {
    regionPresenceListeners.delete(listener);
  };
}

/** Whether the current runtime is a rootless shell with zero mounted surface regions. */
export function isRootlessWithoutRegions(): boolean {
  return (
    regionProviders === 0 && globalThis.__zgui === undefined && globalThis.__zguiShell !== undefined
  );
}

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
  if (regionProviders === 0) return callbackBridge ?? globalThis.__zgui;
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
  currentHostValue: boolean;
  override: () => boolean | null;
  setOverride: (next: boolean | null) => void;
  currentOverride: boolean | null;
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
    const [hostValue, setHostValue] = createSignal(initial, { ownedWrite: true });
    const [override, setOverride] = createSignal<boolean | null>(null);
    state = {
      hostValue,
      setHostValue,
      currentHostValue: initial,
      override,
      setOverride,
      currentOverride: null,
      subscribedBridges: new WeakSet(),
    };
    motionStates.set(key, state);
  }
  return state;
}

function subscribeMotionBridge(
  state: MotionState,
  bridge: UiBridge,
  allowReadOnlySkip: boolean,
): (() => void) | undefined {
  if (state.subscribedBridges.has(bridge)) return undefined;
  const current = bridge.getReducedMotion();
  if (current !== state.currentHostValue) {
    state.currentHostValue = current;
    state.setHostValue(current);
  }
  let unsubscribe: (() => void) | undefined;
  if (bridge.subscribeReducedMotion) {
    try {
      unsubscribe = bridge.subscribeReducedMotion(next => {
        state.currentHostValue = next;
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
  state.override();
  state.hostValue();
  return state.currentOverride ?? state.currentHostValue;
}

/** Override reduced motion dynamically for this session, or pass `null` to follow the host/desktop setting. */
export function setReducedMotion(value: boolean | null): void {
  if (value !== null && typeof value !== "boolean") {
    throw new TypeError("setReducedMotion requires a boolean or null");
  }
  const bridge = tryBridgeFor();
  const state = ensureMotionState(bridge);
  if (bridge) subscribeMotionBridge(state, bridge, true);
  state.currentOverride = value;
  state.setOverride(value);
}

interface BackgroundBlurSource {
  getBackgroundBlurSupported?(): boolean;
  subscribeBackgroundBlurSupported?(callback: (value: boolean) => void): () => void;
}

interface BackgroundBlurState {
  hostValue: () => boolean;
  setHostValue: (next: boolean) => void;
  currentHostValue: boolean;
  subscribedSources: WeakSet<object>;
}

const backgroundBlurStates = new WeakMap<object, BackgroundBlurState>();

function ensureBackgroundBlurState(bridge?: UiBridge): BackgroundBlurState {
  const key = sessionScopeKey(bridge);
  if (!key) throw new ReferenceError("Native UI requires a mount region or shell session");
  let state = backgroundBlurStates.get(key);
  if (!state) {
    const initial =
      bridge?.getBackgroundBlurSupported?.() ??
      globalThis.__zguiShell?.getBackgroundBlurSupported?.() ??
      false;
    const [hostValue, setHostValue] = createSignal(initial, { ownedWrite: true });
    state = {
      hostValue,
      setHostValue,
      currentHostValue: initial,
      subscribedSources: new WeakSet(),
    };
    backgroundBlurStates.set(key, state);
  }
  return state;
}

function subscribeBackgroundBlurSource(
  state: BackgroundBlurState,
  source: BackgroundBlurSource,
  allowReadOnlySkip: boolean,
): (() => void) | undefined {
  if (typeof source.getBackgroundBlurSupported !== "function") return undefined;
  if (state.subscribedSources.has(source)) return undefined;
  const current = source.getBackgroundBlurSupported();
  if (current !== state.currentHostValue) {
    state.currentHostValue = current;
    state.setHostValue(current);
  }
  let unsubscribe: (() => void) | undefined;
  if (source.subscribeBackgroundBlurSupported) {
    try {
      unsubscribe = source.subscribeBackgroundBlurSupported(next => {
        if (next === state.currentHostValue) return;
        state.currentHostValue = next;
        state.setHostValue(next);
        flushMotion();
      });
    } catch (error) {
      if (allowReadOnlySkip) return undefined;
      throw error;
    }
  }
  state.subscribedSources.add(source);
  return unsubscribe
    ? () => {
        state.subscribedSources.delete(source);
        unsubscribe();
      }
    : undefined;
}

/** Eagerly subscribe a mounted region bridge so background-blur readers stay synchronized. */
export function attachBackgroundBlurBridge(bridge: UiBridge): void {
  if (typeof bridge.getBackgroundBlurSupported !== "function") return;
  const state = ensureBackgroundBlurState(bridge);
  const dispose = subscribeBackgroundBlurSource(state, bridge, false);
  if (dispose && getOwner()) onCleanup(dispose);
}

/** Eagerly subscribe the rootless shell bridge so zero-surface shells observe live blur transitions. */
export function attachBackgroundBlurShell(): void {
  const shell = globalThis.__zguiShell;
  if (!shell || typeof shell.getBackgroundBlurSupported !== "function") return;
  const state = ensureBackgroundBlurState();
  const dispose = subscribeBackgroundBlurSource(state, shell, false);
  if (dispose && getOwner()) onCleanup(dispose);
}

/** Read-only reactive compositor background-blur capability (`ext_background_effect_v1` `Capability::Blur`). */
export function getBackgroundBlurSupported(): boolean {
  const bridge = tryBridgeFor();
  const state = ensureBackgroundBlurState(bridge);
  if (bridge?.getBackgroundBlurSupported) {
    const dispose = subscribeBackgroundBlurSource(state, bridge, true);
    if (dispose && getOwner()) onCleanup(dispose);
  } else if (globalThis.__zguiShell?.getBackgroundBlurSupported) {
    const dispose = subscribeBackgroundBlurSource(state, globalThis.__zguiShell, true);
    if (dispose && getOwner()) onCleanup(dispose);
  }
  state.hostValue();
  return state.currentHostValue;
}

export function rememberBridge(node: NativeNode, bridge: UiBridge): NativeNode {
  nodeBridges.set(node, bridge);
  return node;
}

/**
 * Returns true iff both nodes belong to the same native document/bridge.
 * Distinct `PopupSurface` / `LayerSurface` / `WindowSurface` regions own distinct bridges,
 * whereas `Portal` shares its host document's bridge.
 */
export function sameDocument(
  first: NativeNode | null | undefined,
  second: NativeNode | null | undefined,
): boolean {
  if (!first || !second) return false;
  const firstBridge = nodeBridges.get(first);
  if (!firstBridge) return false;
  return firstBridge === nodeBridges.get(second);
}

export function provideRegion(bridge: UiBridge, mount: () => void): void {
  const wasEmpty = regionProviders === 0;
  regionProviders += 1;
  onCleanup(() => {
    regionProviders -= 1;
    if (regionProviders === 0) notifyRegionPresence(false);
  });
  let owner: Owner | null = null;
  const children = Region({
    value: bridge,
    get children() {
      owner = getOwner();
      return null;
    },
  });
  // Evaluate the provider before mounting so native mount errors propagate directly.
  if (typeof children === "function") createMemo(children)();
  runWithOwner(owner, mount);
  if (wasEmpty && regionProviders > 0) notifyRegionPresence(true);
}

export function withCallbackBridge<T>(bridge: UiBridge, fn: () => T): T {
  const previousBridge = callbackBridge;
  const previousOwner = callbackOwner;
  callbackBridge = bridge;
  callbackOwner = getOwner();
  try {
    return fn();
  } finally {
    callbackBridge = previousBridge;
    callbackOwner = previousOwner;
  }
}

export function shellBridge(): NativeHost["__zguiShell"] {
  const bridge = globalThis.__zguiShell;
  if (!bridge) throw new ReferenceError("Native shell operations require a session host");
  return bridge;
}
