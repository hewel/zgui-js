import { createEffect, createMemo, createRoot, createSignal, getOwner, onCleanup, runWithOwner } from "#solid";
import { bridgeFor, shellBridge } from "./host.js";
import type { LayerPlacement, NativeHost, NativeInputRegion, PopupPlacement, UiBridge, WindowOptions } from "./host.js";
import { flush, renderRegion } from "./renderer/index.js";
import type {
  Accessor, LayerAnchor, LayerSurfaceProps, Monitor, NativeNode, NativeWindowRect,
  PopupSurfaceProps, SessionLockRequest, SessionLockState, SessionLockSurfaceProps, WindowSurfaceProps,
} from "./types.js";

declare const console: { warn(message: string): void };

type ShellBridge = NativeHost["__zguiShell"];
const popupParentRegions = new WeakMap<UiBridge, { shell: ShellBridge; name: string }>();
const stylesheetRegistrations = new WeakMap<ShellBridge, Map<string, symbol>>();
const monitorSubscriptions = new WeakMap<ShellBridge, MonitorSubscription>();

interface MonitorSubscription {
  subscribers: Set<(snapshot: readonly Monitor[]) => void>;
  current: Accessor<readonly Monitor[]>;
  unsubscribe: () => void;
}

function subscribeMonitors(shell: ShellBridge): MonitorSubscription {
  let snapshot: readonly Monitor[] = [];
  const subscribers = new Set<(snapshot: readonly Monitor[]) => void>();
  const unsubscribe = shell.subscribeMonitors(next => {
    snapshot = next;
    if (subscribers.size === 0) return;
    flush(() => {
      for (const update of subscribers) update(next);
    });
  });
  return { subscribers, current: () => snapshot, unsubscribe };
}

/** Read the current session's generation, including during module preflight. */
export function getSessionGeneration(): number {
  return shellBridge().getSessionGeneration();
}

/** Mount a session-wide stylesheet in current and future regions. */
export function mountShellStylesheet(name: string, css: string): () => void {
  const shell = shellBridge();
  shell.installStylesheet(name, css);
  let registrations = stylesheetRegistrations.get(shell);
  if (!registrations) {
    registrations = new Map();
    stylesheetRegistrations.set(shell, registrations);
  }
  const token = Symbol();
  registrations.set(name, token);
  const dispose = () => {
    if (registrations.get(name) !== token) return;
    shell.removeStylesheet(name);
    registrations.delete(name);
  };
  if (getOwner()) onCleanup(dispose);
  return dispose;
}

/** Subscribe to output snapshots for the lifetime of the current Solid owner. */
export function useMonitors(): Accessor<readonly Monitor[]> {
  if (!getOwner()) throw new Error("useMonitors requires a Solid owner");
  const shell = shellBridge();
  let subscription = monitorSubscriptions.get(shell);
  if (!subscription) {
    subscription = subscribeMonitors(shell);
    monitorSubscriptions.set(shell, subscription);
  }
  const [monitors, setMonitors] = createSignal(subscription.current());
  subscription.subscribers.add(setMonitors);
  onCleanup(() => {
    subscription.subscribers.delete(setMonitors);
    if (subscription.subscribers.size === 0) {
      monitorSubscriptions.delete(shell);
      subscription.unsubscribe();
    }
  });
  return monitors;
}

const noAnchors: readonly LayerAnchor[] = [];

function placementOf(props: LayerSurfaceProps): LayerPlacement {
  const anchor = props.anchor === undefined ? noAnchors : [...props.anchor];
  const margin = props.margin;
  return {
    output: props.output,
    layer: props.layer ?? "top",
    anchor,
    width: props.width ?? (anchor.includes("left") && anchor.includes("right") ? 0 : 1),
    height: props.height ?? (anchor.includes("top") && anchor.includes("bottom") ? 0 : 1),
    margin: { top: margin?.top ?? 0, right: margin?.right ?? 0, bottom: margin?.bottom ?? 0, left: margin?.left ?? 0 },
    exclusiveZone: props.exclusiveZone ?? 0,
    keyboard: props.keyboard ?? "none",
  };
}

