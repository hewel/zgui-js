import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { readFile } from "node:fs/promises";
import path from "node:path";
import {
  buildExampleApp,
  createTempWorkspace,
  defaultProfile,
  getNode,
  pixelAt,
  readFrame,
  readPng,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

let temp;
let bundle;

before(async () => {
  temp = await createTempWorkspace("preview-stylex-");
  bundle = await buildExampleApp("stylex");
});
after(async () => { await temp?.cleanup(); });

let serial = 0;
const id = () => `stylex-step-${++serial}`;
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const move = testId => ({ id: id(), op: "pointer.move", target: { surface: "main", testId } });
const key = key => ({ id: id(), op: "key.press", surface: "main", key });
const colorScheme = colorScheme => ({ id: id(), op: "surface.colorScheme", surface: "main", colorScheme });
const capture = (captureId, requiredFrameTimeUs) => ({
  id: id(), op: "capture", captureId,
  ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }),
});
const capabilities = ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "input.keyboard", "tree.inspect", "assets.native-readiness"];

function run(steps, name, options = {}) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `stylex-${name}`,
    requiredCapabilities: capabilities,
    steps,
    assertEmptyResources: false,
    ...options,
  });
}

function centerPixel(image, node) {
  const bounds = node.boundsDevice;
  assert.ok(bounds, `${node.testId} has device bounds`);
  return pixelAt(image,
    Math.floor(bounds.x + bounds.width / 2),
    Math.floor(bounds.y + bounds.height / 2));
}

function cornerPixel(image, node) {
  const bounds = node.boundsDevice;
  assert.ok(bounds, `${node.testId} has device bounds`);
  return pixelAt(image, Math.floor(bounds.x + 3), Math.floor(bounds.y + 3));
}

function controlPixel(image, node) {
  const bounds = node.boundsDevice;
  assert.ok(bounds, `${node.testId} has device bounds`);
  return pixelAt(image, Math.floor(bounds.x + bounds.width / 2), Math.floor(bounds.y + bounds.height - 4));
}

test("SX01-SX12/SX15/SX17: real extracted StyleX renders dynamic native output with matched assets and freshness", async () => {
  const { out, result } = await run([
    capture("initial", 0),
    click("progress"), capture("progress"),
    click("compact"), capture("compact"),
    click("compact"), capture("compact-off"),
    click("dark"), capture("dark"),
    click("scheme"), capture("sdk-theme"),
    click("nested"), capture("nested-removed"),
    click("dark"), capture("restored"),
  ], "rendering");
  assert.equal(result.build.buildId.length, 71);

  const initialTree = await readTree(out, "initial");
  const progressTree = await readTree(out, "progress");
  const compactTree = await readTree(out, "compact");
  const compactOffTree = await readTree(out, "compact-off");
  assert.ok(Math.abs(getNode(initialTree, "fill").boundsLogical.width - 50) <= 0.6);
  assert.ok(Math.abs(getNode(progressTree, "fill").boundsLogical.width - 100) <= 0.6);
  assert.ok(getNode(compactTree, "panel").boundsLogical.height < getNode(progressTree, "panel").boundsLogical.height,
    "applying the conditional compact class reduces panel padding");
  assert.equal(getNode(compactOffTree, "panel").boundsLogical.height, getNode(progressTree, "panel").boundsLogical.height,
    "removing the conditional class restores the original padding");

  const image = await readPng(path.join(out, "captures/initial.png"));
  assert.deepEqual(centerPixel(image, getNode(initialTree, "base-order")), [34, 197, 94, 255],
    "generated app.css follows base.css and wins the equal-specificity color");
  assert.deepEqual(centerPixel(image, getNode(initialTree, "fill")), [49, 94, 251, 255]);
  assert.deepEqual(centerPixel(image, getNode(initialTree, "package-image")), [255, 0, 0, 255]);
  assert.deepEqual(cornerPixel(image, getNode(initialTree, "root")), [255, 255, 255, 255]);
  assert.deepEqual(cornerPixel(image, getNode(initialTree, "nested-target")), [24, 34, 48, 255],
    "nested theme overrides the default root tokens");

  for (const [captureId, testId, expected] of [
    ["dark", "root", [24, 34, 48, 255]],
    ["nested-removed", "nested-target", [24, 34, 48, 255]],
    ["restored", "root", [255, 255, 255, 255]],
    ["restored", "nested-target", [255, 255, 255, 255]],
  ]) {
    const themedTree = await readTree(out, captureId);
    const themedImage = await readPng(path.join(out, `captures/${captureId}.png`));
    assert.deepEqual(cornerPixel(themedImage, getNode(themedTree, testId)), expected, `${captureId}:${testId}`);
  }

  const initialFrame = await readFrame(out, "initial");
  const progressFrame = await readFrame(out, "progress");
  const themeFrame = await readFrame(out, "sdk-theme");
  assert.equal(progressFrame.styleRevision, initialFrame.styleRevision,
    "class and inline custom-property updates retain stylesheet identity");
  assert.ok(themeFrame.styleRevision > initialFrame.styleRevision,
    "ThemeProvider's sheet-only replacement has a new consumed style revision");
  assert.equal(themeFrame.buildId, initialFrame.buildId);
  assert.equal(themeFrame.resources.requiredAssetsReady, true);

  const manifest = JSON.parse(await readFile(path.join(bundle, "manifest.json"), "utf8"));
  assert.equal(manifest.schemaVersion, 2);
  assert.deepEqual(manifest.styles, ["base.css", "app.css"]);
  assert.deepEqual(manifest.fonts, [{ family: "StyleX Fixture", path: "assets/StyleXFixture.ttf" }]);
  assert.equal(manifest.styleSystem.name, "stylex");
  assert.match(manifest.styleSystem.nativeProfileDigest, /^sha256:[a-f0-9]{64}$/);
  assert.notEqual(manifest.styleSystem.nativeProfileDigest, manifest.styleSystem.configDigest);
});

