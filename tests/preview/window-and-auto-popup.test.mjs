import assert from "node:assert/strict";
import path from "node:path";
import { after, before, test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  readFrame,
  readPng,
  readTree,
  runScenario,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-window-auto-popup-");
  bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "window-and-auto-popup",
    new URL("./fixtures/window-and-auto-popup.tsx", import.meta.url),
    { capabilities: { shellSurfaces: true } },
  );
});
after(async () => {
  await temp?.cleanup();
});

const monitor = {
  id: "DP-1",
  name: "DP-1",
  x: 0,
  y: 0,
  width: 1280,
  height: 800,
  scale: 1,
  refreshRateMilliHz: 60000,
};
const capture = (id, surface) => ({ id, op: "capture", captureId: id, surface });
const click = (testId, id = testId) => ({
  id,
  op: "pointer.click",
  target: { surface: "main", testId },
  button: "primary",
});
const advance = id => ({ id, op: "clock.advance", byMs: 40 });

test("WindowSurface renders, resizes, and hosts a zero-flicker auto-height PopupSurface", async () => {
  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "window-and-auto-popup",
    requiredCapabilities: [],
    width: 400,
    height: 240,
    scale: 1,
    assertEmptyResources: true,
    steps: [
      { id: "outputs", op: "monitors.set", monitors: [monitor] },
      capture("settings-initial", "settings"),
      capture("auto-120", "auto-menu"),
      click("set-260"),
      advance("settle-260"),
      capture("auto-260", "auto-menu"),
      click("set-700"),
      advance("settle-700"),
      capture("auto-700", "auto-menu"),
      click("set-80"),
      advance("settle-80"),
      capture("auto-80", "auto-menu"),
      click("resize-win"),
      advance("settle-win"),
      capture("settings-resized", "settings"),
    ],
  });

  const settingsInitialPng = await readPng(
    path.join(out, "captures/surfaces/settings/settings-initial.png"),
  );
  assert.deepEqual([settingsInitialPng.width, settingsInitialPng.height], [480, 320]);
  const settingsInitialTree = await readTree(out, "surfaces/settings/settings-initial");
  assert.equal(getNode(settingsInitialTree, "settings-title").text, "Settings");

  const settingsResizedPng = await readPng(
    path.join(out, "captures/surfaces/settings/settings-resized.png"),
  );
  assert.deepEqual([settingsResizedPng.width, settingsResizedPng.height], [560, 320]);
  const settingsInitialFrame = await readFrame(out, "surfaces/settings/settings-initial");
  const settingsResizedFrame = await readFrame(out, "surfaces/settings/settings-resized");
  assert.equal(
    settingsResizedFrame.documentIdentity,
    settingsInitialFrame.documentIdentity,
    "WindowSurface updates preserve document identity",
  );

  // Initial auto-height popup settles directly to 120px (not maxHeight=400px) with no vertical overflow.
  const png120 = await readPng(path.join(out, "captures/surfaces/auto-menu/auto-120.png"));
  assert.deepEqual([png120.width, png120.height], [240, 120]);
  const tree120 = await readTree(out, "surfaces/auto-menu/auto-120");
  assert.equal(getNode(tree120, "auto-scroll").scrollMetrics?.overflowY, false);

  // Growing content expands popup to 260px with no vertical overflow.
  const png260 = await readPng(path.join(out, "captures/surfaces/auto-menu/auto-260.png"));
  assert.deepEqual([png260.width, png260.height], [240, 260]);
  const tree260 = await readTree(out, "surfaces/auto-menu/auto-260");
  assert.equal(getNode(tree260, "auto-scroll").scrollMetrics?.overflowY, false);

  // Content taller than maxHeight clamps to 400px and activates vertical scrolling.
  const png700 = await readPng(path.join(out, "captures/surfaces/auto-menu/auto-700.png"));
  assert.deepEqual([png700.width, png700.height], [240, 400]);
  const tree700 = await readTree(out, "surfaces/auto-menu/auto-700");
  assert.equal(getNode(tree700, "auto-scroll").scrollMetrics?.overflowY, true);

  // Shrinking content contracts popup back to 80px with no vertical overflow.
  const png80 = await readPng(path.join(out, "captures/surfaces/auto-menu/auto-80.png"));
  assert.deepEqual([png80.width, png80.height], [240, 80]);
  const tree80 = await readTree(out, "surfaces/auto-menu/auto-80");
  assert.equal(getNode(tree80, "auto-scroll").scrollMetrics?.overflowY, false);

  assert.equal(result.captureCount, 6);
});

test("PopupSurface inputRegion allows clicks inside active rect and passes through outside it", async () => {
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "popup-input-region-hit",
    requiredCapabilities: [],
    width: 400,
    height: 240,
    scale: 1,
    assertEmptyResources: true,
    steps: [
      { id: "outputs", op: "monitors.set", monitors: [monitor] },
      click("open-shaped"),
      advance("settle-shaped"),
      {
        id: "click-inside",
        op: "pointer.click",
        target: { surface: "shaped-menu", testId: "inside-hit" },
        button: "primary",
      },
      advance("settle-inside"),
      capture("after-inside", "shaped-menu"),
    ],
  });
  const tree = await readTree(out, "surfaces/shaped-menu/after-inside");
  assert.equal(getNode(tree, "shaped-hits").text, "hits=1");

  const { result: blocked } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "popup-input-region-passthrough",
    requiredCapabilities: [],
    width: 400,
    height: 240,
    scale: 1,
    allowFailure: true,
    assertEmptyResources: true,
    steps: [
      { id: "outputs", op: "monitors.set", monitors: [monitor] },
      click("open-shaped"),
      advance("settle-shaped"),
      {
        id: "click-outside",
        op: "pointer.click",
        target: { surface: "shaped-menu", testId: "outside-hit" },
        button: "primary",
      },
    ],
  });
  assert.notEqual(
    blocked.exitCode,
    0,
    "target outside PopupSurface inputRegion must not receive clicks",
  );
});
