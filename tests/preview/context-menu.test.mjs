import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

let project;
before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-context-menu-"));
  const app = path.join(project, "main");
  await mkdir(path.join(app, "src"), { recursive: true });
  const source = await readFile(fileURLToPath(new URL("./fixtures/context-menu.tsx", import.meta.url)), "utf8");
  await writeFile(path.join(app, "src/main.tsx"), source);
  await writeFile(path.join(app, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
    include: ["src/**/*.tsx"],
  }));
  await buildApplication("context-menu", { appDirectory: app });
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
const click = (testId, button = "primary") => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button });
const clickAt = (x, y, button = "primary") => ({ id: id(), op: "pointer.click", surface: "main", position: { x, y }, button });
const move = (x, y) => ({ id: id(), op: "pointer.move", surface: "main", position: { x, y } });
const touch = (op, x, y) => ({
  id: id(), op, surface: "main", position: { x, y }, button: "primary",
  pointer: { id: "1", kind: "touch", primary: true },
});
const key = name => ({ id: id(), op: "key.press", surface: "main", key: name });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const emptyResources = {
  nodes: 0, wrappers: 0, callbacks: 0, focusTraps: 0, scrollLocks: 0, timers: 0,
  frameCallbacks: 0, requests: 0, engineAlive: false,
};

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1,
    id: `context-menu-${name}`,
    requiredCapabilities: [
      "pixels", "virtual-time", "selectors.test-id", "selectors.semantic",
      "input.pointer", "input.keyboard", "tree.inspect", "surface.scale",
    ],
    steps,
  }));
  const process = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), [
    "preview", "run",
    "--bundle", path.join(project, "main/dist"),
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
const get = (t, testId) => {
  const node = t.nodes.find(entry => entry.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};
const positionerBounds = (t, testId) => {
  const surface = get(t, testId);
  const positioner = t.nodes.find(entry => entry.token === surface.parent);
  assert.ok(positioner?.boundsLogical, `missing positioner for ${testId}`);
  return positioner.boundsLogical;
};

test("context menu secondary click, offset point anchoring, edge flip, touch long-press, and dismissal", async () => {
  const measured = await run([capture("measure", 0)], "measure");
  const initialTree = await tree(measured.out, "measure");
  const area = get(initialTree, "area").boundsLogical;
  const btn = get(initialTree, "canvas-btn").boundsLogical;
  assert.ok(area.x >= 79 && area.y >= 71, `expected offset trigger area, got ${JSON.stringify(area)}`);
  const bx = Math.round(btn.x + btn.width / 2);
  const by = Math.round(btn.y + btn.height / 2);

  const { out } = await run([
    check("menu", "exists", false),
    // Secondary click at (220, 200) inside offset area
    clickAt(220, 200, "secondary"),
    check("open-state", "text", "open"),
    check("menu", "exists", true),
    check("menu", "role", "menu"),
    capture("secondary-first"),
    advance(300),
    capture("secondary-settled"),

    // Primary click inside the trigger area (outside menu) must dismiss because only the 0x0 anchor is excluded
    clickAt(150, 380, "primary"),
    advance(300),
    check("open-state", "text", "closed"),
    check("menu", "exists", false),

    // Reopen near bottom-right corner inside trigger area -> Popper flips/shifts into viewport
    clickAt(740, 540, "secondary"),
    advance(300),
    check("menu", "exists", true),
    capture("edge-flipped"),
    key("Escape"),
    advance(300),
    check("menu", "exists", false),

    // Park mouse near top-left outside area, then perform touch long-press over Canvas Button
    move(15, 15),
    touch("pointer.down", bx, by),
    advance(200),
    check("menu", "exists", false),
    advance(350),
    check("open-state", "text", "open"),
    check("menu", "exists", true),
    check("menu-req", "text", `${bx},${by}:touch`),
    advance(300),
    capture("touch-longpress"),
    touch("pointer.up", bx, by),
    check("btn-clicks", "text", "0"),
    check("menu", "exists", true),

    // Navigate submenu with keyboard and close on leaf select
    key("End"),
    key("ArrowRight"),
    advance(150),
    check("export-menu", "exists", true),
    capture("submenu-open"),
    key("Enter"),
    advance(300),
    check("menu", "exists", false),
    check("last-action", "text", "export-png"),

    // Programmatic useContextMenuPlace().openAt(320, 240)
    click("place-btn"),
    advance(300),
    check("menu", "exists", true),
    capture("place-open"),
    click("grid"),
    check("last-action", "text", "grid-on"),
    check("menu", "exists", true),
    click("zoom-200"),
    advance(300),
    check("last-action", "text", "zoom-200"),
    check("menu", "exists", false),
  ], "main");

  const frames = {};
  for (const name of ["secondary-first", "secondary-settled", "edge-flipped", "touch-longpress", "submenu-open", "place-open"]) {
    frames[name] = await tree(out, name);
  }
  const tolerance = 1 / 1.75 + 0.001;
  const pointTolerance = 1 + tolerance;
  assert.deepEqual(
    positionerBounds(frames["secondary-first"], "menu"),
    get(frames["secondary-settled"], "menu").boundsLogical,
    "first paint positioner should match settled context menu surface",
  );
  const p1 = positionerBounds(frames["secondary-settled"], "menu");
  assert.ok(Math.abs(p1.x - 220) <= pointTolerance && Math.abs(p1.y - 200) <= pointTolerance,
    `expected context menu anchored at (220, 200), got ${JSON.stringify(p1)}`);

  const edge = positionerBounds(frames["edge-flipped"], "menu");
  assert.ok(edge.x >= 8 - tolerance && edge.y >= 8 - tolerance
    && edge.x + edge.width <= 800 - 8 + tolerance && edge.y + edge.height <= 600 - 8 + tolerance,
    `expected edge context menu within padded viewport, got ${JSON.stringify(edge)}`);

  const pTouch = positionerBounds(frames["touch-longpress"], "menu");
  assert.ok(Math.abs(pTouch.x - bx) <= pointTolerance && Math.abs(pTouch.y - by) <= pointTolerance,
    `expected touch long-press menu anchored at (${bx}, ${by}) rather than parked mouse, got ${JSON.stringify(pTouch)}`);

  const pPlace = positionerBounds(frames["place-open"], "menu");
  assert.ok(Math.abs(pPlace.x - 320) <= pointTolerance && Math.abs(pPlace.y - 240) <= pointTolerance,
    `expected programmatic openAt at (320, 240), got ${JSON.stringify(pPlace)}`);
});
