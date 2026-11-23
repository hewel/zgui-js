import { createSignal, onCleanup, omit, untrack } from "#solid";
import { createElement, effect, memo, mergeProps, spread } from "../renderer/index.js";
import { focus, getWindowRect } from "../nodes.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeKeyEvent,
  NativeNode,
  NativePointerDownEvent,
  NativePointerEvent,
  NativeValueEvent,
} from "../types.js";
import { callHandler, createControllable } from "../components/state.js";
import { slot, withDefaults } from "../components/shared.js";

/**
 * Reactive state passed to function children of {@link SliderPrimitive}.
 * Bind `trackRef` to the stationary travel track (defaults to the root `<control>` when unbound).
 */
export interface SliderPrimitiveRenderState {
  readonly value: Accessor<number>;
  readonly fraction: Accessor<number>;
  readonly dragging: Accessor<boolean>;
  readonly trackRef: (node: NativeNode) => void;
}

export type SliderPrimitiveChildren =
  | NativeChild
  | ((state: SliderPrimitiveRenderState) => NativeChild);

export interface SliderPrimitiveProps extends Omit<NativeElementProps, "children" | "orientation"> {
  readonly value?: number | undefined;
  readonly defaultValue?: number | undefined;
  readonly onValueChange?: ((value: number) => void) | undefined;
  readonly min?: number | undefined;
  readonly max?: number | undefined;
  readonly step?: number | undefined;
  /**
   * Optional snapping increment for pointer drags and direct numeric AT updates.
   * Defaults to `step`. Set to `1` or `0` to keep fine pointer dragging while
   * keyboard arrows still step by `step`.
   */
  readonly pointerStep?: number | undefined;
  readonly orientation?: "horizontal" | undefined;
  readonly label?: string | null | undefined;
  readonly onDraggingChange?: ((dragging: boolean) => void) | undefined;
  readonly children?: SliderPrimitiveChildren | undefined;
}

interface Bounds {
  readonly min: number;
  readonly max: number;
  readonly step: number;
  readonly pointerStep: number;
}

