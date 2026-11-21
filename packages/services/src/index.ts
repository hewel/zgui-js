import { getOwner, onCleanup } from "#solid";

// The native host installs this before evaluation; functions resolve it lazily.
declare const globalThis: {
  readonly __zguiServices: {
    readTextAsset(path: string): HostRequest<string>;
    registerImageAsset(path: string): ImageHandle;
    registerImageBytes(bytes: Uint8Array): ImageHandle;
    getResolvedCapabilities(): ResolvedCapabilities;
    getAuthorizedEndpoint(name: string): string | null;
    connectUnixSocket(path: string, options: UnixSocketOptions): UnixSocket;
    spawnCommand(
      program: string,
      args: readonly string[],
      options?: BufferedCommandOptions | StreamingCommandOptions,
    ): HostRequest<CommandResult> | CommandStream;
    onControlMessage(handler: ControlMessageHandler): () => void;
  };
};

/** A native service failure; codes are host-defined, not a closed enumeration. */
export interface HostServiceError extends Error {
  readonly code: string;
}

/** Path authority resolved once for the active session from its sealed manifest. */
export type ResolvedCapabilities = {
  readonly imagePrefixes: readonly string[];
  readonly unixSockets: readonly string[];
  readonly namedSockets: Readonly<Record<string, string | null | undefined>>;
};

/** Return fresh copies of the session's resolved path declarations. */
export function getResolvedCapabilities(): ResolvedCapabilities {
  return globalThis.__zguiServices.getResolvedCapabilities();
}

/** Return a declared endpoint, or null if unbound. Undeclared names throw E_CAPABILITY. */
export function getAuthorizedEndpoint(name: string): string | null {
  return globalThis.__zguiServices.getAuthorizedEndpoint(name);
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

/** A session-scoped reference to image bytes owned by the native host. */
export interface ImageHandle {
  /** Pass this opaque source to the native Image component. */
  readonly src: string;
  /** Release the handle when it is no longer in use. Safe to call more than once. */
  release(): void;
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

/**
 * Register the verified bytes of a manifest-declared asset as an image handle.
 * Call during mount, a lifecycle callback or an event. Release the handle explicitly.
 */
export function registerImageAsset(path: string): ImageHandle {
  return globalThis.__zguiServices.registerImageAsset(path);
}

function releaseWithOwner(handle: ImageHandle): ImageHandle {
  if (getOwner()) onCleanup(() => handle.release());
  return handle;
}

/** Register an image asset, releasing it automatically if called inside a Solid owner. */
export function loadImageAsset(path: string): ImageHandle {
  return releaseWithOwner(registerImageAsset(path));
}

/** Copy encoded image bytes into the session. Release the returned handle explicitly. */
export function registerImageBytes(bytes: Uint8Array): ImageHandle {
  return globalThis.__zguiServices.registerImageBytes(bytes);
}

/** Register image bytes, releasing the handle automatically inside a Solid owner. */
export function loadImageBytes(bytes: Uint8Array): ImageHandle {
  return releaseWithOwner(registerImageBytes(bytes));
}

/** Callbacks and wire framing for an allowed Unix domain socket. */
export interface UnixSocketOptions {
  /** LF-delimited UTF-8, or a big-endian u32 count including the kind byte and data. */
  framing: "line" | "length";
  onMessage(message: string): void;
  /** In length mode, receives kind 0x01 image data, even if the data is valid UTF-8. */
  onBinaryImage?(image: ImageHandle): void;
  /** Called on peer EOF or failure, but not on explicit close. */
  onClose?(error: HostServiceError | null): void;
}

/** An open or connecting Unix domain socket. */
export interface UnixSocket {
  /** Queues UTF-8 text; length mode sends it with kind 0x00. */
  send(text: string): void;
  /** Stops callback delivery without waiting for the connection to close. */
  close(): void;
}

/** Connect to a manifest-allowed Unix socket; a Solid owner closes it on cleanup. */
export function connectUnixSocket(path: string, options: UnixSocketOptions): UnixSocket {
  const socket = globalThis.__zguiServices.connectUnixSocket(path, options);
  if (getOwner()) {
    onCleanup(() => socket.close());
  }
  return socket;
}

/** Captured output and exit code; status is null if the process did not exit normally. */
export interface CommandResult {
  status: number | null;
  stdout: string;
  stderr: string;
}

/** Capture stdout and stderr until the command completes (the default). */
export interface BufferedCommandOptions {
  stream?: false;
}

/** Receive stdout and stderr one line at a time instead of buffering them. */
export interface StreamingCommandOptions {
  stream: true;
  onStdout?(line: string): void;
  onStderr?(line: string): void;
  /** Reports the process exit code, or null if it did not exit normally. */
  onExit?(status: number | null): void;
  onError?(error: HostServiceError): void;
}

/** A streaming command that can be cancelled or killed without awaiting completion. */
export interface CommandStream {
  /** Stops callback delivery and requests termination without waiting for the worker. */
  cancel(): void;
  /** Alias for cancel(). */
  kill(): void;
}

/** Start a manifest-allowed executable directly, without a shell. */
export function spawnCommand(
  program: string,
  args: readonly string[],
  options: StreamingCommandOptions,
): CommandStream;
/** Start a manifest-allowed executable and capture its output. */
export function spawnCommand(
  program: string,
  args: readonly string[],
  options?: BufferedCommandOptions,
): HostRequest<CommandResult>;
export function spawnCommand(
  program: string,
  args: readonly string[],
  options?: BufferedCommandOptions | StreamingCommandOptions,
): HostRequest<CommandResult> | CommandStream {
  const command = globalThis.__zguiServices.spawnCommand(program, args, options);
  if (getOwner()) {
    onCleanup(() => {
      // Observe owner cancellation without replacing the caller's promise or handling it early.
      if ("promise" in command) void command.promise.catch(() => {});
      command.cancel();
    });
  }
  return command;
}

/** Data that can be encoded as JSON in a control reply. */
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue | undefined };

/** Return plain text, a JSON value, or undefined for a successful empty reply. */
export type ControlMessageHandler = (args: readonly string[]) => JsonValue | undefined;

/** Subscribe to control messages; returns an unsubscribe function and registers owner cleanup. */
export function onControlMessage(handler: ControlMessageHandler): () => void {
  const unsubscribe = globalThis.__zguiServices.onControlMessage(handler);
  if (getOwner()) {
    onCleanup(unsubscribe);
  }
  return unsubscribe;
}
