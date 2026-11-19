import { createEffect } from "#solid";
import { insert, mountOwned } from "./renderer/index.js";
import type { NativeChild, NativeNode } from "./types.js";
import { bridgeFor } from "./host.js";

/**
 * Mount native children into a dedicated host root. Cleanup is idempotent and
 * attempts both reactive disposal and native clear, including after mount failure.
 * Host teardown separately cancels timers, requests, and callback registrations.
 */
export function renderApp(code: () => NativeChild, root: NativeNode): () => void {
  const bridge = bridgeFor(root);
  return mountOwned(() => insert(root, code()), () => bridge.clearRoot(root));
}

/** Own a rootless shell's reactive graph; native content belongs inside shell surfaces. */
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

