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
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-portal-"));
  await mkdir(path.join(project, "src"));
  await copyFile(fileURLToPath(new URL("./fixtures/portal.tsx", import.meta.url)), path.join(project, "src/main.tsx"));
  await writeFile(path.join(project, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await buildApplication("portal", { appDirectory: project });
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
const press = (x, y) => ({ id: id(), op: "pointer.click", surface: "main", position: { x, y }, button: "primary" });
const key = (name, modifiers) => ({ id: id(), op: "key.press", surface: "main", key: name, ...(modifiers ? { modifiers } : {}) });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `portal-${name}`,
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

test("portal bands stack, events follow the overlay tree, and removal recreates nodes", async () => {
  const { out } = await run([
    capture("initial", 0),
    check("toast", "text", "toast kept 0"), check("modal", "text", "modal kept 0"),
    press(350, 170),
    check("last", "text", "toast"), check("toast", "text", "toast kept 1"),
    check("modal", "text", "modal kept 0"), check("bubbles", "text", "0"), capture("toast-hit"),
    click("toggle-top"), check("toast", "exists", false),
    press(350, 170),
    check("last", "text", "modal"), check("modal", "text", "modal kept 1"),
    check("bubbles", "text", "0"), capture("modal-hit"),
    click("toggle-top"), check("toast", "text", "toast kept 0"), capture("remounted"),
    click("focus-open"), check("focus-first", "focused", true), capture("focused"),
    key("Tab", ["Shift"]), check("focus-close", "focused", true),
    key("Enter"), check("focus-open", "focused", true), check("focus-scope", "exists", false),
    click("focus-open"), check("focus-first", "focused", true),
    click("toggle-source"),
    check("source", "exists", false), check("toast", "exists", false), check("modal", "exists", false),
    check("focus-scope", "exists", false), check("toggle-source", "focused", true), capture("removed"),
    click("toggle-source"),
    check("toast", "text", "toast kept 0"), check("modal", "text", "modal kept 0"),
    check("focus-first", "focused", true), check("bubbles", "text", "0"), capture("restored"),
  ], "bands");

  const initial = await tree(out, "initial");
  const source = get(initial, "source");
  for (const name of ["toast", "modal"]) {
    const node = get(initial, name);
    assert.ok(Math.abs(node.boundsLogical.x - 300) <= 0.1, `${name} x ${JSON.stringify(node.boundsLogical)}`);
    assert.ok(Math.abs(node.boundsLogical.y - 140) <= 0.1, `${name} y ${JSON.stringify(node.boundsLogical)}`);
    assert.ok(Math.abs(node.boundsLogical.width - 160) <= 0.1, `${name} width`);
    assert.ok(Math.abs(node.boundsLogical.height - 72) <= 0.1, `${name} height`);
    assert.ok(node.boundsLogical.x >= source.boundsLogical.x + source.boundsLogical.width,
      `${name} must escape the clipped source bounds`);
  }

  const modalHit = await tree(out, "modal-hit");
  const remounted = await tree(out, "remounted");
  const focused = await tree(out, "focused");
  const restored = await tree(out, "restored");
  const modalToken = get(initial, "modal").token;
  for (const capture of [modalHit, remounted, focused]) {
    assert.equal(get(capture, "modal").token, modalToken, "modal keeps its native node while mounted");
  }
  assert.notEqual(get(remounted, "toast").token, get(initial, "toast").token,
    "a remounted toast portal must build a fresh native node");
  for (const name of ["toast", "modal", "focus-first", "source"]) {
    assert.notEqual(get(restored, name).token, get(focused, name).token,
      `${name} must be rebuilt after the source ancestor was removed`);
  }
  for (const capture of [focused, restored]) {
    const sourceToken = get(capture, "source").token;
    const portals = capture.nodes.filter(node => node.kind === "portal");
    assert.ok(portals.length > 0, "portal nodes must appear in the logical tree");
    for (const node of portals) {
      assert.equal(node.parent, sourceToken, "portal logical parent is its written position");
    }
  }
});
