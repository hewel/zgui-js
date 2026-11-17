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
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-dismiss-"));
  await mkdir(path.join(project, "src"));
  await copyFile(fileURLToPath(new URL("./fixtures/dismiss.tsx", import.meta.url)), path.join(project, "src/main.tsx"));
  await writeFile(path.join(project, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await buildApplication("dismiss", { appDirectory: project });
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
const key = (name, modifiers) => ({ id: id(), op: "key.press", surface: "main", key: name, ...(modifiers ? { modifiers } : {}) });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `dismiss-${name}`,
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

test("native dismissal composes with portals, focus, leaving and source ownership", async () => {
  const { out } = await run([
    capture("initial", 0), check("lower", "exists", false), check("dismisses", "text", "0"),
    click("launch"), check("lower-first", "focused", true),
    click("upper-open"), check("upper-first", "focused", true), capture("nested"),
    click("upper-open"), check("upper", "exists", false), check("lower", "exists", true),
    check("dismisses", "text", "0"), check("last", "text", "none"),
    click("upper-open"), check("upper-first", "focused", true),
    click("lower-first"), check("upper", "exists", false), check("lower", "exists", true),
    check("lower-first", "focused", true), check("dismisses", "text", "1"),
    check("last", "text", "upper:outside-press"), capture("lower-only"),
    click("outside"), check("lower", "exists", false), check("outside", "focused", true),
    check("presses", "text", "1"), check("clicks", "text", "1"),
    check("dismisses", "text", "2"), check("last", "text", "lower:outside-press"), capture("outside-closed"),
    click("launch"), click("upper-open"), click("upper-lock"), click("outside"),
    check("upper", "exists", true), check("lower", "exists", true), check("dismisses", "text", "2"),
    check("presses", "text", "2"), check("clicks", "text", "2"), check("outside", "focused", true),
    key("Escape"), check("dismisses", "text", "2"), check("unhandled", "text", "1"), capture("locked"),
    click("upper-lock"), key("Escape"),
    check("upper-state", "text", "leaving"), check("lower", "exists", true),
    check("dismisses", "text", "3"), check("last", "text", "upper:escape"), capture("leaving"),
    click("lower-first"), check("upper", "exists", false), check("lower", "exists", true),
    check("dismisses", "text", "4"), check("last", "text", "upper:outside-press"),
    click("upper-open"), key("Escape"), check("upper-state", "text", "leaving"),
    check("dismisses", "text", "5"), key("Escape"),
    check("lower", "exists", false), check("upper", "exists", false), check("launch", "focused", true),
    check("dismisses", "text", "6"), check("last", "text", "lower:escape"),
    check("unhandled", "text", "1"), capture("escaped"),
    click("launch"), click("upper-open"), click("upper-lock"),
    click("source-toggle"), check("source", "exists", false), check("lower", "exists", false),
    check("upper", "exists", false), check("source-toggle", "focused", true), check("dismisses", "text", "6"),
    capture("source-removed"), key("Escape"), check("unhandled", "text", "2"),
    click("source-toggle"), check("lower", "exists", true), check("upper", "exists", true),
    check("upper-first", "focused", true), capture("restored"),
    click("upper-lock"), click("outside"), check("upper", "exists", false), check("lower", "exists", true),
    check("dismisses", "text", "7"), click("outside"), check("lower", "exists", false),
    check("dismisses", "text", "8"), check("presses", "text", "4"), check("clicks", "text", "4"),
    check("outside", "focused", true), capture("complete"),
  ], "native");
  const nested = await tree(out, "nested");
  const restored = await tree(out, "restored");
  for (const id of ["source", "lower", "upper", "upper-first"]) {
    assert.notEqual(get(restored, id).token, get(nested, id).token, `${id} remounts as a fresh native node`);
  }
});

test("active nested dismissal leases clean up on session exit", async () => {
  await run([click("launch"), click("upper-open"), check("upper-first", "focused", true),
    check("lower", "exists", true), check("upper", "exists", true), capture("active-exit")], "active-exit");
});
