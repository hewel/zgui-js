import { getOwner, onCleanup } from "#solid";

/** Error payload reported by an isolated compute worker. */
export interface WorkerErrorEvent {
  readonly code: string;
  readonly message: string;
}

/** Options for spawning a manifest-declared compute worker. */
export interface WorkerOptions {
  /** Per-worker QuickJS heap cap in bytes (256 KiB..=16 MiB, default 16 MiB). */
  readonly maxHeapBytes?: number | undefined;
  /** Optional message listener registered before the worker starts executing. */
  readonly onMessage?: ((payload: unknown) => void) | undefined;
  /** Optional error listener registered before the worker starts executing. */
  readonly onError?: ((error: WorkerErrorEvent) => void) | undefined;
}

/** Handle to an isolated background JavaScript compute worker. */
export interface WorkerHandle {
  /** Post a bounded plain JSON value to the worker (<= 64 KiB, depth <= 32). */
  postMessage(payload: unknown): void;
  /** Subscribe to plain JSON messages posted by the worker. */
  onMessage(handler: (payload: unknown) => void): () => void;
  /** Subscribe to worker execution, timeout, OOM, or mailbox errors. */
  onError(handler: (error: WorkerErrorEvent) => void): () => void;
  /** Immediately close admission, interrupt execution, and discard late replies. */
  terminate(): void;
  /** Whether this worker handle has been terminated. */
  readonly terminated: boolean;
}

interface NativeWorkerHandle {
  postMessage(payload: unknown): void;
  onMessage(handler: (payload: unknown) => void): () => void;
  onError(handler: (error: WorkerErrorEvent) => void): () => void;
  terminate(): void;
  isTerminated(): boolean;
}

interface NativeWorkerServices {
  spawnWorker(name: string, options?: WorkerOptions): NativeWorkerHandle;
}

function nativeWorkerServices(): NativeWorkerServices {
  return (globalThis as unknown as { readonly __zguiServices: NativeWorkerServices })
    .__zguiServices;
}

/**
 * Spawn an isolated background JavaScript compute worker declared in `package.json` `zgui.workers`.
 * Automatically terminates on Solid owner cleanup when called inside an owner scope.
 */
export function spawnWorker(name: string, options?: WorkerOptions): WorkerHandle {
  const native = nativeWorkerServices().spawnWorker(name, options);
  let closed = false;
  const terminate = (): void => {
    if (closed) return;
    closed = true;
    native.terminate();
  };
  if (getOwner()) {
    onCleanup(terminate);
  }
  return {
    postMessage(payload: unknown): void {
      native.postMessage(payload);
    },
    onMessage(handler: (payload: unknown) => void): () => void {
      return native.onMessage(handler);
    },
    onError(handler: (error: WorkerErrorEvent) => void): () => void {
      return native.onError(handler);
    },
    terminate,
    get terminated(): boolean {
      return closed || native.isTerminated();
    },
  };
}
