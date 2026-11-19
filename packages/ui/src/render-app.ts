import { createEffect, createRoot, resetErrorHalt } from "#solid";
import { flush, insert } from "./renderer/index.js";
import type { NativeChild, NativeNode } from "./types.js";
import { bridgeFor, provideRegion, rememberBridge } from "./host.js";
import type { UiBridge } from "./host.js";

/**
 * Mount native children into a dedicated host root. Cleanup is idempotent and
 * attempts both reactive disposal and native clear, including after mount failure.
 * Host teardown separately cancels timers, requests, and callback registrations.
 */
export function renderApp(code: () => NativeChild, root: NativeNode): () => void {
  const bridge = bridgeFor(root);
  return mountOwned(() => insert(root, code()), () => bridge.clearRoot(root));
}

/** Own a rootless shell's reactive graph; native content belongs inside LayerSurface. */
export function renderShell(code: () => NativeChild): () => void {
  return mountOwned(() => {
    const children = code();
    createEffect(() => visitShellChildren(children), () => {});
  });
}

function visitShellChildren(child: NativeChild): void {
  if (typeof child === "function") visitShellChildren(child());
  else if (Array.isArray(child)) for (const item of child) visitShellChildren(item);
}

export function renderRegion(code: () => NativeChild, root: NativeNode, bridge: UiBridge): () => void {
  rememberBridge(root, bridge);
  return mountOwned(() => provideRegion(bridge, () => insert(root, code())), () => bridge.clearRoot(root));
}

function mountOwned(mount: () => void, clear?: () => void): () => void {
  let disposeRoot: (() => void) | undefined;
  let disposed = false;

  function dispose() {
    if (disposed) return;
    disposed = true;
    const errors: unknown[] = [];
    try {
      disposeRoot?.();
      flush();
    } catch (error) {
      errors.push(error);
    } finally {
      resetErrorHalt();
    }
    try {
      clear?.();
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
      mount();
    });
    flush();
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
