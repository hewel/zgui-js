import { createEffect, createSignal, getOwner, onCleanup, untrack } from "#solid";
import { isRootlessWithoutRegions, onRegionPresenceChange, tryBridgeFor } from "./host.js";
import type { UiBridge } from "./host.js";
import { getReducedMotion } from "./nodes.js";
import { flush } from "./renderer/index.js";
import type { Accessor, NativeNode } from "./types.js";

/** Numeric spring tuning option accepting either a static number or a reactive accessor. */
export type SpringNumberOption = number | Accessor<number>;

/** Configurable physics parameters; all options must resolve to positive finite numbers. */
export interface SpringPhysicsOptions {
  readonly stiffness?: SpringNumberOption | undefined;
  readonly damping?: SpringNumberOption | undefined;
  readonly mass?: SpringNumberOption | undefined;
  /** Maximum displacement at rest, in target units. */
  readonly precision?: SpringNumberOption | undefined;
  /** Maximum speed at rest, in target units per second. */
  readonly velocityPrecision?: SpringNumberOption | undefined;
}

/** Spring creation options including an optional mount-only starting position. */
export interface SpringOptions extends SpringPhysicsOptions {
  /** Mount-only starting position (read once untracked at construction; never reconfigured). */
  readonly initial?: number | undefined;
}

export type Spring = Accessor<number> & {
  /** Reactive settled state backed by its own boolean signal (never fires on every frame). */
  readonly settled: Accessor<boolean>;
  readonly jump: (value: number) => void;
  /** Add a velocity kick (in target units/second) without moving position. */
  readonly impulse: (velocityDelta: number) => void;
  /** Update spring physics options smoothly without losing position or velocity. */
  readonly configure: (next: SpringPhysicsOptions & { readonly initial?: never }) => void;
};

interface ResolvedSpringPhysics {
  stiffnessPerMass: number;
  dampingPerMass: number;
  precision: number;
  velocityPrecision: number;
}

interface SpringEntry {
  revision: number;
  resetHistory(): void;
  step(time: number): boolean;
}

class SpringTicker {
  private readonly entries = new Set<SpringEntry>();
  private frameId: number | undefined;
  private frameToken = 0;
  private dispatching = false;
  private readonly schedule: (cb: (time: number) => void) => number;
  private readonly isParked: () => boolean;

  constructor(schedule: (cb: (time: number) => void) => number, isParked: () => boolean) {
    this.schedule = schedule;
    this.isParked = isParked;
  }

  enroll(entry: SpringEntry): void {
    entry.revision += 1;
    this.entries.add(entry);
    this.arm();
  }

  unenroll(entry: SpringEntry): void {
    if (!this.entries.delete(entry)) return;
    entry.revision += 1;
    if (this.entries.size === 0) {
      this.cancelFrame();
    }
  }

  has(entry: SpringEntry): boolean {
    return this.entries.has(entry);
  }

  onLastRegionDetached(): void {
    this.cancelFrame();
    for (const entry of this.entries) {
      entry.resetHistory();
    }
  }

  onRegionAttached(): void {
    if (this.entries.size > 0 && !this.dispatching && !this.isParked()) {
      this.arm();
    }
  }

  private cancelFrame(): void {
    this.frameToken += 1;
    if (this.frameId !== undefined) {
      const id = this.frameId;
      this.frameId = undefined;
      cancelAnimationFrame(id);
    }
  }

  private arm(): void {
    if (this.dispatching || this.frameId !== undefined || this.entries.size === 0) return;
    if (this.isParked()) return;
    const token = ++this.frameToken;
    this.frameId = this.schedule(time => {
      this.onFrame(token, time);
    });
  }

