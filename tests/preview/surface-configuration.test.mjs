import assert from "node:assert/strict";
import { before, after, test } from "node:test";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";

// Explicit real-GPU gate: a changed logical configuration can have the same pixel extent.
let project;
before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-configuration-"));
  await mkdir(path.join(project, "src"));
  await copyFile(
    fileURLToPath(new URL("./fixtures/surface-configuration.tsx", import.meta.url)),
    path.join(project, "src/main.tsx"),
  );
  await writeFile(
    path.join(project, "tsconfig.json"),
    JSON.stringify({
      extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
      include: ["src/**/*.tsx"],
    }),
  );
  await buildApplication("surface-configuration", { appDirectory: project });
});
after(async () => {
  if (project) await rm(project, { recursive: true, force: true });
});

const reconfigure = [
  { op: "capture", captureId: "initial" },
  { op: "surface.resize", surface: "main", width: 400, height: 300 },
  { op: "surface.scale", surface: "main", scale: 2 },
];
const click = {
  op: "pointer.click",
  target: { surface: "main", testId: "trip" },
  button: "primary",
};
async function run(name, steps) {
  const scenario = path.join(project, `${name}.json`),
    out = path.join(project, `${name}-run`);
  await writeFile(
    scenario,
    JSON.stringify({
      schemaVersion: 1,
      id: name,
      requiredCapabilities: [],
      steps: steps.map((step, index) => ({ id: `action-${index}`, ...step })),
    }),
  );
  const process = spawnSync(
    path.join(workspaceRoot, "target/debug/zgui-host"),
    [
      "preview",
      "run",
      "--bundle",
      path.join(project, "dist"),
      "--profile",
      "preview/profiles/default.json",
      "--scale",
      "1",
      "--fixture",
      "preview/fixtures/empty.json",
      "--scenario",
      scenario,
      "--out",
      out,
    ],
    { cwd: workspaceRoot, encoding: "utf8", timeout: 25000 },
  );
  assert.ifError(process.error);
  const result = JSON.parse(process.stdout);
  assert.equal(process.status, result.exitCode);
  return { result, out };
}

test("same pixel extent cannot certify a preceding logical configuration", async () => {
  const { result, out } = await run("capture", [
    ...reconfigure,
    { op: "capture", captureId: "changed" },
  ]);
  if (result.exitCode === 5) {
    assert.equal(result.firstError.code, "E_CAPTURE_NOT_READY");
    assert.equal(result.captureCount, 1);
    assert.equal(result.firstError.details.timeUs, 0);
  } else {
    assert.equal(result.exitCode, 0, JSON.stringify(result));
    // A native backend may genuinely complete a same-time frame; it must own the new state.
    const frame = JSON.parse(await readFile(path.join(out, "captures/changed.frame.json"), "utf8"));
    const tree = JSON.parse(await readFile(path.join(out, "captures/changed.tree.json"), "utf8"));
    assert.equal(frame.scale, 2);
    assert.equal(frame.surfaceConfiguration, 2);
    assert.equal(tree.surfaceConfiguration, frame.surfaceConfiguration);
    assert.deepEqual([frame.physicalWidth, frame.physicalHeight], [800, 600]);
  }
});

test("a final action cannot disappear behind deferred reconfiguration", async () => {
  const { result, out } = await run("input", [...reconfigure, click]);
  if (result.exitCode === 5) {
    assert.equal(result.firstError.code, "E_CAPTURE_NOT_READY");
  } else {
    assert.equal(result.exitCode, 3, JSON.stringify(result));
    assert.match(
      await readFile(path.join(out, "diagnostics.jsonl"), "utf8"),
      /configuration-action-dispatched/,
    );
  }
  assert.equal(result.firstError.stage, "pointer.click");
});

test("explicit advance validates the new configuration and dispatches input", async () => {
  const { result, out } = await run("advanced", [
    ...reconfigure,
    { op: "clock.advance", byMs: 20 },
    { op: "capture", captureId: "changed" },
    click,
  ]);
  assert.equal(result.exitCode, 3, JSON.stringify(result));
  assert.equal(result.firstError.stage, "pointer.click");
  assert.match(
    await readFile(path.join(out, "diagnostics.jsonl"), "utf8"),
    /configuration-action-dispatched/,
  );
  const frame = JSON.parse(await readFile(path.join(out, "captures/changed.frame.json"), "utf8"));
  assert.equal(frame.scale, 2);
  assert.equal(frame.surfaceConfiguration, 2);
});
