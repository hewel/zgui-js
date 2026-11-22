import assert from "node:assert/strict";
import { test } from "node:test";
import { access, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const workspaceRoot = fileURLToPath(new URL("../../", import.meta.url));
const gate = path.join(workspaceRoot, "packages/build/src/preview-check.ts");

async function temporaryProject(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "zgui-preview-ci-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}

function check(args, env = process.env) {
  const child = spawnSync(process.execPath, [gate, ...args], {
    cwd: workspaceRoot,
    env,
    encoding: "utf8",
    timeout: 60000,
    maxBuffer: 1024 * 1024,
  });
  assert.ifError(child.error);
  assert.equal(child.signal, null, child.stderr);
  assert.ok(Buffer.byteLength(child.stdout) <= 8192, "gate summary exceeds 8 KiB");
  assert.equal(child.stdout.trim().split("\n").length, 1, "stdout must contain one JSON result");
  const result = JSON.parse(child.stdout);
  assert.equal(child.status, result.exitCode, child.stderr);
  return { child, result };
}

async function json(file) {
  return JSON.parse(await readFile(file, "utf8"));
}

// Deliberately required: missing build prerequisites, baseline, or real GPU fail this test.
test("required preview gate runs counter then compares the reviewed baseline", async t => {
  const directory = await temporaryProject(t);
  const out = path.join(directory, "gate");
  const { child, result } = check(["--out", out]);
  assert.equal(child.status, 0, `${child.stdout}\n${child.stderr}`);
  assert.equal(result.status, "passed");
  assert.equal(result.stage, "compare");
  assert.equal(result.run.exitCode, 0);
  assert.equal(result.comparison.exitCode, 0);
  assert.equal(result.firstError, null);
  assert.deepEqual(await json(path.join(out, "result.json")), result);

  const run = await json(path.join(out, "actual/result.json"));
  assert.equal(run.command, "run");
  assert.equal(run.status, "passed");
  assert.equal(run.captureCount, 2);
  assert.equal(run.assertions.failed, 0);
  assert.ok(run.assertions.passed > 0);
  const comparison = await json(path.join(out, "comparison/result.json"));
  assert.equal(comparison.command, "compare");
  assert.equal(comparison.status, "passed");
  assert.equal(comparison.exitCode, 0);
  await readFile(path.join(out, "comparison/report.json"));
  for (const capture of ["initial", "after-increment"]) {
    const image = await readFile(path.join(out, `actual/captures/${capture}.png`));
    assert.deepEqual(image.subarray(0, 8), Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
    await json(path.join(out, `actual/captures/${capture}.frame.json`));
    await json(path.join(out, `actual/captures/${capture}.tree.json`));
  }
});

test("unavailable Vulkan fails the required gate with exit 7 and never compares", async t => {
  const directory = await temporaryProject(t);
  const out = path.join(directory, "no-graphics");
  const missingDriver = path.join(directory, "unavailable-vulkan-driver.json");
  const { child, result } = check(["--out", out], {
    ...process.env,
    ZGUI_BACKENDS: "vulkan",
    VK_DRIVER_FILES: missingDriver,
    VK_ICD_FILENAMES: missingDriver,
  });
  assert.equal(child.status, 7, `${child.stdout}\n${child.stderr}`);
  assert.equal(result.status, "failed");
  assert.equal(result.stage, "run");
  assert.equal(result.run.exitCode, 7);
  assert.equal(result.firstError.code, "E_GRAPHICS_UNAVAILABLE");
  assert.equal(result.comparison, null);
  await assert.rejects(access(path.join(out, "comparison")), { code: "ENOENT" });
  assert.deepEqual(await json(path.join(out, "result.json")), result);
  const run = await json(path.join(out, "actual/result.json"));
  assert.equal(run.exitCode, 7);
  assert.equal(run.firstError.code, "E_GRAPHICS_UNAVAILABLE");
  assert.deepEqual(await json(path.join(out, "run.stdout.json")), run);
});

test("output creation failure is exit 8, not a skipped or compared success", async t => {
  const directory = await temporaryProject(t);
  const blocker = path.join(directory, "not-a-directory");
  await writeFile(blocker, "preserve this output blocker\n");
  const { child, result } = check(["--out", path.join(blocker, "gate")]);
  assert.equal(child.status, 8, `${child.stdout}\n${child.stderr}`);
  assert.equal(result.status, "failed");
  assert.equal(result.stage, "setup");
  assert.equal(result.firstError.code, "E_OUTPUT_IO");
  assert.equal(result.run, null);
  assert.equal(result.comparison, null);
  assert.equal(await readFile(blocker, "utf8"), "preserve this output blocker\n");
});

test("existing output roots and symlinks are rejected without changing their contents", async t => {
  const directory = await temporaryProject(t);
  const empty = path.join(directory, "empty");
  const used = path.join(directory, "used");
  const linked = path.join(directory, "linked");
  await mkdir(empty);
  await mkdir(used);
  await writeFile(path.join(used, "result.json"), "existing evidence\n");
  await symlink(used, linked);
  for (const out of [empty, used, linked]) {
    const { child, result } = check(["--out", out]);
    assert.equal(child.status, 8, `${child.stdout}\n${child.stderr}`);
    assert.equal(result.firstError.code, "E_OUTPUT_IO");
    assert.equal(result.run, null);
    assert.equal(result.comparison, null);
  }
  assert.equal(await readFile(path.join(used, "result.json"), "utf8"), "existing evidence\n");
  await assert.rejects(access(path.join(empty, "result.json")), { code: "ENOENT" });
  await assert.rejects(access(path.join(used, "actual")), { code: "ENOENT" });
});

test("malformed gate arguments fail before claiming output", async t => {
  const directory = await temporaryProject(t);
  const out = path.join(directory, "unclaimed");
  for (const args of [
    [],
    ["--out"],
    ["--unknown", out],
    ["--out", out, "--out", out],
    ["--out", out, "--policy"],
    ["--out", out, "--baseline", ""],
  ]) {
    const { child, result } = check(args);
    assert.equal(child.status, 2, `${child.stdout}\n${child.stderr}`);
    assert.equal(result.firstError.code, "E_SCHEMA");
    assert.equal(result.run, null);
    assert.equal(result.comparison, null);
  }
  await assert.rejects(access(out), { code: "ENOENT" });
});