  private onFrame(token: number, time: number): void {
    if (token !== this.frameToken) return;
    this.frameId = undefined;
    this.frameToken += 1;

    if (!Number.isFinite(time)) {
      if (this.entries.size > 0 && !this.isParked()) {
        this.arm();
      }
      return;
    }

    const snapshot: Array<{ entry: SpringEntry; revision: number }> = [];
    for (const entry of this.entries) {
      snapshot.push({ entry, revision: entry.revision });
    }

    this.dispatching = true;
    try {
      for (const item of snapshot) {
        if (!this.entries.has(item.entry) || item.entry.revision !== item.revision) {
          continue;
        }
        const stillActive = item.entry.step(time);
        if (!stillActive && item.entry.revision === item.revision) {
          this.entries.delete(item.entry);
          item.entry.revision += 1;
        }
      }
    } finally {
      this.dispatching = false;
      if (this.entries.size > 0 && !this.isParked()) {
        this.arm();
      }
    }
  }
}

const bridgeTickers = new WeakMap<UiBridge, SpringTicker>();
const globalTicker = new SpringTicker(cb => requestAnimationFrame(cb), isRootlessWithoutRegions);

onRegionPresenceChange(hasRegions => {
  if (hasRegions) {
    globalTicker.onRegionAttached();
  } else if (isRootlessWithoutRegions()) {
    globalTicker.onLastRegionDetached();
  }
});

function tickerFor(ownerBridge: UiBridge | undefined): SpringTicker {
  if (ownerBridge && typeof ownerBridge.requestAnimationFrame === "function") {
    let ticker = bridgeTickers.get(ownerBridge);
    if (!ticker) {
      const bridge = ownerBridge;
      ticker = new SpringTicker(
        cb => bridge.requestAnimationFrame!(cb),
        () => false,
      );
      bridgeTickers.set(ownerBridge, ticker);
    }
    return ticker;
  }
  return globalTicker;
}

/** Start an SDK frame step on the same per-region ticker as springs. Return false to stop. */
export function animateFrames(node: NativeNode, step: (time: number) => boolean): () => void {
  const ticker = tickerFor(tryBridgeFor(node));
  const entry = { revision: 0, step, resetHistory() {} };
  ticker.enroll(entry);
  return () => ticker.unenroll(entry);
}

function readOption(value: SpringNumberOption | undefined, fallback: number): number {
  return typeof value === "function" ? value() : (value ?? fallback);
}

function resolvePhysics(options: SpringPhysicsOptions): ResolvedSpringPhysics {
  const stiffness = readOption(options.stiffness, 170);
  const damping = readOption(options.damping, 26);
  const mass = readOption(options.mass, 1);
  const precision = readOption(options.precision, 0.001);
  const velocityPrecision = readOption(options.velocityPrecision, precision);
  const stiffnessPerMass = stiffness / mass;
  const dampingPerMass = damping / mass;
  if (
    ![stiffnessPerMass, dampingPerMass, mass, precision, velocityPrecision].every(
      n => Number.isFinite(n) && n > 0,
    )
  ) {
    throw new RangeError("Spring options must produce finite positive rates and thresholds");
  }
  return { stiffnessPerMass, dampingPerMass, precision, velocityPrecision };
}

function withControls(
  read: Accessor<number>,
  settled: Accessor<boolean>,
  reset: (value: number) => void,
  kick: (velocityDelta: number) => void,
  reconfigure: (next: SpringPhysicsOptions & { readonly initial?: never }) => void,
): Spring {
  Object.defineProperty(read, "settled", { value: settled });
  Object.defineProperty(read, "jump", {
    value(value: number) {
      if (!Number.isFinite(value)) throw new RangeError("Spring jump value must be finite");
      reset(value);
    },
  });
  Object.defineProperty(read, "impulse", {
    value(velocityDelta: number) {
      if (!Number.isFinite(velocityDelta)) {
        throw new RangeError("Spring impulse must be finite");
      }
      kick(velocityDelta);
    },
  });
  Object.defineProperty(read, "configure", {
    value(next: SpringPhysicsOptions & { readonly initial?: never }) {
      reconfigure(next);
    },
  });
  return read as Spring;
}

