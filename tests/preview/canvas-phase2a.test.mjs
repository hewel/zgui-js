import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  pixelAt,
  readPng,
  readTree,
  runScenario,
} from "./harness.mjs";

const APP_SOURCE = `
import { Button, createCanvasScene, renderApp, type NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => {
    const linearScene = createCanvasScene();
    linearScene.update([
      {
        kind: "rect",
        x: 0,
        y: 0,
        width: 120,
        height: 40,
        fill: {
          kind: "linear",
          x0: 0,
          y0: 0,
          x1: 120,
          y1: 0,
          stops: [
            { offset: 0, color: "#ff0000" },
            { offset: 1, color: "#0000ff" },
          ],
        },
      },
    ]);

    const radialScene = createCanvasScene();
    radialScene.update([
      {
        kind: "circle",
        x: 60,
        y: 60,
        radius: 50,
        fill: {
          kind: "radial",
          cx: 60,
          cy: 60,
          radius: 50,
          stops: [
            { offset: 0, color: "#00ff00" },
            { offset: 1, color: "#ff0000" },
          ],
        },
      },
    ]);

    const dashScene = createCanvasScene();
    dashScene.update([
      {
        kind: "path",
        segments: [
          { kind: "moveTo", x: 0, y: 20 },
          { kind: "lineTo", x: 128, y: 20 },
        ],
        stroke: {
          color: "#000000",
          width: 8,
          lineCap: "butt",
          lineDash: [16, 16],
          lineDashOffset: 0,
        },
      },
      {
        kind: "path",
        segments: [
          { kind: "moveTo", x: 0, y: 52 },
          { kind: "lineTo", x: 128, y: 52 },
        ],
        stroke: {
          color: {
            kind: "linear",
            x0: 0,
            y0: 52,
            x1: 128,
            y1: 52,
            stops: [
              { offset: 0, color: "#ff0000" },
              { offset: 1, color: "#0000ff" },
            ],
          },
          width: 8,
          lineCap: "butt",
          lineDash: [16, 16],
          lineDashOffset: 16,
        },
      },
    ]);

    return (
      <column style={{ width: "100%", height: "100%", backgroundColor: "#ffffff", padding: "16px", gap: "12px" }}>
        <vector testId="linear-canvas" canvas={linearScene} style={{ width: "180px", height: "40px", backgroundColor: "#ffffff" }} />
        <vector testId="radial-canvas" canvas={radialScene} style={{ width: "120px", height: "120px", backgroundColor: "#ffffff" }} />
        <vector testId="dash-canvas" canvas={dashScene} style={{ width: "160px", height: "72px", backgroundColor: "#ffffff" }} />
        <Button
          testId="shift-transform"
          onClick={() => {
            linearScene.update(null, [1, 0, 0, 1, 40, 0]);
          }}
        >
          Shift Transform
        </Button>
      </column>
    );
  }, root);
}
`;

async function sampleHelper(out, name) {
  const [image, tree] = await Promise.all([
    readPng(path.join(out, `captures/${name}.png`)),
    readTree(out, name),
  ]);
  return (testId, x, y) => {
    const bounds = getNode(tree, testId).boundsDevice;
    assert.ok(bounds && bounds.width > x && bounds.height > y, `${name}: ${testId} bounds`);
    return pixelAt(image, Math.floor(bounds.x + x), Math.floor(bounds.y + y));
  };
}

