import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

let out;
before(async () => {
  await buildApplication("selection", {
    appDirectory: path.join(workspaceRoot, "apps/examples/selection"),
  });
  out = await mkdtemp(path.join(workspaceRoot, "target/preview-selection-test-"));
});
after(async () => {
  if (!out) return;
  if (process.env.ZGUI_KEEP_PREVIEW_OUTPUT === "1") {
    console.log(`retained preview output: ${out}`);
  } else {
    await rm(out, { recursive: true, force: true });
  }
});

const json = async file => JSON.parse(await readFile(file, "utf8"));
const byTestId = (tree, id) => tree.nodes.find(node => node.testId === id);
const textOf = (tree, id) => byTestId(tree, id)?.text;

test("selection scenario passes and group/slider identity survives reorder and remount", async () => {
  const scenario = path.join(workspaceRoot, "preview/scenarios/selection-virtual.json");
  const expectedAssertions = JSON.parse(await readFile(scenario, "utf8"))
    .steps.filter(step => step.op === "assert").length;
  const process = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "run",
    "--bundle", path.join(workspaceRoot, "apps/examples/selection/dist"),
    "--profile", path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario", scenario,
    "--out", out,
  ], { cwd: workspaceRoot, encoding: "utf8", timeout: 60000 });
  assert.ifError(process.error);
  const result = JSON.parse(process.stdout);
  assert.equal(process.signal, null, process.stderr);
  assert.equal(process.status, result.exitCode);
  assert.equal(result.exitCode, 0, process.stdout);
  assert.equal(result.status, "passed", process.stdout);
  assert.equal(result.assertions.failed, 0);
  assert.equal(result.assertions.passed, expectedAssertions);
  assert.equal(result.teardown.clean, true);

  const captures = {};
  for (const name of ["initial", "reordered", "item-removed", "item-restored",
    "slider-hidden", "hidden-moved", "slider-restored", "final"]) {
    captures[name] = await json(path.join(out, `captures/${name}.tree.json`));
  }
  const formattingOrder = tree => tree.nodes
    .filter(node => node.parent === byTestId(tree, "formatting")?.token)
    .map(node => node.testId);
  assert.deepEqual(formattingOrder(captures.initial), ["format-bold", "format-italic", "format-underline"]);
  assert.deepEqual(formattingOrder(captures.reordered), ["format-underline", "format-bold", "format-italic"]);
  for (const id of ["formatting", "format-bold", "format-italic", "format-underline"]) {
    assert.equal(byTestId(captures.reordered, id).token, byTestId(captures.initial, id).token,
      `${id} kept identity through reorder`);
  }

  assert.equal(byTestId(captures["item-removed"], "format-italic"), undefined,
    "item-removed drops format-italic");
  const restoredItalic = byTestId(captures["item-restored"], "format-italic");
  assert.ok(restoredItalic, "item-restored has format-italic");
  assert.notEqual(restoredItalic.token, byTestId(captures.initial, "format-italic").token,
    "restored italic is a fresh node");
  for (const id of ["formatting", "format-bold", "format-underline"]) {
    for (const capture of ["item-removed", "item-restored"]) {
      assert.equal(byTestId(captures[capture], id).token, byTestId(captures.initial, id).token,
        `${id} kept identity through ${capture}`);
    }
  }
  assert.equal(textOf(captures["item-restored"], "formatting-value"), "Formatting: bold, italic");

  assert.ok(byTestId(captures.initial, "volume"), "initial volume exists");
  assert.equal(byTestId(captures["slider-hidden"], "volume"), undefined, "slider-hidden drops volume");
  assert.equal(byTestId(captures["hidden-moved"], "volume"), undefined, "hidden-moved drops volume");
  assert.equal(textOf(captures["slider-hidden"], "pointer-moves"),
    textOf(captures["hidden-moved"], "pointer-moves"),
    "late pointer events do not reach the disposed slider");
  const restoredVolume = byTestId(captures["slider-restored"], "volume");
  assert.ok(restoredVolume, "slider-restored has volume");
  assert.notEqual(restoredVolume.token, byTestId(captures.initial, "volume").token,
    "restored volume is a fresh node");
  for (const id of ["formatting", "format-bold", "format-italic", "format-underline", "quality"]) {
    assert.equal(byTestId(captures["slider-restored"], id).token,
      byTestId(captures["item-restored"], id).token,
      `${id} unchanged through slider remount`);
  }
  assert.equal(byTestId(captures.final, "volume").token, restoredVolume.token,
    "final volume is the restored node");
  assert.equal(textOf(captures.final, "volume-value"), "Volume: 55");

  for (const id of ["selection-theme", "before-radio", "quality", "quality-low", "quality-balanced",
    "quality-high", "quality-value", "alignment", "align-left", "align-center", "align-right",
    "alignment-value", "formatting", "format-bold", "format-italic", "format-underline",
    "formatting-value", "italic-toggle", "volume", "volume-value", "pointer-value", "pointer-moves",
    "disable-slider", "show-slider", "overflow", "overflow-first", "overflow-second",
    "overflow-third", "overflow-fourth", "overflow-last", "rotate-format"]) {
    assert.ok(byTestId(captures.final, id), `final missing ${id}`);
  }
});
