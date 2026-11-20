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
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  const clock = createClock(t, h);
  const root = h.makeNode("root");
  dispose = mod.mount(root, renderScale);
  return {
    clock, value: mod.value, target: mod.target, runTurn: h.runTurn,
    scale: () => root.children[0].props.get("style").scale,
    setTarget: next => h.runTurn(() => mod.setTarget(next)),
    tick(ms = 16) { clock.advance(ms); clock.frame(); },
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
  assert.equal(value(), firstStep, "neither momentum nor the prior frame timestamp survives a jump");
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
    assert.throws(() => value.jump(nonFinite), { name: "RangeError", message: "Spring jump value must be finite" });
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
  const mod = await moduleFor(t, `
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
  `);
  let dispose;
  globalThis.__zgui = h.host;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  const clock = createClock(t, h);
  dispose = mod.mount(h.makeNode("root"));
  h.runTurn(() => mod.setTarget(1));
  clock.advance(16);
  clock.frame();
  const before = mod.value();
  const pendingFrames = [...clock.frames.keys()];
  assert.equal(pendingFrames.length, 1);

  mod.setMode("nan");
  assert.throws(() => mod.value.jump(0), { name: "RangeError", message: "Spring target must be finite" });
  assert.equal(mod.value(), before, "non-finite target read must not partially reset position");
  assert.deepEqual([...clock.frames.keys()], pendingFrames, "pending frame must stay intact");

  mod.setMode("throw");
  assert.throws(() => mod.value.jump(0), { message: "target boom" });
  assert.equal(mod.value(), before, "throwing target read must not partially reset position");
  assert.deepEqual([...clock.frames.keys()], pendingFrames, "pending frame must stay intact");

  mod.setMode("dispose");
  mod.value.jump(0);
  assert.equal(clock.frames.size, 0, "disposing during target read must not schedule a post-disposal frame");
  assert.equal(mod.value(), before, "disposing during target read must not mutate position");
});

test("jump inside a Solid effect callback converges cleanly with same-turn target updates", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(t, `
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
  `);
  let dispose;
  globalThis.__zgui = h.host;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  const clock = createClock(t, h);
  dispose = mod.mount(h.makeNode("root"));

  h.runTurn(() => mod.setTrigger(1));
  assert.equal(mod.value(), 0);
  assert.equal(clock.frames.size, 1, "retargeted jump inside an effect starts motion");
  clock.advance(16);
  clock.frame();
  assert.ok(mod.value() > 0 && mod.value() < 1);
  for (let i = 0; i < 240 && clock.frames.size; i += 1) { clock.advance(16); clock.frame(); }
  assert.equal(mod.value(), 1);
  assert.equal(clock.frames.size, 0);

  h.runTurn(() => mod.setTrigger(2));
  assert.equal(mod.value(), 0);
  assert.equal(clock.frames.size, 0, "matching target and jump inside an effect settles with no frame");
});
