import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";
import { pngRgba } from "./png.mjs";

let project;
before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-vector-text-"));
  await mkdir(path.join(project, "src"));
  await copyFile(fileURLToPath(new URL("./fixtures/vector-text.tsx", import.meta.url)), path.join(project, "src/main.tsx"));
  await writeFile(path.join(project, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await buildApplication("vector-text", { appDirectory: project });
});
after(async () => {
  if (!project) return;
  if (process.env.ZGUI_KEEP_PREVIEW_OUTPUT === "1") {
    console.log(`retained preview project: ${project}`);
  } else {
    await rm(project, { recursive: true, force: true });
  }
});

let serial = 0;
const id = () => `step-${++serial}`;
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `vector-text-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "tree.inspect", "surface.resize", "surface.recreate"],
    steps,
  }));
  const process = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "run",
    "--bundle", path.join(project, "dist"),
    "--profile", path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario", scenario,
    "--out", out,
  ], { cwd: workspaceRoot, encoding: "utf8", timeout: 60000 });
  if (process.stderr) await writeFile(path.join(project, `${name}.stderr`), process.stderr);
  assert.ifError(process.error);
  const result = JSON.parse(process.stdout);
  assert.equal(process.signal, null, process.stderr);
  assert.equal(process.status, result.exitCode);
  assert.equal(result.exitCode, 0, process.stdout);
  assert.equal(result.status, "passed", process.stdout);
  assert.equal(result.assertions.failed, 0);
  assert.equal(result.assertions.passed, steps.filter(step => step.op === "assert").length);
  assert.equal(result.teardown.clean, true);
  return { result, out };
}

const tree = async (out, name) => JSON.parse(await readFile(path.join(out, `captures/${name}.tree.json`), "utf8"));
const get = (tree, testId) => {
  const node = tree.nodes.find(node => node.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};

test("scaled text rasterizes glyph pixels through capture resize and recreate", async () => {
  const { out } = await run([
    check("plain", "text", "Readable glyphs"), check("scaled", "text", "Readable glyphs"), check("faded", "text", "Readable glyphs"), capture("initial", 0),
    { id: id(), op: "surface.resize", surface: "main", width: 700, height: 500 }, advance(20), capture("resized"),
    { id: id(), op: "surface.recreate", surface: "main" }, advance(20), capture("recreated")
  ], "native");
  for (const name of ["initial", "resized", "recreated"]) {
    const snapshot = await tree(out, name);
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
