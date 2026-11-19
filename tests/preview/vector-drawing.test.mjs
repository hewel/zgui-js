import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import { buildFixtureApp, createTempWorkspace, getNode, pixelAt, readPng, readTree, runScenario } from "./harness.mjs";

const capture = name => ({ id: `capture-${name}`, op: "capture", captureId: name });
const click = testId => ({ id: `click-${testId}`, op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const advance = name => ({ id: `advance-${name}`, op: "clock.advance", byMs: 20 });

async function readPixels(out, name) {
  const [image, tree] = await Promise.all([
    readPng(path.join(out, `captures/${name}.png`)),
    readTree(out, name),
  ]);
  return (testId, x, y, expected) => {
    const bounds = getNode(tree, testId).boundsDevice;
    assert.ok(bounds && bounds.width > x && bounds.height > y, `${name}: ${testId} bounds`);
    assert.deepEqual(pixelAt(image, Math.floor(bounds.x + x), Math.floor(bounds.y + y)), expected,
      `${name}: ${testId} at (${x}, ${y})`);
  };
}

const white = [255, 255, 255, 255];
const blue = [0, 0, 255, 255];
const green = [0, 255, 0, 255];
const red = [255, 0, 0, 255];
const magenta = [255, 0, 255, 255];
const cyan = [0, 255, 255, 255];
const yellow = [255, 255, 0, 255];

function assertSvg(pixel, testId) {
  pixel(testId, 16, 16, red);
  pixel(testId, 50, 16, green);
  pixel(testId, 6, 16, blue);
  pixel(testId, 40, 60, white);
}

test("native vectors render SVG and retained canvas, then fall back or clear on disposal", async t => {
  const temp = await createTempWorkspace("preview-vector-drawing-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(temp.dir, "vector-drawing", new URL("./fixtures/vector-drawing.tsx", import.meta.url));
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "vector-drawing",
    scale: 1,
    assertEmptyResources: true,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect"],
    steps: [
      capture("initial"),
      click("replace"), advance("replace"), capture("replaced"),
      click("translate"), advance("translate"), capture("translated"),
      click("clear"), advance("clear"), capture("cleared"),
      click("dispose"), advance("dispose"), capture("disposed"),
    ],
  });

  const initial = await readPixels(out, "initial");
  assertSvg(initial, "svg");
  initial("owned", 16, 16, blue);
  const cleared = await readPixels(out, "cleared");
  cleared("owned", 16, 16, blue);
  for (const [pixel, left, right] of [
    [initial, cyan, white],
    [await readPixels(out, "replaced"), yellow, white],
    [await readPixels(out, "translated"), white, yellow],
    [cleared, white, white],
  ]) {
    for (const testId of ["canvas-svg", "canvas-path"]) {
      pixel(testId, 16, 16, left);
      pixel(testId, 50, 16, right);
      pixel(testId, 6, 16, white);
    }
  }
  const disposed = await readPixels(out, "disposed");
  disposed("owned", 16, 16, white);
  assertSvg(disposed, "canvas-svg");
  disposed("canvas-path", 40, 60, magenta);
});

test("preview reports an undisposed ownerless canvas scene at session retirement", async t => {
  const temp = await createTempWorkspace("preview-vector-leak-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(temp.dir, "vector-leak", `
    import { createCanvasScene, renderApp, type NativeNode } from "@hewel/zgui";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      const scene = createCanvasScene();
      scene.update([{ kind: "rect", x: 0, y: 0, width: 16, height: 16, fill: "#ff0000" }]);
      return renderApp(() => <vector canvas={scene} style="width: 16px; height: 16px" />, root);
    }
  `, { rawSource: true });
  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "vector-leak",
    allowFailure: true,
    requiredCapabilities: ["pixels"],
    steps: [capture("ownerless")],
  });
  assert.equal(result.captureCount, 1);
  assert.equal(result.exitCode, 9);
  assert.equal(result.firstError?.code, "E_TEARDOWN");
  assert.equal(result.teardown.resources.canvasScenes, 1);
});
