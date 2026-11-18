import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
import { readFile } from "node:fs/promises";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode as get,
  readPng as pngRgba,
  readTree as tree,
  readFrame,
  workspaceRoot,
  runScenario,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-vector-text-");
  bundle = await buildFixtureApp(temp.dir, "vector-text", new URL("./fixtures/vector-text.tsx", import.meta.url), {
    assets: { "package-font.ttf": await readFile(path.join(workspaceRoot, "preview/fonts/NotoSans-Regular.ttf")) },
    fonts: [{ family: "PackageGlyphSans", path: "assets/package-font.ttf" }],
  });
});
after(async () => { await temp?.cleanup(); });

let serial = 0;
const id = () => `step-${++serial}`;
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

function run(steps, name) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `vector-text-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "tree.inspect", "surface.resize", "surface.recreate"],
    steps,
    assertEmptyResources: false,
  });
}

test("scaled text rasterizes glyph pixels through capture resize and recreate", async () => {
  const { out } = await run([
    check("plain", "text", "Readable glyphs"), check("scaled", "text", "Readable glyphs"), check("faded", "text", "Readable glyphs"), capture("initial", 0),
    { id: id(), op: "surface.resize", surface: "main", width: 700, height: 500 }, advance(20), capture("resized"),
    { id: id(), op: "surface.recreate", surface: "main" }, advance(20), capture("recreated")
  ], "native");
  for (const name of ["initial", "resized", "recreated"]) {
    const snapshot = await tree(out, name);
    const frame = await readFrame(out, name);
    assert.equal(frame.resources.requiredFontsReady, true);
    assert.ok(frame.resources.fonts.some(font => font.family === "PackageGlyphSans" && font.path === "assets/package-font.ttf" && font.state === "ready"));
    assert.equal(snapshot.styleRevision, frame.styleRevision);
    assert.equal(typeof frame.pixelPresentation.styleRevision, "number");
    const image = await pngRgba(path.join(out, `captures/${name}.png`));
    for (const testId of ["plain", "scaled", "faded"]) {
      const bounds = get(snapshot, testId).boundsDevice;
      assert.ok(bounds && bounds.width > 0 && bounds.height > 0);
      const left = Math.max(0, Math.ceil(bounds.x));
      const top = Math.max(0, Math.ceil(bounds.y));
      const right = Math.min(image.width, Math.floor(bounds.x + bounds.width));
      const bottom = Math.min(image.height, Math.floor(bounds.y + bounds.height));
      assert.ok(right - left > 4 && bottom - top > 4);
      const background = ((bottom - 3) * image.width + right - 3) * 4;
      assert.ok(image.data[background] > 240 && image.data[background + 1] > 240 && image.data[background + 2] > 240,
        `${name}: ${testId} pixel test requires a white background`);
      let ink = false;
      for (let y = top; y < bottom && !ink; y++) for (let x = left; x < right; x++) {
        const offset = (y * image.width + x) * 4;
        if (image.data[offset] < 200 && image.data[offset + 1] < 200 && image.data[offset + 2] < 200) { ink = true; break; }
      }
      assert.equal(ink, true, `${name}: ${testId} has text in the native tree but no visible glyph pixels`);
    }
  }
});
