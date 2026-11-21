import { createSignal, untrack } from "#solid";
import type { Setter, Signal } from "./types.js";

declare const process: { readonly env: { readonly NODE_ENV?: string } };

interface NativeDevStateChannel {
  read(key: string, expectedVersion?: number): string | null;
  write(key: string, json: string, version?: number): void;
  remove(key: string): void;
}

/** Options controlling dev-reload plain-data persistence for `createDevState`. */
export interface DevStateOptions<T> {
  /** Schema version for this slot; bump to force a clean cold start on reload (default: `1`). */
  readonly version?: number | undefined;
  /** Optional validator/reviver applied to hydrated plain JSON data; throwing falls back to `initial`. */
  readonly validate?: ((raw: unknown) => T) | undefined;
  /** Optional serializer producing plain JSON-compatible data before snapshot storage. */
  readonly serialize?: ((value: T) => unknown) | undefined;
}

const MAX_PLAIN_DEPTH = 32;

/** Returns the host-installed dev state channel when running in a dev-reload session, or `null`. */
export function getDevStateChannel(): NativeDevStateChannel | null {
  if (process.env.NODE_ENV === "production") {
    return null;
  }
  const channel = (globalThis as { __zguiDevState?: NativeDevStateChannel }).__zguiDevState;
  if (!channel || typeof channel.read !== "function" || typeof channel.write !== "function") {
    return null;
  }
  return channel;
}

function assertPlainJsonValue(value: unknown, depth: number, seen: Set<object>): void {
  if (depth > MAX_PLAIN_DEPTH) {
    throw new RangeError("createDevState value exceeds maximum JSON nesting depth");
  }
  if (value === null || value === undefined) return;
  const kind = typeof value;
  if (kind === "boolean" || kind === "string") return;
  if (kind === "number") {
    if (!Number.isFinite(value)) {
      throw new TypeError("createDevState numbers must be finite");
    }
    return;
  }
  if (kind !== "object") {
    throw new TypeError(`createDevState only accepts plain JSON-serializable data (got ${kind})`);
  }
  const obj = value as object;
  if (seen.has(obj)) {
    throw new TypeError("createDevState cannot serialize cyclic object references");
  }
  const proto = Object.getPrototypeOf(obj);
  if (Array.isArray(obj)) {
    seen.add(obj);
    for (let i = 0; i < obj.length; i++) {
      assertPlainJsonValue(obj[i], depth + 1, seen);
    }
    seen.delete(obj);
    return;
  }
  if (proto !== Object.prototype && proto !== null) {
    throw new TypeError("createDevState rejects class instances, Promises, and native handles");
  }
  if (Object.getOwnPropertySymbols(obj).length > 0) {
    throw new TypeError("createDevState rejects objects carrying opaque symbol markers or native handles");
  }
  seen.add(obj);
  for (const entryValue of Object.values(obj as Record<string, unknown>)) {
    if (entryValue !== undefined) {
      assertPlainJsonValue(entryValue, depth + 1, seen);
    }
  }
  seen.delete(obj);
}

/** Serialize a strictly plain JSON value, rejecting closures, Promises, class instances, and handles. */
export function serializePlainDevState(value: unknown): string {
  assertPlainJsonValue(value, 0, new Set<object>());
  const encoded = JSON.stringify(value);
  if (encoded === undefined) {
    return "null";
  }
  return encoded;
}

/** Check whether a value can be safely serialized as plain JSON without throwing. */
export function trySerializePlainDevState(value: unknown): string | null {
  try {
    return serializePlainDevState(value);
  } catch {
    return null;
  }
}

function resolveInitial<T>(initial: T | (() => T)): T {
  return typeof initial === "function" ? (initial as () => T)() : initial;
}

function makePlainSignal<T>(initialValue: T): Signal<T> {
  return createSignal<T>(initialValue as Exclude<T, Function>);
}

/**
 * Dev-only signal backed by the host's bounded plain-data reload snapshot channel.
 * Degrades to a standard `createSignal` in production builds or when no channel is active.
 */
export function createDevState<T>(
  key: string,
  initial: T | (() => T),
  options?: DevStateOptions<T>,
): Signal<T> {
  if (process.env.NODE_ENV === "production") {
    return makePlainSignal<T>(resolveInitial(initial));
  }
  const channel = getDevStateChannel();
  if (!channel) {
    return makePlainSignal<T>(resolveInitial(initial));
  }
  if (typeof key !== "string" || key.length === 0) {
    throw new TypeError("createDevState key must be a non-empty string");
  }
  const version = options?.version ?? 1;
  if (!Number.isSafeInteger(version)) {
    throw new RangeError("createDevState version must be a safe integer");
  }

  let startValue: T;
  const raw = channel.read(key, version);
  if (raw !== null) {
    try {
      const parsed: unknown = JSON.parse(raw);
      startValue = options?.validate ? options.validate(parsed) : (parsed as T);
    } catch {
      startValue = resolveInitial(initial);
    }
  } else {
    startValue = resolveInitial(initial);
  }

  const persist = (value: T): void => {
    const payload = options?.serialize ? options.serialize(value) : value;
    const json = serializePlainDevState(payload);
    channel.write(key, json, version);
  };

  persist(startValue);

  const [get, rawSet] = makePlainSignal<T>(startValue);

  const set = ((value?: unknown) => {
    const next = typeof value === "function"
      ? (value as (previous: T) => T)(untrack(get))
      : (value as T);
    persist(next);
    return (rawSet as (nextValue: T) => T)(next);
  }) as Setter<T>;

  return [get, set];
}
