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
  ActivationTokenOptions,
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
export type NativeBlurRegion =
  | "none"
  | "full"
  | readonly NativeWindowRect[]
  | { readonly nodes: readonly NativeNode[]; readonly minOpacity: number };

/** The native operations Solid Universal needs, independent of session and region services. */
export interface UniversalRendererBridge {
  readonly createElement: (tag: string) => NativeNode;
  readonly createTextNode: (text: string) => NativeNode;
  readonly replaceText: (node: NativeNode, text: string) => void;
  readonly isTextNode: (node: NativeNode) => boolean;
  readonly insertNode: (parent: NativeNode, node: NativeNode, anchor?: NativeNode | null) => void;
  readonly removeNode: (parent: NativeNode, node: NativeNode) => void;
  readonly getParentNode: (node: NativeNode) => NativeNode | null | undefined;
  readonly getFirstChild: (node: NativeNode) => NativeNode | null | undefined;
  readonly getNextSibling: (node: NativeNode) => NativeNode | null | undefined;
  readonly setProperty: (
    node: NativeNode,
    name: string,
    value: unknown,
    previous?: unknown,
  ) => void;
}

interface SessionPreferenceBridge {
  readonly getReducedMotion?: () => boolean;
  readonly subscribeReducedMotion?: (callback: (value: boolean) => void) => () => void;
  readonly getBackgroundBlurSupported?: () => boolean;
  readonly subscribeBackgroundBlurSupported?: (callback: (value: boolean) => void) => () => void;
}

interface RegionBridge extends SessionPreferenceBridge {
  createPortal(layer: NativeOverlayLayer): NativeNode;
  createCanvasScene(): CanvasScene;
  registerDismissLayer(
    node: NativeNode,
    callback: (reason: NativeDismissReason) => void,
    options: Required<NativeDismissLayerOptions>,
  ): { update: (options: Required<NativeDismissLayerOptions>) => void; dispose: () => void };
  readonly getReducedMotion: () => boolean;
  installStylesheet(name: string, css: string, target?: NativeNode): void;
  removeStylesheet(name: string): void;
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
  getCapsLock(): boolean;
  subscribeCapsLock(callback: (capsLock: boolean) => void): () => void;
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
}

export type UiBridge = UniversalRendererBridge & RegionBridge;

