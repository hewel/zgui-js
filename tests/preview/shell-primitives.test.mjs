import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { after, before, test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  pixelAt,
  readPng,
  readTree,
  runScenario,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-shell-primitives-");
  bundle = await buildFixtureApp(
    temp.dir,
    "shell-primitives",
    new URL("./fixtures/shell-primitives.tsx", import.meta.url),
    {
      files: {
        "src/shell-primitives.module.css": `.glass { backdrop-filter: grayscale(1); }\n.lettering { font-variation-settings: "wght" 650; text-shadow: 24px 0 #0000ff; }\n`,
      },
    },
  );
});
after(async () => {
  await temp?.cleanup();
});

const capture = captureId => ({ id: captureId, op: "capture", captureId });
const advance = (id, byMs) => ({ id, op: "clock.advance", byMs });
const wheel = (id, x) => ({
  id,
  op: "wheel",
  surface: "main",
  position: { x: 30, y: 125 },
  delta: { x, y: 3, unit: "lines" },
});

function run(name, steps, options = {}) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `shell-primitives-${name}`,
    steps,
    requiredCapabilities: [
      "pixels",
      "virtual-time",
      "selectors.test-id",
      "input.pointer",
      "input.wheel",
      "tree.inspect",
    ],
    width: 420,
    height: 280,
    scale: 1,
    assertEmptyResources: false,
    ...options,
  });
}

test("CSS Modules and inline visual properties reach native preflight and paint", async () => {
  const css = await readFile(path.join(bundle, "app.css"), "utf8");
  assert.match(css, /font-variation-settings:\s*["']wght["'] 650/);
  const { out } = await run("visual", [capture("visual")]);
  const image = await readPng(path.join(out, "captures/visual.png"));
  assert.deepEqual(pixelAt(image, 40, 36), [255, 0, 0, 255], "unfiltered background");
  const [r, g, b] = pixelAt(image, 110, 36);
  assert.ok(
    r > 0 && r < 255 && Math.abs(r - g) <= 1 && Math.abs(g - b) <= 1,
    "filtered red backdrop must be gray",
  );
  assert.deepEqual(pixelAt(image, 238, 36), [0, 255, 0, 255], "box-shadow outside the native box");
  let textShadow = false;
  for (let y = 16; y < 64 && !textShadow; y += 1) {
    for (let x = 300; x < 350 && !textShadow; x += 1) {
      const [red, , blue] = pixelAt(image, x, y);
      textShadow = blue - red > 100;
    }
  }
  assert.ok(textShadow, "text-shadow produces blue glyph pixels");
});

for (const [name, declaration] of [
  ["backdrop-filter", "backdrop-filter: blur(red);"],
  ["box-shadow", "box-shadow: 1px;"],
]) {
  test(`native preflight rejects invalid CSS ${name} before mounting`, async () => {
    const id = `invalid-${name}`;
    const rejectedBundle = await buildFixtureApp(
      path.join(temp.dir, id),
      id,
      `
      import { renderApp } from "@hewel/zgui";
      import type { NativeNode } from "@hewel/zgui";
      import styles from "./bad.module.css";
      export const hostApiVersion = 1;
      export function mount(root: NativeNode) {
        return renderApp(() => <box class={styles.box} />, root);
      }
    `,
      {
        rawSource: true,
        files: { "src/bad.module.css": `.box { ${declaration} }\n` },
      },
    );
    const { result } = await run(id, [capture("rejected")], {
      bundle: rejectedBundle,
      allowFailure: true,
    });
    assert.equal(result.firstError?.code, "E_STYLE_PROFILE");
    assert.equal(result.captureCount, 0);
  });
}

test("wheel cancellation stops native scrolling and propagation without swallowing other wheels", async () => {
  const { out } = await run("wheel", [
    capture("initial"),
    wheel("cancelled-wheel", 1),
    advance("after-cancel", 400),
    capture("cancelled"),
    wheel("allowed-wheel", 0),
    advance("after-scroll", 400),
    capture("scrolled"),
  ]);
  const [initial, cancelled, scrolled] = await Promise.all(
    ["initial", "cancelled", "scrolled"].map(name => readTree(out, name)),
  );
  const y = snapshot => getNode(snapshot, "wheel-marker").boundsLogical.y;
  assert.equal(y(cancelled), y(initial), "preventDefault suppressed the native scroll offset");
  assert.ok(y(scrolled) < y(initial), "an uncancelled wheel scrolls the native content");
  assert.equal(getNode(cancelled, "wheel-log").text, "capture:mouse:lines:discrete:1,3:30,125");
  assert.equal(
    getNode(scrolled, "wheel-log").text,
    "capture:mouse:lines:discrete:0,3:30,125|target|bubble",
  );
  const initialScroll = getNode(initial, "wheel-scroll").scrollMetrics;
  assert.ok(initialScroll, "scroll container exports scrollMetrics");
  assert.equal(initialScroll.offsetY, 0);
  assert.equal(initialScroll.viewportHeight, 100);
  assert.equal(initialScroll.contentHeight, 400);
  assert.equal(initialScroll.overflowY, true);
  assert.equal(initialScroll.overflowX, false);
  assert.equal(initialScroll.scrollsY, true);
  assert.equal(
    getNode(initial, "wheel-marker").scrollMetrics,
    undefined,
    "non-scroll nodes omit scrollMetrics",
  );
  const scrolledMetrics = getNode(scrolled, "wheel-scroll").scrollMetrics;
  assert.ok(
    scrolledMetrics.offsetY > 0,
    `expected positive scroll offsetY, got ${scrolledMetrics.offsetY}`,
  );
});

test("native spring frames converge and the preview clock advances without scheduling more", async () => {
  const { out } = await run("spring", [
    capture("initial"),
    {
      id: "start",
      op: "pointer.click",
      target: { surface: "main", testId: "start" },
      button: "primary",
    },
    advance("first-motion", 80),
    capture("moving"),
    advance("converge", 1800),
    capture("settled"),
    advance("idle", 250),
  ]);
  const width = async name => getNode(await readTree(out, name), "spring").boundsLogical.width;
  assert.equal(await width("initial"), 40);
  const moving = await width("moving");
  assert.ok(moving > 40 && moving < 140, `expected intermediate width, got ${moving}`);
  assert.equal(await width("settled"), 140);
  const actions = (await readFile(path.join(out, "steps.jsonl"), "utf8"))
    .trim()
    .split("\n")
    .map(JSON.parse);
  const idle = actions.find(step => step.id === "idle").result;
  assert.equal(idle.nativeFrames, 0);
  assert.equal(idle.readiness.frameCurrent, true);
});
