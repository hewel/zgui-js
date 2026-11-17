import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { readFile } from "node:fs/promises";
import path from "node:path";
import {
  buildExampleApp,
  createTempWorkspace,
  getNode as get,
  readTree,
  runPreview,
  workspaceRoot,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  bundle = await buildExampleApp("tabs");
  temp = await createTempWorkspace("preview-tabs-test-");
});
after(async () => { await temp?.cleanup(); });

test("tabs scenario passes and panel/trigger identity survives selection and reorder", async () => {
  const scenario = path.join(workspaceRoot, "preview/scenarios/tabs-virtual.json");
  const scenarioInput = JSON.parse(await readFile(scenario, "utf8"));
  const expectedAssertions = scenarioInput.steps.filter(step => step.op === "assert").length;
  await runPreview({
    bundle,
    scenario,
    out: temp.dir,
    expectedAssertions,
    assertEmptyResources: false,
  });

  const captures = {};
  for (const name of ["initial", "edited", "left", "returned",
    "reordered", "removed", "restored", "disabled", "final"]) {
    captures[name] = await readTree(temp.dir, name);
  }
  const disabledHistory = get(captures.disabled, "auto-history");
  assert.equal(disabledHistory.state.enabled, false);
  assert.equal(disabledHistory.state.visible, true);
  const point = scenarioInput.steps.find(step => step.id === "click-disabled-history").position;
  const bounds = disabledHistory.boundsLogical;
  assert.ok(bounds && point.x > bounds.x && point.x < bounds.x + bounds.width
    && point.y > bounds.y && point.y < bounds.y + bounds.height,
    "disabled click lies inside the actual disabled tab, not empty space");
  for (const name of ["initial", "final"]) {
    const first = get(captures[name], "nested-one").boundsLogical;
    const second = get(captures[name], "nested-two").boundsLogical;
    assert.ok(first && second, `${name} has native bounds for both nested triggers`);
    assert.ok(second.x > first.x && Math.abs(second.y - first.y) < 1,
      `${name} nested horizontal list stays horizontal inside a vertical root: ${JSON.stringify({ first, second })}`);
  }
  const panelIds = ["auto-panel-profile", "auto-panel-locked", "auto-panel-notes", "auto-panel-history"];
  for (const id of panelIds) {
    const token = get(captures.initial, id).token;
    for (const capture of Object.values(captures)) {
      assert.equal(get(capture, id).token, token, `${id} shell identity is stable`);
    }
  }

  assert.equal(get(captures.edited, "profile-field").token, get(captures.initial, "profile-field").token,
    "profile field survives editing");
  assert.equal(get(captures.edited, "profile-field").value, "Changed");
  assert.equal(get(captures.edited, "profile-count").text, "Count: 1");
  for (const id of ["profile-field", "profile-count"]) {
    assert.equal(captures.left.nodes.find(node => node.testId === id), undefined,
      `left unmounts ${id}`);
  }
  const returnedField = get(captures.returned, "profile-field");
  const returnedCount = get(captures.returned, "profile-count");
  assert.notEqual(returnedField.token, get(captures.edited, "profile-field").token,
    "returned profile field is fresh");
  assert.notEqual(returnedCount.token, get(captures.edited, "profile-count").token,
    "returned counter is fresh");
  assert.equal(returnedField.value, "Preview", "returned field resets");
  assert.equal(returnedCount.text, "Count: 0", "returned counter resets");

  const draftTokens = Object.entries(captures).map(([name, tree]) => [name, get(tree, "draft-field").token]);
  for (const [name, token] of draftTokens) {
    assert.equal(token, draftTokens[0][1], `draft-field keeps identity in ${name}`);
  }
  assert.equal(get(captures.left, "draft-field").value, "Saved draft",
    "active notes shows the edited draft");

  const triggerOrder = tree => {
    const parent = get(tree, "auto-list").token;
    return tree.nodes.filter(node => node.parent === parent)
      .map(node => node.testId).filter(id => id?.startsWith("auto-"));
  };
  assert.deepEqual(triggerOrder(captures.initial), ["auto-profile", "auto-locked", "auto-notes", "auto-history"]);
  assert.deepEqual(triggerOrder(captures.reordered), ["auto-notes", "auto-locked", "auto-profile", "auto-history"]);
  for (const id of ["auto-profile", "auto-locked", "auto-notes", "auto-history"]) {
    assert.equal(get(captures.reordered, id).token, get(captures.initial, id).token,
      `${id} keeps identity through reorder`);
  }
  assert.equal(get(captures.reordered, "profile-field").token, returnedField.token,
    "reorder without selection change keeps panel children");
  assert.equal(get(captures.reordered, "profile-count").token, returnedCount.token);

  assert.equal(captures.removed.nodes.find(node => node.testId === "auto-notes"), undefined,
    "removed drops auto-notes");
  const restoredNotes = get(captures.restored, "auto-notes");
  assert.notEqual(restoredNotes.token, get(captures.initial, "auto-notes").token,
    "restored auto-notes is a fresh node");
  for (const id of ["auto-profile", "auto-locked", "auto-history"]) {
    assert.equal(get(captures.restored, id).token, get(captures.reordered, id).token,
      `${id} unchanged through notes removal`);
  }

  for (const id of ["automatic", "manual", "auto-list", "manual-list"]) {
    assert.equal(get(captures.final, id).token, get(captures.restored, id).token,
      `${id} unchanged through theme switch`);
  }
  assert.equal(get(captures.final, "auto-notes").token, restoredNotes.token,
    "restored notes survives into the final capture");
  assert.equal(get(captures.final, "draft-field").value, "Saved draft");
  assert.equal(get(captures.final, "manual-value").text, "Selected: overview");
  assert.equal(get(captures.final, "auto-value").text, "Selected: profile");
});
