import { getOwner, onCleanup } from "#solid";

// The native host installs this before evaluation; functions resolve it lazily.
declare const globalThis: {
  readonly __zguiServices: {
    readTextAsset(path: string): HostRequest<string>;
  };
};

/** A native service failure; codes are host-defined, not a closed enumeration. */
export interface HostServiceError extends Error {
  readonly code: string;
}

/** A host-owned asynchronous operation; cancellation does not return a Promise. */
export interface HostRequest<T> {
  readonly promise: Promise<T>;
  /**
   * Reject a pending promise with an Error whose code is "Cancelled".
   * Idempotent after cancellation or settlement. Worker cancellation is best effort;
   * cancelled results are never delivered as success to this request.
   */
  cancel(): void;
}

/**
 * Read a manifest-declared UTF-8 asset relative to the active application package.
 * Invalid arguments, non-canonical paths and undeclared assets throw synchronously.
 * I/O, UTF-8 and size failures reject promise with an Error carrying a string code.
 * Call during mount, a lifecycle callback or an event, never module evaluation.
 */
export function readTextAsset(path: string): HostRequest<string> {
  return globalThis.__zguiServices.readTextAsset(path);
}

/**
 * Read a declared UTF-8 asset inside a Solid owner scope, cancelling the host
 * request automatically on owner cleanup without triggering strict rejection faults.
 */
export function loadTextAsset(path: string): Promise<string> {
  const request = readTextAsset(path);
  let cancelled = false;
  if (getOwner()) {
    onCleanup(() => {
      cancelled = true;
      request.cancel();
    });
  }
  return request.promise.catch((reason: unknown) => {
    if (cancelled && reason instanceof Error && "code" in reason && reason.code === "Cancelled") {
      return "";
    }
    throw reason;
  });
}
