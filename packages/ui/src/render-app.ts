import { createRoot } from "#solid";
import { insert } from "./renderer/index.js";
import type { NativeChild, NativeNode } from "./types.js";
import type { NativeHost } from "./host.js";

declare const globalThis: NativeHost;

/**
 * Mount native children into a dedicated host root. Cleanup is idempotent and
 * attempts both reactive disposal and native clear, including after mount failure.
 * Host teardown separately cancels timers, requests, and callback registrations.
 */
export function renderApp(code: () => NativeChild, root: NativeNode): () => void {
  let disposeRoot: (() => void) | undefined;
  let disposed = false;

  function dispose() {
    if (disposed) return;
    disposed = true;
    const errors: unknown[] = [];
    try {
      disposeRoot?.();
    } catch (error) {
      errors.push(error);
    }
    try {
      globalThis.__zgui.clearRoot(root);
    } catch (error) {
      errors.push(error);
    }
    if (errors.length === 1) throw errors[0];
    if (errors.length > 1) throw new AggregateError(errors, "Application cleanup failed");
  }

  try {
    // Own the root directly: Universal.render cannot return its disposer if mount throws.
    createRoot(dispose => {
      disposeRoot = dispose;
      insert(root, code());
    });
  } catch (error) {
    try {
      dispose();
    } catch (cleanupError) {
      throw new AggregateError([error, cleanupError], "Application mount and cleanup failed");
    }
    throw error;
  }
  return dispose;
}