function bindInputRegion(props: LayerSurfaceProps, bridge: UiBridge, publish: (region: NativeInputRegion) => void): void {
  createEffect(() => {
    const region = props.inputRegion ?? "full";
    if (typeof region === "string") return region;
    return region.map(item => "x" in item ? { x: item.x, y: item.y, width: item.width, height: item.height } : item);
  }, region => {
    if (typeof region === "string") { publish(region); return; }
    const nodes: NativeNode[] = [];
    const fixed: NativeWindowRect[] = [];
    for (const item of region) {
      if ("x" in item) fixed.push(item);
      else nodes.push(item);
    }
    if (!nodes.length) { publish(fixed); return; }

    const boxes = new Map<NativeNode, NativeWindowRect>();
    const setBox = (node: NativeNode, rect: NativeWindowRect | null) => {
      if (!rect || rect.width <= 0 || rect.height <= 0) return boxes.delete(node);
      boxes.set(node, rect);
      return true;
    };
    const publishBoxes = () => publish([...fixed, ...boxes.values()]);
    let initializing = true;
    const observers = nodes.map(node => bridge.observeGeometry(node, "borderBox", rect => {
      if (setBox(node, rect)) publishBoxes();
    }, attached => {
      if (setBox(node, attached ? bridge.getWindowRect(node) : null) && !initializing) publishBoxes();
    }));
    initializing = false;
    publishBoxes();
    return () => { for (const dispose of observers) dispose(); };
  });
}

interface LayerRequest {
  id: number;
  name: string;
  output: string | undefined;
  closed: boolean;
}

/** Own a named compositor surface without inserting a node in the parent region. */
export function LayerSurface(props: LayerSurfaceProps): null {
  const owner = getOwner();
  if (!owner) throw new Error("LayerSurface requires a Solid owner");
  const shell = shellBridge();
  let current: LayerRequest | undefined;
  const close = () => {
    if (!current) return;
    const request = current;
    current = undefined;
    request.closed = true;
    shell.closeLayer(request.id);
  };
  onCleanup(close);

  createEffect(() => ({ name: props.name, placement: placementOf(props) }), ({ name, placement }) => {
    if (current && current.name === name && current.output === placement.output) {
      shell.updateLayer(current.id, placement);
      return;
    }
    close();
    const request: LayerRequest = {
      id: shell.openLayer(name, placement, (root, bridge) => {
        if (request.closed) return () => {};
        popupParentRegions.set(bridge, { shell, name });
        let attached = true;
        const dispose = runWithOwner(owner, () => renderRegion(() => {
          bindInputRegion(props, bridge, region => {
            if (attached && !request.closed) shell.setInputRegion(request.id, region);
          });
          return () => props.children;
        }, root, bridge));
        return () => {
          attached = false;
          dispose();
        };
      }),
      name,
      output: placement.output,
      closed: false,
    };
    current = request;
  });
  return null;
}

/** Own a compositor popup attached to a named layer or popup in this session. */
export function PopupSurface(props: PopupSurfaceProps): null {
  const owner = getOwner();
  if (!owner) throw new Error("PopupSurface requires a Solid owner");
  const shell = shellBridge();
  const name = createMemo(() => props.name);
  const parent = createMemo(() => props.parent);

  createEffect(() => ({ name: name(), parent: parent() }), ({ name, parent }) => runWithOwner(owner, () => createRoot(dispose => {
    const [dismissed, dismiss] = createSignal(false);
    const [nodeAnchor, setNodeAnchor] = createSignal<{ node: NativeNode; rect: NativeWindowRect }>();
    let id: number | undefined;
    const close = () => {
      if (id !== undefined) {
        const request = id;
        id = undefined;
        shell.closePopup(request);
      }
    };
    onCleanup(close);

    createEffect(() => dismissed() ? undefined : props.anchorNode, node => {
      setNodeAnchor(undefined);
      if (!node) return;
      const bridge = bridgeFor(node);
      const region = popupParentRegions.get(bridge);
      if (region?.shell !== shell || region.name !== parent) {
        console.warn(`PopupSurface "${name}": anchorNode must belong to parent surface "${parent}"`);
        return;
      }
      const publish = (rect: NativeWindowRect | null) => {
        setNodeAnchor(rect && rect.width > 0 && rect.height > 0 ? { node, rect } : undefined);
      };
      return bridge.observeGeometry(node, "borderBox", publish, attached => {
        publish(attached ? bridge.getWindowRect(node) : null);
      });
    });

    createEffect((): PopupPlacement | undefined => {
      if (dismissed()) return;
      const anchor = nodeAnchor();
      const rect = props.anchorRect ?? (anchor && anchor.node === props.anchorNode ? anchor.rect : undefined);
      if (!rect) return;
      const placement = props.placement;
      const offset = props.offset;
      return {
        parent,
        anchorRect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
        anchor: placement?.anchor ?? "bottom-left",
        gravity: placement?.gravity ?? "bottom-right",
        offset: { x: offset?.x ?? 0, y: offset?.y ?? 0 },
        slide: placement?.slide ?? true,
        flip: placement?.flip ?? true,
        resize: placement?.resize ?? false,
        reactive: placement?.reactive ?? false,
        grab: props.grab ?? false,
        width: props.width,
        height: props.height ?? "auto",
        ...(props.minHeight !== undefined ? { minHeight: props.minHeight } : {}),
        ...(props.maxHeight !== undefined ? { maxHeight: props.maxHeight } : {}),
      };
    }, placement => {
      if (!placement) { close(); return; }
      if (id !== undefined) {
        shell.updatePopup(id, placement);
        return;
      }
      const request = shell.openPopup(name, placement, (root, bridge) => {
        if (id !== request) return () => {};
        popupParentRegions.set(bridge, { shell, name });
        return runWithOwner(owner, () => renderRegion(() => () => props.children, root, bridge));
      }, () => {
        if (id !== request) return;
        flush(() => {
          close();
          dismiss(true);
          runWithOwner(owner, () => props.onDismiss?.());
        });
      });
      id = request;
    });
    return dispose;
  })));
  return null;
}

