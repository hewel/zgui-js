import { createEffect, getOwner, onCleanup } from "#solid";
import { tryBridgeFor, withCallbackBridge } from "./host.js";
import { flush } from "./renderer/index.js";
import type { Accessor } from "./types.js";

/** Reusable owner-scoped one-shot timer controller. */
export interface DelayHandle {
  /** Schedule `action` after `delay` ms, cancelling any previously pending timer. */
  readonly schedule: (delay: number, action: () => void) => void;
  /** Alias for `schedule`; schedules `action` after `delay` ms. */
  readonly after: (delay: number, action: () => void) => void;
  /** Cancel any pending timer. */
  readonly cancel: () => void;
  /** Whether a timer is currently pending. */
  readonly pending: () => boolean;
}

function validateDelay(delay: number, label: string, requirePositive = false): void {
  if (!Number.isFinite(delay) || (requirePositive ? delay <= 0 : delay < 0)) {
    throw new RangeError(
      `${label} must be a finite ${requirePositive ? "positive" : "non-negative"} number`,
    );
  }
}

function runScoped(bridge: ReturnType<typeof tryBridgeFor>, action: () => void): void {
  if (bridge) {
    withCallbackBridge(bridge, () => {
      action();
      flush();
    });
  } else {
    action();
    flush();
  }
}

/**
 * Create a reusable, cancellable one-shot timer handle that automatically cancels
 * when its owning Solid scope disposes.
 */
export function createDelay(): DelayHandle {
  const ownerBridge = tryBridgeFor();
  let timer: number | undefined;
  let generation = 0;
  let disposed = false;

  const cancel = () => {
    generation += 1;
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };

  const schedule = (delay: number, action: () => void) => {
    validateDelay(delay, "Delay");
    cancel();
    if (disposed) return;
    const current = generation;
    const invoke = () => {
      if (disposed || current !== generation) return;
      timer = undefined;
      runScoped(ownerBridge ?? tryBridgeFor(), action);
    };
    timer = setTimeout(invoke, delay);
  };

  if (getOwner()) {
    onCleanup(() => {
      disposed = true;
      cancel();
    });
  }

  return {
    schedule,
    after: schedule,
    cancel,
    pending: () => timer !== undefined,
  };
}

/**
 * Schedule a one-shot callback after `delay` ms, automatically cancelling when
 * its owning Solid scope disposes. Returns a manual cancel function.
 */
export function createTimeout(action: () => void, delay: number): () => void {
  const handle = createDelay();
  handle.schedule(delay, action);
  return handle.cancel;
}

/**
 * Schedule a repeating callback every `interval` ms (chained after each tick so
 * ticks never overlap). Accepts a static number or a reactive accessor returning
 * milliseconds (or `false`/`null`/`undefined` to pause). Automatically cancels
 * on owner disposal and returns a manual cancel function.
 */
export function createInterval(
  action: () => void,
  interval: number | Accessor<number | false | null | undefined>,
): () => void {
  const ownerBridge = tryBridgeFor();
  let timer: number | undefined;
  let generation = 0;
  let stopped = false;

  const clear = () => {
    generation += 1;
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };

  const arm = (ms: number | false | null | undefined) => {
    clear();
    if (stopped || ms === false || ms == null) return;
    validateDelay(ms, "Interval", true);
    const current = generation;
    const tick = () => {
      if (stopped || current !== generation) return;
      timer = undefined;
      runScoped(ownerBridge ?? tryBridgeFor(), action);
      if (stopped || current !== generation) return;
      timer = setTimeout(tick, ms);
    };
    timer = setTimeout(tick, ms);
  };

  const cancel = () => {
    stopped = true;
    clear();
  };

  if (getOwner()) onCleanup(cancel);

  if (typeof interval === "function") {
    createEffect(interval, ms => {
      arm(ms);
      return clear;
    });
  } else {
    arm(interval);
  }

  return cancel;
}
