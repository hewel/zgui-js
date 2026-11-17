import assert from "node:assert/strict";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { createHost, moduleFor } from "./native-fixture.mjs";

const internal = fileURLToPath(new URL("../../packages/ui/src/components/presence.ts", import.meta.url));
const fixture = `
import { Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { createPresence } from ${JSON.stringify(internal)};
const [open, setOpen] = createSignal(false);
export { setOpen };
export const log = { mounts: 0, cleanups: 0 };
let presence: ReturnType<typeof createPresence>;
function Body() {
  log.mounts += 1;
  onCleanup(() => { log.cleanups += 1; });
  const [count, setCount] = createSignal(0);
  return <box testId="surface" ref={presence.ref} data-state={presence.state()}
    onAnimationEnd={presence.onEnd} onAnimationCancel={presence.onEnd}
    onTransitionEnd={presence.onEnd} onTransitionCancel={presence.onEnd}>
    <control testId="counter" onClick={() => setCount(n => n + 1)}>{String(count())}</control>
  </box>;
}
export function mount(root: NativeNode) {
  return renderApp(() => { presence = createPresence(open); return <Show when={presence.mounted()}><Body /></Show>; }, root);
}
`;

const textOf = current =>
  current.tag === "#text" ? current.props.get("text") ?? "" : current.children.map(textOf).join("");

function fakeClock(t) {
  let now = 0;
  let nextId = 0;
  const timers = new Map();
  const frames = new Map();
  t.mock.method(globalThis, "setTimeout", (callback, delay = 0) => {
    const id = ++nextId;
    timers.set(id, { at: now + delay, callback });
    return id;
  });
  t.mock.method(globalThis, "clearTimeout", id => { timers.delete(id); });
  const previousFrame = Object.getOwnPropertyDescriptor(globalThis, "requestAnimationFrame");
  const previousCancel = Object.getOwnPropertyDescriptor(globalThis, "cancelAnimationFrame");
  globalThis.requestAnimationFrame = callback => {
    const id = ++nextId;
    frames.set(id, callback);
    return id;
  };
  globalThis.cancelAnimationFrame = id => { frames.delete(id); };
  t.after(() => {
    if (previousFrame) Object.defineProperty(globalThis, "requestAnimationFrame", previousFrame);
    else delete globalThis.requestAnimationFrame;
    if (previousCancel) Object.defineProperty(globalThis, "cancelAnimationFrame", previousCancel);
    else delete globalThis.cancelAnimationFrame;
  });
  const advance = ms => {
    const target = now + ms;
    for (;;) {
      let earliest;
      for (const [timerId, timer] of timers) {
        if (timer.at <= target && (!earliest || timer.at < earliest.timer.at)) earliest = { timerId, timer };
      }
      if (!earliest) break;
      timers.delete(earliest.timerId);
      now = earliest.timer.at;
      earliest.timer.callback();
    }
    now = target;
  };
  const frame = () => {
    const due = [...frames.entries()];
    for (const [timerId] of due) frames.delete(timerId);
    for (const [, callback] of due) callback(now);
  };
  return { advance, frame, timers, frames };
}

async function mounted(t) {
  const queries = [];
  const busy = new Map();
  const harness = createHost();
  harness.host.isAnimating = node => {
    if (harness.byTestId("surface") !== node) throw Error("invalid node read");
    queries.push(node);
    return busy.get(node) ?? false;
  };
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  const clock = fakeClock(t);
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  const surface = () => harness.byTestId("surface");
  const state = () => surface()?.props.get("data-state");
  const counter = () => textOf(harness.byTestId("counter"));
  const event = (target, type) => surface().callbacks[type]({ type, target, currentTarget: surface() });
  return { clock, queries, busy, harness, mod, root, surface, state, counter, event,
    finish: () => { const end = dispose; dispose = undefined; end?.(); } };
}

