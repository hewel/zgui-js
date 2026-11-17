import assert from "node:assert/strict";
import { before, after, test } from "node:test";
import { readFile } from "node:fs/promises";
import path from "node:path";
import {
  buildExampleApp,
  createTempWorkspace,
  runPreview,
  workspaceRoot,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  bundle = await buildExampleApp("controls");
  temp = await createTempWorkspace("preview-controls-test-");
});
after(async () => { await temp?.cleanup(); });

const byTestId = (tree, id) => tree.nodes.find(node => node.testId === id);

test("controls scenario passes and keeps secure text masked", async () => {
  await runPreview({
    bundle,
    scenario: path.join(workspaceRoot, "preview/scenarios/controls-virtual.json"),
    out: temp.dir,
    assertEmptyResources: false,
  });

  const titles = [];
  for (const capture of ["initial", "alert-hidden", "final"]) {
    const raw = await readFile(path.join(temp.dir, `captures/${capture}.tree.json`), "utf8");
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
