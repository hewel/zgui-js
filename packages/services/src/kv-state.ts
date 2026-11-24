import { createSignal, getOwner, onCleanup, untrack } from "#solid";
import type { Accessor } from "#solid";
import type { JsonValue } from "./index.js";

/** Minimal borrowed KV storage contract (`KvStore<JsonValue>` satisfies this directly). */
export interface KvStateStorage {
  readonly get: (key: string) => Promise<JsonValue | null>;
  readonly set: (key: string, value: JsonValue) => Promise<void>;
}

/** Configuration for a versioned, validated single-key persisted KV state. */
export interface KvStateOptions<T extends JsonValue> {
  /** Synchronously validate and normalize initial, disk, and caller-supplied values. */
  readonly validate: (raw: unknown) => T;
  /** Safe-integer envelope schema version (default: 1). */
  readonly version?: number | undefined;
  /** Trailing writeback debounce in milliseconds, integer 0..0xffffffff (default: 180). */
  readonly debounceMs?: number | undefined;
}

/** Reactive handle for one persisted KV key with durability flush barrier. */
export interface KvState<T extends JsonValue> {
  readonly value: Accessor<T>;
  readonly set: (next: T | ((previous: T) => T)) => void;
  readonly reset: () => void;
  readonly hydrated: Accessor<boolean>;
  readonly saving: Accessor<boolean>;
  readonly error: Accessor<Error | undefined>;
  readonly flush: () => Promise<void>;
}

interface ValueBox<T> {
  readonly value: T;
}

interface FlushBarrier {
  readonly targetRevision: number;
  readonly resolve: () => void;
  readonly reject: (error: Error) => void;
}

function toError(reason: unknown): Error {
  if (reason instanceof Error) return reason;
  return new Error(String(reason));
}

function captureJsonSnapshotInternal(value: unknown, seen: Set<object>): JsonValue {
  if (value === null || typeof value === "boolean" || typeof value === "string") {
    return value;
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new TypeError("KV state values must contain only finite JSON numbers");
    }
    return Object.is(value, -0) ? 0 : value;
  }
  if (typeof value !== "object") {
    throw new TypeError(`KV state value of type ${typeof value} is not JSON-serializable`);
  }
  if (seen.has(value)) {
    throw new TypeError("KV state value cannot contain circular references");
  }
  if (Array.isArray(value)) {
    seen.add(value);
    try {
      const out: JsonValue[] = [];
      for (let i = 0; i < value.length; i += 1) {
        if (!(i in value)) {
          throw new TypeError("KV state arrays cannot contain sparse holes");
        }
        out.push(captureJsonSnapshotInternal(value[i], seen));
      }
      return Object.freeze(out);
    } finally {
      seen.delete(value);
    }
  }
  const proto = Object.getPrototypeOf(value);
  if (proto !== Object.prototype && proto !== null) {
    throw new TypeError("KV state objects must be plain JSON objects");
  }
  seen.add(value);
  try {
    const out: Record<string, JsonValue> = {};
    for (const key of Object.keys(value)) {
      const prop = (value as Record<string, unknown>)[key];
      if (prop === undefined) continue;
      out[key] = captureJsonSnapshotInternal(prop, seen);
    }
    return Object.freeze(out);
  } finally {
    seen.delete(value);
  }
}

