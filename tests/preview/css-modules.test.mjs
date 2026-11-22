import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildExampleApp,
  createTempWorkspace,
  getNode,
  pixelAt,
  readPng,
  readTree,
  runScenario,
} from "./harness.mjs";

const CAPS = ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect"];

function centerPixel(image, node) {
  const { x, y, width, height } = node.boundsLogical;
  return pixelAt(image, Math.round(x + width / 2), Math.round(y + height / 2));
}

test(
  "apps/examples/css-modules renders scoped CSS Modules, flattened nesting, --zui-* theme switches, and @layer zgui.components override with clean teardown",
  { timeout: 60000 },
  async t => {
    const temp = await createTempWorkspace("preview-css-modules-");
    t.after(() => temp.cleanup());

    const bundle = await buildExampleApp("css-modules");
    const { result, out } = await runScenario({
      projectDir: temp.dir,
      bundle,
      name: "css-modules-showcase",
      id: "css-modules-showcase",
      width: 640,
      height: 420,
      scale: 1,
      requiredCapabilities: CAPS,
      steps: [
        { id: "s1", op: "clock.advance", byMs: 20 },
        { id: "c1", op: "capture", captureId: "light-idle", surface: "main" },

        // Activate nested &[data-active="true"] rule.
        {
          id: "s2",
          op: "pointer.click",
          target: { surface: "main", testId: "btn-toggle-active" },
          button: "primary",
        },
        { id: "s3", op: "clock.advance", byMs: 20 },
        {
          id: "a1",
          op: "assert",
          target: { surface: "main", testId: "card-state" },
          check: { property: "text", equals: "active" },
        },
        { id: "c2", op: "capture", captureId: "light-active", surface: "main" },

        // Switch ThemeProvider to dark + ocean preset (--zui-* tokens update live).
        {
          id: "s4",
          op: "pointer.click",
          target: { surface: "main", testId: "btn-toggle-theme" },
          button: "primary",
        },
        { id: "s5", op: "clock.advance", byMs: 20 },
        {
          id: "a2",
          op: "assert",
          target: { surface: "main", testId: "card-title" },
          check: { property: "text", equals: "CSS Modules (dark/ocean)" },
        },
        { id: "c3", op: "capture", captureId: "dark-ocean", surface: "main" },
      ],
    });

    assert.equal(result.teardown?.clean, true, JSON.stringify(result.teardown));

    const [lightIdleImg, lightActiveImg, darkOceanImg] = await Promise.all([
      readPng(path.join(out, "captures/light-idle.png")),
      readPng(path.join(out, "captures/light-active.png")),
      readPng(path.join(out, "captures/dark-ocean.png")),
    ]);
    const [lightIdleTree, lightActiveTree, darkOceanTree] = await Promise.all([
      readTree(out, "light-idle"),
      readTree(out, "light-active"),
      readTree(out, "dark-ocean"),
    ]);

    // 1. Nested &[data-active="true"] changes card background/border color in light theme.
    const cardIdle = getNode(lightIdleTree, "scoped-card");
    const cardActive = getNode(lightActiveTree, "scoped-card");
    const idlePx = pixelAt(
      lightIdleImg,
      Math.round(cardIdle.boundsLogical.x + cardIdle.boundsLogical.width - 8),
      Math.round(cardIdle.boundsLogical.y + cardIdle.boundsLogical.height - 8),
    );
    const activePx = pixelAt(
      lightActiveImg,
      Math.round(cardActive.boundsLogical.x + cardActive.boundsLogical.width - 8),
      Math.round(cardActive.boundsLogical.y + cardActive.boundsLogical.height - 8),
    );
    assert.notDeepEqual(
      activePx,
      idlePx,
      "nested &[data-active='true'] must visibly change scoped card fill",
    );

    // 2. --zui-* ThemeProvider dark/ocean switch changes background from light to dark.
    const lightBg = pixelAt(lightIdleImg, 620, 400);
    const darkBg = pixelAt(darkOceanImg, 620, 400);
    assert.ok(
      lightBg[0] > 200 && lightBg[1] > 200 && lightBg[2] > 200,
      `expected light background, got ${lightBg}`,
    );
    assert.ok(
      darkBg[0] < 60 && darkBg[1] < 60 && darkBg[2] < 80,
      `expected dark ocean background, got ${darkBg}`,
    );

    // 3. Unlayered CSS Module .overrideButton overrides @layer zgui.components Button background (#0f766e -> rgb(15, 118, 110)).
    const overrideBtn = getNode(darkOceanTree, "btn-layer-override");
    const btnEdgePx = pixelAt(
      darkOceanImg,
      Math.round(overrideBtn.boundsLogical.x + 4),
      Math.round(overrideBtn.boundsLogical.y + 4),
    );
    assert.ok(
      btnEdgePx[0] < 40 && btnEdgePx[1] > 95 && btnEdgePx[2] > 90,
      `expected unlayered CSS Module teal (#0f766e) to override @layer zgui.components Button, got rgba(${btnEdgePx.join(", ")})`,
    );
    // Sanity-check center pixel is non-transparent.
    const center = centerPixel(darkOceanImg, overrideBtn);
    assert.equal(center[3], 255);
  },
);
