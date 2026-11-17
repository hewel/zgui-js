import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { readFile } from "node:fs/promises";
import path from "node:path";
import {
  buildExampleApp,
  createTempWorkspace,
  readTree,
  runPreview,
  workspaceRoot,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  bundle = await buildExampleApp("selection");
  temp = await createTempWorkspace("preview-selection-test-");
});
after(async () => { await temp?.cleanup(); });

const byTestId = (tree, id) => tree.nodes.find(node => node.testId === id);
const textOf = (tree, id) => byTestId(tree, id)?.text;

test("selection scenario passes and group/slider identity survives reorder and remount", async () => {
  const scenario = path.join(workspaceRoot, "preview/scenarios/selection-virtual.json");
  const expectedAssertions = JSON.parse(await readFile(scenario, "utf8"))
    .steps.filter(step => step.op === "assert").length;
  await runPreview({
    bundle,
    scenario,
    out: temp.dir,
    expectedAssertions,
    assertEmptyResources: false,
  });

  const captures = {};
  for (const name of ["initial", "reordered", "item-removed", "item-restored",
    "slider-hidden", "hidden-moved", "slider-restored", "final"]) {
    captures[name] = await readTree(temp.dir, name);
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
