import { createContext, createEffect, createMemo, createSignal, onCleanup, untrack } from "#solid";
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

export function createPresence(present: Accessor<boolean>): PresenceController {
  const [mounted, setMounted] = createSignal(untrack(present));
  let surface: NativeNode | undefined;
  let frame: number | undefined;
  let pending: number | undefined;
  let deadline: number | undefined;
  let generation = 0;
  const cancel = () => {
    generation += 1;
    if (frame !== undefined) cancelAnimationFrame(frame);
    if (pending !== undefined) clearTimeout(pending);
    if (deadline !== undefined) clearTimeout(deadline);
    frame = pending = deadline = undefined;
  };
  const leave = () => {
    cancel();
    setMounted(false);
  };
  const finish = () => {
    if (!untrack(present) && untrack(mounted) && (!surface || !isAnimating(surface))) leave();
  };
  createEffect(
    () => present(),
    open => {
      cancel();
      if (open) {
        setMounted(true);
        return;
      }
      if (!untrack(mounted)) return;
      const current = generation;
      frame = requestAnimationFrame(() => {
        if (current !== generation) return;
        frame = undefined;
        pending = setTimeout(() => {
          if (current !== generation) return;
          pending = undefined;
          finish();
        }, 0);
      });
      deadline = setTimeout(() => {
        if (current === generation && !untrack(present)) leave();
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
      onCleanup(() => {
        if (surface === node) surface = undefined;
      });
    },
    onEnd(event: NativeAnimationEvent | NativeTransitionEvent) {
      if (frame === undefined && pending === undefined && event.target === surface) finish();
    },
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
