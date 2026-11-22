import assert from "node:assert/strict";
import { before, after, test } from "node:test";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";

// This is an explicit real-GPU gate, not part of the graphics-independent build suite.
let project;
before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-final-input-"));
  await mkdir(path.join(project, "src"));
  await copyFile(
    fileURLToPath(new URL("./fixtures/final-input.tsx", import.meta.url)),
    path.join(project, "src/main.tsx"),
  );
  await writeFile(
    path.join(project, "tsconfig.json"),
    JSON.stringify({
      extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
      include: ["src/**/*.tsx"],
    }),
  );
  await buildApplication("final-input", { appDirectory: project });
});
after(async () => {
  if (project) await rm(project, { recursive: true, force: true });
});

const click = id => ({
  op: "pointer.click",
  target: { surface: "main", testId: id },
  button: "primary",
});
for (const [name, steps, message] of [
  ["pointer", [click("pointer")], "final-pointer-dispatched"],
  [
    "keyboard",
    [click("keyboard"), { op: "key.press", surface: "main", key: "Enter" }],
    "final-key-dispatched",
  ],
  [
    "character",
    [click("keyboard"), { op: "key.press", surface: "main", key: { character: "é" } }],
    "final-key-dispatched",
  ],
  [
    "text",
    [click("text"), { op: "text.commit", surface: "main", text: "界" }],
    "final-text-dispatched",
  ],
]) {
  test(`final ${name} action executes and owns its handler failure`, async () => {
    const scenario = path.join(project, `${name}.json`);
    const out = path.join(project, `${name}-run`);
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
        "preview/profiles/wall-static.json",
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
    assert.equal(process.status, 3, JSON.stringify(result));
    assert.equal(result.firstError.code, "E_JS_EXCEPTION");
    assert.equal(result.firstError.stage, steps.at(-1).op);
    assert.equal(result.firstError.step, `action-${steps.length - 1}`);
    assert.match(await readFile(path.join(out, "diagnostics.jsonl"), "utf8"), new RegExp(message));
    const trace = (await readFile(path.join(out, "steps.jsonl"), "utf8"))
      .trim()
      .split("\n")
      .map(JSON.parse);
    assert.equal(trace.at(-1).status, "failed");
    assert.equal(trace.at(-1).op, steps.at(-1).op);
  });
}
