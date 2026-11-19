import { createEffect, createMemo, createRoot, createSignal, getOwner, onCleanup, runWithOwner } from "#solid";
import { bridgeFor, shellBridge } from "./host.js";
import type { LayerPlacement, NativeHost, NativeInputRegion, PopupPlacement, UiBridge } from "./host.js";
import { renderRegion } from "./renderer/index.js";
import type {
  Accessor, LayerAnchor, LayerSurfaceProps, Monitor, NativeNode, NativeWindowRect,
  PopupSurfaceProps, SessionLockRequest, SessionLockState, SessionLockSurfaceProps,
} from "./types.js";

declare const console: { warn(message: string): void };

const popupParentRegions = new WeakMap<UiBridge, { shell: NativeHost["__zguiShell"]; name: string }>();

/** Read the current session's generation, including during module preflight. */
export function getSessionGeneration(): number {
  return shellBridge().getSessionGeneration();
}

/** Subscribe to output snapshots for the lifetime of the current Solid owner. */
export function useMonitors(): Accessor<readonly Monitor[]> {
  if (!getOwner()) throw new Error("useMonitors requires a Solid owner");
  let initial: readonly Monitor[] = [];
  let update = (snapshot: readonly Monitor[]) => { initial = snapshot; };
  onCleanup(shellBridge().subscribeMonitors(snapshot => update(snapshot)));
  const [monitors, setMonitors] = createSignal(initial);
  update = setMonitors;
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
        height: props.height,
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
        close();
        dismiss(true);
        runWithOwner(owner, () => props.onDismiss?.());
      });
      id = request;
    });
    return dispose;
  })));
  return null;
}

/** Request compositor locking. Unlock only from an explicit authorized application action. */
export function requestSessionLock(): SessionLockRequest {
  const [state, setState] = createSignal<SessionLockState>("unlocked");
  const raw = shellBridge().requestSessionLock(setState);
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
