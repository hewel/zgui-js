import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

const json = async file => JSON.parse(await readFile(file, "utf8"));
function native(args) {
  const child = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", ...args], {
    cwd: workspaceRoot, encoding: "utf8", timeout: 25000, maxBuffer: 1024 * 1024,
  });
  assert.ifError(child.error);
  assert.equal(child.signal, null, child.stderr);
  const result = JSON.parse(child.stdout);
  assert.equal(child.status, result.exitCode);
  assert.equal(result.exitCode, 0, `${child.stdout}\n${child.stderr}`);
  return result;
}
const emptyResources = { nodes: 0, wrappers: 0, callbacks: 0, focusTraps: 0, scrollLocks: 0, timers: 0,
  frameCallbacks: 0, requests: 0, engineAlive: false };

// Real GPU/native lifecycle gate: no platform fallback, graphics skip, or synthetic success.
test("surface recreation retires ownership and rebinds input, captures and fixture work", async t => {
  const directory = await mkdtemp(path.join(workspaceRoot, "target/preview-lifecycle-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const app = path.join(directory, "app");
  await mkdir(path.join(app, "src"), { recursive: true });
  await mkdir(path.join(app, "assets"));
  await writeFile(path.join(app, "assets/message.txt"), "delivered");
  await writeFile(path.join(app, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await writeFile(path.join(app, "src/main.tsx"), `
    import { Button, createSignal, renderApp, type NativeNode } from "@hewel/zgui";
    import { readTextAsset } from "@hewel/zgui-services";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      let retired = false;
      const [count, setCount] = createSignal(0);
      const [asset, setAsset] = createSignal("pending");
      const random = String(Math.random());
      const mountedAt = String(Date.now());
      const live = () => { if (retired) throw new Error("retired-generation-entered"); };
      readTextAsset("assets/message.txt").promise.then(value => { live(); setAsset(value); });
      const dispose = renderApp(() => <column>
        <Button testId="increment" onClick={() => setCount(count() + 1)}>Increment</Button>
        <Button testId="arm" onClick={() => { setTimeout(() => { throw new Error("retired-timer-fired"); }, 100); }}>Arm retired work</Button>
        <text testId="count">{String(count())}</text>
        <text testId="asset">{asset()}</text>
        <text testId="random">{random}</text>
        <text testId="mounted-at">{mountedAt}</text>
      </column>, root);
      return () => { retired = true; dispose(); };
    }
  `);
  await buildApplication("surface-lifecycle-regression", { appDirectory: app });
  const fixture = path.join(directory, "fixture.json");
  await writeFile(fixture, JSON.stringify({ schemaVersion: 1, id: "lifecycle", services: {
    assets: { provider: "mock-text-assets-v1", latencyMs: 200,
      initialState: { assets: { "assets/message.txt": {} } } },
  }, events: [{ atMs: 100, channel: "assets.state", value: { assets: {} } }] }));
  const profile = await json(path.join(workspaceRoot, "preview/profiles/default.json"));
  const steps = [
    { op: "surface.resize", surface: "main", width: 400, height: 300 },
    { op: "surface.scale", surface: "main", scale: 2 },
    { op: "clock.advance", byMs: 20 },
    { op: "capture", captureId: "before", requiredFrameTimeUs: 20000 },
    { op: "pointer.click", target: { surface: "main", testId: "arm" }, button: "primary" },
    { op: "surface.recreate", surface: "main" },
    { op: "capture", captureId: "resumed", requiredFrameTimeUs: 20000 },
    { op: "pointer.click", target: { surface: "main", role: "button", name: "Increment" }, button: "primary" },
    { op: "clock.advance", byMs: 220 },
    ...[["count", "1"], ["asset", "delivered"], ["mounted-at", String(profile.clock.epochMs + 20)]].map(([testId, text]) => ({
      op: "assert", target: { surface: "main", testId }, check: { property: "text", equals: text },
    })),
    { op: "assert", serviceCall: { service: "assets", method: "readTextAsset", count: 2 } },
    { op: "capture", captureId: "after", requiredFrameTimeUs: 240000 },
  ];
  const scenario = path.join(directory, "scenario.json");
  await writeFile(scenario, JSON.stringify({ schemaVersion: 1, id: "surface-lifecycle", requiredCapabilities: ["surface.recreate"],
    steps: steps.map((step, index) => ({ id: `action-${index}`, ...step })),
  }));
  const out = path.join(directory, "run");
  const result = native(["run", "--bundle", path.join(app, "dist"), "--profile", "preview/profiles/default.json",
    "--scale", "1", "--fixture", fixture, "--scenario", scenario, "--out", out]);
  assert.equal(result.generation, 2);
  assert.equal(result.captureCount, 3);
  assert.equal(result.teardown.clean, true);
  assert.deepEqual(result.teardown.resources, emptyResources);
  const rows = (await readFile(path.join(out, "steps.jsonl"), "utf8")).trim().split("\n").map(JSON.parse);
  const lifecycle = rows.find(row => row.op === "surface.recreate").result;
  assert.notEqual(lifecycle.oldSurfaceId, lifecycle.newSurfaceId);
  assert.notEqual(lifecycle.oldDocumentId, lifecycle.newDocumentId);
  assert.deepEqual([lifecycle.oldGeneration, lifecycle.generation], [1, 2]);
  assert.deepEqual([lifecycle.fromTimeUs, lifecycle.timeUs], [20000, 20000]);
  assert.equal(lifecycle.retired.clean, true);
  assert.ok(lifecycle.retired.before.nodes > 0);
  assert.ok(lifecycle.retired.before.callbacks > 0);
  assert.equal(lifecycle.retired.before.timers, 1);
  assert.equal(lifecycle.retired.before.requests, 1);
  assert.deepEqual(lifecycle.retired.resources, emptyResources);
  const before = await json(path.join(out, "captures/before.frame.json"));
  const resumed = await json(path.join(out, "captures/resumed.frame.json"));
  const after = await json(path.join(out, "captures/after.frame.json"));
  assert.equal(before.generation, 1);
  for (const frame of [resumed, after]) {
    assert.equal(frame.generation, 2);
    assert.notEqual(frame.documentIdentity, before.documentIdentity);
    assert.equal(frame.scale, 2);
    assert.deepEqual([frame.physicalWidth, frame.physicalHeight], [800, 600]);
    assert.ok(frame.configurationId > before.configurationId);
    assert.ok(frame.pixelPresentation.sequence > before.pixelPresentation.sequence);
    assert.equal(frame.surfaceConfiguration, before.surfaceConfiguration + 1);
  }
  const beforeTree = await json(path.join(out, "captures/before.tree.json"));
  const resumedTree = await json(path.join(out, "captures/resumed.tree.json"));
  const node = (tree, id) => tree.nodes.find(node => node.testId === id);
  assert.notEqual(node(beforeTree, "random").text, node(resumedTree, "random").text, "the run RNG must not restart");
  assert.equal(Number(node(resumedTree, "mounted-at").text) - Number(node(beforeTree, "mounted-at").text), 20);
  const trace = (await json(path.join(out, "services.json"))).records;
  const requests = trace.filter(row => row.kind === "request").map(row => row.detail.requestId);
  assert.equal(requests.length, 2);
  assert.equal(new Set(requests).size, 2, "fixture identities must not alias across generations");
  for (const oldId of requests.slice(0, 1)) {
    assert.ok(trace.some(row => row.kind === "cancellation" && row.detail.requestId === oldId));
    assert.ok(!trace.some(row => row.kind === "response" && row.detail.requestId === oldId));
  }
  assert.equal(trace.filter(row => row.kind === "state").length, 1, "scheduled scenario state survives recreation");
  assert.equal(trace.filter(row => row.kind === "disposal").length, 1, "only whole-run finish disposes fixture history");
  for (const capture of ["before", "resumed", "after"]) {
    const inspected = native(["inspect", "--run", out, "--capture", capture, "--test-id", "count"]);
    assert.equal(inspected.generation, capture === "before" ? 1 : 2);
  }
});
