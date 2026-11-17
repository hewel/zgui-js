import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import {
  buildExampleApp,
  createTempWorkspace,
  readMetrics,
  readTree,
  runScenario as runPreviewScenario,
} from "./harness.mjs";
import { readFile } from "node:fs/promises";
import path from "node:path";

let temp;
let bundle;

before(async () => {
  temp = await createTempWorkspace("preview-gallery-test-");
  bundle = await buildExampleApp("gallery");
});

after(async () => {
  await temp?.cleanup();
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

const STACK_LIMIT_BYTES = 240 * 1024;

async function runScenario(name, steps, expectedCaptures) {
  const { out } = await runPreviewScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `gallery-${name}`,
    requiredCapabilities: [
      "pixels", "virtual-time", "selectors.test-id", "input.pointer",
      "input.wheel", "input.keyboard", "tree.inspect", "surface.resize", "surface.scale",
    ],
    steps,
    maxJsStackBytes: STACK_LIMIT_BYTES,
  });

  const metrics = await readMetrics(out);
  assert.equal(metrics.jsStackLimitBytes, STACK_LIMIT_BYTES);
  assert.ok(metrics.jsHeapBytes > 0, `expected non-zero jsHeapBytes: ${JSON.stringify(metrics)}`);
  assert.ok(
    metrics.maxJsEntryDurationUs > 0 && metrics.maxJsEntryDurationUs < 25_000,
    `unexpected maxJsEntryDurationUs: ${JSON.stringify(metrics)}`,
  );

  for (const captureName of expectedCaptures) {
    const png = await readFile(path.join(out, `captures/${captureName}.png`));
    assert.ok(png.length > 0, `empty png for ${captureName}`);
  }
  return out;
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

  const beforeTree = await readTree(runOut, "scroll-before");
  const wheelTree = await readTree(runOut, "scroll-wheel");
  const afterTree = await readTree(runOut, "scroll-after");
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
