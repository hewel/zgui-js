import assert from "node:assert/strict";
import { before, after, test } from "node:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

let out;
before(async () => {
  await buildApplication("controls", {
    appDirectory: path.join(workspaceRoot, "apps/examples/controls"),
  });
  out = await mkdtemp(path.join(workspaceRoot, "target/preview-controls-test-"));
});
after(async () => { if (out) await rm(out, { recursive: true, force: true }); });

const byTestId = (tree, id) => tree.nodes.find(node => node.testId === id);

test("controls scenario passes and keeps secure text masked", async () => {
  const process = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "run",
    "--bundle", path.join(workspaceRoot, "apps/examples/controls/dist"),
    "--profile", path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario", path.join(workspaceRoot, "preview/scenarios/controls-virtual.json"),
    "--out", out,
  ], { cwd: workspaceRoot, encoding: "utf8", timeout: 60000 });
  assert.ifError(process.error);
  const result = JSON.parse(process.stdout);
  assert.equal(process.signal, null);
  assert.equal(process.status, result.exitCode);
  assert.equal(result.exitCode, 0, process.stdout);
  assert.equal(result.status, "passed", process.stdout);
  assert.equal(result.assertions.failed, 0);

  const titles = [];
  for (const capture of ["initial", "alert-hidden", "final"]) {
    const raw = await readFile(path.join(out, `captures/${capture}.tree.json`), "utf8");
    assert.equal(raw.includes("secret"), false, `${capture} leaked plaintext`);
    const tree = JSON.parse(raw);
    for (const id of ["password", "password-reversed"]) {
      const field = byTestId(tree, id);
      assert.ok(field, `${capture} missing ${id}`);
      assert.equal(field.role, "passwordInput");
      assert.equal(field.value, "••••••", `${capture} ${id} value`);
    }
    const title = byTestId(tree, "alert-title");
    assert.ok(title, `${capture} missing alert-title`);
    titles.push(title.token);
  }
  assert.equal(titles[1], titles[0], "hidden capture kept the same title node");
  assert.equal(titles[2], titles[0], "final capture kept the same title node");
});
