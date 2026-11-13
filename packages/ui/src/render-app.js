import { createRoot } from "solid-js";
import { insert } from "./renderer/index.js";

export function renderApp(code, root) {
  let disposeRoot;
  let disposed = false;

  function dispose() {
    if (disposed) return;
    disposed = true;
    const errors = [];
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
