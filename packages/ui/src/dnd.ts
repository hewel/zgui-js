import { getOwner, onCleanup } from "#solid";
import type { NativeNode } from "./native-node.js";

export type { NativeNode } from "./native-node.js";
export type {
  DroppedSystemEntry,
  ReadSystemDirOptions,
  SystemDirHandle,
  SystemDirPage,
  SystemEntryInfo,
  SystemEntryKind,
} from "./events.js";

/** Drop effect negotiated synchronously during `onDragOver`. */
export type DragEffect = "copy" | "move" | "none";

/** Outcome delivered to `DragStartOptions.onEnd` when an internal drag finishes. */
export interface DragEndResult {
  readonly dropped: boolean;
  readonly effect: DragEffect;
  readonly reason: "drop" | "cancel" | "escape" | "detach" | "surface-lost" | "retired";
}

/** Options for starting a session-local hit-tested internal drag operation. */
export interface DragStartOptions<T = unknown> {
  /** MIME-like type tags advertised to `onDragOver` for synchronous acceptance negotiation. */
  readonly types: readonly string[];
  /** Opaque session-local payload delivered exclusively on an accepted `onDrop`. */
  readonly payload: T;
  /** Optional callback invoked when the drag completes or cancels. */
  readonly onEnd?: ((result: DragEndResult) => void) | undefined;
}

/** Handle to an active drag operation. */
export interface DragHandle {
  readonly id: number;
  /** Cancels the drag immediately, notifying hovered targets (`onDragLeave`) and `onEnd`. */
  cancel(): void;
}

/** One MIME representation offered to external desktop apps during an outbound drag. */
export interface OutboundDragItem {
  readonly mime: string;
  readonly data: string | Uint8Array;
}

/** Outcome delivered to `OutboundDragOptions.onEnd` when an outbound drag finishes or cancels. */
export interface OutboundDragEndResult {
  readonly accepted: boolean;
  readonly effect: DragEffect;
  readonly reason: "finished" | "cancel" | "surface-lost" | "retired";
}

/** Options for starting a platform-driven outbound drag. */
export interface OutboundDragOptions {
  /** Non-empty list of MIME representations (1..=16 entries, <= 4 MiB total). */
  readonly items: readonly OutboundDragItem[];
  /** Actions the destination may choose. Defaults to Copy only. */
  readonly allowedEffects?: readonly ("copy" | "move")[] | undefined;
  /** Optional callback invoked when the external drag completes or cancels. */
  readonly onEnd?: ((result: OutboundDragEndResult) => void) | undefined;
}

interface NativeDndBridge {
  startInternalDrag(sourceNode: NativeNode, options: DragStartOptions<unknown>): DragHandle;
  startOutboundDrag(sourceNode: NativeNode, options: OutboundDragOptions): DragHandle;
}

function nativeDnd(): NativeDndBridge {
  return (globalThis as unknown as { readonly __zguiDnd: NativeDndBridge }).__zguiDnd;
}

/**
 * Starts a session-local hit-tested drag operation from `sourceNode`.
 * Only one drag may be active per session; pointer capture stays on the source
 * while drop targets are resolved through native `Window::chain_at` hit-testing.
 * Auto-cancels on Solid owner cleanup, Escape, source node detach, surface loss, or retirement.
 */
export function startInternalDrag<T = unknown>(
  sourceNode: NativeNode,
  options: DragStartOptions<T>,
): DragHandle {
  const handle = nativeDnd().startInternalDrag(sourceNode, options);
  if (getOwner()) {
    onCleanup(() => handle.cancel());
  }
  return handle;
}

/**
 * Starts a platform-driven outbound drag from `sourceNode` to external applications.
 * Requires an active live pointer/touch press on the originating surface (throws `E_DRAG_NO_LIVE_PRESS`
 * otherwise). Auto-cancels on Solid owner cleanup or session retirement. The host never deletes
 * source data; only act on a Move after `onEnd` reports it as committed.
 */
export function startOutboundDrag(
  sourceNode: NativeNode,
  options: OutboundDragOptions,
): DragHandle {
  const handle = nativeDnd().startOutboundDrag(sourceNode, options);
  if (getOwner()) {
    onCleanup(() => handle.cancel());
  }
  return handle;
}
