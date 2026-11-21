import { getOwner, onCleanup } from "#solid";
import type { HostRequest, HostServiceError } from "./index";

/** HTTP request headers accepted by `httpRequest` and `createScopedFetch`. */
export type HttpHeadersInit =
  | Readonly<Record<string, string | undefined>>
  | readonly (readonly [string, string])[];

/** Options for an origin-allowlisted HTTP request. */
export interface HttpRequestOptions {
  readonly url: string;
  readonly method?: string | undefined;
  readonly headers?: HttpHeadersInit | undefined;
  readonly body?: string | Uint8Array | undefined;
  /** Per-phase network timeout in milliseconds (100..=300000). */
  readonly timeoutMs?: number | undefined;
  /** Maximum decompressed response body bytes (default 16 MiB, ceiling 64 MiB). */
  readonly maxResponseBytes?: number | undefined;
  /** Maximum redirect hops (0..=10, default 5). Each hop is re-authorized against the manifest. */
  readonly maxRedirects?: number | undefined;
}

/** Pull-driven bounded HTTP response handle. */
export interface HttpResponse {
  readonly ok: boolean;
  readonly status: number;
  readonly statusText: string;
  readonly url: string;
  readonly redirected: boolean;
  readonly headers: Readonly<Record<string, string>>;
  readonly bodyUsed: boolean;
  /** Pulls the next bounded decompressed chunk, or `null` at EOF. */
  readChunk(): Promise<Uint8Array | null>;
  /** Drains all remaining bounded chunks into a single `Uint8Array`. */
  bytes(): Promise<Uint8Array>;
  /** Drains all remaining bounded chunks and decodes UTF-8 text. */
  text(): Promise<string>;
  /** Drains all remaining bounded chunks and parses JSON. */
  json<T = unknown>(): Promise<T>;
  /** Cancels and releases the underlying response stream immediately. */
  close(): void;
}

/** Options for opening an origin-allowlisted Server-Sent Events stream. */
export interface EventStreamOptions {
  readonly url: string;
  readonly headers?: HttpHeadersInit | undefined;
  readonly lastEventId?: string | undefined;
  readonly timeoutMs?: number | undefined;
  readonly maxEventBytes?: number | undefined;
  readonly maxRedirects?: number | undefined;
}

/** Metadata delivered when an SSE connection is established. */
export interface EventStreamOpenInfo {
  readonly status: number;
  readonly url: string;
  readonly headers: Readonly<Record<string, string>>;
}

/** Parsed Server-Sent Events message frame. */
export interface ServerSentEvent {
  readonly event: string;
  readonly data: string;
  readonly id?: string | undefined;
  readonly retryMs?: number | undefined;
}

/** Callbacks for `openEventStream`. */
export interface EventStreamCallbacks {
  onOpen?(info: EventStreamOpenInfo): void;
  onMessage(event: ServerSentEvent): void;
  onError?(error: HostServiceError): void;
  onClose?(): void;
}

/** Handle to an active Server-Sent Events stream. */
export interface EventStream {
  /** Stops callback delivery and closes the underlying transport immediately. */
  close(): void;
}

/** Options for opening an origin-allowlisted full-duplex WebSocket connection. */
export interface WebSocketOptions {
  readonly url: string;
  readonly headers?: HttpHeadersInit | undefined;
  readonly protocols?: readonly string[] | undefined;
  readonly timeoutMs?: number | undefined;
  readonly maxMessageBytes?: number | undefined;
  /** Maximum queued outbound bytes before `send` fails with `E_BACKPRESSURE`. */
  readonly maxOutboundBytes?: number | undefined;
}

/** Metadata delivered when a WebSocket handshake completes. */
export interface WebSocketOpenInfo {
  readonly url: string;
  readonly protocol: string | null;
}

/** Inbound WebSocket text or binary frame. */
export type WebSocketMessage =
  | { readonly kind: "text"; readonly data: string }
  | { readonly kind: "binary"; readonly data: Uint8Array };

/** Metadata delivered when a WebSocket closes. */
export interface WebSocketCloseInfo {
  readonly code: number;
  readonly reason: string;
}

/** Callbacks for `connectWebSocket`. */
export interface WebSocketCallbacks {
  onOpen?(info: WebSocketOpenInfo): void;
  onMessage(message: WebSocketMessage): void;
  onError?(error: HostServiceError): void;
  onClose?(info: WebSocketCloseInfo): void;
}

/** Handle to an active full-duplex WebSocket with explicit send backpressure. */
export interface WebSocketConnection {
  /** Queues a text or binary frame; throws `HostServiceError` (`E_BACKPRESSURE`) when full. */
  send(data: string | Uint8Array): void;
  /** Initiates a clean close handshake and stops callback delivery. */
  close(code?: number, reason?: string): void;
}

