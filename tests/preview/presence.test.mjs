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
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-presence-"));
  await mkdir(path.join(project, "src"));
  await copyFile(fileURLToPath(new URL("./fixtures/presence.tsx", import.meta.url)), path.join(project, "src/main.tsx"));
  await writeFile(path.join(project, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await buildApplication("presence", { appDirectory: project });
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
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `presence-${name}`,
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
  return { result, out };
}

const tree = async (out, name) => JSON.parse(await readFile(path.join(out, `captures/${name}.tree.json`), "utf8"));
const get = (tree, testId) => {
  const node = tree.nodes.find(node => node.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};

test("presence keeps a surface through its exit animation and releases it on its last motion", async () => {
  const { out } = await run([
    capture("initial", 0), check("surface", "exists", false),
    click("open"), advance(100), click("counter"), check("counter", "text", "1"), capture("opened"),
    click("close"), check("surface", "exists", true), check("state", "text", "closed"), capture("leaving"),
    advance(80), check("surface", "exists", true), capture("mid-exit"),
    click("open"), check("state", "text", "open"), check("counter", "text", "1"),
    advance(200), check("surface", "exists", true), capture("reopened"),
    click("close"), advance(220), check("surface", "exists", false), capture("closed"),
    click("open"), advance(100), check("counter", "text", "0"), capture("fresh"),
    click("close"), advance(40), click("none"), check("surface", "exists", false), capture("cancelled"),
    click("open"), click("close"), check("surface", "exists", false),
    click("zero"), click("open"), click("close"), check("surface", "exists", false), capture("zero"),
    click("forever"), click("open"), click("close"), advance(900), check("surface", "exists", true),
    advance(120), check("surface", "exists", false), capture("deadline"),
    click("animated"), click("open"), advance(100), click("close"), click("source-toggle"),
    check("surface", "exists", false), advance(1200), check("surface", "exists", false), capture("removed"),
    click("source-toggle"), check("surface", "exists", false), click("open"), advance(100),
    check("surface", "exists", true), check("counter", "text", "0"), capture("remounted"),
  ], "lifecycle");
  const opened = await tree(out, "opened");
  const leaving = await tree(out, "leaving");
  const midExit = await tree(out, "mid-exit");
  const reopened = await tree(out, "reopened");
  const fresh = await tree(out, "fresh");
  const remounted = await tree(out, "remounted");
  for (const name of ["surface", "counter"]) {
    const token = get(opened, name).token;
    assert.equal(get(leaving, name).token, token, `${name} remounted while leaving`);
    assert.equal(get(midExit, name).token, token, `${name} remounted mid-exit`);
    assert.equal(get(reopened, name).token, token, `${name} remounted on reopen`);
    assert.notEqual(get(fresh, name).token, token, `${name} not rebuilt after full removal`);
    assert.notEqual(get(remounted, name).token, get(fresh, name).token, `${name} not rebuilt on remount`);
  }
});

test("a presence leaving with active timers tears its session down cleanly", async () => {
  await run([click("open"), advance(100), click("close"), check("surface", "exists", true),
    capture("active-exit")], "active-exit");
});
