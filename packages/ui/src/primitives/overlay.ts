import {
  createContext,
  createEffect,
  createMemo,
  createSignal,
  getOwner,
  onCleanup,
  untrack,
} from "#solid";
import { tryBridgeFor } from "../host.js";
import { isAnimating, registerDismissLayer } from "../nodes.js";
import type {
  Accessor,
  NativeAnimationEvent,
  NativeChild,
  NativeDismissLayer,
  NativeDismissLayerOptions,
  NativeDismissReason,
  NativeNode,
  NativeTransitionEvent,
} from "../types.js";
import { createControllable } from "../components/state.js";

export interface OverlayRootProps {
  open?: boolean | undefined;
  defaultOpen?: boolean | undefined;
  onOpenChange?: ((open: boolean) => void) | undefined;
  children?: NativeChild | undefined;
}

export interface PresenceController {
  readonly mounted: Accessor<boolean>;
  readonly state: () => "open" | "closed";
  readonly ref: (node: NativeNode) => void;
  readonly onEnd: (event: NativeAnimationEvent | NativeTransitionEvent) => void;
}

export interface RetainedPresenceOptions {
  readonly settled?: Accessor<boolean> | undefined;
}

export interface RetainedPresenceController<T> extends PresenceController {
  readonly present: Accessor<boolean>;
  readonly value: Accessor<NonNullable<T> | undefined>;
}

export interface OverlayNodeRef {
  readonly node: Accessor<NativeNode | null>;
  ref(value: NativeNode): void;
}

export interface OverlayState {
  readonly open: Accessor<boolean>;
  setOpen(next: boolean): void;
  readonly trigger: OverlayNodeRef;
  readonly content: OverlayNodeRef;
  readonly title: OverlayNodeRef;
  readonly description: OverlayNodeRef;
}

interface PresenceLifecycleOptions {
  readonly settled?: Accessor<boolean> | undefined;
  readonly onExitComplete?: (() => void) | undefined;
}

function createPresenceLifecycle(
  present: Accessor<boolean>,
  options?: PresenceLifecycleOptions,
): PresenceController {
  let isMounted = untrack(present);
  const [mountedSignal, setMountedSignal] = createSignal(isMounted);
  const setMounted = (next: boolean) => {
    isMounted = next;
    setMountedSignal(next);
  };
  const mounted: Accessor<boolean> = () => {
    mountedSignal();
    return isMounted;
  };
  let isJsGateArmed = false;
  const [jsGateArmedSignal, setJsGateArmedSignal] = createSignal(false);
  const setJsGateArmed = (next: boolean) => {
    isJsGateArmed = next;
    setJsGateArmedSignal(next);
  };
  const settledAccessor = options?.settled;
  let surface: NativeNode | undefined;
  let frame: number | undefined;
  let pending: number | undefined;
  let deadline: number | undefined;
  let generation = 0;
  let exiting = false;
  let cssGateSatisfied = false;

  const clearDeadline = () => {
    if (deadline !== undefined) {
      clearTimeout(deadline);
      deadline = undefined;
    }
  };
  const clearFrameAndPending = () => {
    if (frame !== undefined) {
      cancelAnimationFrame(frame);
      frame = undefined;
    }
    if (pending !== undefined) {
      clearTimeout(pending);
      pending = undefined;
    }
  };
  const cancel = () => {
    generation += 1;
    exiting = false;
    cssGateSatisfied = false;
    clearFrameAndPending();
    clearDeadline();
    if (settledAccessor) setJsGateArmed(false);
  };
  const completeExit = () => {
    cancel();
    setMounted(false);
    options?.onExitComplete?.();
  };
  const tryComplete = () => {
    if (!exiting || !cssGateSatisfied) return;
    if (untrack(present) || !isMounted) return;
    if (settledAccessor && (!isJsGateArmed || !untrack(settledAccessor))) return;
    completeExit();
  };
  const scheduleFrame = (cb: (time: number) => void): number => {
    const bridge = surface ? tryBridgeFor(surface) : tryBridgeFor();
    return bridge?.requestAnimationFrame
      ? bridge.requestAnimationFrame(cb)
      : requestAnimationFrame(cb);
  };
  const checkCssGate = () => {
    if (!exiting || cssGateSatisfied) return;
    if (!surface || !isAnimating(surface)) {
      cssGateSatisfied = true;
      clearDeadline();
    }
  };

  if (settledAccessor) {
    createEffect(
      () => {
        jsGateArmedSignal();
        return isJsGateArmed ? Boolean(settledAccessor()) : false;
      },
      isSettled => {
        if (isSettled) tryComplete();
      },
    );
  }

  createEffect(
    () => present(),
    open => {
      cancel();
      if (open) {
        setMounted(true);
        return;
      }
      if (!isMounted) return;
      exiting = true;
      cssGateSatisfied = false;
      const current = generation;
      frame = scheduleFrame(() => {
        if (current !== generation) return;
        frame = undefined;
        pending = setTimeout(() => {
          if (current !== generation) return;
          pending = undefined;
          checkCssGate();
          if (settledAccessor) setJsGateArmed(true);
          tryComplete();
        }, 0);
      });
      deadline = setTimeout(() => {
        if (current !== generation) return;
        deadline = undefined;
        clearFrameAndPending();
        cssGateSatisfied = true;
        if (settledAccessor) setJsGateArmed(true);
        tryComplete();
      }, 1000);
    },
  );

  onCleanup(() => {
    cancel();
    surface = undefined;
  });

  return {
    mounted,
    state: (): "open" | "closed" => (present() ? "open" : "closed"),
    ref(node: NativeNode) {
      surface = node;
      if (getOwner()) {
        onCleanup(() => {
          if (surface === node) surface = undefined;
        });
      }
    },
    onEnd(event: NativeAnimationEvent | NativeTransitionEvent) {
      if (
        exiting &&
        !cssGateSatisfied &&
        frame === undefined &&
        pending === undefined &&
        event.target === surface
      ) {
        checkCssGate();
        tryComplete();
      }
    },
  };
}

