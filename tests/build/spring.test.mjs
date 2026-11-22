import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { createSignal, createSpring, renderApp } from "@hewel/zgui";
import type { NativeNode, Spring } from "@hewel/zgui";

const [target, setTarget] = createSignal(0);
export { target, setTarget };
export let value: Spring;

export function mount(root: NativeNode, renderScale: boolean) {
  return renderApp(() => {
    value = createSpring(target, { stiffness: 170, damping: 18 });
    return <box style={renderScale ? { scale: 1 + value() / 100 } : undefined} />;
  }, root);
}
`;

async function mounted(t, { reducedMotion = false, renderScale = true } = {}) {
  const h = createHost({ reducedMotion, strictStyles: true });
  const mod = await moduleFor(t, fixture);
  let dispose;
  globalThis.__zgui = h.host;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  const clock = createClock(t, h);
  const root = h.makeNode("root");
  dispose = mod.mount(root, renderScale);
  return {
    clock,
    value: mod.value,
    target: mod.target,
    runTurn: h.runTurn,
    scale: () => root.children[0].props.get("style").scale,
    setTarget: next => h.runTurn(() => mod.setTarget(next)),
    tick(ms = 16) {
      clock.advance(ms);
      clock.frame();
    },
    dispose: () => h.runTurn(dispose),
  };
}

test("spring carries momentum through a reversal and releases its last frame on convergence", async t => {
  const { clock, value, scale, setTarget, tick } = await mounted(t);
  assert.equal(clock.frames.size, 0, "initial target is already settled");
  assert.equal(scale(), "1");
  setTarget(100);
  assert.equal(clock.frames.size, 1);
  setTarget(0);
  assert.equal(clock.frames.size, 0, "settling before the first frame cancels it");
  setTarget(100);
  for (let i = 0; i < 5; i += 1) tick();
  const before = value();
  assert.ok(before > 10 && before < 90, `expected motion in progress: ${before}`);
  setTarget(0);
  assert.equal(clock.frames.size, 1, "retarget reuses the pending frame");
  tick();
  assert.ok(value() > before, "momentum still moves briefly away from the new target");
  const moving = value();
  setTarget(moving);
  tick();
  assert.ok(value() > moving, "zero displacement does not settle a moving spring");
  setTarget(0);
  tick(10_000);
  assert.ok(Math.abs(value()) < 100, "a frame stall must not destabilize the spring");
  for (let i = 0; i < 240 && clock.frames.size; i += 1) tick();
  assert.equal(value(), 0);
  assert.equal(scale(), "1");
  assert.equal(clock.frames.size, 0, "no callbacks survive convergence");
  setTarget(25);
  for (let i = 0; i < 240 && clock.frames.size; i += 1) tick();
  assert.equal(scale(), "1.25", "an idle spring wakes for a new target");
  assert.equal(clock.frames.size, 0);
});

test("spring snaps to extreme finite targets without leaving a pending frame", async t => {
  const { clock, value, setTarget, tick } = await mounted(t, { renderScale: false });
  for (const target of [1.5e308, Number.MAX_VALUE, -Number.MAX_VALUE]) {
    setTarget(target);
    for (let i = 0; i < 240 && clock.frames.size; i += 1) {
      tick();
      assert.ok(Number.isFinite(value()), `non-finite spring value targeting ${target}`);
    }
    assert.equal(clock.frames.size, 0, `frame still pending targeting ${target}`);
    assert.equal(value(), target);
  }
});

test("jump restarts motion toward an unchanged target with fresh velocity and frame timing", async t => {
  const { clock, value, scale, setTarget, runTurn, tick } = await mounted(t);
  setTarget(1);
  for (let i = 0; i < 240 && clock.frames.size; i += 1) tick();
  assert.equal(value(), 1);
  assert.equal(clock.frames.size, 0);

  runTurn(() => {
    value.jump(0);
    assert.equal(value(), 0, "jump is visible before the turn ends");
  });
  assert.equal(scale(), "1");
  assert.equal(clock.frames.size, 1);
  tick();
  const firstStep = value();
  assert.ok(firstStep > 0 && firstStep < 1);
  tick();
  assert.ok(value() > firstStep && value() < 1);

  const [oldFrame] = clock.frames.keys();
  runTurn(() => {
    value.jump(0);
    assert.equal(value(), 0);
  });
  assert.equal(clock.frames.size, 1);
  assert.equal(clock.frames.has(oldFrame), false);
  tick(100);
  assert.equal(
    value(),
    firstStep,
    "neither momentum nor the prior frame timestamp survives a jump",
  );
  for (let i = 0; i < 240 && clock.frames.size; i += 1) tick();
  assert.equal(value(), 1);
  assert.equal(scale(), "1.01");
  assert.equal(clock.frames.size, 0);
});

test("jump to the destination snaps and cancels the pending frame", async t => {
  const { clock, value, setTarget, runTurn, tick } = await mounted(t);
  setTarget(1);
  tick();
  assert.ok(value() > 0 && value() < 1);
  assert.equal(clock.frames.size, 1);
  runTurn(() => {
    value.jump(1);
    assert.equal(value(), 1);
    assert.equal(clock.frames.size, 0);
  });
  tick();
  assert.equal(value(), 1);
  assert.equal(clock.frames.size, 0);
});

test("jump uses a target written in the same turn before effects flush", async t => {
  const { clock, value, setTarget, runTurn, tick } = await mounted(t);
  runTurn(() => {
    setTarget(1);
    assert.equal(clock.frames.size, 0, "target effect has not run yet");
    value.jump(0);
    assert.equal(value(), 0);
    assert.equal(clock.frames.size, 1, "jump already sees the new target");
  });
  tick();
  assert.ok(value() > 0 && value() < 1);
  for (let i = 0; i < 240 && clock.frames.size; i += 1) tick();
  assert.equal(value(), 1);
  assert.equal(clock.frames.size, 0);
});

function assertRejectsNonFiniteJump(value) {
  for (const nonFinite of [NaN, Infinity, -Infinity]) {
    assert.throws(() => value.jump(nonFinite), {
      name: "RangeError",
      message: "Spring jump value must be finite",
    });
  }
}

test("invalid jumps leave a moving spring and its pending frame alone", async t => {
  const { clock, value, setTarget, tick } = await mounted(t);
  setTarget(1);
  tick();
  const before = value();
  const frames = [...clock.frames.keys()];
  assertRejectsNonFiniteJump(value);
  assert.equal(value(), before);
  assert.deepEqual([...clock.frames.keys()], frames);
});

test("spring exposes a fixed jump method without modifying the target accessor", async t => {
  const { value, target } = await mounted(t, { reducedMotion: true });
  assert.notEqual(value, target);
  assert.equal(Object.hasOwn(target, "jump"), false);
  const descriptor = Object.getOwnPropertyDescriptor(value, "jump");
  assert.equal(typeof descriptor?.value, "function");
  assert.equal(descriptor.writable, false);
  assert.equal(descriptor.configurable, false);
});

test("reduced motion validates jumps but keeps following the target without requesting a frame", async t => {
  const { clock, value, scale, setTarget, runTurn } = await mounted(t, { reducedMotion: true });
  setTarget(50);
  assertRejectsNonFiniteJump(value);
  runTurn(() => {
    value.jump(0);
    assert.equal(value(), 50);
  });
  assert.equal(scale(), "1.5");
  setTarget(25);
  assert.equal(value(), 25);
  assert.equal(clock.frames.size, 0);
});

test("disposing the owner cancels the pending frame and detaches the target", async t => {
  const { clock, value, setTarget, runTurn, tick, dispose } = await mounted(t);
  setTarget(100);
  tick();
  tick();
  assert.equal(clock.frames.size, 1);
  const before = value();
  dispose();
  assert.equal(clock.frames.size, 0);
  assertRejectsNonFiniteJump(value);
  runTurn(() => {
    value.jump(0);
    assert.equal(value(), before);
    assert.equal(clock.frames.size, 0);
  });
  setTarget(50);
  tick();
  assert.equal(clock.frames.size, 0);
  assert.equal(value(), before);
});

test("jump preserves in-flight motion when reading the target fails or disposes the owner", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(
    t,
    `
    import { createSignal, createSpring, renderApp } from "@hewel/zgui";
    import type { NativeNode, Spring } from "@hewel/zgui";

    const [target, setTarget] = createSignal(0);
    export { setTarget };
    export let mode: "ok" | "nan" | "throw" | "dispose" = "ok";
    export function setMode(next: "ok" | "nan" | "throw" | "dispose") { mode = next; }
    export let value: Spring;
    export let stop: (() => void) | undefined;

    export function mount(root: NativeNode) {
      stop = renderApp(() => {
        value = createSpring(() => {
          const current = target();
          if (mode === "nan") return Number.NaN;
          if (mode === "throw") throw new Error("target boom");
          if (mode === "dispose") { stop?.(); return current; }
          return current;
        }, { stiffness: 170, damping: 18 });
        return <box style={{ opacity: value() }} />;
      }, root);
      return stop;
    }
  `,
  );
  let dispose;
  globalThis.__zgui = h.host;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  const clock = createClock(t, h);
  dispose = mod.mount(h.makeNode("root"));
  h.runTurn(() => mod.setTarget(1));
  clock.advance(16);
  clock.frame();
  const before = mod.value();
  const pendingFrames = [...clock.frames.keys()];
  assert.equal(pendingFrames.length, 1);

  mod.setMode("nan");
  assert.throws(() => mod.value.jump(0), {
    name: "RangeError",
    message: "Spring target must be finite",
  });
  assert.equal(mod.value(), before, "non-finite target read must not partially reset position");
  assert.deepEqual([...clock.frames.keys()], pendingFrames, "pending frame must stay intact");

  mod.setMode("throw");
  assert.throws(() => mod.value.jump(0), { message: "target boom" });
  assert.equal(mod.value(), before, "throwing target read must not partially reset position");
  assert.deepEqual([...clock.frames.keys()], pendingFrames, "pending frame must stay intact");

  mod.setMode("dispose");
  mod.value.jump(0);
  assert.equal(
    clock.frames.size,
    0,
    "disposing during target read must not schedule a post-disposal frame",
  );
  assert.equal(mod.value(), before, "disposing during target read must not mutate position");
});

test("jump inside a Solid effect callback converges cleanly with same-turn target updates", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(
    t,
    `
    import { createEffect, createSignal, createSpring, renderApp } from "@hewel/zgui";
    import type { NativeNode, Spring } from "@hewel/zgui";

    const [trigger, setTrigger] = createSignal(0);
    const [target, setTarget] = createSignal(0);
    export { setTrigger };
    export let value: Spring;

    export function mount(root: NativeNode) {
      return renderApp(() => {
        value = createSpring(target, { stiffness: 170, damping: 18 });
        createEffect(trigger, step => {
          if (step === 1) {
            setTarget(1);
            value.jump(0);
          } else if (step === 2) {
            setTarget(0);
            value.jump(0);
          }
        });
        return <box style={{ opacity: value() }} />;
      }, root);
    }
  `,
  );
  let dispose;
  globalThis.__zgui = h.host;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  const clock = createClock(t, h);
  dispose = mod.mount(h.makeNode("root"));

  h.runTurn(() => mod.setTrigger(1));
  assert.equal(mod.value(), 0);
  assert.equal(clock.frames.size, 1, "retargeted jump inside an effect starts motion");
  clock.advance(16);
  clock.frame();
  assert.ok(mod.value() > 0 && mod.value() < 1);
  for (let i = 0; i < 240 && clock.frames.size; i += 1) {
    clock.advance(16);
    clock.frame();
  }
  assert.equal(mod.value(), 1);
  assert.equal(clock.frames.size, 0);

  h.runTurn(() => mod.setTrigger(2));
  assert.equal(mod.value(), 0);
  assert.equal(
    clock.frames.size,
    0,
    "matching target and jump inside an effect settles with no frame",
  );
});

test("dynamic reducedMotion settles active springs, updates ThemeProvider tokens, and follows host updates", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(
    t,
    `
    import { createSignal, createSpring, getReducedMotion, renderApp, setReducedMotion } from "@hewel/zgui";
    import { ThemeProvider } from "@hewel/zgui/components";
    import type { NativeNode, Spring } from "@hewel/zgui";

    const [target, setTarget] = createSignal(0);
    export { setTarget, getReducedMotion, setReducedMotion };
    export let value: Spring;

    export function mount(root: NativeNode) {
      return renderApp(() => {
        value = createSpring(target, { stiffness: 170, damping: 18 });
        return (
          <ThemeProvider>
            <box style={{ opacity: value() }} />
          </ThemeProvider>
        );
      }, root);
    }
  `,
  );
  let dispose;
  globalThis.__zgui = h.host;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  const clock = createClock(t, h);
  dispose = mod.mount(h.makeNode("root"));

  const themeCss = () =>
    [...h.stylesheets.entries()].find(([name]) => name.startsWith("zgui-ui-theme-"))?.[1] ?? "";
  assert.equal(mod.getReducedMotion(), false);
  assert.ok(!themeCss().includes("--zui-motion-duration-fast: 0ms;"));

  // Start a spring in flight, then flip reduced motion on mid-flight.
  h.runTurn(() => mod.setTarget(1));
  clock.advance(16);
  clock.frame();
  assert.ok(mod.value() > 0 && mod.value() < 1);
  assert.equal(clock.frames.size, 1);

  h.runTurn(() => mod.setReducedMotion(true));
  assert.equal(mod.getReducedMotion(), true);
  assert.equal(mod.value(), 1, "active spring settles immediately when reduced motion turns on");
  assert.equal(clock.frames.size, 0, "pending spring frame is cancelled immediately");
  assert.ok(
    themeCss().includes("--zui-motion-duration-fast: 0ms;"),
    "ThemeProvider reinstalls 0ms motion tokens live",
  );

  // Re-enabling motion resumes spring animation on the next target change.
  h.runTurn(() => mod.setReducedMotion(false));
  assert.ok(!themeCss().includes("--zui-motion-duration-fast: 0ms;"));
  h.runTurn(() => mod.setTarget(0));
  assert.equal(
    clock.frames.size,
    1,
    "spring schedules animation again after reduced motion turns off",
  );
  clock.advance(16);
  clock.frame();
  assert.ok(mod.value() > 0 && mod.value() < 1);

  // Clearing the override (null) follows live host/portal updates.
  h.runTurn(() => mod.setReducedMotion(null));
  h.emitReducedMotion(true);
  assert.equal(mod.getReducedMotion(), true);
  assert.equal(mod.value(), 0, "host portal reduced-motion update settles active spring");
  assert.equal(clock.frames.size, 0);
  assert.ok(themeCss().includes("--zui-motion-duration-fast: 0ms;"));
});

test("pure-shell unscoped getReducedMotion syncs eagerly from attached surfaces and receives live updates", async t => {
  const h = createHost({ strictStyles: true, reducedMotion: true });
  const mod = await moduleFor(
    t,
    `
    import { LayerSurface, createEffect, getReducedMotion, renderShell } from "@hewel/zgui";

    export const observed: boolean[] = [];

    export function mount() {
      return renderShell(() => {
        createEffect(() => getReducedMotion(), value => {
          observed.push(value);
        });
        return (
          <LayerSurface name="bar">
            <text>hello</text>
          </LayerSurface>
        );
      });
    }
  `,
  );
  const previousShell = globalThis.__zguiShell;
  globalThis.__zguiShell = h.shell;
  let dispose;
  t.after(() => {
    try {
      if (dispose) h.runTurn(dispose);
    } finally {
      if (previousShell === undefined) delete globalThis.__zguiShell;
      else globalThis.__zguiShell = previousShell;
    }
  });
  dispose = h.runTurn(() => mod.mount());
  h.flush();
  assert.equal(
    mod.observed.at(-1),
    true,
    "root effect syncs initial reducedMotion when first shell surface attaches",
  );

  h.emitReducedMotion(false);
  assert.equal(
    mod.observed.at(-1),
    false,
    "root effect receives live reducedMotion updates without any surface reading getReducedMotion",
  );
});

test("createSpring accepts reactive physics accessors and configure() without losing position or velocity", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(
    t,
    `
    import { createSignal, createSpring, renderApp } from "@hewel/zgui";
    import type { NativeNode, Spring } from "@hewel/zgui";

    const [target, setTarget] = createSignal(0);
    const [stiffness, setStiffness] = createSignal(120);
    const [damping, setDamping] = createSignal(14);
    export { setTarget, setStiffness, setDamping };
    export let spring!: Spring;

    export function mount(root: NativeNode) {
      return renderApp(() => {
        spring = createSpring(target, { stiffness, damping, mass: 1 });
        return <box style={{ opacity: spring() }} />;
      }, root);
    }
  `,
  );
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  const clock = createClock(t, h);
  dispose = mod.mount(h.makeNode("root"));

  h.runTurn(() => mod.setTarget(100));
  clock.advance(16);
  clock.frame();
  const midBefore = mod.spring();
  assert.ok(midBefore > 0 && midBefore < 100);

  // Updating reactive physics mid-flight keeps position continuous and keeps animating.
  h.runTurn(() => {
    mod.setStiffness(420);
    mod.setDamping(28);
  });
  assert.equal(mod.spring(), midBefore, "reactive physics update preserves current position");
  clock.advance(16);
  clock.frame();
  const midAfterReactive = mod.spring();
  assert.ok(
    midAfterReactive > midBefore,
    "spring keeps advancing smoothly after reactive option update",
  );

  // configure() validates eagerly and updates physics live without resetting position.
  assert.throws(() => mod.spring.configure({ stiffness: 0 }), RangeError);
  mod.spring.configure({ stiffness: 600, damping: 32 });
  assert.equal(mod.spring(), midAfterReactive, "configure() preserves position");
  clock.advance(16);
  clock.frame();
  assert.ok(mod.spring() > midAfterReactive);
});

test("createSpring initial option validates synchronously, reads once untracked, and handles sub-precision & reduced motion", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(
    t,
    `
    import { createEffect, createSignal, createSpring, renderApp, setReducedMotion } from "@hewel/zgui";
    import type { NativeNode, Spring } from "@hewel/zgui";

    const [target, setTarget] = createSignal(10);
    const [dep, setDep] = createSignal(0);
    export { setTarget, setDep, setReducedMotion };
    export let initialReads = 0;
    export let sOmitted!: Spring;
    export let sUndefined!: Spring;
    export let sEqual!: Spring;
    export let sDistinct!: Spring;
    export let sSubPrecision!: Spring;
    export let sEarlyImpulse!: Spring;
    export let firstDistinctRead = Number.NaN;
    export let firstDistinctSettled = true;
    export let firstSubPrecisionRead = Number.NaN;
    export let firstSubPrecisionSettled = true;
    export let effectRuns = 0;

    export function mount(root: NativeNode) {
      return renderApp(() => {
        sOmitted = createSpring(target);
        sUndefined = createSpring(target, { initial: undefined });
        sEqual = createSpring(target, { initial: 10 });
        createEffect(
          () => {
            effectRuns += 1;
            const created = createSpring(target, {
              stiffness: 170,
              damping: 20,
              get initial() {
                initialReads += 1;
                dep();
                return 0;
              },
            });
            sDistinct = created;
          },
          () => {
            firstDistinctRead = sDistinct();
            firstDistinctSettled = sDistinct.settled();
          },
        );
        sSubPrecision = createSpring(() => 0, {
          initial: 0.0001,
          precision: 0.01,
          stiffness: 170,
          damping: 26,
        });
        firstSubPrecisionRead = sSubPrecision();
        firstSubPrecisionSettled = sSubPrecision.settled();

        sEarlyImpulse = createSpring(() => 0, { stiffness: 170, damping: 20 });
        sEarlyImpulse.impulse(120);
        return <box />;
      }, root);
    }

    export function attemptNonFinite(root: NativeNode, badInitial: number) {
      return renderApp(() => {
        createSpring(() => 0, { initial: badInitial });
        return <box />;
      }, root);
    }
  `,
  );
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  const clock = createClock(t, h);

  // Non-finite initial fails synchronously even under reduced motion, before registering work.
  h.emitReducedMotion(true);
  for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
    assert.throws(() => mod.attemptNonFinite(h.makeNode("root"), bad), {
      name: "RangeError",
      message: "Spring initial value must be finite",
    });
  }
  assert.equal(clock.frames.size, 0, "invalid initial never schedules a frame");
  h.emitReducedMotion(false);

  dispose = mod.mount(h.makeNode("root"));
  assert.equal(mod.sOmitted(), 10);
  assert.equal(mod.sOmitted.settled(), true);
  assert.equal(mod.sUndefined(), 10);
  assert.equal(mod.sUndefined.settled(), true);
  assert.equal(mod.sEqual(), 10);
  assert.equal(mod.sEqual.settled(), true);

  assert.equal(mod.firstDistinctRead, 0);
  assert.equal(mod.firstDistinctSettled, false);
  assert.equal(mod.sDistinct.settled(), false, "initial target effect must not undo mount initial");
  assert.equal(mod.firstSubPrecisionRead, 0.0001);
  assert.equal(mod.firstSubPrecisionSettled, false);
  assert.equal(mod.sSubPrecision.settled(), false, "sub-precision initial still enrolls once");
  assert.equal(
    mod.sEarlyImpulse.settled(),
    false,
    "initial target effect must not undo early impulse",
  );
  assert.equal(mod.initialReads, 1, "initial getter is read once at construction");
  assert.equal(clock.frames.size, 1, "active springs share a single rAF frame");

  // Updating dep read inside initial getter does NOT re-trigger the effect or re-read initial.
  h.runTurn(() => mod.setDep(1));
  assert.equal(mod.effectRuns, 1, "initial getter read is untracked");
  assert.equal(mod.initialReads, 1);

  // configure() must never re-read initial or spread it into overrides.
  mod.sDistinct.configure({ stiffness: 200 });
  assert.equal(mod.initialReads, 1);

  // Step one frame: sub-precision settles, distinct & earlyImpulse move.
  clock.advance(16);
  clock.frame();
  assert.equal(mod.sSubPrecision(), 0);
  assert.equal(mod.sSubPrecision.settled(), true);
  assert.ok(mod.sDistinct() > 0 && mod.sDistinct() < 10);
  assert.ok(mod.sEarlyImpulse() > 0);

  for (let i = 0; i < 300 && clock.frames.size; i += 1) {
    clock.advance(16);
    clock.frame();
  }
  assert.equal(mod.sDistinct(), 10);
  assert.equal(mod.sDistinct.settled(), true);
  assert.equal(mod.sEarlyImpulse(), 0);
  assert.equal(mod.sEarlyImpulse.settled(), true);
  assert.equal(clock.frames.size, 0);

  // Under reduced motion, distinct finite initial snaps immediately to target with zero frames.
  h.emitReducedMotion(true);
  const rmMod = await moduleFor(
    t,
    `
    import { createSpring, renderApp } from "@hewel/zgui";
    import type { NativeNode, Spring } from "@hewel/zgui";
    export let spring!: Spring;
    export function mount(root: NativeNode) {
      return renderApp(() => {
        spring = createSpring(() => 42, { initial: 0 });
        return <box />;
      }, root);
    }
  `,
  );
  const disposeRm = rmMod.mount(h.makeNode("root"));
  t.after(() => disposeRm());
  assert.equal(rmMod.spring(), 42);
  assert.equal(rmMod.spring.settled(), true);
  assert.equal(clock.frames.size, 0);
});

test("settled is backed by its own boolean signal and covers the full state table", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(
    t,
    `
    import { createEffect, createSignal, createSpring, renderApp, setReducedMotion } from "@hewel/zgui";
    import type { NativeNode, Spring } from "@hewel/zgui";

    const [target, setTarget] = createSignal(0);
    export { setTarget, setReducedMotion };
    export let spring!: Spring;
    export const settledTransitions: boolean[] = [];
    export let positionUpdates = 0;

    export function mount(root: NativeNode) {
      return renderApp(() => {
        spring = createSpring(target, { stiffness: 170, damping: 16 });
        createEffect(
          () => spring.settled(),
          next => {
            settledTransitions.push(next);
          },
        );
        createEffect(
          () => spring(),
          () => {
            positionUpdates += 1;
          },
        );
        return <box />;
      }, root);
    }
  `,
  );
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  const clock = createClock(t, h);
  dispose = mod.mount(h.makeNode("root"));

  assert.deepEqual(mod.settledTransitions, [true]);
  const initialPosUpdates = mod.positionUpdates;

  h.runTurn(() => mod.setTarget(100));
  assert.equal(mod.spring.settled(), false);
  assert.deepEqual(mod.settledTransitions, [true, false]);

  for (let i = 0; i < 10; i += 1) {
    clock.advance(16);
    clock.frame();
  }
  assert.ok(mod.positionUpdates >= initialPosUpdates + 10, "position updated every frame");
  assert.deepEqual(
    mod.settledTransitions,
    [true, false],
    "settled-only effect must NOT re-run per frame while animating",
  );

  // Crossing target with nonzero velocity stays unsettled.
  const mid = mod.spring();
  h.runTurn(() => mod.setTarget(mid));
  assert.equal(mod.spring.settled(), false);
  clock.advance(16);
  clock.frame();
  assert.equal(mod.spring.settled(), false);

  for (let i = 0; i < 300 && clock.frames.size; i += 1) {
    clock.advance(16);
    clock.frame();
  }
  assert.equal(mod.spring.settled(), true);
  assert.deepEqual(mod.settledTransitions, [true, false, true]);

  // Imperative jump exposes settled state synchronously on return.
  mod.spring.jump(0);
  assert.equal(mod.spring.settled(), false);
  mod.spring.jump(mid);
  assert.equal(mod.spring.settled(), true);

  // Disposal sets settled() === true permanently.
  mod.spring.jump(0);
  assert.equal(mod.spring.settled(), false);
  dispose();
  dispose = undefined;
  assert.equal(mod.spring.settled(), true);
  assert.equal(clock.frames.size, 0);
});

test("impulse validates inputs, syncs same-turn targets without zeroing velocity, and handles rest/reduced-motion/disposal", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(
    t,
    `
    import { createSignal, createSpring, renderApp, setReducedMotion } from "@hewel/zgui";
    import type { NativeNode, Spring } from "@hewel/zgui";

    const [target, setTarget] = createSignal(0);
    export { setTarget, setReducedMotion };
    export let mode: "ok" | "nan" | "throw" | "dispose" = "ok";
    export function setMode(next: "ok" | "nan" | "throw" | "dispose") { mode = next; }
    export let spring!: Spring;
    export let stop: (() => void) | undefined;

    export function mount(root: NativeNode) {
      stop = renderApp(() => {
        spring = createSpring(() => {
          const current = target();
          if (mode === "nan") return Number.NaN;
          if (mode === "throw") throw new Error("impulse target boom");
          if (mode === "dispose") { stop?.(); return current; }
          return current;
        }, { stiffness: 170, damping: 20 });
        return <box />;
      }, root);
      return stop;
    }
  `,
  );
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  const clock = createClock(t, h);
  dispose = mod.mount(h.makeNode("root"));

  // Fixed method descriptors.
  for (const prop of ["settled", "jump", "impulse", "configure"]) {
    const desc = Object.getOwnPropertyDescriptor(mod.spring, prop);
    assert.equal(typeof desc?.value, "function", `${prop} should be a function`);
    assert.equal(desc.writable, false);
    assert.equal(desc.configurable, false);
  }

  // Non-finite impulse rejected synchronously.
  for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
    assert.throws(() => mod.spring.impulse(bad), {
      name: "RangeError",
      message: "Spring impulse must be finite",
    });
  }
  assert.equal(clock.frames.size, 0);

  // Zero impulse is a no-op.
  mod.spring.impulse(0);
  assert.equal(mod.spring.settled(), true);
  assert.equal(clock.frames.size, 0);

  // Nonzero impulse from rest at zero displacement schedules a step without moving position yet.
  mod.spring.impulse(0.0001);
  assert.equal(mod.spring(), 0);
  assert.equal(mod.spring.settled(), false);
  assert.equal(clock.frames.size, 1);

  // Opposing impulse cancelling back to zero at rest settles immediately.
  mod.spring.impulse(-0.0001);
  assert.equal(mod.spring(), 0);
  assert.equal(mod.spring.settled(), true);
  assert.equal(clock.frames.size, 0);

  // Same-turn target update + impulse syncs target via guarded flush without zeroing velocity.
  h.runTurn(() => {
    mod.setTarget(10);
    mod.spring.impulse(200);
    assert.equal(mod.spring(), 0);
    assert.equal(mod.spring.settled(), false);
    assert.equal(clock.frames.size, 1);
  });
  clock.advance(16);
  clock.frame();
  const afterKick = mod.spring();
  assert.ok(afterKick > 2, `expected strong positive velocity boost, got ${afterKick}`);

  // Non-finite velocity sum throws RangeError before mutating.
  const beforeOverflow = mod.spring();
  const framesBeforeOverflow = [...clock.frames.keys()];
  mod.spring.impulse(Number.MAX_VALUE);
  assert.throws(() => mod.spring.impulse(Number.MAX_VALUE), {
    name: "RangeError",
    message: "Spring velocity must remain finite",
  });
  assert.equal(mod.spring(), beforeOverflow);
  assert.deepEqual([...clock.frames.keys()], framesBeforeOverflow);
  // Restore finite reasonable velocity so remaining checks stay clean.
  mod.spring.jump(beforeOverflow);

  // Failing target reads during impulse never partially apply.
  mod.setMode("nan");
  assert.throws(() => mod.spring.impulse(10), {
    name: "RangeError",
    message: "Spring target must be finite",
  });
  assert.equal(mod.spring(), beforeOverflow);
  mod.setMode("throw");
  assert.throws(() => mod.spring.impulse(10), { message: "impulse target boom" });
  assert.equal(mod.spring(), beforeOverflow);
  mod.setMode("ok");

  // Reduced motion: invalid impulse still throws, valid impulse is a no-op with zero frames.
  h.runTurn(() => mod.setReducedMotion(true));
  assert.equal(mod.spring.settled(), true);
  assert.equal(clock.frames.size, 0);
  assert.throws(() => mod.spring.impulse(Number.NaN), RangeError);
  mod.spring.impulse(500);
  assert.equal(mod.spring.settled(), true);
  assert.equal(clock.frames.size, 0);
  h.runTurn(() => mod.setReducedMotion(false));

  // Disposal during target read wins and leaves zero frames & settled === true.
  mod.setMode("dispose");
  mod.spring.impulse(50);
  dispose = undefined;
  assert.equal(mod.spring.settled(), true);
  assert.equal(clock.frames.size, 0);

  // Post-disposal: non-finite impulse still throws, valid impulse is a no-op.
  assert.throws(() => mod.spring.impulse(Number.POSITIVE_INFINITY), RangeError);
  mod.spring.impulse(100);
  assert.equal(mod.spring.settled(), true);
  assert.equal(clock.frames.size, 0);
});

test("shared rAF ticker coalesces per-bridge, isolates bridges, preserves order/snapshot/revision, and handles timestamps & exceptions", async t => {
  const h = createHost({ strictStyles: true });
  const clock = createClock(t, h);

  const bridgeFrames = new Map();
  const bridgeByName = new Map();
  let nextBridgeFrameId = 10_000;
  const origOpenLayer = h.shell.openLayer.bind(h.shell);
  h.shell.openLayer = (name, placement, attach) =>
    origOpenLayer(name, placement, (root, bridge) => {
      const frames = new Map();
      bridgeFrames.set(name, frames);
      bridgeByName.set(name, bridge);
      bridge.requestAnimationFrame = function requestAnimationFrame(cb) {
        assert.equal(
          this,
          bridge,
          `bridge ${name} requestAnimationFrame must receive bridge as this`,
        );
        const id = ++nextBridgeFrameId;
        frames.set(id, cb);
        return id;
      };
      return attach(root, bridge);
    });

  const origCancel = globalThis.cancelAnimationFrame;
  globalThis.cancelAnimationFrame = id => {
    for (const frames of bridgeFrames.values()) frames.delete(id);
    origCancel(id);
  };
  t.after(() => {
    globalThis.cancelAnimationFrame = origCancel;
  });

  const mod = await moduleFor(
    t,
    `
    import { LayerSurface, createSignal, createSpring, renderShell } from "@hewel/zgui";
    import type { Spring } from "@hewel/zgui";

    const [t1, setT1] = createSignal(0);
    const [t2, setT2] = createSignal(0);
    const [t3, setT3] = createSignal(0);
    const [tB, setTB] = createSignal(0);
    const [tG, setTG] = createSignal(0);
    export { setT1, setT2, setT3, setTB, setTG };
    export let sA1!: Spring;
    export let sA2!: Spring;
    export let sA3!: Spring;
    export let sB!: Spring;
    export let sG!: Spring;

    function SurfaceA() {
      sA1 = createSpring(t1, { stiffness: 170, damping: 20 });
      sA2 = createSpring(t2, { stiffness: 170, damping: 20 });
      sA3 = createSpring(t3, { stiffness: 170, damping: 20 });
      return <box />;
    }
    function SurfaceB() {
      sB = createSpring(tB, { stiffness: 170, damping: 20 });
      return <box />;
    }

    export function mount() {
      return renderShell(() => {
        sG = createSpring(tG, { stiffness: 170, damping: 20 });
        return (
          <>
            <LayerSurface name="surface-a">
              <SurfaceA />
            </LayerSurface>
            <LayerSurface name="surface-b">
              <SurfaceB />
            </LayerSurface>
          </>
        );
      });
    }
  `,
  );

  const prevShell = globalThis.__zguiShell;
  globalThis.__zguiShell = h.shell;
  let dispose;
  t.after(() => {
    try {
      if (dispose) h.runTurn(dispose);
    } finally {
      if (prevShell === undefined) delete globalThis.__zguiShell;
      else globalThis.__zguiShell = prevShell;
    }
  });

  dispose = h.runTurn(() => mod.mount());
  h.flush();

  const framesA = bridgeFrames.get("surface-a");
  const framesB = bridgeFrames.get("surface-b");
  const bridgeA = bridgeByName.get("surface-a");
  assert.ok(framesA && framesB && bridgeA);

  // Coalesce multiple springs on bridge A into ONE frame, separate from bridge B and global.
  h.runTurn(() => {
    mod.setT1(100);
    mod.setT2(50);
    mod.setTB(80);
    mod.setTG(40);
  });
  assert.equal(framesA.size, 1, "two springs on bridge A share one rAF");
  assert.equal(framesB.size, 1, "bridge B has its own ticker");
  assert.equal(clock.frames.size, 1, "global fallback has its own ticker");

  // Settling one spring on bridge A does NOT cancel its sibling's frame.
  const [frameAId] = framesA.keys();
  h.runTurn(() => mod.setT1(0));
  assert.equal(mod.sA1.settled(), true);
  assert.equal(framesA.size, 1);
  assert.equal(framesA.has(frameAId), true, "removing one spring never cancels sibling frame");

  // Removing the last spring on bridge A cancels its frame ID and invalidates stale replay.
  const staleCallbackA = framesA.get(frameAId);
  h.runTurn(() => mod.setT2(0));
  assert.equal(framesA.size, 0);
  staleCallbackA(16);
  assert.equal(mod.sA2(), 0);
  assert.equal(framesA.size, 0);

  // Non-finite timestamp is ignored without corrupting position and rearms cleanly.
  const [frameBId] = framesB.keys();
  const cbB1 = framesB.get(frameBId);
  framesB.delete(frameBId);
  cbB1(Number.NaN);
  assert.equal(mod.sB(), 0, "NaN timestamp is ignored");
  assert.equal(framesB.size, 1, "ticker rearms after ignoring NaN timestamp");

  // Backward timestamp uses dt = 0 and never moves history backward.
  const runB = ts => {
    const [[id, cb]] = framesB.entries();
    framesB.delete(id);
    cb(ts);
  };
  runB(100); // first step uses 1/60s, sets previousTime = 100
  const afterFirstB = mod.sB();
  assert.ok(afterFirstB > 0);
  runB(50); // backward timestamp: dt = 0, position unchanged, previousTime stays 100
  assert.equal(mod.sB(), afterFirstB, "backward timestamp uses dt = 0");
  runB(116); // dt = 16ms from 100
  const afterNormalB = mod.sB();
  assert.ok(afterNormalB > afterFirstB);

  // Snapshot + revision: restarting sA2 and enrolling sA3 mid-tick skips sA2 & defers sA3 to next frame.
  h.runTurn(() => {
    mod.setT1(100);
    mod.setT2(100);
  });
  assert.equal(framesA.size, 1);
  let hookTriggered = false;
  const withShellGetter = (onRead, fn) => {
    Object.defineProperty(globalThis, "__zguiShell", {
      configurable: true,
      get() {
        onRead();
        return h.shell;
      },
    });
    try {
      return fn();
    } finally {
      Object.defineProperty(globalThis, "__zguiShell", {
        configurable: true,
        writable: true,
        value: h.shell,
      });
    }
  };
  const [[snapId, snapCb]] = framesA.entries();
  framesA.delete(snapId);
  withShellGetter(
    () => {
      if (!hookTriggered) {
        hookTriggered = true;
        // During sA1's step: restart sA2 via jump (bumps revision) and enroll sA3 via impulse.
        mod.sA2.jump(5);
        mod.sA3.impulse(120);
      }
    },
    () => snapCb(16),
  );
  assert.equal(hookTriggered, true, "re-entrant hook executed during sA1 step");
  assert.ok(mod.sA1() > 0, "sA1 stepped in the tick");
  assert.equal(mod.sA2(), 5, "restarted sA2 was skipped for the remainder of the current tick");
  assert.equal(mod.sA3(), 0, "newly enrolled sA3 was deferred to the next frame");
  assert.equal(framesA.size, 1, "ticker rearmed in dispatch epilogue");
  const [[nextSnapId, nextSnapCb]] = framesA.entries();
  framesA.delete(nextSnapId);
  nextSnapCb(32);
  assert.ok(mod.sA2() > 5, "restarted sA2 steps on the next frame");
  assert.ok(mod.sA3() > 0, "newly enrolled sA3 steps on the next frame");

  // Exception during tick propagates without being swallowed while bookkeeping & rearming survive.
  let throwOnce = true;
  const [[errFrameId, errCb]] = framesA.entries();
  framesA.delete(errFrameId);
  assert.throws(
    () =>
      withShellGetter(
        () => {
          if (throwOnce) {
            throwOnce = false;
            throw new Error("tick boom");
          }
        },
        () => errCb(48),
      ),
    { message: "tick boom" },
  );
  assert.equal(framesA.size, 1, "ticker rearms in finally epilogue even after exception");
  const beforeRecover = mod.sA1();
  const [[recoveredId, recoveredCb]] = framesA.entries();
  framesA.delete(recoveredId);
  recoveredCb(64);
  assert.ok(mod.sA1() > beforeRecover, "spring resumes cleanly after recovered tick");
});

test("rootless shell global-fallback spring parks at zero surfaces and rearms on next attach", async t => {
  const h = createHost({ strictStyles: true });
  const clock = createClock(t, h);

  // Mirror native-host: global requestAnimationFrame throws when zero surfaces are mounted.
  let openSurfaces = 0;
  let rafAttemptsWhileClosed = 0;
  const baseRaf = globalThis.requestAnimationFrame;
  globalThis.requestAnimationFrame = cb => {
    if (openSurfaces === 0) {
      rafAttemptsWhileClosed += 1;
      throw new ReferenceError("requestAnimationFrame requires an open surface");
    }
    return baseRaf(cb);
  };
  t.after(() => {
    globalThis.requestAnimationFrame = baseRaf;
  });

  // Track surface attach/detach on the shell fixture.
  const origOpenLayer = h.shell.openLayer.bind(h.shell);
  h.shell.openLayer = (name, placement, attach) =>
    origOpenLayer(name, placement, (root, bridge) => {
      openSurfaces += 1;
      const disposeSurface = attach(root, bridge);
      return () => {
        try {
          disposeSurface();
        } finally {
          openSurfaces -= 1;
        }
      };
    });

  const mod = await moduleFor(
    t,
    `
    import { LayerSurface, Show, createSignal, createSpring, renderShell } from "@hewel/zgui";
    import type { Spring } from "@hewel/zgui";

    const [showSurface, setShowSurface] = createSignal(false);
    const [target, setTarget] = createSignal(0);
    export { setShowSurface, setTarget };
    export let rootSpring!: Spring;

    export function mount() {
      return renderShell(() => {
        rootSpring = createSpring(target, { stiffness: 170, damping: 20 });
        return (
          <Show when={showSurface()}>
            <LayerSurface name="bar">
              <text>hello</text>
            </LayerSurface>
          </Show>
        );
      });
    }
  `,
  );

  const prevShell = globalThis.__zguiShell;
  globalThis.__zguiShell = h.shell;
  let dispose;
  t.after(() => {
    try {
      if (dispose) h.runTurn(dispose);
    } finally {
      if (prevShell === undefined) delete globalThis.__zguiShell;
      else globalThis.__zguiShell = prevShell;
    }
  });

  dispose = h.runTurn(() => mod.mount());
  h.flush();

  // Start animating while zero surfaces exist: parks cleanly without calling global rAF.
  h.runTurn(() => mod.setTarget(100));
  assert.equal(rafAttemptsWhileClosed, 0);
  assert.equal(clock.frames.size, 0);
  assert.equal(mod.rootSpring.settled(), false);

  // Attach first surface: parked ticker automatically arms.
  h.runTurn(() => mod.setShowSurface(true));
  h.flush();
  assert.equal(openSurfaces, 1);
  assert.equal(clock.frames.size, 1, "global fallback ticker rearms when first surface attaches");

  clock.advance(16);
  clock.frame();
  const afterAttach = mod.rootSpring();
  assert.ok(afterAttach > 0 && afterAttach < 100);
  const staleBeforeDetach = [...clock.frames.values()];

  // Detach last surface: cancels outstanding frame, invalidates stale callback, resets history.
  h.runTurn(() => mod.setShowSurface(false));
  h.flush();
  assert.equal(openSurfaces, 0);
  assert.equal(clock.frames.size, 0, "last detach cancels pending global fallback frame");
  for (const cb of staleBeforeDetach) cb(500);
  assert.equal(mod.rootSpring(), afterAttach, "stale callback after detach is ignored");
  assert.equal(rafAttemptsWhileClosed, 0);

  // Reattach a new surface after a long pause: rearms and steps with fresh 1/60s history.
  clock.advance(5000);
  h.runTurn(() => mod.setShowSurface(true));
  h.flush();
  assert.equal(openSurfaces, 1);
  assert.equal(clock.frames.size, 1, "reattach rearms parked global fallback ticker");
  clock.frame();
  assert.ok(mod.rootSpring() > afterAttach);
  for (let i = 0; i < 300 && clock.frames.size; i += 1) {
    clock.advance(16);
    clock.frame();
  }
  assert.equal(mod.rootSpring(), 100);
  assert.equal(mod.rootSpring.settled(), true);
  assert.equal(clock.frames.size, 0);
  assert.equal(rafAttemptsWhileClosed, 0);
});
