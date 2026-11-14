// The native host installs this before evaluation; functions resolve it lazily.
declare const globalThis: {
  readonly __zguiServices: {
    readTextAsset(path: string): HostRequest<string>;
    watchNiri(
      onSnapshot: (snapshot: NiriSnapshot) => void,
      onError: (error: HostServiceError) => void,
    ): HostSubscription;
  };
};

/** A native service failure; codes are host-defined, not a closed enumeration. */
export interface HostServiceError extends Error {
  readonly code: string;
}

/** An owner-thread subscription; cancellation does not return a Promise. */
export interface HostSubscription {
  /** Synchronous and idempotent. Neither callback runs after cancellation. */
  cancel(): void;
}

export interface NiriWindow {
  readonly id: string;
  readonly title: string | null;
  readonly appId: string | null;
  readonly pid: number | null;
  readonly workspaceId: string | null;
  readonly focused: boolean;
  readonly floating: boolean;
  readonly urgent: boolean;
}

export interface NiriWorkspace {
  readonly id: string;
  readonly index: number;
  readonly name: string | null;
  readonly output: string | null;
  readonly urgent: boolean;
  readonly active: boolean;
  readonly focused: boolean;
  readonly activeWindowId: string | null;
}

/** Readonly compositor state. All IDs are lossless decimal strings, never numbers. */
export interface NiriSnapshot {
  readonly windows: readonly NiriWindow[];
  readonly workspaces: readonly NiriWorkspace[];
}

/**
 * Subscribe to the native niri IPC event stream. Both callbacks are required.
 * The first snapshot arrives asynchronously after windows and workspaces initialize;
 * subsequent snapshots may coalesce intermediate compositor events.
 * onError is terminal: no silent reconnect or subsequent snapshots.
 * Call during mount, a lifecycle callback or an event, never preflight/module
 * evaluation or disposal. The host admits at most four subscriptions per session.
 * Invalid arguments, lifecycle violations and admission failures throw synchronously.
 * Cancel in the UI owner's cleanup to release the subscription.
 */
export function watchNiri(
  onSnapshot: (snapshot: NiriSnapshot) => void,
  onError: (error: HostServiceError) => void,
): HostSubscription {
  return globalThis.__zguiServices.watchNiri(onSnapshot, onError);
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
