import { createEffect, createSignal, getOwner, onCleanup, untrack } from "#solid";
import { getReducedMotion } from "./nodes.js";
import { flush } from "./renderer/index.js";
import type { Accessor } from "./types.js";

/** All options must be positive finite numbers. */
export interface SpringOptions {
  readonly stiffness?: number;
  readonly damping?: number;
  readonly mass?: number;
  /** Maximum displacement at rest, in target units. */
  readonly precision?: number;
  /** Maximum speed at rest, in target units per second. */
  readonly velocityPrecision?: number;
}

export type Spring = Accessor<number> & { readonly jump: (value: number) => void };

function withJump(read: Accessor<number>, reset?: (value: number) => void): Spring {
  return Object.defineProperty(read, "jump", {
    value(value: number) {
      if (!Number.isFinite(value)) throw new RangeError("Spring jump value must be finite");
      reset?.(value);
    },
  }) as Spring;
}

/** Follow a finite numeric target; retargeting preserves velocity and owner disposal cancels motion. */
export function createSpring(target: Accessor<number>, {
  stiffness = 170, damping = 26, mass = 1, precision = 0.001, velocityPrecision = precision,
}: SpringOptions = {}): Spring {
  const stiffnessPerMass = stiffness / mass;
  const dampingPerMass = damping / mass;
  if (![stiffnessPerMass, dampingPerMass, mass, precision, velocityPrecision].every(n => Number.isFinite(n) && n > 0)) {
    throw new RangeError("Spring options must produce finite positive rates and thresholds");
  }
  const readTarget = () => {
    const next = target();
    if (!Number.isFinite(next)) throw new RangeError("Spring target must be finite");
    return next;
  };
  if (getReducedMotion()) return withJump(readTarget);

  let destination = untrack(readTarget);
  let position = destination;
  let velocity = 0;
  let frame: number | undefined;
  let previousTime: number | undefined;
  let disposed = false;
  let refreshingTarget = false;
  const [value, setValue] = createSignal(position);

  const cancelFrame = () => {
    if (frame !== undefined) cancelAnimationFrame(frame);
    frame = previousTime = undefined;
  };
  const atRest = () => Math.abs(destination - position) <= precision && Math.abs(velocity) <= velocityPrecision;
  const settle = () => {
    cancelFrame();
    velocity = 0;
    position = destination;
    setValue(position);
  };
  const step = (time: number) => {
    frame = undefined;
    if (disposed) return;
    const dt = previousTime === undefined ? 1 / 60 : Math.min((time - previousTime) / 1000, 0.032);
    previousTime = time;
    // Implicit midpoint keeps the damped oscillator stable at irregular intervals.
    const half = dt / 2;
    const divisor = 1 + half * (dampingPerMass + half * stiffnessPerMass);
    const midpointVelocity = (velocity + half * stiffnessPerMass * (destination - position)) / divisor;
    position += dt * midpointVelocity;
    velocity = 2 * midpointVelocity - velocity;
    if (!Number.isFinite(divisor) || !Number.isFinite(position) || !Number.isFinite(velocity) || atRest()) settle();
    else {
      frame = requestAnimationFrame(step);
      setValue(position);
    }
  };
  if (getOwner()) onCleanup(() => {
    disposed = true;
    cancelFrame();
  });
  createEffect(readTarget, next => {
    if (refreshingTarget) return;
    destination = next;
    if (atRest()) settle();
    else frame ??= requestAnimationFrame(step);
  });
  return withJump(() => {
    value();
    return position;
  }, next => {
    if (disposed) return;
    // Solid stages same-turn target writes until a flush.
    if (!refreshingTarget) {
      refreshingTarget = true;
      try { flush(); } finally { refreshingTarget = false; }
    }
    if (disposed) return;
    const nextDestination = untrack(readTarget);
    if (disposed) return;
    cancelFrame();
    position = next;
    velocity = 0;
    destination = nextDestination;
    if (atRest()) settle();
    else {
      setValue(position);
      frame = requestAnimationFrame(step);
    }
  });
}
