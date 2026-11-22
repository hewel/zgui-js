import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { buildFixtureApp, createTempWorkspace, runScenario } from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-keyboard-character-");
  bundle = await buildFixtureApp(
    temp.dir,
    "keyboard-character",
    new URL("./fixtures/keyboard-character.tsx", import.meta.url),
  );
});
after(async () => {
  await temp?.cleanup();
});

let serial = 0;
const id = () => `step-${++serial}`;
const char = text => ({ character: text });
const key = (name, modifiers) => ({
  id: id(),
  op: "key.press",
  surface: "main",
  key: name,
  ...(modifiers ? { modifiers } : {}),
});
const click = testId => ({
  id: id(),
  op: "pointer.click",
  target: { surface: "main", testId },
  button: "primary",
});
const commit = text => ({ id: id(), op: "text.commit", surface: "main", text });
const check = (testId, property, equals) => ({
  id: id(),
  op: "assert",
  target: { surface: "main", testId },
  check: { property, equals },
});
const capture = (captureId, requiredFrameTimeUs) => ({
  id: id(),
  op: "capture",
  captureId,
  ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }),
});

function run(steps, name) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `keyboard-character-${name}`,
    requiredCapabilities: [
      "pixels",
      "virtual-time",
      "selectors.test-id",
      "input.pointer",
      "input.keyboard",
      "input.text",
      "tree.inspect",
    ],
    steps,
  });
}

test("native character keys deliver exact Key::Character payloads through the editing and snapshot model", async () => {
  const steps = [
    click("target"),
    check("target", "focused", true),
    check("clicks", "text", "1"),
    key(char("é")),
    check("down", "text", "e9|e9|e9|0000"),
    check("up", "text", "e9|e9|e9|0000"),
    check("clicks", "text", "1"),
    key(char("é")),
    check("down", "text", "65.301|65.301|65.301|0000"),
    check("up", "text", "65.301|65.301|65.301|0000"),
    key(char("𐐷")),
    check("down", "text", "10437|10437|10437|0000"),
    check("up", "text", "10437|10437|10437|0000"),
    click("inspect"),
    check("retained", "text", "10437|10437|10437|0000"),
    check("authority", "text", "expired"),
    click("target"),
    check("clicks", "text", "2"),
    key(char("Enter")),
    check("down", "text", "45.6e.74.65.72|45.6e.74.65.72|45.6e.74.65.72|0000"),
    check("up", "text", "45.6e.74.65.72|45.6e.74.65.72|45.6e.74.65.72|0000"),
    check("clicks", "text", "2"),
    key("Enter"),
    check("down", "text", "45.6e.74.65.72|none|45.6e.74.65.72|0000"),
    check("up", "text", "45.6e.74.65.72|none|45.6e.74.65.72|0000"),
    check("clicks", "text", "3"),
    key(char(" ")),
    check("down", "text", "20|20|20|0000"),
    check("up", "text", "20|20|20|0000"),
    check("clicks", "text", "3"),
    key("Space"),
    check("down", "text", "20|none|20|0000"),
    check("up", "text", "20|none|20|0000"),
    check("clicks", "text", "4"),
    key(char("a"), ["Shift", "Control"]),
    check("down", "text", "61|61|61|1100"),
    check("up", "text", "61|61|61|1100"),
    key(char("b")),
    check("down", "text", "62|62|62|0000"),
    check("up", "text", "62|62|62|0000"),
    check("events", "text", "9"),
    click("editor"),
    check("editor", "focused", true),
    key(char("é")),
    check("editor", "value", "é"),
    check("down", "text", "e9|e9|e9|0000"),
    check("up", "text", "e9|e9|e9|0000"),
    key(char("x")),
    check("editor", "value", "é"),
    check("down", "text", "78|78|78|0000"),
    check("up", "text", "78|78|78|0000"),
    key("Space"),
    check("editor", "value", "é "),
    check("down", "text", "20|none|20|0000"),
    check("up", "text", "20|none|20|0000"),
    check("events", "text", "12"),
    commit("Q"),
    check("editor", "value", "é Q"),
    check("events", "text", "12"),
    capture("completed"),
  ];
  const { out } = await run(steps, "native");
  const trace = (await readFile(path.join(out, "steps.jsonl"), "utf8"))
    .trim()
    .split("\n")
    .map(JSON.parse);
  const presses = steps.filter(step => step.op === "key.press");
  const pressRows = trace.filter(row => row.op === "key.press");
  assert.equal(pressRows.length, presses.length);
  for (const [index, row] of pressRows.entries()) {
    assert.deepEqual(row.result.key, presses[index].key, `${row.id} key echo`);
    assert.deepEqual(
      row.result.modifiers,
      presses[index].modifiers ?? [],
      `${row.id} modifiers echo`,
    );
    assert.deepEqual(
      row.result.burst,
      ["modifiers", "key.down", "key.up", "modifiers.clear"],
      `${row.id} burst`,
    );
    assert.equal(row.result.surface, "main", `${row.id} surface`);
    assert.ok(
      !row.result.burst.some(entry => entry.includes("ime")),
      `${row.id} key burst contains ime`,
    );
  }
  const commitRows = trace.filter(row => row.op === "text.commit");
  assert.equal(commitRows.length, 1);
  assert.deepEqual(commitRows[0].result.burst, ["native.ime.commit"]);
});