/** Defaults shared by a `createScopedFetch` adapter instance. */
export interface ScopedFetchDefaults {
  readonly baseUrl?: string | undefined;
  readonly headers?: HttpHeadersInit | undefined;
  readonly timeoutMs?: number | undefined;
  readonly maxResponseBytes?: number | undefined;
  readonly maxRedirects?: number | undefined;
}

/** Minimal abort-signal surface compatible with standard `AbortSignal` or custom tokens. */
export interface ScopedAbortSignal {
  readonly aborted: boolean;
  addEventListener?(type: "abort", listener: () => void, options?: { once?: boolean }): void;
  removeEventListener?(type: "abort", listener: () => void): void;
}

/** Per-call init options for a `createScopedFetch` adapter. */
export interface ScopedFetchInit {
  readonly method?: string | undefined;
  readonly headers?: HttpHeadersInit | undefined;
  readonly body?: string | Uint8Array | undefined;
  readonly timeoutMs?: number | undefined;
  readonly maxResponseBytes?: number | undefined;
  readonly maxRedirects?: number | undefined;
  readonly signal?: ScopedAbortSignal | undefined;
}

/** Explicit opt-in fetch-like function backed by `httpRequest` (no cookies, no `clone()`). */
export type ScopedFetch = (input: string, init?: ScopedFetchInit) => Promise<HttpResponse>;

interface NativeHttpHead {
  readonly status: number;
  readonly statusText: string;
  readonly url: string;
  readonly redirected: boolean;
  readonly headers: Readonly<Record<string, string>>;
}

interface NativeHttpHandle {
  readonly headPromise: Promise<NativeHttpHead>;
  readChunk(): Promise<Uint8Array | null>;
  cancel(): void;
}

interface NativeWebSocketHandle {
  sendText(text: string): void;
  sendBinary(data: Uint8Array): void;
  close(code?: number, reason?: string): void;
}

interface NativeNetworkBridge {
  httpRequest(options: HttpRequestOptions): NativeHttpHandle;
  openEventStream(options: EventStreamOptions, callbacks: EventStreamCallbacks): EventStream;
  connectWebSocket(options: WebSocketOptions, callbacks: WebSocketCallbacks): NativeWebSocketHandle;
}

function nativeServices(): NativeNetworkBridge {
  return (globalThis as unknown as { readonly __zguiServices: NativeNetworkBridge }).__zguiServices;
}

function decodeUtf8(bytes: Uint8Array): string {
  const parts: string[] = [];
  let i = 0;
  const len = bytes.length;
  while (i < len) {
    let chunk = "";
    const end = Math.min(len, i + 4096);
    while (i < end) {
      const b0 = bytes[i++]!;
      if (b0 < 0x80) {
        chunk += String.fromCharCode(b0);
      } else if ((b0 & 0xe0) === 0xc0 && i < len) {
        const b1 = bytes[i++]!;
        chunk += String.fromCharCode(((b0 & 0x1f) << 6) | (b1 & 0x3f));
      } else if ((b0 & 0xf0) === 0xe0 && i + 1 < len) {
        const b1 = bytes[i++]!;
        const b2 = bytes[i++]!;
        chunk += String.fromCharCode(((b0 & 0x0f) << 12) | ((b1 & 0x3f) << 6) | (b2 & 0x3f));
      } else if ((b0 & 0xf8) === 0xf0 && i + 2 < len) {
        const b1 = bytes[i++]!;
        const b2 = bytes[i++]!;
        const b3 = bytes[i++]!;
        const cp =
          (((b0 & 0x07) << 18) | ((b1 & 0x3f) << 12) | ((b2 & 0x3f) << 6) | (b3 & 0x3f)) - 0x10000;
        chunk += String.fromCharCode(0xd800 + (cp >> 10), 0xdc00 + (cp & 0x3ff));
      } else {
        chunk += "\uFFFD";
      }
    }
    parts.push(chunk);
  }
  return parts.join("");
}

