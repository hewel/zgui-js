import assert from "node:assert/strict";
import { test } from "node:test";
import { cp, mkdtemp, readFile, rm, unlink, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { workspaceRoot } from "../../packages/build/src/build.mjs";

const baseline = path.join(workspaceRoot, "preview/baselines/counter");
const strictPolicy = path.join(workspaceRoot, "preview/comparison/strict.json");
const hash = bytes => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
const json = async file => JSON.parse(await readFile(file, "utf8"));
const save = (file, value) => writeFile(file, `${JSON.stringify(value)}\n`);

async function fixture(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "zgui-compare-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const actual = path.join(directory, "actual");
  await cp(path.join(baseline, "run"), actual, { recursive: true });
  return { directory, actual };
}

async function rehash(run) {
  const manifest = await json(path.join(run, "artifacts.json"));
  for (const entry of manifest.artifacts) {
    const bytes = await readFile(path.join(run, entry.path));
    entry.bytes = bytes.length;
    entry.digest = hash(bytes);
  }
  await save(path.join(run, "artifacts.json"), manifest);
}

async function policyFixture(directory, mutate) {
  const root = path.join(directory, "reviewed");
  await cp(baseline, root, { recursive: true });
  const policy = await json(strictPolicy);
  mutate(policy);
  const bytes = Buffer.from(`${JSON.stringify(policy)}\n`);
  await writeFile(path.join(root, "policy.json"), bytes);
  const review = await json(path.join(root, "baseline.json"));
  review.policyDigest = hash(bytes);
  review.review.by = "comparison-regression-test";
  review.review.summary = "Isolated explicit policy fixture; never updates the production baseline.";
  await save(path.join(root, "baseline.json"), review);
  return { root, policy: path.join(root, "policy.json") };
}

async function compare(actual, out, reviewed = baseline, policy = strictPolicy) {
  // Saved comparisons must not initialize a graphics backend or connect to Wayland.
  const child = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "compare",
    "--baseline", reviewed, "--actual", actual, "--policy", policy, "--out", out,
  ], { cwd: workspaceRoot, encoding: "utf8", timeout: 15000, maxBuffer: 1024 * 1024,
    env: { ...process.env, VK_DRIVER_FILES: "/nonexistent/preview-no-vulkan.json", VK_ICD_FILENAMES: "/nonexistent/preview-no-vulkan.json", WAYLAND_DISPLAY: "preview-no-wayland" },
  });
  assert.ifError(child.error);
  assert.equal(child.signal, null, child.stderr);
  const result = JSON.parse(child.stdout);
  assert.equal(child.status, result.exitCode);
  assert.ok(Buffer.byteLength(child.stdout) <= 8192);
  assert.deepEqual(await json(path.join(out, "result.json")), result);
  return result;
}

test("complete saved native captures compare exactly without graphics access", async t => {
  const { directory, actual } = await fixture(t);
  const result = await compare(actual, path.join(directory, "out"));
  assert.equal(result.exitCode, 0, JSON.stringify(result));
  assert.equal(result.scope, "all");
  assert.equal(result.captures.compared, 2);
  assert.equal(result.captures.different, 0);
});

test("an unrelated corrupt artifact cannot be ignored by a visual pass", async t => {
  const { directory, actual } = await fixture(t);
  await writeFile(path.join(actual, "steps.jsonl"), "corrupted trace\n");
  const result = await compare(actual, path.join(directory, "out"));
  assert.equal(result.exitCode, 8);
  assert.equal(result.firstError.code, "E_OUTPUT_IO");
});

test("missing expected captures fail even when remaining pixels match", async t => {
  const { directory, actual } = await fixture(t);
  const manifest = await json(path.join(actual, "artifacts.json"));
  const removed = manifest.artifacts.filter(entry => entry.path.startsWith("captures/after-increment."));
  for (const entry of removed) await unlink(path.join(actual, entry.path));
  manifest.artifacts = manifest.artifacts.filter(entry => !removed.includes(entry));
  await save(path.join(actual, "artifacts.json"), manifest);
  const terminal = await json(path.join(actual, "result.json"));
  terminal.captureCount = 1;
  await save(path.join(actual, "result.json"), terminal);
  await rehash(actual);
  const out = path.join(directory, "out");
  const result = await compare(actual, out);
  assert.equal(result.exitCode, 2);
  assert.equal(result.firstError.code, "E_BASELINE_INCOMPATIBLE");
  const report = await json(path.join(out, "report.json"));
  assert.ok(report.compatibilityIssues.some(issue => issue.field === "captureSet" && issue.missing.some(key => key.captureId === "after-increment")));
});

