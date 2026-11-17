import { insert } from "./renderer/index.js";
import type { NativeHost } from "./host.js";
import type { NativeChild, NativeNode, NativeOverlayLayer } from "./types.js";

declare const globalThis: NativeHost;

export interface PortalProps {
  layer?: NativeOverlayLayer;
  children?: NativeChild;
}

export function Portal(props: PortalProps): NativeNode {
  const root = globalThis.__zgui.createPortal(props.layer ?? "popover");
  insert(root, () => props.children);
  return root;
}
