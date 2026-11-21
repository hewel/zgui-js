import { getOwner, onCleanup } from "#solid";
import type { NativeNode } from "./types.js";

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

/** Handle to an active internal drag operation. */
export interface DragHandle {
  readonly id: number;
  /** Cancels the drag immediately, notifying hovered targets (`onDragLeave`) and `onEnd`. */
  cancel(): void;
}

interface NativeDndBridge {
  startInternalDrag(sourceNode: NativeNode, options: DragStartOptions<unknown>): DragHandle;
}

function nativeDnd(): NativeDndBridge {
  return (globalThis as unknown as { readonly __zguiDnd: NativeDndBridge }).__zguiDnd;
}

/**
 * Starts a session-local hit-tested drag operation from `sourceNode`.
 * Only one internal drag may be active per session; pointer capture stays on the source
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
