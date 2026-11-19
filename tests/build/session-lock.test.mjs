import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const source = `
import { For, SessionLockSurface, getSessionGeneration, onCleanup, renderShell, requestSessionLock, useMonitors } from "@hewel/zgui";
let lock;
let duringCleanup;
let generationDuringCleanup;
export const generation = () => getSessionGeneration();
export const request = () => requestSessionLock();
export const state = () => lock.state();
export const unlock = () => lock.unlock();
export const cleanupError = () => duringCleanup;
export const cleanupGeneration = () => generationDuringCleanup;
export function mount() {
  return renderShell(() => {
    lock = requestSessionLock();
    onCleanup(() => {
      generationDuringCleanup = getSessionGeneration();
      try { lock.unlock(); } catch (error) { duringCleanup = error; }
    });
    const monitors = useMonitors();
    return <For each={monitors()} keyed={monitor => monitor.id}>{monitor =>
      <SessionLockSurface name={"lock-" + monitor().id.toLowerCase()} output={monitor().id}>
        <text testId={monitor().id}>{lock.state()}</text>
      </SessionLockSurface>
    }</For>;
  });
}
`;

const monitor = id => ({ id, name: id, x: 0, y: 0, width: 100, height: 100, scale: 1, refreshRateMilliHz: 60000 });
async function setup(t, options) {
  const mod = await moduleFor(t, source);
  const harness = createHost({ monitors: [monitor("DP-1")], ...options });
  const previous = globalThis.__zguiShell;
  globalThis.__zguiShell = harness.shell;
  t.after(() => {
    if (previous === undefined) delete globalThis.__zguiShell;
    else globalThis.__zguiShell = previous;
  });
  return { harness, mod };
}

test("session lock is denied by default with a typed capability error", async t => {
  const { harness, mod } = await setup(t);
  assert.throws(() => harness.runTurn(mod.mount), { code: "E_CAPABILITY" });
  assert.equal(harness.sessionLockState(), "unlocked");
});

test("lock handles keep read-only IDs that increase across requests and replacement hosts", async t => {
  const { harness, mod } = await setup(t, { sessionLock: "locked" });
  const generation = mod.generation();
  assert.ok(Number.isSafeInteger(generation) && generation > 0);
  const first = harness.runTurn(mod.request);
  const second = harness.runTurn(mod.request);
  const firstId = first.id;
  assert.ok(Number.isSafeInteger(firstId) && firstId > 0);
  assert.ok(Number.isSafeInteger(second.id) && second.id > firstId);
  first.dispose();
  second.dispose();
  assert.equal(Reflect.set(first, "id", 0), false);
  assert.throws(() => Object.defineProperty(first, "id", { value: 0 }), TypeError);
  assert.equal(first.id, firstId);
  assert.equal(mod.generation(), generation);
  harness.beginStop();

  const successor = createHost({ sessionLock: "locked" });
  globalThis.__zguiShell = successor.shell;
  assert.ok(mod.generation() > generation);
  const third = successor.runTurn(mod.request);
  assert.ok(Number.isSafeInteger(third.id) && third.id > second.id);
  third.dispose();
  successor.beginStop();
});

test("lock subscriptions with a shared observer are disposed independently", async t => {
  const { harness } = await setup(t, { sessionLock: "locked" });
  const states = [];
  const observe = state => states.push(state);
  const [first, second] = harness.runTurn(() => [
    harness.shell.requestSessionLock(observe),
    harness.shell.requestSessionLock(observe),
  ]);
  harness.flush();
  assert.deepEqual(states, ["unlocked", "unlocked", "pending", "pending", "locked", "locked"]);

  first.unsubscribe();
  assert.throws(() => harness.runTurn(first.unlock), ReferenceError);
  harness.runTurn(second.unlock);
  harness.flush();
  assert.deepEqual(states.slice(6), ["unlocked"]);
  assert.throws(() => Object.defineProperty(first, "id", { value: 0 }), TypeError);
  second.unsubscribe();
  harness.beginStop();
});

test("a synchronous lock observer failure consumes its request ID", async t => {
  const { harness } = await setup(t, { sessionLock: "locked" });
  harness.runTurn(() => {
    const first = harness.shell.requestSessionLock(() => {});
    const failure = new Error("initial observer failed");
    assert.throws(() => harness.shell.requestSessionLock(() => { throw failure; }), error => error === failure);
    const next = harness.shell.requestSessionLock(() => {});
    assert.equal(next.id, first.id + 2);
    first.unsubscribe();
    next.unsubscribe();
  });
  harness.flush();
  harness.beginStop();
});

test("lock surfaces follow hotplug and only explicit unlock releases the lock", async t => {
  const { harness, mod } = await setup(t, { sessionLock: "locked" });
  const dispose = harness.runTurn(mod.mount);
  assert.equal(mod.state(), "unlocked", "a bridge request does not report acquisition synchronously");
  harness.flush();
  assert.equal(mod.state(), "locked");
  const first = harness.byTestId("DP-1");
  assert.ok(first, "the first output has its own mount region");
  harness.emitMonitors([monitor("DP-1"), monitor("DP-2")]);
  harness.flush();
  assert.equal(harness.byTestId("DP-1"), first, "hotplug does not remount unchanged outputs");
  assert.ok(harness.byTestId("DP-2"), "the hotplugged output mounted");
  harness.emitMonitors([monitor("DP-2")]);
  harness.flush();
  assert.equal(harness.byTestId("DP-1"), undefined);
  harness.runTurn(mod.unlock);
  harness.flush();
  assert.deepEqual(harness.sessionLockLog, ["lock", "locked", "unlock", "sync"]);
  assert.equal(mod.state(), "unlocked");
  assert.equal(harness.surfaces.size, 0);
  harness.beginStop();
  harness.runTurn(dispose);
});

test("refusal destroys only the unacquired request", async t => {
  const refused = await setup(t, { sessionLock: "finished" });
  const disposeRefused = refused.harness.runTurn(refused.mod.mount);
  refused.harness.flush();
  assert.equal(refused.mod.state(), "refused");
  assert.equal(refused.harness.surfaces.size, 0);
  refused.harness.beginStop();
  refused.harness.runTurn(disposeRefused);
  assert.deepEqual(refused.harness.sessionLockLog, ["lock", "destroy"]);
});

test("retirement revokes unlock authority but keeps the platform locked", async t => {
  const locked = await setup(t, { sessionLock: "locked" });
  const generation = locked.mod.generation();
  const disposeLocked = locked.harness.runTurn(locked.mod.mount);
  locked.harness.flush();
  locked.harness.beginStop();
  locked.harness.runTurn(disposeLocked);
  locked.harness.flush();
  assert.equal(locked.mod.cleanupGeneration(), generation, "cleanup sees the stopping session's generation");
  assert.ok(locked.mod.cleanupError() instanceof TypeError, "cleanup cannot request unlock");
  assert.equal(locked.harness.surfaces.size, 0);
  assert.equal(locked.harness.sessionLockState(), "locked");
  assert.deepEqual(locked.harness.sessionLockLog, ["lock", "locked"]);
});
