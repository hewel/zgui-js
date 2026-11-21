import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

function installFakeTimers(t) {
  const originalSet = globalThis.setTimeout;
  const originalClear = globalThis.clearTimeout;
  let nextId = 1;
  let now = 0;
  const scheduled = new Map();
  globalThis.setTimeout = (fn, delay = 0) => {
    const id = nextId++;
    scheduled.set(id, { fn, at: now + delay });
    return id;
  };
  globalThis.clearTimeout = id => {
    scheduled.delete(id);
  };
  t.after(() => {
    globalThis.setTimeout = originalSet;
    globalThis.clearTimeout = originalClear;
  });
  return {
    get count() { return scheduled.size; },
    advance(ms) {
      const target = now + ms;
      while (true) {
        let earliestId;
        let earliest;
        for (const [id, entry] of scheduled) {
          if (entry.at <= target && (!earliest || entry.at < earliest.at)) {
            earliestId = id;
            earliest = entry;
          }
        }
        if (!earliest) break;
        now = earliest.at;
        scheduled.delete(earliestId);
        earliest.fn();
      }
      now = target;
    },
  };
}

test("createDelay, createTimeout, and createInterval schedule, pause, and cancel cleanly on owner disposal", async t => {
  const timers = installFakeTimers(t);
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(t, `
    import { createDelay, createInterval, createSignal, createTimeout, renderApp } from "@hewel/zgui";
    import type { DelayHandle, NativeNode } from "@hewel/zgui";

    const [period, setPeriod] = createSignal<number | false>(100);
    export const log: string[] = [];
    export let delayHandle!: DelayHandle;
    export { setPeriod };

    export function mount(root: NativeNode) {
      return renderApp(() => {
        delayHandle = createDelay();
        createTimeout(() => log.push("timeout"), 50);
        createInterval(() => log.push("tick"), period);
        return <text>timers</text>;
      }, root);
    }
  `);
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try { dispose?.(); }
    finally { delete globalThis.__zgui; }
  });

  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  assert.equal(timers.count, 2, "one-shot timeout and interval armed on mount");

  // Debouncing with createDelay cancels earlier pending call.
  mod.delayHandle.schedule(30, () => mod.log.push("first"));
  assert.equal(mod.delayHandle.pending(), true);
  mod.delayHandle.schedule(40, () => mod.log.push("second"));
  timers.advance(45);
  assert.deepEqual(mod.log, ["second"]);
  assert.equal(mod.delayHandle.pending(), false);

  // One-shot timeout fires at 50ms, interval fires at 100ms and 200ms.
  timers.advance(160);
  assert.deepEqual(mod.log, ["second", "timeout", "tick", "tick"]);

  // Pausing reactive interval clears timer; resuming re-arms with new period.
  h.runTurn(() => mod.setPeriod(false));
  assert.equal(timers.count, 0);
  timers.advance(300);
  assert.deepEqual(mod.log, ["second", "timeout", "tick", "tick"]);

  h.runTurn(() => mod.setPeriod(25));
  timers.advance(55);
  assert.deepEqual(mod.log, ["second", "timeout", "tick", "tick", "tick", "tick"]);

  // Validation rejects invalid delays.
  assert.throws(() => mod.delayHandle.schedule(-1, () => {}), RangeError);
  assert.throws(() => mod.delayHandle.schedule(Number.NaN, () => {}), RangeError);

  // Owner disposal cancels all pending timers.
  mod.delayHandle.schedule(500, () => mod.log.push("after-dispose"));
  dispose();
  dispose = undefined;
  assert.equal(timers.count, 0, "disposing owner cancels every active timer");
  timers.advance(1000);
  assert.deepEqual(mod.log, ["second", "timeout", "tick", "tick", "tick", "tick"]);
});