test("driver incompatibility retains original review images without scoring", async t => {
  const { directory, actual } = await fixture(t);
  const graphics = await json(path.join(actual, "graphics.json"));
  graphics.driverInfo = "different reviewed driver environment";
  await save(path.join(actual, "graphics.json"), graphics);
  for (const id of ["initial", "after-increment"]) {
    const file = path.join(actual, `captures/${id}.frame.json`);
    const frame = await json(file);
    frame.renderer = graphics;
    await save(file, frame);
  }
  await rehash(actual);
  const out = path.join(directory, "out");
  const result = await compare(actual, out);
  assert.equal(result.exitCode, 2);
  assert.equal(result.captures.compared, 0);
  const report = await json(path.join(out, "report.json"));
  for (const capture of report.captures) {
    assert.equal(capture.status, "incompatible");
    assert.equal(capture.metrics, null);
    const id = capture.capture.captureId;
    assert.deepEqual(await readFile(path.join(out, capture.artifacts.actual)), await readFile(path.join(actual, `captures/${id}.png`)));
    assert.deepEqual(await readFile(path.join(out, capture.artifacts.baseline)), await readFile(path.join(baseline, `run/captures/${id}.png`)));
  }
});

test("a reviewed subset reports exclusions but still validates their PNGs", async t => {
  const { directory, actual } = await fixture(t);
  const reviewed = await policyFixture(directory, policy => {
    policy.captureSet = { mode: "subset", captures: [{ captureId: "initial", surface: "main" }], reason: "Exercise the explicitly selected initial state." };
  });
  const passed = await compare(actual, path.join(directory, "selected"), reviewed.root, reviewed.policy);
  assert.equal(passed.exitCode, 0, JSON.stringify(passed));
  assert.equal(passed.scope, "subset");
  assert.equal(passed.captures.compared, 1);
  assert.equal(passed.captures.excludedActual, 1);
  const png = path.join(actual, "captures/after-increment.png");
  const bytes = (await readFile(png)).subarray(0, 32);
  await writeFile(png, bytes);
  const frameFile = path.join(actual, "captures/after-increment.frame.json");
  const frame = await json(frameFile);
  frame.pngDigest = hash(bytes);
  await save(frameFile, frame);
  await rehash(actual);
  const rejected = await compare(actual, path.join(directory, "malformed-excluded"), reviewed.root, reviewed.policy);
  assert.notEqual(rejected.exitCode, 0, "an inventory hash cannot certify a truncated excluded capture");
});

test("an all-image mask cannot pass identical images even with explicit review", async t => {
  const { directory, actual } = await fixture(t);
  const frame = await json(path.join(actual, "captures/initial.frame.json"));
  const reviewed = await policyFixture(directory, policy => {
    policy.masks = [{ captureId: "initial", surface: "main", rect: { x: 0, y: 0, width: frame.physicalWidth, height: frame.physicalHeight }, reason: "Deliberately invalid whole-image mask regression." }];
  });
  const result = await compare(actual, path.join(directory, "out"), reviewed.root, reviewed.policy);
  assert.equal(result.exitCode, 2);
  assert.equal(result.firstError.code, "E_SCHEMA");
});

test("policy edits cannot silently weaken a reviewed baseline", async t => {
  const { directory, actual } = await fixture(t);
  const policy = await json(strictPolicy);
  Object.assign(policy, { mode: "tolerant", thresholds: { channel: 1, maxChangedPixels: 0, maxChangedRatio: 0 }, reviewReason: "Not bound by the existing baseline review." });
  const file = path.join(directory, "unreviewed.json");
  await save(file, policy);
  const result = await compare(actual, path.join(directory, "out"), baseline, file);
  assert.equal(result.exitCode, 2);
  assert.equal(result.firstError.code, "E_BASELINE_INCOMPATIBLE");
  assert.equal(result.captures.compared, 0);
});

test("non-regular artifacts fail promptly instead of blocking on a FIFO", async t => {
  const { directory, actual } = await fixture(t);
  const file = path.join(actual, "inputs/profile.json");
  await unlink(file);
  const fifo = spawnSync("mkfifo", [file], { encoding: "utf8" });
  assert.equal(fifo.status, 0, fifo.stderr);
  const result = await compare(actual, path.join(directory, "out"));
  assert.equal(result.exitCode, 8);
});
