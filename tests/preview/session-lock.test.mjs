import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { after, before, test } from "node:test";
import { buildFixtureApp, createTempWorkspace, emptyResources, getNode, readPng, readTree, runScenario, writeJson } from "./harness.mjs";

let temp;
let bundle;
let refusal;
before(async () => {
  temp = await createTempWorkspace("preview-session-lock-");
  refusal = path.join(temp.dir, "refusal.json");
  await writeJson(refusal, { schemaVersion: 1, id: "lock-refusal", services: { sessionLock: { outcome: "finished" } }, events: [] });
  bundle = await buildFixtureApp(path.join(temp.dir, "authorized"), "authorized-lock", `
    import { Button, For, SessionLockSurface, Show, createSignal, onCleanup, renderApp, requestSessionLock, useMonitors } from "@hewel/zgui";
    import type { NativeNode, SessionLockRequest } from "@hewel/zgui";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => {
        const [lock, setLock] = createSignal<SessionLockRequest>();
        const monitors = useMonitors();
        let unlockDuringCleanup = false;
        onCleanup(() => { if (unlockDuringCleanup) lock()?.unlock(); });
        return <column>
          <Button testId="start" onClick={() => setLock(requestSessionLock())}>Request lock</Button>
          <text testId="state">{lock()?.state() ?? "unlocked"}</text>
          <Show when={lock()}>{request => <For each={monitors().filter(monitor => monitor.id !== "spare")} keyed={monitor => monitor.id}>{monitor =>
            <SessionLockSurface name={"lock-" + monitor().id} output={monitor().id}>
              <column style={{ backgroundColor: "#244062", width: "100%", height: "100%" }}>
                <text testId="state">{request().state()}:{monitor().id}</text>
                <Button testId="unlock" onClick={() => request().unlock()}>Unlock</Button>
                <Button testId="throw" onClick={() => { request().unlock(); throw new Error("lock UI fault"); }}>Throw</Button>
                <Button testId="budget" onClick={() => { for (;;) {} }}>Budget</Button>
                <Button testId="arm" onClick={() => { unlockDuringCleanup = true; }}>Arm cleanup</Button>
              </column>
            </SessionLockSurface>
          }</For>}</Show>
        </column>;
      }, root);
    }
  `, { rawSource: true, capabilities: { sessionLock: true } });
});
after(async () => { await temp?.cleanup(); });

const left = { id: "left", name: "Left", x: 0, y: 0, width: 320, height: 240, scale: 1, refreshRateMilliHz: 60000 };
const right = { ...left, id: "right", name: "Right", x: 320, width: 400, scale: 2 };
const spare = { ...left, id: "spare", name: "Spare", x: 720 };
const monitors = (id, ...monitors) => ({ id, op: "monitors.set", monitors });
const capture = (id, surface = "main") => ({ id, op: "capture", surface, captureId: id });
const click = (surface, testId) => ({ id: testId, op: "pointer.click", target: { surface, testId }, button: "primary" });
const advance = id => ({ id, op: "clock.advance", byMs: 40 });
const start = [monitors("initial-outputs", left, spare), click("main", "start"), advance("dispatch-start")];

function run(name, steps, options = {}) {
  return runScenario({ projectDir: temp.dir, bundle, name, steps,
    requiredCapabilities: ["session-lock", "surface.multiple", "pixels", "input.pointer", "selectors.test-id", "virtual-time"],
    width: 400, height: 240, scale: 1,
    ...options });
}

function verifyLock(snapshot, state, unlockCount, coveredOutputs = [], fallbackOutputs = coveredOutputs) {
  assert.deepEqual(snapshot, { state, unlockCount, coveredOutputs, fallbackOutputs });
}

async function lockTrace(out) {
  const trace = JSON.parse(await readFile(path.join(out, "services.json"), "utf8"));
  return trace.records.findLast(record => record.kind === "sessionLock").detail;
}

