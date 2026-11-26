import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const gate = fileURLToPath(new URL("../../packages/build/src/preview-check.ts", import.meta.url));

test("preview gate resolves one prebuilt host for run and compare without Cargo", async t => {
  const cwd = await mkdtemp(path.join(tmpdir(), "zgui-preview-gate-"));
  t.after(() => rm(cwd, { recursive: true, force: true }));
  const executable = path.join(cwd, "fake-host");
  const log = path.join(cwd, "calls");
  const out = path.join(cwd, "gate");
  await writeFile(
    executable,
    `#!${process.execPath}
import { appendFileSync } from "node:fs";
const [mode, command] = process.argv.slice(2);
if (process.cwd() !== ${JSON.stringify(cwd)}) process.exit(72);
if (mode === "--version" && command === "--json") {
  appendFileSync(${JSON.stringify(log)}, "probe\\n");
  console.log(JSON.stringify({ name: "zgui-host", version: "1.0.0", hostApiVersion: 1,
    schemaVersion: 2, platform: ${JSON.stringify(`${process.platform}-${process.arch}`)} }));
} else if (mode === "preview" && (command === "run" || command === "compare")) {
  appendFileSync(${JSON.stringify(log)}, command + "\\n");
  console.log(JSON.stringify({ schemaVersion: 1, command, status: "passed", exitCode: 0 }));
} else process.exit(64);
`,
    { mode: 0o755 },
  );
  const child = Bun.spawn([process.execPath, gate, "--out", out], {
    cwd,
    env: { ...process.env, PATH: cwd, ZGUI_JS_HOST_EXECUTABLE: executable },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [code, stdout, stderr] = await Promise.all([
    child.exited,
    child.stdout.text(),
    child.stderr.text(),
  ]);
  assert.equal(code, 0, stderr || stdout);
  const result = JSON.parse(stdout);
  assert.equal(result.status, "passed");
  assert.equal(result.stage, "compare");
  assert.equal(await readFile(log, "utf8"), "probe\nrun\ncompare\n");
  for (const command of ["run", "compare"]) {
    const filename = command === "run" ? "run.stdout.json" : "comparison.stdout.json";
    assert.equal(JSON.parse(await readFile(path.join(out, filename), "utf8")).command, command);
  }
});
