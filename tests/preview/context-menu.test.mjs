import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
import {
  buildFixtureApp,
  createTempWorkspace,
  functionalProfile,
  getNode as get,
  readTree as tree,
  runScenario,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-context-menu-");
  bundle = await buildFixtureApp(
    path.join(temp.dir, "main"),
    "context-menu",
    new URL("./fixtures/context-menu.tsx", import.meta.url),
  );
});
after(async () => {
  await temp?.cleanup();
});

let serial = 0;
const id = () => `step-${++serial}`;
const click = (testId, button = "primary") => ({
  id: id(),
  op: "pointer.click",
  target: { surface: "main", testId },
  button,
});
const clickAt = (x, y, button = "primary") => ({
  id: id(),
  op: "pointer.click",
  surface: "main",
  position: { x, y },
  button,
});
const move = (x, y) => ({ id: id(), op: "pointer.move", surface: "main", position: { x, y } });
const touch = (op, x, y) => ({
  id: id(),
  op,
  surface: "main",
  position: { x, y },
  button: "primary",
  pointer: { id: "1", kind: "touch", primary: true },
});
const key = name => ({ id: id(), op: "key.press", surface: "main", key: name });
const check = (testId, property, equals) => ({
  id: id(),
  op: "assert",
  target: { surface: "main", testId },
  check: { property, equals },
});
const capture = (captureId, requiredFrameTimeUs) => ({
  id: id(),
  op: "capture",
  captureId,
  ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }),
});
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

function run(steps, name) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    profile: functionalProfile,
    name,
    id: `context-menu-${name}`,
    requiredCapabilities: [
      "pixels",
      "virtual-time",
      "selectors.test-id",
      "selectors.semantic",
      "input.pointer",
      "input.keyboard",
      "tree.inspect",
      "surface.scale",
    ],
    steps,
  });
}
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
  assert.ok(
    area.x >= 79 && area.y >= 71,
    `expected offset trigger area, got ${JSON.stringify(area)}`,
  );
  const bx = Math.round(btn.x + btn.width / 2);
  const by = Math.round(btn.y + btn.height / 2);

  const { out } = await run(
    [
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
    ],
    "main",
  );

  const frames = {};
  for (const name of [
    "secondary-first",
    "secondary-settled",
    "edge-flipped",
    "touch-longpress",
    "submenu-open",
    "place-open",
  ]) {
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
  assert.ok(
    Math.abs(p1.x - 220) <= pointTolerance && Math.abs(p1.y - 200) <= pointTolerance,
    `expected context menu anchored at (220, 200), got ${JSON.stringify(p1)}`,
  );

  const edge = positionerBounds(frames["edge-flipped"], "menu");
  assert.ok(
    edge.x >= 8 - tolerance &&
      edge.y >= 8 - tolerance &&
      edge.x + edge.width <= 800 - 8 + tolerance &&
      edge.y + edge.height <= 600 - 8 + tolerance,
    `expected edge context menu within padded viewport, got ${JSON.stringify(edge)}`,
  );

  const pTouch = positionerBounds(frames["touch-longpress"], "menu");
  assert.ok(
    Math.abs(pTouch.x - bx) <= pointTolerance && Math.abs(pTouch.y - by) <= pointTolerance,
    `expected touch long-press menu anchored at (${bx}, ${by}) rather than parked mouse, got ${JSON.stringify(pTouch)}`,
  );

  const pPlace = positionerBounds(frames["place-open"], "menu");
  assert.ok(
    Math.abs(pPlace.x - 320) <= pointTolerance && Math.abs(pPlace.y - 240) <= pointTolerance,
    `expected programmatic openAt at (320, 240), got ${JSON.stringify(pPlace)}`,
  );
});