test("presence mounts on open, defers its exit check past a styled frame and removes an idle surface", async t => {
  const { clock, queries, mod, surface, state, counter, event } = await mounted(t);
  assert.equal(surface(), undefined);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(queries.length, 0);
  mod.setOpen(true);
  assert.ok(surface());
  assert.equal(state(), "open");
  mod.setOpen(false);
  const first = surface();
  assert.equal(state(), "closed");
  assert.equal(clock.frames.size, 1);
  assert.equal(clock.timers.size, 1, "only the deadline timer is queued before the styled frame");
  assert.equal(queries.length, 0);
  event(first, "onAnimationEnd");
  assert.ok(surface(), "lifecycle event before the styled frame must not remove");
  assert.equal(queries.length, 0);
  clock.frame();
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 2);
  event(first, "onAnimationEnd");
  assert.ok(surface(), "lifecycle event before the deferred check must not remove");
  assert.equal(queries.length, 0);
  clock.advance(0);
  assert.equal(surface(), undefined);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(queries.length, 1);
  assert.equal(mod.log.cleanups, 1);
  mod.setOpen(true);
  assert.ok(surface());
  assert.notEqual(surface(), first);
  assert.equal(counter(), "0");
  assert.equal(state(), "open");
});

test("a busy surface is retained until own lifecycle event reports all motion gone", async t => {
  const { clock, queries, busy, mod, surface, event } = await mounted(t);
  mod.setOpen(true);
  const current = surface();
  busy.set(current, true);
  mod.setOpen(false);
  clock.frame();
  clock.advance(0);
  assert.equal(surface(), current, "busy surface survives the styled-frame check");
  assert.equal(queries.length, 1);
  event(current, "onAnimationEnd");
  assert.equal(surface(), current);
  assert.equal(queries.length, 2);
  busy.set(current, false);
  const counter = surface().children[0];
  surface().callbacks.onAnimationEnd({ type: "animationend", target: counter, currentTarget: current });
  assert.equal(surface(), current, "a descendant's event does not own the exit");
  assert.equal(queries.length, 2, "descendant event must not query");
  surface().callbacks.onTransitionCancel({ type: "transitioncancel", target: current, currentTarget: current });
  assert.equal(surface(), undefined);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.log.cleanups, 1);
});

test("reopen cancels a queued exit and stale callbacks cannot close the new exit", async t => {
  const { clock, queries, busy, mod, surface, state, counter } = await mounted(t);
  mod.setOpen(true);
  surface().children[0].callbacks.onClick();
  assert.equal(counter(), "1");
  mod.setOpen(false);
  const stale = [...clock.frames.values(), ...[...clock.timers.values()].map(timer => timer.callback)];
  mod.setOpen(true);
  const current = surface();
  assert.equal(state(), "open");
  assert.equal(counter(), "1");
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  for (const callback of stale) callback();
  assert.equal(surface(), current);
  assert.equal(queries.length, 0);
  mod.setOpen(false);
  busy.set(current, true);
  for (const callback of stale) callback();
  assert.equal(surface(), current, "stale callbacks must not close the new exit");
  assert.equal(clock.frames.size, 1);
  assert.equal(clock.timers.size, 1, "stale callbacks cannot schedule timers");
  clock.frame();
  clock.advance(0);
  assert.equal(surface(), current, "endless motion survives the styled-frame check");
  clock.advance(999);
  assert.equal(surface(), current);
  clock.advance(1);
  assert.equal(surface(), undefined, "the one-second deadline ends endless motion");
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(queries.length, 1);
});

test("disposal before the styled frame clears timers and saved callbacks stay inert", async t => {
  const { clock, queries, mod, surface, finish } = await mounted(t);
  mod.setOpen(true);
  mod.setOpen(false);
  assert.equal(clock.frames.size, 1);
  assert.equal(clock.timers.size, 1);
  const stale = [...clock.frames.values(), ...[...clock.timers.values()].map(timer => timer.callback)];
  finish();
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.log.cleanups, 1);
  assert.equal(surface(), undefined);
  for (const callback of stale) callback();
  assert.equal(queries.length, 0);
});

test("disposal after the styled frame queued the deferred check clears timers and saved callbacks stay inert", async t => {
  const { clock, queries, mod, surface, finish } = await mounted(t);
  mod.setOpen(true);
  mod.setOpen(false);
  clock.frame();
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 2);
  const stale = [...clock.timers.values()].map(timer => timer.callback);
  finish();
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.log.cleanups, 1);
  assert.equal(surface(), undefined);
  for (const callback of stale) callback();
  assert.equal(queries.length, 0);
});
