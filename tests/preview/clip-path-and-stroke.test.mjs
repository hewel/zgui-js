import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
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

const DIAMOND = 'path("M 20 0 L 40 20 L 20 40 L 0 20 Z")';

const source = `
import { Image, createCanvasScene, mountStylesheet, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet("reset", ":root { padding: 0; background: #10141c; }");
    const roundScene = createCanvasScene();
    roundScene.update([
      {
        kind: "path",
        segments: [
          { kind: "moveTo", x: 20, y: 20 },
          { kind: "lineTo", x: 60, y: 20 },
        ],
        stroke: { color: "#38bdf8", width: 16, lineCap: "round", lineJoin: "round" },
      },
    ]);
    const buttScene = createCanvasScene();
    buttScene.update([
      {
        kind: "path",
        segments: [
          { kind: "moveTo", x: 20, y: 20 },
          { kind: "lineTo", x: 60, y: 20 },
        ],
        stroke: { color: "#38bdf8", width: 16, lineCap: "butt", lineJoin: "miter" },
      },
    ]);
    return <column style={{ width: "320px", height: "200px", backgroundColor: "#10141c", padding: "10px", gap: "10px" }}>
      <row style={{ gap: "10px" }}>
        <Image
          testId="clipped-img"
          src="assets/red.png"
          style={{ width: "40px", height: "40px", objectFit: "cover", clipPath: '${DIAMOND}' }}
        />
        <box
          testId="clipped-wrapper"
          style={{ width: "40px", height: "40px", clipPath: '${DIAMOND}' }}
        >
          <Image src="assets/red.png" style={{ width: "40px", height: "40px", objectFit: "cover" }} />
        </box>
        <box
          testId="clipped-box"
          style={{ width: "40px", height: "40px", backgroundColor: "#00dd88", clipPath: '${DIAMOND}' }}
        />
        <box
          testId="dashed-box"
          style={{ width: "60px", height: "40px", borderWidth: "2px", borderTopStyle: "dashed", borderRightStyle: "dashed", borderBottomStyle: "dashed", borderLeftStyle: "dashed", borderColor: "#ffffff" }}
        />
      </row>
      <row style={{ gap: "10px" }}>
        <vector testId="canvas-round" canvas={roundScene} style={{ width: "80px", height: "40px" }} />
        <vector testId="canvas-butt" canvas={buttScene} style={{ width: "80px", height: "40px" }} />
        <vector
          testId="svg-round"
          d="M 20 20 L 60 20"
          viewBox="0 0 80 40"
          style={{
            width: "80px",
            height: "40px",
            "--zgui-fill": "transparent",
            "--zgui-stroke": "#ffaa00",
            "--zgui-stroke-width": "16px",
            "--zgui-stroke-linecap": "round",
            "--zgui-stroke-linejoin": "round",
          }}
        />
      </row>
    </column>;
  }, root);
}
`;

function expectBackground(pixel, label) {
  // #10141c = (16, 20, 28)
  assert.ok(
    pixel[0] < 35 && pixel[1] < 35 && pixel[2] < 45,
    `${label}: expected dark background pixel, got rgba(${pixel.join(", ")})`,
  );
}

