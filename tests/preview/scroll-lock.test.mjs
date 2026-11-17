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
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-scroll-lock-"));
  await mkdir(path.join(project, "src"));
  await copyFile(fileURLToPath(new URL("./fixtures/scroll-lock.tsx", import.meta.url)), path.join(project, "src/main.tsx"));
  await writeFile(path.join(project, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await buildApplication("scroll-lock", { appDirectory: project });
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
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const wheel = testId => ({ id: id(), op: "wheel", target: { surface: "main", testId }, delta: { x: 0, y: 3, unit: "lines" } });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `scroll-lock-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "input.wheel", "tree.inspect"],
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
  assert.equal(result.teardown.resources.scrollLocks, 0);
  return { result, out };
}

const tree = async (out, name) => JSON.parse(await readFile(path.join(out, `captures/${name}.tree.json`), "utf8"));
const get = (tree, testId) => {
  const node = tree.nodes.find(node => node.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};

test("scroll locks freeze the window root while inner scrollers and geometry stay live", async () => {
  const { out } = await run([
    capture("initial", 0), click("page-save"), check("page-status", "text", "saved"),
    wheel("page"), advance(400), click("page-compare"), check("page-status", "text", "down"), capture("scrolled"),
    click("page-save"), click("outer-lock"), check("holds", "text", "1"),
    click("page-compare"), check("page-status", "text", "unchanged"), capture("outer-locked"),
    wheel("page"), advance(400), click("page-compare"), check("page-status", "text", "unchanged"),
    click("inner-lock"), check("holds", "text", "2"), wheel("page"), advance(400),
    click("page-compare"), check("page-status", "text", "unchanged"), capture("nested-locks"),
    click("inner-release"), check("holds", "text", "1"), wheel("page"), advance(400),
    click("page-compare"), check("page-status", "text", "unchanged"),
    click("inner-save"), click("focus-inner"), check("inner-scroll", "focused", true),
    wheel("inner-scroll"), advance(400), click("inner-compare"), check("inner-status", "text", "down"),
    click("page-compare"), check("page-status", "text", "unchanged"), capture("inner-scrolled"),
    click("outer-release"), check("holds", "text", "0"), wheel("page"), advance(400),
    click("page-compare"), check("page-status", "text", "down"), capture("unlocked"),
    click("page-save"), click("outer-lock"), click("inner-lock"), check("holds", "text", "2"), capture("before-removal"),
    click("source-toggle"), check("holds", "text", "0"), check("outer-lease", "exists", false),
    check("inner-lease", "exists", false), wheel("page"), advance(400),
    click("page-compare"), check("page-status", "text", "down"), capture("owner-removed"),
    click("page-save"), click("source-toggle"), check("holds", "text", "2"),
    wheel("page"), advance(400), click("page-compare"), check("page-status", "text", "unchanged"), capture("remounted"),
  ], "lifecycle");
  const rows = (await readFile(path.join(out, "steps.jsonl"), "utf8")).trim().split("\n").map(JSON.parse);
  const events = rows.filter(row => row.op === "wheel").map(row => row.result);
  assert.equal(events.length, 8);
  for (const event of events) assert.equal(event.nativeHitVerified, true, `wheel target missed native hit verification: ${JSON.stringify(event)}`);
  const initial = await tree(out, "initial");
  const beforeRemoval = await tree(out, "before-removal");
  const ownerRemoved = await tree(out, "owner-removed");
  const remounted = await tree(out, "remounted");
  const pageToken = get(initial, "page").token;
  assert.equal(get(ownerRemoved, "page").token, pageToken, "the page remounted");
  assert.equal(get(remounted, "page").token, pageToken, "the page remounted");
  const outerLease = get(beforeRemoval, "outer-lease").token;
  const innerLease = get(beforeRemoval, "inner-lease").token;
  assert.notEqual(get(remounted, "outer-lease").token, outerLease, "outer lease node was not rebuilt");
  assert.notEqual(get(remounted, "inner-lease").token, innerLease, "inner lease node was not rebuilt");
});

test("a session exiting with active scroll locks tears them down cleanly", async () => {
  await run([click("outer-lock"), click("inner-lock"), check("holds", "text", "2"), capture("active-locks")], "active-locks");
});

test("a root elastic edge stays drawn in place until its scroll lock is released", async () => {
  const at = (y, phase) => ({ id: id(), op: "wheel", surface: "main", position: { x: 200, y: 500 }, delta: { x: 0, y, unit: "pixels" }, phase });
  const { out } = await run([
    capture("elastic-initial", 0), click("page-save"), at(0, "started"), advance(8), at(-60, "moved"),
    click("page-compare"), check("page-status", "text", "up"), click("page-save"), capture("elastic-held"),
    click("outer-lock"), at(0, "ended"), advance(400), click("page-compare"),
    check("page-status", "text", "unchanged"), capture("elastic-frozen"),
    click("outer-release"), advance(800), click("page-compare"), check("page-status", "text", "down"), capture("elastic-thawed"),
  ], "elastic");
  const initial = get(await tree(out, "elastic-initial"), "page").boundsLogical;
  const held = get(await tree(out, "elastic-held"), "page").boundsLogical;
  const frozen = get(await tree(out, "elastic-frozen"), "page").boundsLogical;
  const thawed = get(await tree(out, "elastic-thawed"), "page").boundsLogical;
  assert.ok(held.y > initial.y, "control: the native page is visibly pulled past the top");
  assert.deepEqual(frozen, held, "the frozen edge moved");
  assert.deepEqual(thawed, initial, "thaw did not restore the native resting position");
});

test("wheel phases, coordinates, and deltas deliver native-scale input exactly", async () => {
  const at = (y, phase) => ({ id: id(), op: "wheel", surface: "main", position: { x: 200, y: 500 }, delta: { x: 0, y, unit: "pixels" }, phase });
  const { out } = await run([
    click("page-save"), at(0, "started"), advance(8), at(60, "moved"), advance(8), at(0, "ended"),
    click("page-compare"), check("page-status", "text", "down"), check("page-delta", "text", "-60"), capture("precision"),
  ], "precision");
  const rows = (await readFile(path.join(out, "steps.jsonl"), "utf8")).trim().split("\n").map(JSON.parse);
  const events = rows.filter(row => row.op === "wheel").map(row => row.result);
  assert.deepEqual(events.map(e => e.phase), ["started", "moved", "ended"]);
  assert.deepEqual(events.map(e => e.timeUs), [0, 8000, 16000]);
  assert.deepEqual(events[1].delta, { x: 0, y: 60, unit: "pixels" });
  assert.deepEqual(events[1].positionDevice, { x: 350, y: 875 });
});
