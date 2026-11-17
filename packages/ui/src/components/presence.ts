import { createEffect, createSignal, onCleanup, untrack } from "#solid";
import { isAnimating } from "../nodes.js";
import type { Accessor, NativeAnimationEvent, NativeNode, NativeTransitionEvent } from "../types.js";

export function createPresence(present: Accessor<boolean>) {
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
  const leave = () => { cancel(); setMounted(false); };
  const finish = () => {
    if (!untrack(present) && untrack(mounted) && (!surface || !isAnimating(surface))) leave();
  };
  createEffect(() => {
    const open = present();
    cancel();
    if (open) { setMounted(true); return; }
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
  });
  onCleanup(() => { cancel(); surface = undefined; });
  return {
    mounted,
    state: (): "open" | "closed" => present() ? "open" : "closed",
    ref(node: NativeNode) {
      surface = node;
      onCleanup(() => { if (surface === node) surface = undefined; });
    },
    onEnd(event: NativeAnimationEvent | NativeTransitionEvent) {
      if (frame === undefined && pending === undefined && event.target === surface) finish();
    },
  };
}