function finite(value: number, name: string): number {
  if (!Number.isFinite(value)) throw new TypeError(`Slider ${name} must be finite`);
  return value;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function boundsFor(min: number, max: number, step: number, pointerStep: number): Bounds {
  finite(min, "min");
  finite(max, "max");
  finite(step, "step");
  finite(pointerStep, "pointerStep");
  const span = max - min;
  if (
    max < min ||
    step < 0 ||
    pointerStep < 0 ||
    !Number.isFinite(span) ||
    (step > 0 && !Number.isFinite(span / step)) ||
    (pointerStep > 0 && !Number.isFinite(span / pointerStep))
  ) {
    throw new RangeError(
      "Slider requires ordered finite bounds and a non-negative representable step",
    );
  }
  return { min, max, step, pointerStep };
}

function snappedTo(value: number, min: number, max: number, step: number): number {
  finite(value, "value");
  const clamped = clamp(value, min, max);
  if (step === 0 || clamped === min || clamped === max) return clamped;
  return clamp(min + Math.round((clamped - min) / step) * step, min, max);
}

function normalizeDisplay(value: number, bound: Bounds): number {
  finite(value, "value");
  if (bound.pointerStep !== bound.step) {
    return clamp(value, bound.min, bound.max);
  }
  return snappedTo(value, bound.min, bound.max, bound.step);
}

/**
 * Unstyled headless horizontal slider (`role="slider"`) with pointer capture,
 * single-pointer isolation, keyboard navigation, and AccessKit `Change` handling.
 * Installs zero built-in stylesheets.
 */
export function SliderPrimitive(props: SliderPrimitiveProps = {}): NativeNode {
  const rest = omit(
    props,
    "value",
    "defaultValue",
    "onValueChange",
    "min",
    "max",
    "step",
    "pointerStep",
    "orientation",
    "label",
    "disabled",
    "onDraggingChange",
    "children",
    "onKeyDown",
    "onChange",
    "onPointerDown",
    "onPointerMove",
    "onPointerUp",
    "onPointerCancel",
  );
  const bound = memo(() => {
    const step = props.step ?? 1;
    const pointerStep = props.pointerStep ?? step;
    return boundsFor(props.min ?? 0, props.max ?? 100, step, pointerStep);
  });
  effect(
    () => props.orientation,
    orientation => {
      if (orientation !== undefined && orientation !== "horizontal") {
        throw new TypeError("Slider orientation must be horizontal");
      }
    },
  );
  const [read, write] = createControllable<number>(
    {
      value: () => props.value,
      defaultValue: () => normalizeDisplay(props.defaultValue ?? 0, bound()),
      onChange: () => props.onValueChange,
    },
    0,
  );
  const value = memo(() => normalizeDisplay(read(), bound()));
  const fraction = memo(() => {
    const b = bound();
    return b.max === b.min ? 0 : (value() - b.min) / (b.max - b.min);
  });
  const disabled = () => !!props.disabled;
  const root = createElement("control");

  let trackNode: NativeNode | undefined;
  const trackRef = (node: NativeNode) => {
    trackNode = node;
  };

  let captured: string | undefined;
  let isDragging = false;
  const [dragging, setDragging] = createSignal(false);
  const updateDragging = (next: boolean) => {
    if (isDragging === next) return;
    isDragging = next;
    setDragging(next);
    untrack(() => props.onDraggingChange?.(next));
  };

  effect(
    () => !!props.disabled,
    isDisabled => {
      if (isDisabled) updateDragging(false);
    },
  );
  onCleanup(() => {
    captured = undefined;
    trackNode = undefined;
    updateDragging(false);
  });

  const commit = (next: number, snapStep: number) => {
    const b = bound();
    const normalized = snappedTo(next, b.min, b.max, snapStep);
    if (!Object.is(normalized, value())) write(normalized);
  };

  const atPoint = (event: NativePointerEvent): number | undefined => {
    const rect = getWindowRect(trackNode ?? root);
    if (!rect || rect.width <= 0 || !Number.isFinite(event.position.x)) return;
    const at = clamp((event.position.x - rect.x) / rect.width, 0, 1);
    const b = bound();
    return snappedTo(b.min + at * (b.max - b.min), b.min, b.max, b.pointerStep);
  };

  const end = (event: NativePointerEvent) => {
    captured = undefined;
    updateDragging(false);
    event.releasePointer();
  };

  const renderState: SliderPrimitiveRenderState = {
    value,
    fraction,
    dragging,
    trackRef,
  };

  spread(
    root,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "slider" as const,
          tabIndex: 0,
          orientation: "horizontal" as const,
          get disabled() {
            return disabled();
          },
          get ariaLabel() {
            return props.label ?? null;
          },
          get ariaValueMin() {
            return bound().min;
          },
          get ariaValueMax() {
            return bound().max;
          },
          get ariaValueStep() {
            return bound().step;
          },
          get ariaValueNow() {
            return value();
          },
          get "data-dragging"() {
            return dragging() ? "true" : null;
          },
          get "data-disabled"() {
            return disabled() ? "true" : null;
          },
        },
        rest,
      ),
      {
        get onKeyDown() {
          return (event: NativeKeyEvent) => {
            if (
              !callHandler(event, props.onKeyDown) ||
              disabled() ||
              event.target !== root ||
              event.character != null ||
              event.isComposing
            )
              return;
            const b = bound();
            const step = b.step > 0 ? b.step : 1;
            let next: number;
            switch (event.key) {
              case "ArrowRight":
              case "ArrowUp":
                next = value() + step;
                break;
              case "ArrowLeft":
              case "ArrowDown":
                next = value() - step;
                break;
              case "PageUp":
                next = value() + step * 10;
                break;
              case "PageDown":
                next = value() - step * 10;
                break;
              case "Home":
                next = b.min;
                break;
              case "End":
                next = b.max;
                break;
              default:
                return;
            }
            commit(clamp(next, b.min, b.max), b.step);
            event.preventDefault();
            event.stopPropagation();
          };
        },
        get onChange() {
          return (event: NativeValueEvent) => {
            if (!callHandler(event, props.onChange) || disabled() || event.target !== root) return;
            if (typeof event.value !== "string") return;
            const trimmed = event.value.trim();
            if (trimmed.length === 0) return;
            const parsed = Number(trimmed);
            if (!Number.isFinite(parsed)) return;
            const b = bound();
            commit(parsed, b.pointerStep);
          };
        },
        get onPointerDown() {
          return (event: NativePointerDownEvent) => {
            if (!callHandler(event, props.onPointerDown) || disabled() || !event.primary) return;
            if (
              !(
                event.button === "primary" ||
                (event.button === null && event.pointerKind !== "mouse")
              )
            )
              return;
            if (isDragging && captured !== event.pointerId) return;
            const next = atPoint(event);
            if (next === undefined) return;
            captured = event.pointerId;
            updateDragging(true);
            event.capturePointer();
            focus(root);
            commit(next, bound().pointerStep);
          };
        },
        get onPointerMove() {
          return (event: NativePointerEvent) => {
            const allowed = callHandler(event, props.onPointerMove);
            if (event.pointerId !== captured) return;
            if (!isDragging || disabled()) {
              end(event);
              return;
            }
            if (!allowed) return;
            const next = atPoint(event);
            if (next !== undefined) commit(next, bound().pointerStep);
          };
        },
        get onPointerUp() {
          return (event: NativePointerEvent) => {
            const allowed = callHandler(event, props.onPointerUp);
            if (event.pointerId !== captured) return;
            if (!(event.button === "primary" || event.button === null)) return;
            if (allowed && isDragging && !disabled()) {
              const next = atPoint(event);
              if (next !== undefined) commit(next, bound().pointerStep);
            }
            end(event);
          };
        },
        get onPointerCancel() {
          return (event: NativePointerEvent) => {
            callHandler(event, props.onPointerCancel);
            if (event.pointerId === captured) end(event);
          };
        },
      },
    ),
  );

  if ("children" in props) {
    slot(root, () => {
      const child = props.children;
      return typeof child === "function" ? child(renderState) : child;
    });
  }

  return root;
}