async function stepResult(out, id) {
  const steps = (await readFile(path.join(out, "steps.jsonl"), "utf8")).trim().split("\n").map(JSON.parse);
  return steps.find(step => step.id === id).result;
}

test("caught default-denied locking is a sticky E_CAPABILITY in preview", async () => {
  const denied = await buildFixtureApp(path.join(temp.dir, "denied"), "denied-lock", `
    import { renderApp, requestSessionLock } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => {
        try { requestSessionLock(); } catch (error) {
          if (!(error instanceof Error && "code" in error && error.code === "E_CAPABILITY")) throw error;
        }
        return <text>caught</text>;
      }, root);
    }
  `, { rawSource: true });
  const { result } = await run("denied", [capture("denied")], { bundle: denied, allowFailure: true });
  assert.equal(result.firstError?.code, "E_CAPABILITY");
  assert.match(result.firstError?.message, /sessionLock/);
  assert.equal(result.captureCount, 0);
});

test("confirmed lock uses distinct output regions, covers undeclared outputs, and explicitly unlocks", async () => {
  const { out, result } = await run("unlock", [
    ...start, capture("left-locked", "lock-left"), monitors("hotplug", left, right, spare),
    advance("present-hotplug"), capture("right-locked", "lock-right"), monitors("remove-left", right, spare),
    advance("present-after-removal"), click("lock-right", "unlock"), advance("dispatch-unlock"), capture("unlocked"),
  ]);
  for (const [output, width] of [["left", 320], ["right", 400]]) {
    const file = `surfaces/lock-${output}/${output}-locked`;
    const tree = await readTree(out, file);
    assert.equal(getNode(tree, "state").text, `locked:${output}`);
    const image = await readPng(path.join(out, `captures/${file}.png`));
    assert.equal(image.width, width);
  }
  const removed = await stepResult(out, "remove-left");
  assert.deepEqual(removed.surfaces.toSorted(), ["lock-right", "main"]);
  verifyLock(removed.readiness.sessionLock, "locked", 0, ["right", "spare"], ["spare"]);
  assert.equal(getNode(await readTree(out, "unlocked"), "state").text, "unlocked");
  verifyLock(await lockTrace(out), "unlocked", 1);
  assert.deepEqual(result.teardown.resources, emptyResources);
});

test("compositor refusal never attaches lock regions or sends unlock", async () => {
  const { out, result } = await run("refused", [...start, capture("refused")], { fixture: refusal });
  assert.equal(getNode(await readTree(out, "refused"), "state").text, "refused");
  verifyLock(await lockTrace(out), "refused", 0);
  assert.deepEqual(result.teardown.resources, emptyResources);
});

for (const [action, code] of [["throw", "E_JS_EXCEPTION"], ["budget", "E_JOB_BUDGET"], ["arm", "E_TEARDOWN"]]) {
  test(`${code} keeps Rust fallback coverage without unlocking`, async () => {
    const { out, result } = await run(action, [
      ...start, capture("locked", "lock-left"), click("lock-left", action), advance("dispatch-action"),
    ], { allowFailure: true });
    assert.equal(result.captureCount, 1, "the lock UI must have presented before fault injection");
    assert.equal(result.firstError?.code, code);
    if (code === "E_TEARDOWN") assert.match(result.firstError.message, /disposal/);
    verifyLock(await lockTrace(out), "locked", 0, ["left", "spare"]);
    assert.deepEqual(result.teardown.resources, emptyResources);
  });
}

test("surfaces lost retires JavaScript but does not release the platform lock", async () => {
  const { out, result } = await run("retire", [
    ...start, capture("locked", "lock-left"), { id: "retire", op: "surface.recreate", surface: "main" },
  ]);
  const recreated = await stepResult(out, "retire");
  assert.equal(recreated.retired.clean, true);
  verifyLock(recreated.readiness.sessionLock, "locked", 0, ["left", "spare"]);
  verifyLock(await lockTrace(out), "locked", 0, ["left", "spare"]);
  assert.equal(result.generation, 2);
});