/** Follow a finite numeric target; retargeting preserves velocity and owner disposal cancels motion. */
export function createSpring(target: Accessor<number>, options: SpringOptions = {}): Spring {
  // Read mount-only initial position once, untracked, and validate synchronously even under reduced motion.
  const rawInitial = untrack(() => options.initial);
  if (rawInitial !== undefined && !Number.isFinite(rawInitial)) {
    throw new RangeError("Spring initial value must be finite");
  }

  const [overrideOptions, setOverrideOptions] = createSignal<SpringPhysicsOptions | undefined>(
    undefined,
  );
  const currentOptions = (): SpringPhysicsOptions => {
    const overrides = overrideOptions();
    if (!overrides) return options;
    return {
      get stiffness() {
        return overrides.stiffness !== undefined ? overrides.stiffness : options.stiffness;
      },
      get damping() {
        return overrides.damping !== undefined ? overrides.damping : options.damping;
      },
      get mass() {
        return overrides.mass !== undefined ? overrides.mass : options.mass;
      },
      get precision() {
        return overrides.precision !== undefined ? overrides.precision : options.precision;
      },
      get velocityPrecision() {
        return overrides.velocityPrecision !== undefined
          ? overrides.velocityPrecision
          : options.velocityPrecision;
      },
    };
  };

  let physics = untrack(() => resolvePhysics(currentOptions()));
  const readTarget = () => {
    const next = target();
    if (!Number.isFinite(next)) throw new RangeError("Spring target must be finite");
    return next;
  };

  const ticker = tickerFor(tryBridgeFor());

  let destination = untrack(readTarget);
  const initialReduced = untrack(getReducedMotion);
  const startFromInitial =
    !initialReduced && rawInitial !== undefined && rawInitial !== destination;
  let position = startFromInitial ? rawInitial : destination;
  let velocity = 0;
  let previousTime: number | undefined;
  let disposed = false;
  let refreshingTarget = false;
  let effectInitialized = false;

  const [value, setValue] = createSignal(position);
  let isSettled = !startFromInitial;
  const [settledSignal, setSettledSignal] = createSignal(isSettled);
  const updateSettled = (next: boolean) => {
    isSettled = next;
    setSettledSignal(next);
  };

  const atRest = () =>
    Math.abs(destination - position) <= physics.precision &&
    Math.abs(velocity) <= physics.velocityPrecision;

  const entry: SpringEntry = {
    revision: 0,
    resetHistory() {
      previousTime = undefined;
    },
    step(time: number): boolean {
      if (disposed) return false;
      if (untrack(getReducedMotion)) {
        settle();
        return false;
      }
      let dt: number;
      if (previousTime === undefined) {
        dt = 1 / 60;
        previousTime = time;
      } else if (time < previousTime) {
        dt = 0;
      } else {
        dt = Math.min((time - previousTime) / 1000, 0.032);
        previousTime = time;
      }
      // Implicit midpoint keeps the damped oscillator stable at irregular intervals.
      const half = dt / 2;
      const divisor = 1 + half * (physics.dampingPerMass + half * physics.stiffnessPerMass);
      const midpointVelocity =
        (velocity + half * physics.stiffnessPerMass * (destination - position)) / divisor;
      position += dt * midpointVelocity;
      velocity = 2 * midpointVelocity - velocity;
      if (
        !Number.isFinite(divisor) ||
        !Number.isFinite(position) ||
        !Number.isFinite(velocity) ||
        atRest()
      ) {
        settle();
        return false;
      }
      setValue(position);
      return true;
    },
  };

  const deactivate = () => {
    ticker.unenroll(entry);
    previousTime = undefined;
  };

  const activate = () => {
    updateSettled(false);
    if (!ticker.has(entry)) {
      previousTime = undefined;
      ticker.enroll(entry);
    }
  };

  const settle = () => {
    deactivate();
    velocity = 0;
    position = destination;
    setValue(position);
    updateSettled(true);
  };

  if (startFromInitial) {
    ticker.enroll(entry);
  }

  if (getOwner()) {
    onCleanup(() => {
      disposed = true;
      deactivate();
      velocity = 0;
      updateSettled(true);
    });
  }

  createEffect(
    () => resolvePhysics(currentOptions()),
    next => {
      physics = next;
      if (!disposed && !untrack(getReducedMotion) && !atRest()) {
        activate();
      }
    },
  );

  createEffect(
    () => [readTarget(), getReducedMotion()] as const,
    ([next, reduced]) => {
      const isFirstRun = !effectInitialized;
      effectInitialized = true;
      if (refreshingTarget || disposed) return;
      if (isFirstRun) {
        if (reduced) {
          destination = next;
          settle();
          return;
        }
        if (next === destination) return;
      }
      destination = next;
      if (reduced || atRest()) settle();
      else activate();
    },
  );

  const readPosition: Accessor<number> = () => {
    if (disposed) return position;
    if (getReducedMotion()) return readTarget();
    value();
    return position;
  };

  const readSettled: Accessor<boolean> = () => {
    if (disposed) return true;
    if (getReducedMotion()) return true;
    settledSignal();
    return isSettled;
  };

  const syncPendingTarget = (): number | undefined => {
    if (!refreshingTarget) {
      refreshingTarget = true;
      try {
        flush();
      } finally {
        refreshingTarget = false;
      }
    }
    if (disposed) return undefined;
    const nextDestination = untrack(readTarget);
    if (disposed) return undefined;
    return nextDestination;
  };

  return withControls(
    readPosition,
    readSettled,
    next => {
      if (disposed) return;
      const nextDestination = syncPendingTarget();
      if (nextDestination === undefined) return;
      deactivate();
      position = next;
      velocity = 0;
      destination = nextDestination;
      if (untrack(getReducedMotion) || atRest()) {
        settle();
      } else {
        setValue(position);
        activate();
      }
    },
    velocityDelta => {
      if (disposed || velocityDelta === 0) return;
      if (untrack(getReducedMotion)) return;
      if (!Number.isFinite(velocity + velocityDelta)) {
        throw new RangeError("Spring velocity must remain finite");
      }
      const nextDestination = syncPendingTarget();
      if (nextDestination === undefined) return;
      if (untrack(getReducedMotion)) {
        destination = nextDestination;
        settle();
        return;
      }
      const nextVelocity = velocity + velocityDelta;
      if (!Number.isFinite(nextVelocity)) {
        throw new RangeError("Spring velocity must remain finite");
      }
      destination = nextDestination;
      velocity = nextVelocity;
      if (nextVelocity === 0 && atRest()) {
        settle();
      } else {
        activate();
      }
    },
    next => {
      if (disposed) return;
      const prev = untrack(overrideOptions);
      const merged: SpringPhysicsOptions = {
        stiffness: "stiffness" in next ? next.stiffness : prev?.stiffness,
        damping: "damping" in next ? next.damping : prev?.damping,
        mass: "mass" in next ? next.mass : prev?.mass,
        precision: "precision" in next ? next.precision : prev?.precision,
        velocityPrecision:
          "velocityPrecision" in next ? next.velocityPrecision : prev?.velocityPrecision,
      };
      // Validate eagerly so invalid calls fail synchronously at the call site.
      const nextPhysics = untrack(() =>
        resolvePhysics({
          stiffness: merged.stiffness !== undefined ? merged.stiffness : options.stiffness,
          damping: merged.damping !== undefined ? merged.damping : options.damping,
          mass: merged.mass !== undefined ? merged.mass : options.mass,
          precision: merged.precision !== undefined ? merged.precision : options.precision,
          velocityPrecision:
            merged.velocityPrecision !== undefined
              ? merged.velocityPrecision
              : options.velocityPrecision,
        }),
      );
      physics = nextPhysics;
      setOverrideOptions(merged);
      if (!untrack(getReducedMotion) && !atRest()) {
        activate();
      }
    },
  );
}
