import { onCleanup, omit } from "#solid";
import { createElement, effect, memo, mergeProps, spread } from "../renderer/index.js";
import { focus, getWindowRect } from "../nodes.js";
import type { NativeElementProps, NativeKeyEvent, NativeNode, NativePointerDownEvent, NativePointerEvent } from "../types.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, element, forbidChildren, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/SliderStyle.js";

export interface SliderProps extends Omit<NativeElementProps, "children" | "orientation"> {
  children?: never;
  value?: number;
  defaultValue?: number;
  onValueChange?: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
  orientation?: "horizontal";
  label?: string | null;
}

interface Bounds { min: number; max: number; step: number; }
function finite(value: number, name: string): number {
  if (!Number.isFinite(value)) throw new TypeError(`Slider ${name} must be finite`);
  return value;
}
function boundsFor(min: number, max: number, step: number): Bounds {
  finite(min, "min"); finite(max, "max"); finite(step, "step");
  const span = max - min;
  if (max < min || step < 0 || !Number.isFinite(span) || (step > 0 && !Number.isFinite(span / step))) {
    throw new RangeError("Slider requires ordered finite bounds and a non-negative representable step");
  }
  return { min, max, step };
}
function snapped(value: number, bound: Bounds): number {
  finite(value, "value");
  const clamped = Math.max(bound.min, Math.min(bound.max, value));
  if (bound.step === 0 || clamped === bound.min || clamped === bound.max) return clamped;
  return Math.max(bound.min, Math.min(bound.max,
    bound.min + Math.round((clamped - bound.min) / bound.step) * bound.step));
}

export function Slider(props: SliderProps = {}): NativeNode {
  useStylesheet(sheet);
  forbidChildren("Slider", props);
  const rest = omit(props, "value", "defaultValue", "onValueChange", "min", "max", "step", "orientation", "label", "disabled", "onKeyDown", "onPointerDown", "onPointerMove", "onPointerUp", "onPointerCancel");
  const bound = memo(() => boundsFor(props.min ?? 0, props.max ?? 100, props.step ?? 1));
  effect(() => props.orientation, orientation => {
    if (orientation !== undefined && orientation !== "horizontal") {
      throw new TypeError("Slider orientation must be horizontal");
    }
  });
  const [read, write] = createControllable<number>({
    value: () => props.value,
    defaultValue: () => snapped(props.defaultValue ?? 0, bound()),
    onChange: () => props.onValueChange,
  }, 0);
  const value = memo(() => snapped(read(), bound()));
  const fraction = memo(() => {
    const b = bound();
    return b.max === b.min ? 0 : (value() - b.min) / (b.max - b.min);
  });
  const disabled = () => !!props.disabled;
  const root = createElement("control");
  const track = element("box", { class: "zui-slider__track", ariaHidden: true });
  append(track, element("box", { class: "zui-slider__range", ariaHidden: true }));
  const thumb = element("box", { class: "zui-slider__thumb", ariaHidden: true });

  let captured: string | undefined;
  let dragging = false;
  effect(() => !!props.disabled, isDisabled => {
    if (isDisabled) dragging = false;
  });
  onCleanup(() => { captured = undefined; dragging = false; });
  const change = (next: number) => {
    const normalized = snapped(next, bound());
    if (!Object.is(normalized, value())) write(normalized);
  };
  const atPoint = (event: NativePointerEvent): number | undefined => {
    const rect = getWindowRect(track);
    if (!rect || rect.width <= 0 || !Number.isFinite(event.position.x)) return;
    const at = Math.max(0, Math.min(1, (event.position.x - rect.x) / rect.width));
    const b = bound();
    return snapped(b.min + at * (b.max - b.min), b);
  };
  const end = (event: NativePointerEvent) => {
    captured = undefined;
    dragging = false;
    event.releasePointer();
  };

  spread(root, mergeProps(
    rest,
    withDefaults({
      role: "slider" as const,
      tabIndex: 0,
      orientation: "horizontal" as const,
      get disabled() { return disabled(); },
      get ariaLabel() { return props.label ?? null; },
      get ariaValueMin() { return bound().min; },
      get ariaValueMax() { return bound().max; },
      get ariaValueStep() { return bound().step; },
      get ariaValueNow() { return value(); },
      get __zguiStyle() { return { "--zui-slider-fraction": `${(fraction() * 100).toFixed(4)}%` }; },
    }, rest),
    {
      get __zguiClass() { return classes(sheet.className, "zui-slider"); },
      get onKeyDown() {
        return (event: NativeKeyEvent) => {
          if (!callHandler(event, props.onKeyDown) || disabled() || event.target !== root || event.character != null) return;
          const b = bound();
          const step = b.step > 0 ? b.step : 1;
          let next: number;
          switch (event.key) {
            case "ArrowRight": case "ArrowUp": next = value() + step; break;
            case "ArrowLeft": case "ArrowDown": next = value() - step; break;
            case "PageUp": next = value() + step * 10; break;
            case "PageDown": next = value() - step * 10; break;
            case "Home": next = b.min; break;
            case "End": next = b.max; break;
            default: return;
          }
          change(Math.max(b.min, Math.min(b.max, next)));
          event.preventDefault();
          event.stopPropagation();
        };
      },
      get onPointerDown() {
        return (event: NativePointerDownEvent) => {
          if (!callHandler(event, props.onPointerDown) || disabled() || !event.primary) return;
          if (!(event.button === "primary" || (event.button === null && event.pointerKind !== "mouse"))) return;
          if (dragging && captured !== event.pointerId) return;
          const next = atPoint(event);
          if (next === undefined) return;
          captured = event.pointerId;
          dragging = true;
          event.capturePointer();
          focus(root);
          change(next);
        };
      },
      get onPointerMove() {
        return (event: NativePointerEvent) => {
          const allowed = callHandler(event, props.onPointerMove);
          if (event.pointerId !== captured) return;
          if (!dragging || disabled()) { end(event); return; }
          if (!allowed) return;
          const next = atPoint(event);
          if (next !== undefined) change(next);
        };
      },
      get onPointerUp() {
        return (event: NativePointerEvent) => {
          const allowed = callHandler(event, props.onPointerUp);
          if (event.pointerId !== captured) return;
          if (!(event.button === "primary" || event.button === null)) return;
          if (allowed && dragging && !disabled()) {
            const next = atPoint(event);
            if (next !== undefined) change(next);
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
  ));
  append(root, track);
  append(root, thumb);
  return root;
}