function captureJsonSnapshot<T extends JsonValue>(value: unknown): T {
  return captureJsonSnapshotInternal(value, new Set<object>()) as T;
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

/**
 * Bind one versioned key of a borrowed `KvStateStorage` (`KvStore`) to Solid reactive signals.
 * Never opens or closes the underlying database; requires an active Solid owner.
 */
export function createKvState<T extends JsonValue>(
  kv: KvStateStorage,
  key: string,
  initial: T,
  options: KvStateOptions<T>,
): KvState<T> {
  if (!getOwner()) {
    throw new Error("createKvState requires an active Solid owner");
  }
  if (!kv || typeof kv.get !== "function" || typeof kv.set !== "function") {
    throw new TypeError("createKvState requires a storage object with get() and set()");
  }
  if (typeof key !== "string" || key.length === 0) {
    throw new TypeError("KV state key must be a non-empty string");
  }
  if (!options || typeof options.validate !== "function") {
    throw new TypeError("createKvState requires a synchronous validate function");
  }
  const version = options.version ?? 1;
  if (typeof version !== "number" || !Number.isSafeInteger(version)) {
    throw new RangeError("KV state version must be a safe integer");
  }
  const debounceMs = options.debounceMs ?? 180;
  if (
    typeof debounceMs !== "number" ||
    !Number.isInteger(debounceMs) ||
    debounceMs < 0 ||
    debounceMs > 0xffffffff
  ) {
    throw new RangeError("KV state debounceMs must be an integer between 0 and 0xffffffff");
  }
  const validate = options.validate;

  // Validate and freeze initial synchronously before any I/O.
  const initialCandidate = captureJsonSnapshot<JsonValue>(initial);
  const initialSnapshot = captureJsonSnapshot<T>(untrack(() => validate(initialCandidate)));

  let currentSnapshot: T = initialSnapshot;
  const [valueSignal, setValueSignal] = createSignal<ValueBox<T>>({ value: currentSnapshot });
  const publishValue = (next: T) => {
    currentSnapshot = next;
    setValueSignal({ value: next });
  };

  let isHydrated = false;
  const [hydratedSignal, setHydratedSignal] = createSignal(false);
  const publishHydrated = (next: boolean) => {
    isHydrated = next;
    setHydratedSignal(next);
  };

  let isSaving = false;
  const [savingSignal, setSavingSignal] = createSignal(false);
  const publishSaving = (next: boolean) => {
    isSaving = next;
    setSavingSignal(next);
  };

  let currentError: Error | undefined;
  const [errorSignal, setErrorSignal] = createSignal<Error | undefined>(undefined);
  const publishError = (next: Error | undefined) => {
    currentError = next;
    setErrorSignal(next);
  };

  let localRevision = 0;
  let persistedRevision = 0;
  let inFlightRevision: number | null = null;
  let debounceTimer: number | undefined;
  let debounceDue = false;
  let writeBlockedByError = false;
  let disposed = false;
  const barriers: FlushBarrier[] = [];

  const clearDebounce = () => {
    if (debounceTimer !== undefined) {
      clearTimeout(debounceTimer);
      debounceTimer = undefined;
    }
  };

  const resolveSatisfiedBarriers = () => {
    if (!isHydrated) return;
    let writeIdx = 0;
    for (let i = 0; i < barriers.length; i += 1) {
      const barrier = barriers[i]!;
      if (barrier.targetRevision <= persistedRevision) {
        barrier.resolve();
      } else {
        barriers[writeIdx++] = barrier;
      }
    }
    barriers.length = writeIdx;
  };

  const rejectDirtyBarriers = (err: Error) => {
    let writeIdx = 0;
    for (let i = 0; i < barriers.length; i += 1) {
      const barrier = barriers[i]!;
      if (barrier.targetRevision > persistedRevision) {
        barrier.reject(err);
      } else {
        barriers[writeIdx++] = barrier;
      }
    }
    barriers.length = writeIdx;
  };

  const rejectAllBarriers = (err: Error) => {
    const pending = barriers.splice(0, barriers.length);
    for (const barrier of pending) {
      barrier.reject(err);
    }
  };

  const maybeStartWrite = () => {
    if (disposed || !isHydrated || inFlightRevision !== null) return;
    if (localRevision <= persistedRevision) return;
    if (writeBlockedByError || !debounceDue) return;

    const writeRevision = localRevision;
    const writeSnapshot = currentSnapshot;
    inFlightRevision = writeRevision;
    debounceDue = false;
    publishSaving(true);

    let writePromise: Promise<void>;
    try {
      const envelope: JsonValue = Object.freeze({ v: version, data: writeSnapshot });
      writePromise = kv.set(key, envelope);
    } catch (err) {
      writePromise = Promise.reject(err);
    }

    void writePromise.then(
      () => {
        if (disposed) return;
        inFlightRevision = null;
        if (writeRevision > persistedRevision) {
          persistedRevision = writeRevision;
        }
        publishError(undefined);
        resolveSatisfiedBarriers();
        if (localRevision > persistedRevision && debounceDue && !writeBlockedByError) {
          maybeStartWrite();
        }
        if (inFlightRevision === null) {
          publishSaving(false);
        }
      },
      (reason: unknown) => {
        if (disposed) return;
        const normalized = toError(reason);
        inFlightRevision = null;
        writeBlockedByError = true;
        debounceDue = false;
        clearDebounce();
        publishSaving(false);
        publishError(normalized);
        rejectDirtyBarriers(normalized);
      },
    );
  };

  const scheduleWrite = () => {
    clearDebounce();
    debounceDue = false;
    debounceTimer = setTimeout(() => {
      debounceTimer = undefined;
      if (disposed) return;
      debounceDue = true;
      maybeStartWrite();
    }, debounceMs);
  };

  // Launch single initial hydration read immediately.
  let hydratePromise: Promise<JsonValue | null>;
  try {
    hydratePromise = kv.get(key);
  } catch (err) {
    hydratePromise = Promise.reject(err);
  }

  void hydratePromise.then(
    raw => {
      if (disposed) return;
      if (localRevision === 0 && raw !== null) {
        if (
          !isPlainRecord(raw) ||
          Object.keys(raw).length !== 2 ||
          !Object.prototype.hasOwnProperty.call(raw, "v") ||
          typeof raw.v !== "number" ||
          !Number.isSafeInteger(raw.v) ||
          !Object.prototype.hasOwnProperty.call(raw, "data")
        ) {
          publishError(new Error(`Malformed KV envelope for key "${key}"`));
        } else if (raw.v === version) {
          try {
            const diskCandidate = captureJsonSnapshot<JsonValue>(raw.data);
            const validated = captureJsonSnapshot<T>(untrack(() => validate(diskCandidate)));
            publishValue(validated);
          } catch (err) {
            publishError(toError(err));
          }
        }
        // Well-formed version mismatch keeps defaults with no error and no write.
      }
      publishHydrated(true);
      resolveSatisfiedBarriers();
      maybeStartWrite();
    },
    (reason: unknown) => {
      if (disposed) return;
      if (localRevision === 0) {
        publishError(toError(reason));
      }
      publishHydrated(true);
      // Clean flushes never misreport read-only hydration failure as a write failure.
      resolveSatisfiedBarriers();
      maybeStartWrite();
    },
  );

  onCleanup(() => {
    disposed = true;
    clearDebounce();
    publishSaving(false);
    rejectAllBarriers(new Error(`KV state "${key}" was disposed before flush completed`));
  });

  const acceptLocal = (snapshot: T) => {
    localRevision += 1;
    writeBlockedByError = false;
    publishValue(snapshot);
    scheduleWrite();
  };

  return {
    value: () => {
      valueSignal();
      return currentSnapshot;
    },
    set(next: T | ((previous: T) => T)) {
      if (disposed) return;
      const candidate =
        typeof next === "function"
          ? untrack(() => (next as (previous: T) => T)(currentSnapshot))
          : next;
      const preClone = captureJsonSnapshot<JsonValue>(candidate);
      const nextSnapshot = captureJsonSnapshot<T>(untrack(() => validate(preClone)));
      acceptLocal(nextSnapshot);
    },
    reset() {
      if (disposed) return;
      acceptLocal(initialSnapshot);
    },
    hydrated: () => {
      hydratedSignal();
      return isHydrated;
    },
    saving: () => {
      savingSignal();
      return isSaving;
    },
    error: () => {
      errorSignal();
      return currentError;
    },
    flush(): Promise<void> {
      if (disposed) {
        return Promise.reject(new Error(`KV state "${key}" is disposed`));
      }
      const targetRevision = localRevision;
      if (targetRevision <= persistedRevision && isHydrated) {
        return Promise.resolve();
      }
      if (targetRevision > persistedRevision) {
        clearDebounce();
        debounceDue = true;
        writeBlockedByError = false;
      }
      return new Promise<void>((resolve, reject) => {
        barriers.push({ targetRevision, resolve, reject });
        maybeStartWrite();
      });
    },
  };
}