export function createPresence(present: Accessor<boolean>): PresenceController {
  return createPresenceLifecycle(present);
}

interface RetainedBox<T> {
  readonly value: NonNullable<T> | undefined;
}

const EMPTY_RETAINED: RetainedBox<never> = { value: undefined };

function isPresentValue<T>(value: T | null | undefined): value is NonNullable<T> {
  return value !== null && value !== undefined;
}

/**
 * Retain the last non-nullish payload while exit animations and/or a reactive JS gate (`settled`)
 * finish, then unmount and release the payload atomically.
 */
export function createRetainedPresence<T>(
  source: Accessor<T | null | undefined>,
  options?: RetainedPresenceOptions,
): RetainedPresenceController<T> {
  const initialRaw = untrack(source);
  let retainedValue: NonNullable<T> | undefined = isPresentValue(initialRaw)
    ? initialRaw
    : undefined;
  const [retained, setRetainedSignal] = createSignal<RetainedBox<T>>(
    retainedValue !== undefined ? { value: retainedValue } : EMPTY_RETAINED,
  );
  const setRetained = (next: NonNullable<T> | undefined) => {
    retainedValue = next;
    setRetainedSignal(next !== undefined ? { value: next } : EMPTY_RETAINED);
  };
  const present = createMemo(() => isPresentValue(source()));

  const lifecycle = createPresenceLifecycle(present, {
    settled: options?.settled,
    onExitComplete: () => {
      setRetained(undefined);
    },
  });

  createEffect(
    () => source(),
    next => {
      if (isPresentValue(next) && !Object.is(retainedValue, next)) {
        setRetained(next);
      }
    },
  );

  onCleanup(() => {
    setRetained(undefined);
  });

  const value: Accessor<NonNullable<T> | undefined> = () => {
    const current = source();
    if (isPresentValue(current)) return current;
    retained();
    return retainedValue;
  };

  return {
    mounted: lifecycle.mounted,
    state: lifecycle.state,
    ref: lifecycle.ref,
    onEnd: lifecycle.onEnd,
    present,
    value,
  };
}

function reference(): OverlayNodeRef {
  const [node, setNode] = createSignal<NativeNode | null>(null, { ownedWrite: true });
  return {
    node,
    ref(value: NativeNode) {
      setNode(value);
      onCleanup(() => setNode(current => (current === value ? null : current)));
    },
  };
}

export function createOverlayState(props: OverlayRootProps = {}): OverlayState {
  const [read, setOpen] = createControllable<boolean>(
    {
      value: () => props.open,
      defaultValue: () => props.defaultOpen,
      onChange: () => props.onOpenChange,
    },
    false,
  );
  const open = createMemo(() => {
    const value = read();
    if (typeof value !== "boolean") throw new TypeError("Overlay open must be a boolean");
    return value;
  });
  return {
    open,
    setOpen,
    trigger: reference(),
    content: reference(),
    title: reference(),
    description: reference(),
  };
}

export const OverlayContext = createContext<OverlayState | null>(null);

/**
 * Owner-scoped reactive dismiss layer decoupled from Portal, Popper, and built-in stylesheets.
 * Works identically inside WindowSurface, LayerSurface, and PopupSurface.
 */
export function createDismissLayer(
  surface: NativeNode,
  onDismiss: (reason: NativeDismissReason) => void,
  options: Accessor<NativeDismissLayerOptions> | NativeDismissLayerOptions = {},
): NativeDismissLayer {
  const readOptions: Accessor<NativeDismissLayerOptions> =
    typeof options === "function" ? options : () => options;
  const handle = registerDismissLayer(surface, onDismiss, untrack(readOptions));
  if (typeof options === "function") {
    createEffect(readOptions, next => {
      handle.update(next);
    });
  }
  return handle;
}
