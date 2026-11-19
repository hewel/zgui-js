import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const source = `
import { For, SessionLockSurface, onCleanup, renderShell, requestSessionLock, useMonitors } from "@hewel/zgui";
let lock;
let duringCleanup;
export const state = () => lock.state();
export const unlock = () => lock.unlock();
export const cleanupError = () => duringCleanup;
export function mount() {
  return renderShell(() => {
    lock = requestSessionLock();
    onCleanup(() => { try { lock.unlock(); } catch (error) { duringCleanup = error; } });
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
  const disposeLocked = locked.harness.runTurn(locked.mod.mount);
  locked.harness.flush();
  locked.harness.beginStop();
  locked.harness.runTurn(disposeLocked);
  locked.harness.flush();
  assert.ok(locked.mod.cleanupError() instanceof TypeError, "cleanup cannot request unlock");
  assert.equal(locked.harness.surfaces.size, 0);
  assert.equal(locked.harness.sessionLockState(), "locked");
  assert.deepEqual(locked.harness.sessionLockLog, ["lock", "locked"]);
});
