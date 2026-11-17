import { insert } from "./renderer/index.js";
import type { NativeHost } from "./host.js";
import type { NativeChild, NativeNode, NativeOverlayLayer } from "./types.js";

declare const globalThis: NativeHost;

export interface PortalProps {
  layer?: NativeOverlayLayer;
  children?: NativeChild;
}

export function allocatePortal(layer: NativeOverlayLayer = "popover"): NativeNode {
  return globalThis.__zgui.createPortal(layer);
}

export function Portal(props: PortalProps): NativeNode {
  const root = allocatePortal(props.layer);
  insert(root, () => props.children);
  return root;
}