/** Installed by the host; resolving either bridge must remain lazy during preflight. */
export interface NativeHost {
  readonly __zgui: UiBridge;
  readonly __zguiShell: SessionPreferenceBridge & {
    getSessionGeneration(): number;
    requestActivationToken(options?: ActivationTokenOptions): Promise<string | null>;
    installStylesheet(name: string, css: string): void;
    removeStylesheet(name: string): void;
    subscribeMonitors(callback: (monitors: readonly Monitor[]) => void): () => void;
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
  readonly __zguiShellHost?: {
    listApprovedEntries(): readonly {
      readonly entryId: string;
      readonly extensionId: string;
      readonly version: string;
      readonly kind: string;
      readonly bundle: string;
      readonly canonicalSpecifier: string;
    }[];
    loadApprovedEntry(entryId: string): Promise<Record<string, unknown>>;
    registerSharedRuntimeModules(modules: Readonly<Record<string, Record<string, unknown>>>): void;
    registerSessionHandoff(capture: () => unknown): () => void;
    takeSessionHandoff(): unknown;
    onSessionCommitted(callback: (status: unknown) => void): () => void;
    onBeforeCommit(callback: () => void): () => void;
    getOverlayStatus(): {
      readonly generation: number;
      readonly overlayId: string | null;
      readonly phase: "base" | "staged" | "observing" | "committed" | "safe-base-shell";
      readonly safeBaseShell: boolean;
      readonly activeExtensionIds: readonly string[];
      readonly clientMetadata: unknown;
      readonly lastFailure: {
        readonly overlayId: string | null;
        readonly phase: "preflight" | "mount" | "commit" | "observation" | "rollback";
        readonly reason: string;
        readonly rollbackBlockedByPolicy: boolean;
        readonly clientMetadata: unknown;
      } | null;
    };
  };
}

declare const globalThis: NativeHost;

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

/** Returns the owning bridge if `value` is a genuine branded native node handle. */
export function knownBridgeForNode(value: unknown): UiBridge | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  return nodeBridges.get(value as NativeNode);
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

const MOTION = ["getReducedMotion", "subscribeReducedMotion"] as const;
const BACKGROUND_BLUR = ["getBackgroundBlurSupported", "subscribeBackgroundBlurSupported"] as const;
type Preference = typeof MOTION | typeof BACKGROUND_BLUR;

interface HostPreference {
  current: boolean;
  readonly value: () => boolean;
  readonly set: (value: boolean) => void;
  readonly sources: WeakMap<
    SessionPreferenceBridge,
    {
      mounts: number;
      readonly unsubscribe: (() => void) | undefined;
    }
  >;
}

interface SessionPreferences {
  readonly motion: HostPreference;
  readonly backgroundBlur: HostPreference;
  readonly override: () => boolean | null;
  readonly setOverride: (value: boolean | null) => void;
  currentOverride: boolean | null;
}

const preferencesBySession = new WeakMap<object, SessionPreferences>();

function newHostPreference(): HostPreference {
  const [value, set] = createSignal(false, { ownedWrite: true });
  return { current: false, value, set, sources: new WeakMap() };
}

function preferencesFor(key: object): SessionPreferences {
  let state = preferencesBySession.get(key);
  if (!state) {
    const [override, setOverride] = createSignal<boolean | null>(null);
    state = {
      motion: newHostPreference(),
      backgroundBlur: newHostPreference(),
      override,
      setOverride,
      currentOverride: null,
    };
    preferencesBySession.set(key, state);
  }
  return state;
}

function preferenceSource(
  [read, subscribe]: Preference,
  bridge?: UiBridge,
): SessionPreferenceBridge | undefined {
  const shell = globalThis.__zguiShell;
  if (shell?.[read] || shell?.[subscribe]) return shell;
  return bridge?.[read] || bridge?.[subscribe] ? bridge : undefined;
}

function attachPreference(state: HostPreference, preference: Preference, bridge?: UiBridge): void {
  const source = preferenceSource(preference, bridge);
  if (!source) return;
  const [read, subscribe] = preference;
  let lease = state.sources.get(source);
  if (!lease) {
    const get = source[read]?.() ?? bridge?.[read]?.();
    if (get !== undefined && get !== state.current) {
      state.current = get;
      state.set(get);
    }
    let mounting = true;
    const unsubscribe = source[subscribe]?.(next => {
      if (next === state.current) return;
      state.current = next;
      state.set(next);
      if (!mounting) {
        try {
          solidFlush();
        } catch (error) {
          resetErrorHalt();
          throw error;
        }
      }
    });
    mounting = false;
    lease = { mounts: 0, unsubscribe };
    state.sources.set(source, lease);
  }
  const mounted = lease;
  mounted.mounts++;
  onCleanup(() => {
    if (--mounted.mounts !== 0) return;
    state.sources.delete(source);
    mounted.unsubscribe?.();
  });
}

/** Mount owners share shell preferences; older hosts supply them through their region bridges. */
export function attachSessionPreferences(bridge?: UiBridge): void {
  const key = sessionScopeKey(bridge);
  if (!key) return;
  const state = preferencesFor(key);
  attachPreference(state.motion, MOTION, bridge);
  attachPreference(state.backgroundBlur, BACKGROUND_BLUR, bridge);
}

function readHostPreference(
  state: HostPreference,
  preference: Preference,
  bridge?: UiBridge,
): boolean {
  state.value();
  const source = preferenceSource(preference, bridge);
  if (source && !state.sources.get(source)?.unsubscribe) {
    // Getter-only or pre-mount access is a snapshot, not an ownerless subscription.
    return source[preference[0]]?.() ?? state.current;
  }
  return state.current;
}

/** Reactive reduced-motion preference (runtime override when set, otherwise host/desktop preference). */
export function getReducedMotion(): boolean {
  const bridge = tryBridgeFor();
  const key = sessionScopeKey(bridge);
  if (!key) throw new ReferenceError("Native UI requires a mount region or shell session");
  const state = preferencesFor(key);
  state.override();
  return state.currentOverride ?? readHostPreference(state.motion, MOTION, bridge);
}

/** Override reduced motion for the whole session, or pass `null` to follow the host. */
export function setReducedMotion(value: boolean | null): void {
  if (value !== null && typeof value !== "boolean") {
    throw new TypeError("setReducedMotion requires a boolean or null");
  }
  const key = sessionScopeKey();
  if (!key) throw new ReferenceError("Native UI requires a mount region or shell session");
  const state = preferencesFor(key);
  state.currentOverride = value;
  state.setOverride(value);
}

/**
 * Reactive background blur support for eligible surfaces: `niri_blur_shape_v1` or
 * `ext_background_effect_v1` with `Capability::Blur`. Lock surfaces never request blur.
 */
export function getBackgroundBlurSupported(): boolean {
  const bridge = tryBridgeFor();
  const key = sessionScopeKey(bridge);
  if (!key) throw new ReferenceError("Native UI requires a mount region or shell session");
  return readHostPreference(preferencesFor(key).backgroundBlur, BACKGROUND_BLUR, bridge);
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
