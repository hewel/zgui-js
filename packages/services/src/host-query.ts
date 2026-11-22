import { createEffect, createSignal, getOwner, onCleanup, untrack } from "#solid";
import type { Accessor } from "#solid";
import type { HostRequest } from "./index";

/** Configuration for `createHostQuery`. */
export interface HostQueryOptions<T> {
  readonly initialValue?: T | undefined;
  readonly keepPreviousData?: boolean | undefined;
  readonly onError?: ((error: Error) => void) | undefined;
}

/** Non-suspending latest-wins reactive state for replaceable cancellable `HostRequest` reads. */
export interface HostQuery<T> {
  readonly data: Accessor<T | undefined>;
  readonly loading: Accessor<boolean>;
  readonly error: Accessor<Error | undefined>;
  readonly refetch: () => void;
}

interface DataBox<T> {
  readonly value: T | undefined;
}

function toError(reason: unknown): Error {
  if (reason instanceof Error) return reason;
  return new Error(String(reason));
}

function isHostRequest<T>(value: unknown): value is HostRequest<T> {
  if (value === null || typeof value !== "object") return false;
  const candidate = value as { readonly promise?: unknown; readonly cancel?: unknown };
  return (
    typeof candidate.cancel === "function" &&
    candidate.promise !== null &&
    typeof candidate.promise === "object" &&
    typeof (candidate.promise as { readonly then?: unknown }).then === "function"
  );
}

/**
 * Bind a reactive source to a replaceable, cancellable `HostRequest<T>` read.
 * Tracks only `source` and explicit `refetch()` calls; requires an active Solid owner.
 */
export function createHostQuery<S, T>(
  source: Accessor<S | null | undefined | false>,
  fetcher: (input: NonNullable<Exclude<S, false>>) => HostRequest<T>,
  options?: HostQueryOptions<T>,
): HostQuery<T> {
  if (!getOwner()) {
    throw new Error("createHostQuery requires an active Solid owner");
  }
  if (typeof source !== "function") {
    throw new TypeError("createHostQuery requires a reactive source accessor");
  }
  if (typeof fetcher !== "function") {
    throw new TypeError("createHostQuery requires a fetcher function returning HostRequest<T>");
  }

  const initialValue = options?.initialValue;
  const keepPreviousData = options?.keepPreviousData ?? true;
  const onError = options?.onError;

  let currentData: T | undefined = initialValue;
  const [dataSignal, setDataSignal] = createSignal<DataBox<T>>({ value: currentData });
  const publishData = (next: T | undefined) => {
    currentData = next;
    setDataSignal({ value: next });
  };

  let isLoading = false;
  const [loadingSignal, setLoadingSignal] = createSignal(false);
  const publishLoading = (next: boolean) => {
    isLoading = next;
    setLoadingSignal(next);
  };

  let currentError: Error | undefined;
  const [errorSignal, setErrorSignal] = createSignal<Error | undefined>(undefined);
  const publishError = (next: Error | undefined) => {
    currentError = next;
    setErrorSignal(next);
  };

  const [refetchTick, setRefetchTick] = createSignal(0);

  let generation = 0;
  let activeRequest: HostRequest<T> | null = null;
  let disposed = false;

  const invalidateAndCancel = () => {
    generation += 1;
    const previous = activeRequest;
    activeRequest = null;
    if (previous !== null) {
      previous.cancel();
    }
  };

  createEffect(
    () => [source(), refetchTick()] as const,
    ([input]) => {
      if (disposed) return;
      invalidateAndCancel();

      if (input === null || input === undefined || input === false) {
        publishData(initialValue);
        publishError(undefined);
        publishLoading(false);
        return;
      }

      const runGeneration = generation;
      if (!keepPreviousData) {
        publishData(initialValue);
      }
      publishError(undefined);
      publishLoading(true);

      let request: HostRequest<T>;
      try {
        const candidate = untrack(() => fetcher(input as NonNullable<Exclude<S, false>>));
        if (!isHostRequest<T>(candidate)) {
          throw new TypeError("createHostQuery fetcher must synchronously return a HostRequest");
        }
        request = candidate;
      } catch (err) {
        if (disposed || runGeneration !== generation) return;
        const normalized = toError(err);
        publishError(normalized);
        publishLoading(false);
        if (onError) untrack(() => onError(normalized));
        return;
      }

      if (disposed || runGeneration !== generation) {
        void request.promise.then(
          () => {},
          () => {},
        );
        request.cancel();
        return;
      }

      activeRequest = request;
      void request.promise.then(
        result => {
          if (disposed || runGeneration !== generation) return;
          if (activeRequest === request) activeRequest = null;
          publishData(result);
          publishError(undefined);
          publishLoading(false);
        },
        (reason: unknown) => {
          if (disposed || runGeneration !== generation) return;
          if (activeRequest === request) activeRequest = null;
          const normalized = toError(reason);
          publishError(normalized);
          publishLoading(false);
          if (onError) untrack(() => onError(normalized));
        },
      );
    },
  );

  onCleanup(() => {
    disposed = true;
    invalidateAndCancel();
    publishLoading(false);
  });

  return {
    data: () => {
      dataSignal();
      return currentData;
    },
    loading: () => {
      loadingSignal();
      return isLoading;
    },
    error: () => {
      errorSignal();
      return currentError;
    },
    refetch() {
      if (disposed) return;
      setRefetchTick(tick => tick + 1);
    },
  };
}