test("clip-path: path(...) clips decoded images and boxes, and rounded stroke caps render on canvas and vector paths", async t => {
  const temp = await createTempWorkspace("preview-clip-path-stroke-");
  t.after(() => temp.cleanup());
  const redPng = await readFile(new URL("./fixtures/red.png", import.meta.url));
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "clip-path-and-stroke", source, {
    rawSource: true,
    assets: { "red.png": redPng },
  });
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "clip-path-and-stroke",
    requiredCapabilities: [],
    width: 320,
    height: 200,
    scale: 1,
    steps: [
      { id: "wait-img", op: "clock.advance", byMs: 50 },
      { id: "snap", op: "capture", captureId: "snap" },
    ],
  });

  const tree = await readTree(out, "snap");
  const png = await readPng(path.join(out, "captures/snap.png"));

  // 1. Direct clip-path: path(...) on <Image>
  const imgBounds = getNode(tree, "clipped-img").boundsLogical;
  const imgCenter = pixelAt(png, Math.round(imgBounds.x + 20), Math.round(imgBounds.y + 20));
  assert.ok(
    imgCenter[0] > 180 && imgCenter[1] < 60,
    `clipped-img center must be red, got ${imgCenter}`,
  );
  for (const [dx, dy] of [
    [2, 2],
    [38, 2],
    [2, 38],
    [38, 38],
  ]) {
    expectBackground(
      pixelAt(png, Math.round(imgBounds.x + dx), Math.round(imgBounds.y + dy)),
      `clipped-img corner (${dx},${dy})`,
    );
  }

  // 2. Ancestor <box> clip-path: path(...) clipping child <Image>
  const wrapBounds = getNode(tree, "clipped-wrapper").boundsLogical;
  const wrapCenter = pixelAt(png, Math.round(wrapBounds.x + 20), Math.round(wrapBounds.y + 20));
  assert.ok(
    wrapCenter[0] > 180 && wrapCenter[1] < 60,
    `clipped-wrapper center must be red, got ${wrapCenter}`,
  );
  expectBackground(
    pixelAt(png, Math.round(wrapBounds.x + 2), Math.round(wrapBounds.y + 2)),
    "clipped-wrapper corner (2,2)",
  );

  // 3. Solid <box> background with clip-path: path(...)
  const boxBounds = getNode(tree, "clipped-box").boundsLogical;
  const boxCenter = pixelAt(png, Math.round(boxBounds.x + 20), Math.round(boxBounds.y + 20));
  assert.ok(
    boxCenter[1] > 180 && boxCenter[0] < 60,
    `clipped-box center must be green, got ${boxCenter}`,
  );
  expectBackground(
    pixelAt(png, Math.round(boxBounds.x + 2), Math.round(boxBounds.y + 2)),
    "clipped-box corner (2,2)",
  );

  // 4. CanvasCommand.stroke lineCap: "round" extends 8px past x=20; "butt" stops sharply at x=20
  const roundBounds = getNode(tree, "canvas-round").boundsLogical;
  const roundCapPx = pixelAt(png, Math.round(roundBounds.x + 15), Math.round(roundBounds.y + 20));
  assert.ok(roundCapPx[2] > 180, `canvas-round cap at x=15 must be cyan, got ${roundCapPx}`);

  const buttBounds = getNode(tree, "canvas-butt").boundsLogical;
  expectBackground(
    pixelAt(png, Math.round(buttBounds.x + 15), Math.round(buttBounds.y + 20)),
    "canvas-butt outside cap at x=15",
  );
  const buttBodyPx = pixelAt(png, Math.round(buttBounds.x + 40), Math.round(buttBounds.y + 20));
  assert.ok(buttBodyPx[2] > 180, `canvas-butt stroke body at x=40 must be cyan, got ${buttBodyPx}`);

  // 5. --zgui-stroke-linecap: round on <vector d="..."> extends 8px past x=20
  const svgBounds = getNode(tree, "svg-round").boundsLogical;
  const svgCapPx = pixelAt(png, Math.round(svgBounds.x + 15), Math.round(svgBounds.y + 20));
  assert.ok(
    svgCapPx[0] > 200 && svgCapPx[1] > 120,
    `svg-round cap at x=15 must be amber, got ${svgCapPx}`,
  );
});