/** Own a compositor-managed toplevel window (xdg_toplevel) without inserting a node in the parent region. */
export function WindowSurface(props: WindowSurfaceProps): null {
  const owner = getOwner();
  if (!owner) throw new Error("WindowSurface requires a Solid owner");
  const shell = shellBridge();
  const name = createMemo(() => props.name);
  const appId = createMemo(() => props.appId);
  const transparent = createMemo(() => props.transparent ?? false);

  createEffect(
    () => ({ name: name(), appId: appId(), transparent: transparent() }),
    ({ name, appId, transparent }) => runWithOwner(owner, () => createRoot(dispose => {
      const [closed, setClosed] = createSignal(false);
      let id: number | undefined;
      const close = () => {
        if (id !== undefined) {
          const request = id;
          id = undefined;
          shell.closeWindow(request);
        }
      };
      onCleanup(close);

      createEffect((): WindowOptions | undefined => {
        if (closed()) return;
        return {
          title: props.title,
          ...(appId !== undefined ? { appId } : {}),
          width: props.width,
          height: props.height,
          ...(props.minWidth !== undefined ? { minWidth: props.minWidth } : {}),
          ...(props.minHeight !== undefined ? { minHeight: props.minHeight } : {}),
          ...(props.maxWidth !== undefined ? { maxWidth: props.maxWidth } : {}),
          ...(props.maxHeight !== undefined ? { maxHeight: props.maxHeight } : {}),
          resizable: props.resizable ?? true,
          decorations: props.decorations ?? "full",
          transparent,
        };
      }, options => {
        if (!options) { close(); return; }
        if (id !== undefined) {
          shell.updateWindow(id, options);
          return;
        }
        const request = shell.openWindow(name, options, (root, bridge) => {
          if (id !== request) return () => {};
          popupParentRegions.set(bridge, { shell, name });
          return runWithOwner(owner, () => renderRegion(() => () => props.children, root, bridge));
        }, () => {
          if (id !== request) return;
          flush(() => {
            close();
            setClosed(true);
            runWithOwner(owner, () => props.onClose?.());
          });
        });
        id = request;
      });
      return dispose;
    })),
  );
  return null;
}

/** Request compositor locking. Unlock only from an explicit authorized application action. */
export function requestSessionLock(): SessionLockRequest {
  const [state, setState] = createSignal<SessionLockState>("unlocked");
  // Registration delivers the initial state synchronously, during the caller's setup.
  let update: (next: SessionLockState) => void = setState;
  const raw = shellBridge().requestSessionLock(next => update(next));
  update = next => { flush(() => setState(next)); };
  const { unlock, unsubscribe: dispose } = raw;
  if (getOwner()) onCleanup(dispose);
  return Object.defineProperty({ state, unlock, dispose }, "id", {
    value: raw.id,
    enumerable: true,
    writable: false,
    configurable: false,
  }) as SessionLockRequest;
}

/** Mount content on an output while locked, without owning the underlying lock role. */
export function SessionLockSurface(props: SessionLockSurfaceProps): null {
  const owner = getOwner();
  if (!owner) throw new Error("SessionLockSurface requires a Solid owner");
  const shell = shellBridge();
  let current: { name: string; output: string; close: () => void } | undefined;
  const close = () => {
    const previous = current;
    current = undefined;
    previous?.close();
  };
  onCleanup(close);
  createEffect(() => [props.name, props.output] as const, ([name, output]) => {
    if (current?.name === name && current.output === output) return;
    close();
    let closed = false;
    const id = shell.openLock(name, output, (root, bridge) => {
      if (closed) return () => {};
      return runWithOwner(owner, () => renderRegion(() => props.children, root, bridge));
    });
    current = { name, output, close() {
      closed = true;
      shell.closeLock(id);
    } };
  });
  return null;
}
