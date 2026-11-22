import { createSignal, untrack } from "#solid";
import type { Accessor } from "../types.js";
import type { NativeEvent } from "../events.js";

export function createControllable<T>(
  options: {
    value: Accessor<T | undefined>;
    defaultValue: Accessor<T | undefined>;
    onChange: Accessor<((value: T) => void) | undefined>;
  },
  fallback: T,
): readonly [Accessor<T>, (value: T) => void] {
  const initial = untrack(options.defaultValue);
  const [inner, setInner] = createSignal(
    (initial === undefined ? fallback : initial) as Exclude<T, Function>,
  );
  const read = (): T => {
    const value = options.value();
    return value === undefined ? (inner() as T) : value;
  };
  const write = (next: T) => {
    const controlled = untrack(options.value);
    const previous = controlled === undefined ? untrack(inner) : controlled;
    if (Object.is(previous, next)) return;
    if (controlled === undefined) setInner(() => next as Exclude<T, Function>);
    untrack(() => options.onChange()?.(next));
  };
  return [read, write];
}

export function callHandler<E extends NativeEvent>(
  event: E,
  handler: ((event: E) => void) | null | undefined,
): boolean {
  if (!handler) return true;
  let cancelled = false;
  handler({
    ...event,
    preventDefault() {
      event.preventDefault();
      cancelled = true;
    },
  });
  return !cancelled;
}