function concatChunks(chunks: readonly Uint8Array[], totalLength: number): Uint8Array {
  if (chunks.length === 0) return new Uint8Array(0);
  if (chunks.length === 1) return chunks[0]!;
  const out = new Uint8Array(totalLength);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

function createHttpResponse(head: NativeHttpHead, native: NativeHttpHandle): HttpResponse {
  let bodyUsed = false;
  let closed = false;

  const readChunk = (): Promise<Uint8Array | null> => {
    bodyUsed = true;
    return native.readChunk();
  };

  const drainBytes = async (): Promise<Uint8Array> => {
    if (bodyUsed) {
      throw Object.assign(new Error("HTTP response body has already been consumed"), {
        code: "E_PROTOCOL",
      });
    }
    bodyUsed = true;
    const chunks: Uint8Array[] = [];
    let total = 0;
    for (;;) {
      const next = await native.readChunk();
      if (next === null) break;
      chunks.push(next);
      total += next.length;
    }
    return concatChunks(chunks, total);
  };

  return {
    ok: head.status >= 200 && head.status < 300,
    status: head.status,
    statusText: head.statusText,
    url: head.url,
    redirected: head.redirected,
    headers: head.headers,
    get bodyUsed() {
      return bodyUsed;
    },
    readChunk,
    bytes: drainBytes,
    async text(): Promise<string> {
      return decodeUtf8(await drainBytes());
    },
    async json<T = unknown>(): Promise<T> {
      return JSON.parse(decodeUtf8(await drainBytes())) as T;
    },
    close(): void {
      if (closed) return;
      closed = true;
      native.cancel();
    },
  };
}

/**
 * Starts an origin-allowlisted HTTP request with pull-driven bounded response chunks.
 * Automatically cancels on Solid owner cleanup when called inside an owner scope.
 */
export function httpRequest(options: HttpRequestOptions): HostRequest<HttpResponse> {
  const native = nativeServices().httpRequest(options);
  const promise = native.headPromise.then(head => createHttpResponse(head, native));
  const cancel = (): void => {
    native.cancel();
  };
  if (getOwner()) {
    onCleanup(() => {
      void promise.catch(() => {});
      cancel();
    });
  }
  return { promise, cancel };
}

/**
 * Opens an origin-allowlisted Server-Sent Events stream parsed incrementally in Rust.
 * Automatically closes on Solid owner cleanup when called inside an owner scope.
 */
export function openEventStream(
  options: EventStreamOptions,
  callbacks: EventStreamCallbacks,
): EventStream {
  const stream = nativeServices().openEventStream(options, callbacks);
  if (getOwner()) {
    onCleanup(() => stream.close());
  }
  return stream;
}

/**
 * Opens an origin-allowlisted full-duplex WebSocket with explicit send backpressure.
 * Automatically closes on Solid owner cleanup when called inside an owner scope.
 */
export function connectWebSocket(
  options: WebSocketOptions,
  callbacks: WebSocketCallbacks,
): WebSocketConnection {
  const native = nativeServices().connectWebSocket(options, callbacks);
  const connection: WebSocketConnection = {
    send(data: string | Uint8Array): void {
      if (typeof data === "string") {
        native.sendText(data);
      } else {
        native.sendBinary(data);
      }
    },
    close(code?: number, reason?: string): void {
      native.close(code, reason);
    },
  };
  if (getOwner()) {
    onCleanup(() => connection.close(1000, "owner cleanup"));
  }
  return connection;
}

function normalizeHeaderEntries(headers: HttpHeadersInit | undefined): [string, string][] {
  if (!headers) return [];
  if (Array.isArray(headers)) {
    return headers.map(([k, v]) => [k, v]);
  }
  const out: [string, string][] = [];
  for (const [k, v] of Object.entries(headers)) {
    if (v !== undefined) out.push([k, v]);
  }
  return out;
}

function mergeHeaders(
  base: HttpHeadersInit | undefined,
  override: HttpHeadersInit | undefined,
): [string, string][] | undefined {
  const merged = new Map<string, [string, string]>();
  for (const [k, v] of normalizeHeaderEntries(base)) {
    merged.set(k.toLowerCase(), [k, v]);
  }
  for (const [k, v] of normalizeHeaderEntries(override)) {
    merged.set(k.toLowerCase(), [k, v]);
  }
  return merged.size > 0 ? [...merged.values()] : undefined;
}

function resolveScopedUrl(baseUrl: string | undefined, input: string): string {
  if (!baseUrl || /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(input)) {
    return input;
  }
  const cleanBase = baseUrl.endsWith("/") ? baseUrl.slice(0, -1) : baseUrl;
  const cleanPath = input.startsWith("/") ? input : `/${input}`;
  return `${cleanBase}${cleanPath}`;
}

/**
 * Creates an explicit, opt-in fetch-like adapter backed by `httpRequest`.
 * Never mutates `globalThis`, maintains no Cookie Jar, and omits `Response.clone()`.
 */
export function createScopedFetch(defaults: ScopedFetchDefaults = {}): ScopedFetch {
  return (input: string, init?: ScopedFetchInit): Promise<HttpResponse> => {
    if (init?.signal?.aborted) {
      return Promise.reject(
        Object.assign(new Error("HTTP request cancelled"), { code: "E_CANCELLED" }),
      );
    }
    const headers = mergeHeaders(defaults.headers, init?.headers);
    const req = httpRequest({
      url: resolveScopedUrl(defaults.baseUrl, input),
      method: init?.method,
      headers,
      body: init?.body,
      timeoutMs: init?.timeoutMs ?? defaults.timeoutMs,
      maxResponseBytes: init?.maxResponseBytes ?? defaults.maxResponseBytes,
      maxRedirects: init?.maxRedirects ?? defaults.maxRedirects,
    });
    const signal = init?.signal;
    if (signal?.addEventListener) {
      const onAbort = () => req.cancel();
      signal.addEventListener("abort", onAbort, { once: true });
      return req.promise.finally(() => {
        signal.removeEventListener?.("abort", onAbort);
      });
    }
    return req.promise;
  };
}
