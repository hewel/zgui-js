import { createEffect, createSignal, getOwner, onCleanup, untrack } from "#solid";
import { tryBridgeFor } from "./host.js";
import { getReducedMotion } from "./nodes.js";
import { flush } from "./renderer/index.js";
import type { Accessor } from "./types.js";

/** Numeric spring tuning option accepting either a static number or a reactive accessor. */
export type SpringNumberOption = number | Accessor<number>;

/** All options must resolve to positive finite numbers. */
export interface SpringOptions {
  readonly stiffness?: SpringNumberOption;
  readonly damping?: SpringNumberOption;
  readonly mass?: SpringNumberOption;
  /** Maximum displacement at rest, in target units. */
  readonly precision?: SpringNumberOption;
  /** Maximum speed at rest, in target units per second. */
  readonly velocityPrecision?: SpringNumberOption;
}

export type Spring = Accessor<number> & {
  readonly jump: (value: number) => void;
  /** Update spring physics options smoothly without losing position or velocity. */
  readonly configure: (next: SpringOptions) => void;
};

interface ResolvedSpringPhysics {
  stiffnessPerMass: number;
  dampingPerMass: number;
  precision: number;
  velocityPrecision: number;
}

function readOption(value: SpringNumberOption | undefined, fallback: number): number {
  return typeof value === "function" ? value() : (value ?? fallback);
}

function resolvePhysics(options: SpringOptions): ResolvedSpringPhysics {
  const stiffness = readOption(options.stiffness, 170);
  const damping = readOption(options.damping, 26);
  const mass = readOption(options.mass, 1);
  const precision = readOption(options.precision, 0.001);
  const velocityPrecision = readOption(options.velocityPrecision, precision);
  const stiffnessPerMass = stiffness / mass;
  const dampingPerMass = damping / mass;
  if (![stiffnessPerMass, dampingPerMass, mass, precision, velocityPrecision].every(n => Number.isFinite(n) && n > 0)) {
    throw new RangeError("Spring options must produce finite positive rates and thresholds");
  }
  return { stiffnessPerMass, dampingPerMass, precision, velocityPrecision };
}

function withControls(
  read: Accessor<number>,
  reset: (value: number) => void,
  reconfigure: (next: SpringOptions) => void,
): Spring {
  Object.defineProperty(read, "jump", {
    value(value: number) {
      if (!Number.isFinite(value)) throw new RangeError("Spring jump value must be finite");
      reset(value);
    },
  });
  Object.defineProperty(read, "configure", {
    value(next: SpringOptions) {
      reconfigure(next);
    },
  });
  return read as Spring;
}

/** Follow a finite numeric target; retargeting preserves velocity and owner disposal cancels motion. */
export function createSpring(target: Accessor<number>, options: SpringOptions = {}): Spring {
  const [overrideOptions, setOverrideOptions] = createSignal<SpringOptions | undefined>(undefined);
  const currentOptions = (): SpringOptions => {
    const overrides = overrideOptions();
    if (!overrides) return options;
    return {
      get stiffness() { return overrides.stiffness !== undefined ? overrides.stiffness : options.stiffness; },
      get damping() { return overrides.damping !== undefined ? overrides.damping : options.damping; },
      get mass() { return overrides.mass !== undefined ? overrides.mass : options.mass; },
      get precision() { return overrides.precision !== undefined ? overrides.precision : options.precision; },
      get velocityPrecision() {
        return overrides.velocityPrecision !== undefined ? overrides.velocityPrecision : options.velocityPrecision;
      },
    };
  };
  let physics = untrack(() => resolvePhysics(currentOptions()));
  const readTarget = () => {
    const next = target();
    if (!Number.isFinite(next)) throw new RangeError("Spring target must be finite");
    return next;
  };
  const ownerBridge = tryBridgeFor();
  const scheduleFrame = (cb: (time: number) => void): number => {
    const bridge = ownerBridge ?? tryBridgeFor();
    return bridge?.requestAnimationFrame ? bridge.requestAnimationFrame(cb) : requestAnimationFrame(cb);
  };

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
  const atRest = () => Math.abs(destination - position) <= physics.precision && Math.abs(velocity) <= physics.velocityPrecision;
  const settle = () => {
    cancelFrame();
    velocity = 0;
    position = destination;
    setValue(position);
  };
  const step = (time: number) => {
    frame = undefined;
    if (disposed) return;
    if (untrack(getReducedMotion)) {
      settle();
      return;
    }
    const dt = previousTime === undefined ? 1 / 60 : Math.min((time - previousTime) / 1000, 0.032);
    previousTime = time;
    // Implicit midpoint keeps the damped oscillator stable at irregular intervals.
    const half = dt / 2;
    const divisor = 1 + half * (physics.dampingPerMass + half * physics.stiffnessPerMass);
    const midpointVelocity = (velocity + half * physics.stiffnessPerMass * (destination - position)) / divisor;
    position += dt * midpointVelocity;
    velocity = 2 * midpointVelocity - velocity;
    if (!Number.isFinite(divisor) || !Number.isFinite(position) || !Number.isFinite(velocity) || atRest()) settle();
    else {
      frame = scheduleFrame(step);
      setValue(position);
    }
  };
  if (getOwner()) onCleanup(() => {
    disposed = true;
    cancelFrame();
  });
  createEffect(() => resolvePhysics(currentOptions()), next => {
    physics = next;
    if (!disposed && !untrack(getReducedMotion) && !atRest()) {
      frame ??= scheduleFrame(step);
    }
  });
  createEffect(() => [readTarget(), getReducedMotion()] as const, ([next, reduced]) => {
    if (refreshingTarget) return;
    destination = next;
    if (reduced || atRest()) settle();
    else frame ??= scheduleFrame(step);
  });
  return withControls(() => {
    if (getReducedMotion()) return readTarget();
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
    if (untrack(getReducedMotion) || atRest()) settle();
    else {
      setValue(position);
      frame = scheduleFrame(step);
    }
  }, next => {
    if (disposed) return;
    const merged: SpringOptions = { ...untrack(overrideOptions), ...next };
    // Validate eagerly so invalid calls fail synchronously at the call site.
    const nextPhysics = untrack(() => resolvePhysics({
      stiffness: merged.stiffness !== undefined ? merged.stiffness : options.stiffness,
      damping: merged.damping !== undefined ? merged.damping : options.damping,
      mass: merged.mass !== undefined ? merged.mass : options.mass,
      precision: merged.precision !== undefined ? merged.precision : options.precision,
      velocityPrecision: merged.velocityPrecision !== undefined ? merged.velocityPrecision : options.velocityPrecision,
    }));
    physics = nextPhysics;
    setOverrideOptions(merged);
    if (!untrack(getReducedMotion) && !atRest()) {
      frame ??= scheduleFrame(step);
    }
  });
}