test("CanvasScene Phase 2a renders linear/radial gradients, dashed strokes, and view-only transforms", { timeout: 30000 }, async t => {
  const temp = await createTempWorkspace("preview-canvas-phase2a-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(temp.dir, "canvas-phase2a", APP_SOURCE, { rawSource: true });
  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "canvas-phase2a",
    scale: 1,
    assertEmptyResources: true,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect"],
    steps: [
      { id: "s1", op: "clock.advance", byMs: 40 },
      { id: "s2", op: "capture", captureId: "initial" },
      { id: "s3", op: "pointer.click", target: { surface: "main", testId: "shift-transform" }, button: "primary" },
      { id: "s4", op: "clock.advance", byMs: 40 },
      { id: "s5", op: "capture", captureId: "shifted" },
    ],
  });

  const initial = await sampleHelper(out, "initial");

  // 1. Linear gradient ramp (#ff0000 -> #0000ff over x=0..120)
  const linLeft = initial("linear-canvas", 4, 20);
  const linMid = initial("linear-canvas", 60, 20);
  const linRight = initial("linear-canvas", 116, 20);
  const linPast = initial("linear-canvas", 140, 20);
  assert.ok(linLeft[0] > 210 && linLeft[2] < 45, `expected red at linear start, got ${linLeft}`);
  assert.ok(
    linMid[0] > 70 && linMid[0] < 190 && linMid[2] > 70 && linMid[2] < 190,
    `expected interpolated purple at linear midpoint, got ${linMid}`,
  );
  assert.ok(linRight[2] > 210 && linRight[0] < 45, `expected blue at linear end, got ${linRight}`);
  assert.deepEqual(linPast, [255, 255, 255, 255], `expected white outside rect, got ${linPast}`);

  // 2. Radial gradient (#00ff00 center -> #ff0000 edge)
  const radCenter = initial("radial-canvas", 60, 60);
  const radMid = initial("radial-canvas", 85, 60);
  const radEdge = initial("radial-canvas", 107, 60);
  assert.ok(radCenter[1] > 220 && radCenter[0] < 35, `expected green at radial center, got ${radCenter}`);
  assert.ok(radMid[0] > 60 && radMid[1] > 60, `expected blended mid radial ring, got ${radMid}`);
  assert.ok(radEdge[0] > 200 && radEdge[1] < 60, `expected red near radial edge, got ${radEdge}`);

  // 3. Dashed stroke (general vector route): offset 0 -> dashes [0..16], [32..48], gaps [16..32], [48..64]
  const dash0On1 = initial("dash-canvas", 8, 20);
  const dash0Gap1 = initial("dash-canvas", 24, 20);
  const dash0On2 = initial("dash-canvas", 40, 20);
  const dash0Gap2 = initial("dash-canvas", 56, 20);
  assert.ok(dash0On1[0] < 30 && dash0On1[1] < 30 && dash0On1[2] < 30, `expected inked dash at x=8, got ${dash0On1}`);
  assert.deepEqual(dash0Gap1, [255, 255, 255, 255], `expected white gap at x=24, got ${dash0Gap1}`);
  assert.ok(dash0On2[0] < 30 && dash0On2[1] < 30 && dash0On2[2] < 30, `expected inked dash at x=40, got ${dash0On2}`);
  assert.deepEqual(dash0Gap2, [255, 255, 255, 255], `expected white gap at x=56, got ${dash0Gap2}`);

  // 4. Phase-shifted gradient dashed stroke (offset 16): gaps at x=8, 40; inked red->blue dashes at x=24, 88
  const dash1Gap1 = initial("dash-canvas", 8, 52);
  const dash1OnRed = initial("dash-canvas", 24, 52);
  const dash1Gap2 = initial("dash-canvas", 40, 52);
  const dash1OnBlue = initial("dash-canvas", 88, 52);
  assert.deepEqual(dash1Gap1, [255, 255, 255, 255], `expected shifted gap at x=8, got ${dash1Gap1}`);
  assert.ok(dash1OnRed[0] > 160 && dash1OnRed[0] > dash1OnRed[2] + 80, `expected reddish inked dash at x=24, got ${dash1OnRed}`);
  assert.deepEqual(dash1Gap2, [255, 255, 255, 255], `expected shifted gap at x=40, got ${dash1Gap2}`);
  assert.ok(dash1OnBlue[2] > 130 && dash1OnBlue[2] > dash1OnBlue[0] + 40, `expected bluish inked dash at x=88, got ${dash1OnBlue}`);

  // 5. View-only update(null, transform) shifts linear rect right by 40px without re-uploading geometry
  const shifted = await sampleHelper(out, "shifted");
  assert.deepEqual(shifted("linear-canvas", 10, 20), [255, 255, 255, 255], "expected vacated left strip to be white");
  const shiftedStart = shifted("linear-canvas", 44, 20);
  const shiftedEnd = shifted("linear-canvas", 156, 20);
  assert.ok(shiftedStart[0] > 210 && shiftedStart[2] < 45, `expected shifted red start at x=44, got ${shiftedStart}`);
  assert.ok(shiftedEnd[2] > 210 && shiftedEnd[0] < 45, `expected shifted blue end at x=156, got ${shiftedEnd}`);

  assert.equal(result.teardown?.clean, true);
});
