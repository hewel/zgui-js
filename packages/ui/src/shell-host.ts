import "./runtime-globals.js";
export { SessionLockSurface, requestSessionLock } from "./shell.js";
export type {
  SessionLockCapabilityError,
  SessionLockRequest,
  SessionLockState,
  SessionLockSurfaceProps,
} from "./types.js";
import type { NativeHost } from "./host.js";

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue =
  | JsonPrimitive
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

export interface ApprovedExtensionEntryMeta {
  readonly entryId: string;
  readonly extensionId: string;
  readonly version: string;
  readonly kind: string;
  readonly bundle: string;
  readonly canonicalSpecifier: string;
}

export interface OverlayFailureInfo {
  readonly overlayId: string | null;
  readonly phase: "preflight" | "mount" | "commit" | "observation" | "rollback";
  readonly reason: string;
  readonly rollbackBlockedByPolicy: boolean;
  readonly clientMetadata: JsonValue | null;
}

export interface OverlaySessionStatus {
  readonly generation: number;
  readonly overlayId: string | null;
  readonly phase: "base" | "staged" | "observing" | "committed" | "safe-base-shell";
  readonly safeBaseShell: boolean;
  readonly activeExtensionIds: readonly string[];
  readonly clientMetadata: JsonValue | null;
  readonly lastFailure: OverlayFailureInfo | null;
}

type ShellHostBridge = NonNullable<NativeHost["__zguiShellHost"]>;

function shellHostBridge(): ShellHostBridge {
  const bridge = (globalThis as unknown as NativeHost).__zguiShellHost;
  if (!bridge) {
    throw new ReferenceError("Native shell-host operations require a zgui-host session");
  }
  return bridge;
}

/** List metadata for all entry points authorized by the current extension overlay. */
export function listApprovedExtensionEntries(): readonly ApprovedExtensionEntryMeta[] {
  return shellHostBridge().listApprovedEntries();
}

/** Alias of `listApprovedExtensionEntries`. */
export const listApprovedPluginEntries = listApprovedExtensionEntries;

/** Load an approved extension entry module via host-verified `zgui-ext://` resolution. */
export function loadApprovedExtensionEntry<
  T extends Record<string, unknown> = Record<string, unknown>,
>(entryId: string): Promise<T> {
  if (typeof entryId !== "string" || entryId.length === 0) {
    throw new TypeError("loadApprovedExtensionEntry requires a non-empty entryId string");
  }
  return shellHostBridge().loadApprovedEntry(entryId) as Promise<T>;
}

/** Alias of `loadApprovedExtensionEntry`. */
export const loadApprovedPluginEntry = loadApprovedExtensionEntry;

/** Register a bounded (<= 64 KiB) JSON state serializer invoked on normal generation handoff. */
export function registerSessionHandoff(capture: () => JsonValue | null): () => void {
  if (typeof capture !== "function") {
    throw new TypeError("registerSessionHandoff requires a function");
  }
  return shellHostBridge().registerSessionHandoff(capture);
}

/** Consume the bounded in-memory handoff snapshot transferred from the previous generation. */
export function takeSessionHandoff<T extends JsonValue = JsonValue>(): T | null {
  return shellHostBridge().takeSessionHandoff() as T | null;
}

/** Subscribe to Phase 3 session commit notifications (after predecessor retirement). */
export function onSessionCommitted(callback: (status: OverlaySessionStatus) => void): () => void {
  if (typeof callback !== "function") {
    throw new TypeError("onSessionCommitted requires a function");
  }
  return shellHostBridge().onSessionCommitted(status => {
    callback(status as OverlaySessionStatus);
  });
}

/** Subscribe to Phase 2 pre-retirement notifications on the outgoing generation. */
export function onBeforeSessionCommit(callback: () => void): () => void {
  if (typeof callback !== "function") {
    throw new TypeError("onBeforeSessionCommit requires a function");
  }
  return shellHostBridge().onBeforeCommit(callback);
}

/** Read the current generic overlay status, including Safe Base Shell fallback diagnostics. */
export function getOverlayStatus(): OverlaySessionStatus {
  return shellHostBridge().getOverlayStatus() as OverlaySessionStatus;
}

/** Register live shared runtime module namespaces made importable to verified extensions. */
export function registerSharedRuntimeModules(
  modules: Readonly<Record<string, Record<string, unknown>>>,
): void {
  if (!modules || typeof modules !== "object" || Array.isArray(modules)) {
    throw new TypeError("registerSharedRuntimeModules requires a plain specifier map");
  }
  shellHostBridge().registerSharedRuntimeModules(modules);
}
