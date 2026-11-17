import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

let outDir;
const galleryDir = path.join(workspaceRoot, "apps/examples/gallery");

before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  outDir = await mkdtemp(path.join(workspaceRoot, "target/preview-gallery-test-"));
  await buildApplication("gallery", { appDirectory: galleryDir });
});

after(async () => {
  if (!outDir) return;
  if (process.env.ZGUI_KEEP_PREVIEW_OUTPUT === "1") {
    console.log(`retained preview output: ${outDir}`);
  } else {
    await rm(outDir, { recursive: true, force: true });
  }
});

let serial = 0;
const id = () => `step-${++serial}`;
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const wheelAt = (x, y, dy) => ({ id: id(), op: "wheel", surface: "main", position: { x, y }, delta: { x: 0, y: dy, unit: "pixels" } });
const wheelPhase = (x, y, dy, phase) => ({ id: id(), op: "wheel", surface: "main", position: { x, y }, delta: { x: 0, y: dy, unit: "pixels" }, phase });
const key = name => ({ id: id(), op: "key.press", surface: "main", key: name });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = captureId => ({ id: id(), op: "capture", captureId });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const resize = (width, height) => ({ id: id(), op: "surface.resize", surface: "main", width, height });

async function runScenario(name, steps, expectedCaptures) {
  const scenarioPath = path.join(outDir, `${name}.json`);
  const runOut = path.join(outDir, `run-${name}`);
  await writeFile(scenarioPath, JSON.stringify({
    schemaVersion: 1,
    id: `gallery-${name}`,
    requiredCapabilities: [
      "pixels", "virtual-time", "selectors.test-id", "input.pointer",
      "input.wheel", "input.keyboard", "tree.inspect", "surface.resize", "surface.scale",
    ],
    steps,
  }));

  const proc = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), [
    "preview", "run",
    "--bundle", path.join(galleryDir, "dist"),
    "--profile", path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario", scenarioPath,
    "--out", runOut,
  ], { cwd: workspaceRoot, encoding: "utf8", timeout: 90000 });

  assert.ifError(proc.error);
  assert.equal(proc.signal, null, proc.stderr);
  const result = JSON.parse(proc.stdout);
  assert.equal(proc.status, result.exitCode);
  assert.equal(result.exitCode, 0, proc.stdout);
  assert.equal(result.status, "passed", proc.stdout);
  assert.equal(result.assertions.failed, 0, proc.stdout);
  assert.equal(result.assertions.passed, steps.filter(s => s.op === "assert").length);
  assert.equal(result.teardown.clean, true, proc.stdout);

  for (const captureName of expectedCaptures) {
    const png = await readFile(path.join(runOut, `captures/${captureName}.png`));
    assert.ok(png.length > 0, `empty png for ${captureName}`);
  }
  return runOut;
}

test("gallery renders atoms, feedback, fields, choices, and composites tabs cleanly", async () => {
  await runScenario("sections-a", [
    resize(1280, 860),
    advance(100),
    check("section-atoms", "exists", true),
    capture("tab-atoms"),

    click("tab-feedback"),
    advance(100),
    check("section-feedback", "exists", true),
    capture("tab-feedback"),

    click("tab-fields"),
    advance(100),
    check("section-fields", "exists", true),
    capture("tab-fields"),

    click("tab-choices"),
    advance(100),
    check("section-choices", "exists", true),
    capture("tab-choices"),

    click("tab-composites"),
    advance(250),
    check("section-composites", "exists", true),
    check("composites-card", "exists", true),
    capture("tab-composites"),
  ], ["tab-atoms", "tab-feedback", "tab-fields", "tab-choices", "tab-composites"]);
});

test("gallery renders overlays, menus, and dark ocean theme cleanly", async () => {
  await runScenario("sections-b", [
    resize(1280, 860),
    advance(100),

    click("tab-overlays"),
    advance(100),
    check("section-overlays", "exists", true),
    click("overlay-dialog-trigger"),
    advance(250),
    check("overlay-dialog-content", "exists", true),
    capture("tab-overlays-dialog"),
    key("Escape"),
    advance(300),
    check("overlay-dialog-content", "exists", false),

    click("tab-menus"),
    advance(100),
    check("section-menus", "exists", true),
    click("menus-dropdown-trigger"),
    advance(250),
    check("menus-dropdown-content", "exists", true),
    capture("tab-menus-dropdown"),
    key("Escape"),
    advance(300),
    check("menus-dropdown-content", "exists", false),

    click("menus-editor-view"),
    advance(200),
    check("menus-editor-view-content", "exists", true),
    check("menus-editor-spacious", "exists", true),
    click("menus-editor-spacious"),
    advance(250),
    check("menus-editor-view-content", "exists", false),

    click("scheme-switch"),
    click("preset-trigger"),
    advance(250),
    check("preset-ocean", "exists", true),
    click("preset-ocean"),
    advance(600),
    check("scheme-badge", "text", "ocean / dark"),
    capture("dark-theme"),
  ], ["tab-overlays-dialog", "tab-menus-dropdown", "dark-theme"]);
});

test("gallery scrolls vertically via wheel and touchpad in a compact desktop window", async () => {
  const runOut = await runScenario("scroll-compact", [
    resize(560, 180),
    advance(100),
    check("gallery-page", "exists", true),
    capture("scroll-before"),
    wheelAt(280, 120, 220),
    advance(40),
    capture("scroll-wheel"),
    wheelPhase(280, 120, 0, "started"),
    advance(16),
    wheelPhase(280, 120, 180, "moved"),
    wheelPhase(280, 120, 0, "ended"),
    advance(40),
    capture("scroll-after"),
  ], ["scroll-before", "scroll-wheel", "scroll-after"]);

  const beforeTree = JSON.parse(await readFile(path.join(runOut, "captures/scroll-before.tree.json"), "utf8"));
  const wheelTree = JSON.parse(await readFile(path.join(runOut, "captures/scroll-wheel.tree.json"), "utf8"));
  const afterTree = JSON.parse(await readFile(path.join(runOut, "captures/scroll-after.tree.json"), "utf8"));
  const beforePage = beforeTree.nodes.find(node => node.testId === "gallery-page");
  const wheelPage = wheelTree.nodes.find(node => node.testId === "gallery-page");
  const afterPage = afterTree.nodes.find(node => node.testId === "gallery-page");
  assert.ok(beforePage?.boundsLogical, "missing gallery-page before bounds");
  assert.ok(wheelPage?.boundsLogical, "missing gallery-page wheel bounds");
  assert.ok(afterPage?.boundsLogical, "missing gallery-page after bounds");
  assert.ok(
    wheelPage.boundsLogical.y < beforePage.boundsLogical.y - 30,
    `expected discrete wheel to scroll down by >30px: before=${beforePage.boundsLogical.y} wheel=${wheelPage.boundsLogical.y}`,
  );
  assert.ok(
    afterPage.boundsLogical.y < wheelPage.boundsLogical.y - 150,
    `expected continuous scroll to advance >150px further: wheel=${wheelPage.boundsLogical.y} after=${afterPage.boundsLogical.y}`,
  );
});