const clipAndBordersSource = `
import { mountStylesheet, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet("reset", ":root { padding: 0; background: #10141c; }");
    return <column style={{ width: "600px", height: "300px", backgroundColor: "#10141c", padding: "36px 20px", gap: "24px" }}>
      <row style={{ gap: "20px" }}>
        <box
          testId="wide-circle"
          style={{ width: "260px", height: "110px", backgroundColor: "#2563eb", clipPath: "circle(45% at 50% 50%)" }}
        />
        <box
          testId="sibling-inset"
          style={{ width: "260px", height: "110px", backgroundColor: "#059669", clipPath: "inset(10% 8% round 22px)" }}
        />
      </row>
      <box
        testId="mixed-borders"
        style={{
          width: "240px",
          height: "60px",
          backgroundColor: "#10141c",
          borderTopWidth: "4px",
          borderRightWidth: "4px",
          borderBottomWidth: "6px",
          borderLeftWidth: "4px",
          borderTopStyle: "dashed",
          borderRightStyle: "dotted",
          borderBottomStyle: "double",
          borderLeftStyle: "solid",
          borderTopColor: "#3b82f6",
          borderRightColor: "#10b981",
          borderBottomColor: "#f59e0b",
          borderLeftColor: "#ec4899",
        }}
      />
    </column>;
  }, root);
}
`;

test("oversized clip-path stays inside border_box without sibling bleed (#41) and per-side border styles including double render independently (#45)", async t => {
  const temp = await createTempWorkspace("preview-clip-borders-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "clip-borders",
    clipAndBordersSource,
    {
      rawSource: true,
    },
  );
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "clip-borders",
    requiredCapabilities: [],
    width: 600,
    height: 300,
    scale: 1,
    steps: [{ id: "snap", op: "capture", captureId: "snap" }],
  });

  const tree = await readTree(out, "snap");
  const png = await readPng(path.join(out, "captures/snap.png"));

  // #41: Wide circle extends above/below 110px height, so pixels just outside its border_box and above its sibling must stay dark background.
  const circleBounds = getNode(tree, "wide-circle").boundsLogical;
  const siblingBounds = getNode(tree, "sibling-inset").boundsLogical;
  const centerPx = pixelAt(
    png,
    Math.round(circleBounds.x + circleBounds.width / 2),
    Math.round(circleBounds.y + circleBounds.height / 2),
  );
  assert.ok(
    centerPx[2] > 180 && centerPx[0] < 80,
    `wide-circle center must be blue, got ${centerPx}`,
  );
  for (
    let x = Math.round(circleBounds.x);
    x < Math.round(siblingBounds.x + siblingBounds.width);
    x += 8
  ) {
    for (const dy of [-4, -12, -20]) {
      expectBackground(
        pixelAt(png, x, Math.round(circleBounds.y + dy)),
        `above clip row at (${x}, ${Math.round(circleBounds.y + dy)})`,
      );
    }
    expectBackground(
      pixelAt(png, x, Math.round(circleBounds.y + circleBounds.height + 6)),
      `below clip row at x=${x}`,
    );
  }

  // #45: Left solid border is continuous pink along its vertical span; bottom 6px double border has two distinct amber bands separated by a dark gap.
  const borderBox = getNode(tree, "mixed-borders").boundsLogical;
  const leftX = Math.round(borderBox.x + 1);
  for (
    let y = Math.round(borderBox.y + 8);
    y < Math.round(borderBox.y + borderBox.height - 8);
    y += 3
  ) {
    const px = pixelAt(png, leftX, y);
    assert.ok(
      px[0] > 180 && px[2] > 100 && px[1] < 110,
      `left solid border at (${leftX},${y}) must be continuous pink, got ${px}`,
    );
  }
  const midX = Math.round(borderBox.x + borderBox.width / 2);
  const bottomY = Math.round(borderBox.y + borderBox.height);
  const outerAmber = pixelAt(png, midX, bottomY - 1);
  const gapPx = pixelAt(png, midX, bottomY - 3);
  const innerAmber = pixelAt(png, midX, bottomY - 5);
  assert.ok(
    outerAmber[0] > 180 && outerAmber[1] > 120 && outerAmber[2] < 60,
    `double bottom outer band must be amber, got ${outerAmber}`,
  );
  expectBackground(gapPx, "double bottom middle gap");
  assert.ok(
    innerAmber[0] > 180 && innerAmber[1] > 120 && innerAmber[2] < 60,
    `double bottom inner band must be amber, got ${innerAmber}`,
  );
});
