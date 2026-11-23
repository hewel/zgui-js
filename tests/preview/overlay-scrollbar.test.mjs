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

const source = `
import { mountStylesheet, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet(
      "overlay-scrollbar-test",
      \`
      :root {
        --zgui-scrollbar-mode: overlay;
        padding: 0;
        background: #0f172a;
      }
      .stage {
        display: flex;
        flex-direction: row;
        gap: 12px;
        padding: 12px;
        width: 480px;
        height: 160px;
        background: #0f172a;
      }
      .panel {
        width: 140px;
        height: 120px;
        overflow-y: auto;
        background: #0f172a;
        --zgui-scrollbar-thumb: #ff2020;
      }
      .classic {
        --zgui-scrollbar-mode: classic;
      }
      .row {
        width: 100%;
        height: 30px;
        background: #1e40af;
        flex-shrink: 0;
      }
      \`,
    );

    const rows = Array.from({ length: 12 }, (_, index) => index);

    return (
      <row class="stage">
        <scroll testId="fits-scroll" class="panel">
          <box testId="fits-row" class="row" />
        </scroll>
        <scroll testId="overflow-scroll" class="panel">
          {rows.map(index => (
            <box testId={\`overflow-row-\${index}\`} class="row" />
          ))}
        </scroll>
        <scroll testId="classic-scroll" class="panel classic">
          {rows.map(index => (
            <box testId={\`classic-row-\${index}\`} class="row" />
          ))}
        </scroll>
      </row>
    );
  }, root);
}
`;

function expectBlueRow(pixel, label) {
  // #1e40af = (30, 64, 175)
  assert.ok(
    pixel[0] < 55 && pixel[1] > 45 && pixel[1] < 90 && pixel[2] > 150,
    `${label}: expected blue row pixel (#1e40af), got rgba(${pixel.join(", ")})`,
  );
}

test("native overlay scrollbar reserves zero gutter, floats above content on scroll, and fades cleanly back to content", async t => {
  const temp = await createTempWorkspace("preview-overlay-scrollbar-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "overlay-scrollbar", source, {
    rawSource: true,
  });

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "overlay-scrollbar",
    requiredCapabilities: ["input.wheel"],
    width: 480,
    height: 160,
    scale: 1,
    steps: [
      { id: "settle-initial", op: "clock.advance", byMs: 32 },
      { id: "cap-initial", op: "capture", captureId: "initial" },
      {
        id: "wheel-overflow",
        op: "wheel",
        target: { surface: "main", testId: "overflow-scroll" },
        delta: { x: 0, y: 120, unit: "pixels" },
      },
      { id: "settle-revealed", op: "clock.advance", byMs: 32 },
      { id: "cap-revealed", op: "capture", captureId: "revealed" },
      { id: "advance-hold", op: "clock.advance", byMs: 850 },
      { id: "advance-fade", op: "clock.advance", byMs: 350 },
      { id: "cap-faded", op: "capture", captureId: "faded" },
    ],
  });

  assert.equal(result.teardown?.clean, true, "preview teardown must be clean");

  const initialTree = await readTree(out, "initial");
  const fitsRow = getNode(initialTree, "fits-row").boundsLogical;
  const overflowRow = getNode(initialTree, "overflow-row-0").boundsLogical;
  const classicRow = getNode(initialTree, "classic-row-0").boundsLogical;
  const overflowBounds = getNode(initialTree, "overflow-scroll").boundsLogical;

  // 1. Zero layout gutter in overlay mode; classic override still reserves a gutter.
  assert.equal(
    overflowRow.width,
    fitsRow.width,
    `overlay scrollbar must not narrow overflowing content (fits=${fitsRow.width}, overflow=${overflowRow.width})`,
  );
  assert.ok(
    classicRow.width < overflowRow.width - 8,
    `local --zgui-scrollbar-mode: classic override must still reserve a gutter (classic=${classicRow.width}, overlay=${overflowRow.width})`,
  );

  // Sample along the 4px centered capsule strip (x = right - 6 -> right - 4; right - 6 = 134 in 140px box).
  const thumbX = Math.round(overflowBounds.x + overflowBounds.width - 6);
  const clearX = Math.round(overflowBounds.x + overflowBounds.width - 10);

  // 2. Initial state: overlay scrollbar is hidden, so the entire right edge shows blue row content.
  const initialPng = await readPng(path.join(out, "captures/initial.png"));
  for (let dy = 10; dy < 110; dy += 10) {
    expectBlueRow(
      pixelAt(initialPng, thumbX, Math.round(overflowBounds.y + dy)),
      `initial right edge at dy=${dy}`,
    );
  }

  // 3. After wheel scroll: content scrolled and floating red 4px capsule renders over the right edge.
  const revealedTree = await readTree(out, "revealed");
  const scrolledMetrics = getNode(revealedTree, "overflow-scroll").scrollMetrics;
  assert.ok(
    scrolledMetrics && scrolledMetrics.offsetY > 20,
    `expected overflow-scroll to scroll down, got ${JSON.stringify(scrolledMetrics)}`,
  );

  const revealedPng = await readPng(path.join(out, "captures/revealed.png"));
  let redHits = 0;
  for (let dy = 2; dy < 118; dy += 2) {
    const px = pixelAt(revealedPng, thumbX, Math.round(overflowBounds.y + dy));
    if (px[0] > 210 && px[1] < 65 && px[2] < 65) {
      redHits += 1;
    }
  }
  assert.ok(
    redHits >= 10,
    `expected visible red overlay capsule along x=${thumbX} after scroll, found only ${redHits} red sample rows`,
  );
  // 10px from the right edge is outside the 4px resting capsule (inset 4px..8px from right edge),
  // so it must remain pure blue row content.
  expectBlueRow(
    pixelAt(revealedPng, clearX, Math.round(overflowBounds.y + 40)),
    "revealed content outside 4px capsule",
  );

  // 4. After advancing virtual clock past 800ms hold + 200ms fade: thumb fades completely back to blue.
  const fadedPng = await readPng(path.join(out, "captures/faded.png"));
  for (let dy = 10; dy < 110; dy += 10) {
    expectBlueRow(
      pixelAt(fadedPng, thumbX, Math.round(overflowBounds.y + dy)),
      `faded right edge at dy=${dy}`,
    );
  }
});
