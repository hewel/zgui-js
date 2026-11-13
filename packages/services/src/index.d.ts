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
export declare function readTextAsset(path: string): HostRequest<string>;
