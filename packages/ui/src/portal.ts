import { insert } from "./renderer/index.js";
import { bridgeFor, rememberBridge } from "./host.js";
import type { NativeChild, NativeNode, NativeOverlayLayer } from "./types.js";

export interface PortalProps {
  layer?: NativeOverlayLayer;
  children?: NativeChild;
}

export function allocatePortal(layer: NativeOverlayLayer = "popover"): NativeNode {
  const bridge = bridgeFor();
  return rememberBridge(bridge.createPortal(layer), bridge);
}

export function Portal(props: PortalProps): NativeNode {
  const root = allocatePortal(props.layer);
  insert(root, () => props.children);
  return root;
}
