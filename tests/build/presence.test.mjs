import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Popover, PopoverContent, PopoverTrigger, ThemeProvider } from "@hewel/zgui/components";
const [open, setOpen] = createSignal(false);
export { setOpen };
export const log = { mounts: 0, cleanups: 0 };
function Body() {
  log.mounts += 1;
  onCleanup(() => { log.cleanups += 1; });
  const [count, setCount] = createSignal(0);
  return <control testId="counter" onClick={() => setCount(n => n + 1)}>{String(count())}</control>;
}
export function mount(root: NativeNode) {
  return renderApp(() => (
    <ThemeProvider>
      <Popover open={open()} onOpenChange={setOpen}>
        <PopoverTrigger testId="trigger">Toggle</PopoverTrigger>
        <PopoverContent testId="surface">
          <Body />
        </PopoverContent>
      </Popover>
    </ThemeProvider>
  ), root);
}
`;

const textOf = current =>
  current.tag === "#text" ? current.props.get("text") ?? "" : current.children.map(textOf).join("");

async function mounted(t) {
  const harness = createHost();
  const { animating: busy, animatingQueries: queries } = harness;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  const clock = createClock(t, harness);
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  const surface = () => harness.byTestId("surface");
  const state = () => surface()?.props.get("data-state");
  const counter = () => textOf(harness.byTestId("counter"));
  const event = (target, type) => harness.dispatch(target, type);
  return { clock, queries, busy, harness, mod, root, surface, state, counter, event,
    finish: () => { const end = dispose; dispose = undefined; end?.(); } };
}

test("presence mounts on open, defers its exit check past a styled frame and removes an idle surface", async t => {
  const { clock, queries, mod, surface, state, counter, event, harness } = await mounted(t);
  assert.equal(surface(), undefined);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(queries.length, 0);
  mod.setOpen(true);
  harness.flush();
  assert.ok(surface());
  assert.equal(state(), "open");
  mod.setOpen(false);
  harness.flush();
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
  harness.flush();
  assert.ok(surface());
  assert.notEqual(surface(), first);
  assert.equal(counter(), "0");
  assert.equal(state(), "open");
});

test("a busy surface is retained until own lifecycle event reports all motion gone", async t => {
  const { clock, queries, busy, mod, surface, event, harness } = await mounted(t);
  mod.setOpen(true);
  harness.flush();
  const current = surface();
  busy.set(current, true);
  mod.setOpen(false);
  harness.flush();
  clock.frame();
  clock.advance(0);
  assert.equal(surface(), current, "busy surface survives the styled-frame check");
  assert.equal(queries.length, 1);
  event(current, "onAnimationEnd");
  assert.equal(surface(), current);
  assert.equal(queries.length, 2);
  busy.set(current, false);
  const counter = surface().children[0];
  harness.dispatch(counter, "AnimationEnd");
  assert.equal(surface(), current, "a descendant's event does not own the exit");
  assert.equal(queries.length, 2, "descendant event must not query");
  harness.dispatch(current, "TransitionCancel");
  assert.equal(surface(), undefined);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.log.cleanups, 1);
});

test("reopen cancels a queued exit and stale callbacks cannot close the new exit", async t => {
  const { clock, queries, busy, mod, surface, state, counter, harness } = await mounted(t);
  mod.setOpen(true);
  harness.flush();
  harness.dispatch(harness.byTestId("counter"), "Click");
  assert.equal(counter(), "1");
  mod.setOpen(false);
  harness.flush();
  const stale = [...clock.frames.values(), ...[...clock.timers.values()].map(timer => timer.callback)];
  mod.setOpen(true);
  harness.flush();
  const current = surface();
  assert.equal(state(), "open");
  assert.equal(counter(), "1");
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  for (const callback of stale) callback();
  assert.equal(surface(), current);
  assert.equal(queries.length, 0);
  mod.setOpen(false);
  harness.flush();
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
  const { clock, queries, mod, surface, finish, harness } = await mounted(t);
  mod.setOpen(true);
  harness.flush();
  mod.setOpen(false);
  harness.flush();
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
  const { clock, queries, mod, surface, finish, harness } = await mounted(t);
  mod.setOpen(true);
  harness.flush();
  mod.setOpen(false);
  harness.flush();
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
