import { createContext, createEffect, createSignal, onCleanup, untrack, useContext } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { Accessor, NativeChild } from "../types.js";
import { OverlayContext, OverlayRoot } from "./overlay.js";
import type { OverlayRootProps, OverlayState } from "./overlay.js";
import { createDelayed } from "./delay.js";
export interface HoverProps extends OverlayRootProps { delay?: number; closeDelay?: number; }
interface Showing { state: OverlayState; setInstant(value: boolean): void; closeNow(): void; }
export function createHandoff() {
  let current: Showing | undefined;
  return {
    anotherShowing(me: Showing) { return !!current && current !== me && untrack(current.state.open); },
    takeOver(next: Showing) {
      const previous = current;
      current = next;
      if (previous && previous !== next) { previous.setInstant(true); previous.closeNow(); }
    },
    clear(me: Showing) { if (current === me) current = undefined; },
  };
}
function createHoverIntent(state: OverlayState, delay: Accessor<number>, closeDelay: Accessor<number>, handoff?: ReturnType<typeof createHandoff>) {
  const [instant, setInstant] = createSignal(false);
  const opening = createDelayed(), closing = createDelayed();
  let alive = true;
  const cancel = () => { opening.cancel(); closing.cancel(); };
  const closeNow = () => { cancel(); if (alive) state.setOpen(false); };
  const me = { state, setInstant, closeNow };
  const show = () => {
    if (!alive) return;
    setInstant(false);
    handoff?.takeOver(me);
    if (alive) state.setOpen(true);
  };
  createEffect(() => { if (!state.open()) cancel(); });
  onCleanup(() => { alive = false; cancel(); handoff?.clear(me); });
  return { state, instant, closeNow,
    openNow() { cancel(); show(); },
    enter() { if (alive) { closing.cancel(); opening.after(handoff?.anotherShowing(me) ? 0 : delay(), show); } },
    leave() { if (alive) { opening.cancel(); closing.after(closeDelay(), () => state.setOpen(false)); } },
  };
}
export const HoverContext = createContext<ReturnType<typeof createHoverIntent>>();
export function HoverRoot(props: HoverProps, defaults: {
  delay: Accessor<number>; closeDelay: Accessor<number>; handoff?: ReturnType<typeof createHandoff>;
}): NativeChild {
  return OverlayRoot(mergeProps(props, { get children() {
    const state = useContext(OverlayContext)!;
    const intent = createHoverIntent(state, () => props.delay ?? defaults.delay(), () => props.closeDelay ?? defaults.closeDelay(), defaults.handoff);
    return HoverContext.Provider({ value: intent, get children() { return props.children; } });
  } }));
}
