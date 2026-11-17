import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

let project;
before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-pointer-boundaries-"));
  await mkdir(path.join(project, "src"));
  await copyFile(fileURLToPath(new URL("./fixtures/pointer-boundaries.tsx", import.meta.url)), path.join(project, "src/main.tsx"));
  await writeFile(path.join(project, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await buildApplication("pointer-boundaries", { appDirectory: project });
});
after(async () => {
  if (!project) return;
  if (process.env.ZGUI_KEEP_PREVIEW_OUTPUT === "1") {
    console.log(`retained preview project: ${project}`);
  } else {
    await rm(project, { recursive: true, force: true });
  }
});

let serial = 0;
const id = () => `step-${++serial}`;
const key = (name, modifiers) => ({ id: id(), op: "key.press", surface: "main", key: name, ...(modifiers ? { modifiers } : {}) });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const move = (x, y) => ({ id: id(), op: "pointer.move", surface: "main", position: { x, y } });
const emptyResources = { nodes: 0, wrappers: 0, callbacks: 0, focusTraps: 0, scrollLocks: 0, timers: 0,
  frameCallbacks: 0, requests: 0, engineAlive: false };

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `pointer-boundaries-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "input.keyboard", "tree.inspect"],
    steps,
  }));
  const process = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "run",
    "--bundle", path.join(project, "dist"),
    "--profile", path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario", scenario,
    "--out", out,
  ], { cwd: workspaceRoot, encoding: "utf8", timeout: 60000 });
  if (process.stderr) await writeFile(path.join(project, `${name}.stderr`), process.stderr);
  assert.ifError(process.error);
  const result = JSON.parse(process.stdout);
  assert.equal(process.signal, null, process.stderr);
  assert.equal(process.status, result.exitCode);
  assert.equal(result.exitCode, 0, process.stdout);
  assert.equal(result.status, "passed", process.stdout);
  assert.equal(result.assertions.failed, 0);
  assert.equal(result.assertions.passed, steps.filter(step => step.op === "assert").length);
  assert.equal(result.teardown.clean, true);
  assert.deepEqual(result.teardown.resources, emptyResources);
  return { result, out };
}

const tree = async (out, name) => JSON.parse(await readFile(path.join(out, `captures/${name}.tree.json`), "utf8"));
const get = (tree, testId) => {
  const node = tree.nodes.find(node => node.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};

test("native pointer boundaries preserve subtree semantics, snapshots and owner cleanup", async () => {
  const { out } = await run([
    capture("initial", 0), key("Tab"), check("toggle", "focused", true),
    move(105, 105), check("region-count", "text", "1/0"), check("a-count", "text", "0/0"), check("b-count", "text", "0/0"),
    check("last", "text", "pointerenter|105,105|mouse"),
    move(150, 140), check("region-count", "text", "1/0"), check("a-count", "text", "1/0"), check("capture", "text", "capture|true"),
    check("last", "text", "pointerenter|150,140|mouse"),
    move(250, 140), check("region-count", "text", "1/0"), check("a-count", "text", "1/1"), check("b-count", "text", "1/0"), capture("inside"),
    move(380, 280), check("region-count", "text", "1/1"), check("b-count", "text", "1/1"), capture("outside"),
    move(150, 140), check("region-count", "text", "2/1"), check("a-count", "text", "2/1"), check("toggle", "focused", true),
    key("Space"), check("region", "exists", false), check("retained", "text", "pointerenter|150,140"), check("authority", "text", "expired"), capture("removed"),
    move(20, 200), check("region-count", "text", "2/1"), check("a-count", "text", "2/1"),
    key("Space"), check("region", "exists", true), move(250, 140), check("region-count", "text", "3/1"), check("b-count", "text", "2/1"), capture("remounted")
  ], "native");
  const inside = await tree(out, "inside"), remounted = await tree(out, "remounted");
  for (const testId of ["region", "a", "b"]) assert.notEqual(get(inside, testId).token, get(remounted, testId).token);
  const contains = (rect, x, y) => x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height;
  assert.ok(contains(get(inside, "region").boundsLogical, 105, 105));
  assert.ok(!contains(get(inside, "a").boundsLogical, 105, 105));
  assert.ok(contains(get(inside, "a").boundsLogical, 150, 140));
  assert.ok(contains(get(inside, "b").boundsLogical, 250, 140));
  assert.ok(!contains(get(inside, "region").boundsLogical, 380, 280));
});
