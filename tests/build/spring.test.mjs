import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { createSignal, createSpring, renderApp } from "@hewel/zgui";
import type { NativeNode, Spring } from "@hewel/zgui";

const [target, setTarget] = createSignal(0);
export { setTarget };
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
  const clock = createClock(t, h);
  let dispose;
  globalThis.__zgui = h.host;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  const root = h.makeNode("root");
  dispose = mod.mount(root, renderScale);
  return {
    clock, value: () => mod.value(),
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

test("reduced motion follows the target without requesting a frame", async t => {
  const { clock, scale, setTarget } = await mounted(t, { reducedMotion: true });
  setTarget(50);
  assert.equal(scale(), "1.5");
  assert.equal(clock.frames.size, 0);
});

test("disposing the owner cancels the pending frame and detaches the target", async t => {
  const { clock, value, setTarget, tick, dispose } = await mounted(t);
  setTarget(100);
  tick();
  tick();
  assert.equal(clock.frames.size, 1);
  const before = value();
  dispose();
  assert.equal(clock.frames.size, 0);
  setTarget(50);
  tick();
  assert.equal(clock.frames.size, 0);
  assert.equal(value(), before);
});
