import {
  createContext,
  createEffect,
  createSignal,
  onCleanup,
  omit,
  untrack,
  useContext,
} from "#solid";
import { mergeProps } from "../renderer/index.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeFocusEvent,
  NativeKeyEvent,
  NativeNode,
  NativePointerBoundaryEvent,
  NativeRole,
} from "../types.js";
import { anchoredSurface, OverlayContext, OverlayRoot } from "./overlay.js";
import type { AnchoredContentProps, OverlayRootProps, OverlayState } from "./overlay.js";
import { createDelayed } from "./delay.js";
import { callHandler } from "./state.js";
import {
  classes,
  element,
  provide,
  type Stylesheet,
  useStylesheet,
  withDefaults,
} from "./shared.js";

export interface HoverProps extends OverlayRootProps {
  delay?: number;
  closeDelay?: number;
}
interface Showing {
  state: OverlayState;
  setInstant(value: boolean): void;
  closeNow(): void;
}

export interface HoverHandoff {
  anotherShowing(me: Showing): boolean;
  takeOver(next: Showing): void;
  clear(me: Showing): void;
}

export function createHandoff(): HoverHandoff {
  let current: Showing | undefined;
  return {
    anotherShowing(me: Showing) {
      return !!current && current !== me && untrack(current.state.open);
    },
    takeOver(next: Showing) {
      const previous = current;
      current = next;
      if (previous && previous !== next) {
        previous.setInstant(true);
        previous.closeNow();
      }
    },
    clear(me: Showing) {
      if (current === me) current = undefined;
    },
  };
}

function createHoverIntent(
  state: OverlayState,
  delay: Accessor<number>,
  closeDelay: Accessor<number>,
  handoff?: HoverHandoff,
) {
  const [instant, setInstant] = createSignal(false);
  const opening = createDelayed(),
    closing = createDelayed();
  let alive = true;
  const cancel = () => {
    opening.cancel();
    closing.cancel();
  };
  const closeNow = () => {
    cancel();
    if (alive) state.setOpen(false);
  };
  const me = { state, setInstant, closeNow };
  const show = () => {
    if (!alive) return;
    setInstant(false);
    handoff?.takeOver(me);
    if (alive) state.setOpen(true);
  };
  createEffect(
    () => state.open(),
    open => {
      if (!open) cancel();
    },
  );
  onCleanup(() => {
    alive = false;
    cancel();
    handoff?.clear(me);
  });
  return {
    state,
    instant,
    closeNow,
    openNow() {
      cancel();
      show();
    },
    enter() {
      if (alive) {
        closing.cancel();
        opening.after(handoff?.anotherShowing(me) ? 0 : delay(), show);
      }
    },
    leave() {
      if (alive) {
        opening.cancel();
        closing.after(closeDelay(), () => state.setOpen(false));
      }
    },
  };
}

export type HoverIntent = ReturnType<typeof createHoverIntent>;
export const HoverContext = createContext<HoverIntent | null>(null);

export function HoverRoot(
  props: HoverProps,
  defaults: {
    delay: Accessor<number>;
    closeDelay: Accessor<number>;
    handoff?: HoverHandoff | undefined;
  },
): NativeChild {
  return OverlayRoot(
    mergeProps(props, {
      get children() {
        const state = useContext(OverlayContext)!;
        const intent = createHoverIntent(
          state,
          () => props.delay ?? defaults.delay(),
          () => props.closeDelay ?? defaults.closeDelay(),
          defaults.handoff,
        );
        return provide(HoverContext, intent, props);
      },
    }),
  );
}

export function hoverTrigger(
  config: {
    sheet: Stylesheet;
    class: string;
    focus: "always" | "keyboard";
    attrs: (state: OverlayState | undefined) => NativeElementProps;
  },
  props: NativeElementProps = {},
): NativeNode {
  useStylesheet(config.sheet);
  const intent = useContext(HoverContext),
    state = intent?.state;
  const rest = omit(props, "onPointerEnter", "onPointerLeave", "onFocus", "onBlur", "onKeyDown");
  const node = element(
    "box",
    mergeProps(
      rest,
      withDefaults(config.attrs(state), rest),
      withDefaults(
        {
          get "data-state"() {
            return state?.open() ? "open" : "closed";
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(config.class);
        },
        onPointerEnter(event: NativePointerBoundaryEvent) {
          if (callHandler(event, props.onPointerEnter)) intent?.enter();
        },
        onPointerLeave(event: NativePointerBoundaryEvent) {
          if (callHandler(event, props.onPointerLeave)) intent?.leave();
        },
        onFocus(event: NativeFocusEvent) {
          if (
            callHandler(event, props.onFocus) &&
            (config.focus === "always" || event.cause === "keyboard")
          )
            intent?.openNow();
        },
        onBlur(event: NativeFocusEvent) {
          if (callHandler(event, props.onBlur)) intent?.closeNow();
        },
        onKeyDown(event: NativeKeyEvent) {
          if (
            callHandler(event, props.onKeyDown) &&
            event.character == null &&
            event.key === "Escape" &&
            state?.open()
          ) {
            event.stopPropagation();
            intent?.closeNow();
          }
        },
      },
    ),
  );
  state?.trigger.ref(node);
  return node;
}

export function hoverSurface(
  config: {
    sheet: Stylesheet;
    class: string;
    role: NativeRole;
    attrs?: (intent: HoverIntent | null | undefined) => NativeElementProps;
  },
  props: AnchoredContentProps = {},
): NativeChild {
  useStylesheet(config.sheet);
  const intent = useContext(HoverContext);
  const rest = omit(props, "onPointerEnter", "onPointerLeave");
  return anchoredSurface(
    intent?.state,
    mergeProps(
      rest,
      withDefaults({ dismissOnOutsidePress: false }, rest),
      withDefaults(config.attrs?.(intent) ?? {}, rest),
      {
        onPointerEnter(event: NativePointerBoundaryEvent) {
          if (callHandler(event, props.onPointerEnter)) intent?.enter();
        },
        onPointerLeave(event: NativePointerBoundaryEvent) {
          if (callHandler(event, props.onPointerLeave)) intent?.leave();
        },
      },
    ),
    {
      class: `${config.sheet.className} ${config.class}`,
      role: config.role,
      onDismiss: () => intent?.closeNow(),
    },
  );
}
