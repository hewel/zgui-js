import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode as get,
  readTree as tree,
  runScenario,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-pointer-boundaries-");
  bundle = await buildFixtureApp(temp.dir, "pointer-boundaries", new URL("./fixtures/pointer-boundaries.tsx", import.meta.url));
});
after(async () => { await temp?.cleanup(); });

let serial = 0;
const id = () => `step-${++serial}`;
const key = (name, modifiers) => ({ id: id(), op: "key.press", surface: "main", key: name, ...(modifiers ? { modifiers } : {}) });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const move = (x, y) => ({ id: id(), op: "pointer.move", surface: "main", position: { x, y } });

function run(steps, name) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `pointer-boundaries-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "input.keyboard", "tree.inspect"],
    steps,
  });
}

test("native pointer boundaries preserve subtree semantics, snapshots and owner cleanup", async () => {
  const { out } = await run([
    capture("initial", 0), key("Tab"), check("toggle", "focused", true),
    move(105, 105), check("region-count", "text", "1/0"), check("a-count", "text", "0/0"), check("b-count", "text", "0/0"),
    check("last", "text", "pointerenter|105,105|mouse"),
    move(150, 140), check("region-count", "text", "1/0"), check("a-count", "text", "1/0"), check("capture", "text", "capture|true"),
    check("last", "text", "pointerenter|150,140|mouse"),
    move(250, 140), check("region-count", "text", "1/0"), check("a-count", "text", "1/1"), check("b-count", "text", "1/0"), capture("inside"),
    move(380, 280), check("region-count", "text", "1/1"), check("b-count", "text", "1/1"), capture("outside"),
    move(150, 140), check("region-count", "text", "2/1"), check("a-count", "text", "2/1"), check("toggle", "focused", true),
    key("Space"), check("region", "exists", false), check("retained", "text", "pointerenter|150,140"), check("authority", "text", "expired"), capture("removed"),
    move(20, 200), check("region-count", "text", "2/1"), check("a-count", "text", "2/1"),
    key("Space"), check("region", "exists", true), move(250, 140), check("region-count", "text", "3/1"), check("b-count", "text", "2/1"), capture("remounted")
  ], "native");
  const inside = await tree(out, "inside"), remounted = await tree(out, "remounted");
  for (const testId of ["region", "a", "b"]) assert.notEqual(get(inside, testId).token, get(remounted, testId).token);
  const contains = (rect, x, y) => x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height;
  assert.ok(contains(get(inside, "region").boundsLogical, 105, 105));
  assert.ok(!contains(get(inside, "a").boundsLogical, 105, 105));
  assert.ok(contains(get(inside, "a").boundsLogical, 150, 140));
  assert.ok(contains(get(inside, "b").boundsLogical, 250, 140));
  assert.ok(!contains(get(inside, "region").boundsLogical, 380, 280));
});
