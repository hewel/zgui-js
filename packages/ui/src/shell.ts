import { createEffect, createSignal, getOwner, onCleanup, runWithOwner } from "#solid";
import { shellBridge } from "./host.js";
import type { LayerPlacement, NativeInputRegion, UiBridge } from "./host.js";
import { renderRegion } from "./renderer/index.js";
import type {
  Accessor, LayerAnchor, LayerSurfaceProps, Monitor, NativeNode, NativeWindowRect,
  SessionLockRequest, SessionLockState, SessionLockSurfaceProps,
} from "./types.js";

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

/** Request compositor locking. Unlock only from an explicit authorized application action. */
export function requestSessionLock(): SessionLockRequest {
  const [state, setState] = createSignal<SessionLockState>("unlocked");
  const { unlock, unsubscribe: dispose } = shellBridge().requestSessionLock(setState);
  if (getOwner()) onCleanup(dispose);
  return { state, unlock, dispose };
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