test("SX10: generated focus, hover, and viewport-media rules affect real native output", async () => {
  const { out } = await run([
    key("Tab"),
    { id: id(), op: "clock.advance", byMs: 300 },
    capture("focus"),
    move("compact"),
    { id: id(), op: "clock.advance", byMs: 300 },
    capture("hover"),
    { id: id(), op: "surface.resize", surface: "main", width: 500, height: 650 },
    { id: id(), op: "clock.advance", byMs: 20 },
    capture("small"),
  ], "conditions");
  const focusTree = await readTree(out, "focus");
  const focusImage = await readPng(path.join(out, "captures/focus.png"));
  assert.deepEqual(controlPixel(focusImage, getNode(focusTree, "progress")), [37, 99, 235, 255]);

  const hoverTree = await readTree(out, "hover");
  const hoverImage = await readPng(path.join(out, "captures/hover.png"));
  assert.deepEqual(controlPixel(hoverImage, getNode(hoverTree, "compact")), [220, 38, 38, 255]);
  const smallTree = await readTree(out, "small");
  assert.ok(Math.abs(getNode(smallTree, "track").boundsLogical.width - 150) <= 0.6);
});

test("font validation follows configuration-only computed family changes", async () => {
  const { result } = await run([
    capture("before-font-resize", 0),
    click("font-mismatch"),
    { id: id(), op: "surface.resize", surface: "main", width: 500, height: 650 },
    { id: id(), op: "clock.advance", byMs: 20 },
    capture("invalid-font"),
  ], "font-reconfiguration", { allowFailure: true });
  assert.equal(result.firstError.code, "E_FONT_FAMILY");
  assert.match(result.firstError.message, /Missing Font/);
});

test("font validation follows native color-scheme changes", async () => {
  const { result } = await run([
    capture("before-scheme-change", 0),
    click("scheme-font-mismatch"),
    colorScheme("dark"),
  ], "font-color-scheme", { allowFailure: true, theme: "light", requiredCapabilities: [...capabilities, "surface.color-scheme"] });
  assert.equal(result.firstError.code, "E_FONT_FAMILY");
  assert.match(result.firstError.message, /Missing Font/);
});

test("SX06: invalid compiled dynamic values fail Preview instead of certifying partial native state", async () => {
  const { result, stderr } = await run([
    capture("before-invalid", 0), click("invalid"),
  ], "invalid-dynamic", { allowFailure: true });
  assert.notEqual(result.exitCode, 0);
  assert.match(`${JSON.stringify(result)}\n${stderr}`, /NaNpx|style|declaration/i);
  assert.equal(result.captureCount, 1, "only the pre-failure capture is current valid evidence");
});

test("SX05: a real effective alias conflict fails Preview with its diagnostic", async () => {
  const { result, stderr } = await run([
    capture("before-conflict", 0), click("conflict"),
  ], "alias-conflict", { allowFailure: true });
  assert.notEqual(result.exitCode, 0);
  assert.match(`${JSON.stringify(result)}\n${stderr}`, /CLASS_ALIAS_CONFLICT/);
  assert.equal(result.captureCount, 1);
});

test("keyframes and the host reduced-motion preference produce current native evidence", async () => {
  const profile = path.join(temp.dir, "reduced-motion-profile.json");
  const base = JSON.parse(await readFile(defaultProfile, "utf8"));
  await writeJson(profile, {
    ...base,
    id: "stylex-reduced-motion",
    reducedMotion: true,
    fonts: { ...base.fonts, manifest: "../../preview/fonts/fonts.json" },
  });
  const { out } = await run([
    capture("reduced", 0),
  ], "reduced-motion", { profile });
  const tree = await readTree(out, "reduced");
  assert.equal(getNode(tree, "motion").text, "ended");
  const frame = await readFrame(out, "reduced");
  assert.equal(frame.buildId.length, 71);
});
