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
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-disclosure-"));
  await mkdir(path.join(project, "src"));
  await copyFile(fileURLToPath(new URL("./fixtures/disclosure.tsx", import.meta.url)), path.join(project, "src/main.tsx"));
  await writeFile(path.join(project, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await buildApplication("disclosure", { appDirectory: project });
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
const wait = byMs => ({ id: id(), op: "clock.advance", byMs });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const edit = text => ({ id: id(), op: "text.commit", surface: "main", text });

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `disclosure-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "input.keyboard", "input.text", "tree.inspect"],
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
const find = (tree, testId) => tree.nodes.find(node => node.testId === testId);
const get = (tree, testId) => {
  const node = find(tree, testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};

test("disclosure scenario exercises animation, editing, keyboard, and lifecycle", async () => {
  const baseline = await run([capture("initial", 0)], "baseline");
  const baselineTree = await tree(baseline.out, "initial");
  const disabledBounds = get(baselineTree, "acc-b").boundsLogical;
  assert.ok(disabledBounds && disabledBounds.width > 0 && disabledBounds.height > 0);
  const disabledPoint = { x: disabledBounds.x + disabledBounds.width / 2, y: disabledBounds.y + disabledBounds.height / 2 };

  const steps = [
    capture("initial", 0), check("c-state", "text", "closed"), check("acc-state", "text", "a"),
    check("acc-b", "enabled", false), check("details-trigger", "role", "button"), check("acc-content-a", "role", "region"),
    { id: id(), op: "pointer.click", surface: "main", position: disabledPoint, button: "primary" },
    check("acc-state", "text", "a"),
    click("before"), key("Tab"), check("details-trigger", "focused", true),
    key("Tab"), check("after", "focused", true),
    click("details-trigger"), wait(300), capture("opened"), check("c-state", "text", "open"),
    click("details-input"), key("Home"), key("End", ["Shift"]), edit("kept"), check("details-input", "value", "kept"),
    click("details-count"), check("details-count", "text", "Count: 1"), capture("edited"),
    click("grow"), wait(80), capture("growing"), wait(220), capture("grown"),
    click("restyle"), click("details-input"), key("Escape"), check("details-trigger", "focused", true),
    check("c-state", "text", "closed"), wait(80), capture("closing"), wait(220), capture("closed"),
    check("details-input", "value", "kept"), check("details-count", "text", "Count: 1"),
    key("Tab"), check("after", "focused", true), key("Tab", ["Shift"]), check("details-trigger", "focused", true),
    key("Space"), wait(300), capture("reopened"), check("c-state", "text", "open"), check("details-input", "value", "kept"),
    click("before-accordion"), key("Tab"), check("acc-a", "focused", true),
    key("ArrowDown"), check("acc-c", "focused", true), check("acc-state", "text", "a"),
    key("Home"), check("acc-a", "focused", true), key("End"), check("acc-c", "focused", true),
    key("Enter"), wait(300), check("acc-state", "text", "c"), capture("single"),
    key("Space"), check("acc-state", "text", "c"),
    click("empty"), click("acc-c"), wait(300), check("acc-state", "text", "none"),
    click("multiple"), check("mode-state", "text", "multiple"), click("acc-a"), click("acc-c"), wait(300),
    check("acc-state", "text", "a,c"), click("acc-count-c"), check("acc-count-c", "text", "Count: 1"), capture("multiple"),
    click("order"), wait(300), capture("reordered"), check("acc-count-c", "text", "Count: 1"),
    click("before-accordion"), key("Tab"), check("acc-c", "focused", true),
    key("End"), check("acc-a", "focused", true), key("Home"), check("acc-c", "focused", true),
    check("acc-state", "text", "a,c"),
    click("disable"), capture("disabled"), check("acc-a", "enabled", false), check("acc-c", "enabled", false),
    click("disable"), click("theme"), wait(300), capture("dark"),
    click("show"), capture("removed"), check("scene", "exists", false),
    click("show"), wait(300), capture("remounted"),
    check("details-input", "value", "seed"), check("details-count", "text", "Count: 0"),
    check("acc-count-c", "text", "Count: 0"), check("c-state", "text", "open"), check("acc-state", "text", "a,c"),
  ];
  const full = await run(steps, "full");
  const fullTree = name => tree(full.out, name);

  const initial = await fullTree("initial");
  const initialB = get(initial, "acc-b");
  assert.equal(initialB.state.enabled, false);
  assert.equal(initialB.state.visible, true);
  const bBounds = initialB.boundsLogical;
  assert.ok(disabledPoint.x > bBounds.x && disabledPoint.x < bBounds.x + bBounds.width &&
    disabledPoint.y > bBounds.y && disabledPoint.y < bBounds.y + bBounds.height,
    "derived point must land inside the full run's b bounds");

  const heightOf = (tree, testId) => {
    const bounds = get(tree, testId).boundsLogical;
    assert.ok(bounds, `${testId} boundsLogical missing`);
    return bounds.height;
  };
  const near = (actual, expected, label) => assert.ok(Math.abs(actual - expected) < 0.05, `${label}: ${actual} != ${expected}`);
  for (const name of ["initial", "closed"]) near(heightOf(await fullTree(name), "details-content"), 0, `${name} details-content`);
  for (const name of ["opened", "edited"]) near(heightOf(await fullTree(name), "details-content"), 96, `${name} details-content`);
  for (const name of ["grown", "reopened", "dark", "remounted"]) near(heightOf(await fullTree(name), "details-content"), 144, `${name} details-content`);
  const growing = heightOf(await fullTree("growing"), "details-content");
  assert.ok(growing > 96 && growing < 144, `growing must be mid-animation: ${growing}`);
  const closing = heightOf(await fullTree("closing"), "details-content");
  assert.ok(closing > 0 && closing < 144, `closing must be mid-animation: ${closing}`);
  const grownNode = get(await fullTree("grown"), "details-content");
  assert.ok(Math.abs(grownNode.boundsDevice.height - grownNode.boundsLogical.height * 1.75) < 0.1,
    `device height ${grownNode.boundsDevice.height} != logical*1.75`);

  for (const name of ["initial", "multiple", "reordered", "dark", "remounted"]) {
    near(heightOf(await fullTree(name), "nested-content"), 40, `${name} nested-content`);
  }

  const stableIds = [
    "details-content", "details-body", "details-input", "details-count",
    "acc-item-a", "acc-item-b", "acc-item-c", "acc-a", "acc-b", "acc-c",
    "acc-content-a", "acc-content-b", "acc-content-c", "acc-count-c", "nested-content",
  ];
  const captures = ["initial", "opened", "edited", "growing", "grown", "closing", "closed",
    "reopened", "single", "multiple", "reordered", "disabled", "dark", "removed", "remounted"];
  const trees = {};
  for (const name of captures) trees[name] = await fullTree(name);
  const removed = trees.removed;
  const remounted = trees.remounted;
  for (const testId of stableIds) {
    const token = get(initial, testId).token;
    for (const name of captures) {
      if (name === "removed" || name === "remounted") continue;
      assert.equal(get(trees[name], testId).token, token, `${testId} token changed at ${name}`);
    }
    assert.equal(find(removed, testId), undefined, `${testId} must be absent after removal`);
    assert.notEqual(get(remounted, testId).token, token, `${testId} must be a fresh node after remount`);
  }

  for (const name of ["edited", "closed", "reopened", "dark"]) {
    assert.equal(get(trees[name], "details-input").value, "kept", `details-input value at ${name}`);
    assert.equal(get(trees[name], "details-count").text, "Count: 1", `details-count text at ${name}`);
  }
  assert.equal(get(remounted, "details-input").value, "seed");
  assert.equal(get(remounted, "details-count").text, "Count: 0");
  assert.equal(get(remounted, "acc-count-c").text, "Count: 0");

  const itemOrder = tree => {
    const token = get(tree, "accordion").token;
    return tree.nodes.filter(node => node.parent === token).map(node => node.testId);
  };
  assert.deepEqual(itemOrder(initial), ["acc-item-a", "acc-item-b", "acc-item-c"]);
  assert.deepEqual(itemOrder(trees.reordered), ["acc-item-c", "acc-item-a", "acc-item-b"]);

  for (const name of ["initial", "dark", "remounted"]) {
    const parentToken = get(trees[name], "acc-a").parent;
    const heading = trees[name].nodes.find(node => node.token === parentToken);
    assert.ok(heading, `acc-a heading parent missing at ${name}`);
    assert.equal(heading.role, "heading", `acc-a parent role at ${name}`);
  }
  assert.equal(get(initial, "acc-content-a").role, "region");

  const inViewport = (tree, testId, name) => {
    const bounds = get(tree, testId).boundsLogical;
    assert.ok(bounds && bounds.x >= -0.05 && bounds.y >= -0.05 &&
      bounds.x + bounds.width <= 800.05 && bounds.y + bounds.height <= 600.05,
      `${testId} outside viewport at ${name}: ${JSON.stringify(bounds)}`);
  };
  const targeted = ["before", "after", "before-accordion", "grow", "restyle", "show", "theme",
    "multiple", "empty", "order", "disable", "details-trigger", "details-input", "details-count",
    "acc-a", "acc-b", "acc-c", "acc-count-c", "nested-trigger"];
  for (const name of ["multiple", "dark"]) for (const testId of targeted) inViewport(trees[name], testId, name);
});
