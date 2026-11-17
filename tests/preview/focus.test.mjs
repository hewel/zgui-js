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
  temp = await createTempWorkspace("preview-focus-");
  bundle = await buildFixtureApp(temp.dir, "focus", new URL("./fixtures/focus.tsx", import.meta.url));
});
after(async () => { await temp?.cleanup(); });

let serial = 0;
const id = () => `step-${++serial}`;
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const key = (name, modifiers) => ({ id: id(), op: "key.press", surface: "main", key: name, ...(modifiers ? { modifiers } : {}) });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });

function run(steps, name) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `focus-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "input.keyboard", "tree.inspect"],
    steps,
    assertEmptyResources: false,
  });
}

test("focus traps enter, wrap, nest, restore and release with their owners", async () => {
  const { out } = await run([
    capture("initial", 0),
    click("launch"), check("first", "focused", true), capture("outer-open"),
    key("Tab", ["Shift"]), check("outer-close", "focused", true),
    key("Tab"), check("first", "focused", true),
    key("Tab"), check("inner-launch", "focused", true),
    key("Enter"), check("inner-first", "focused", true), capture("inner-open"),
    key("Tab", ["Shift"]), check("inner-close", "focused", true),
    key("Enter"), check("inner-launch", "focused", true), check("inner", "exists", false), capture("inner-closed"),
    key("Tab"), check("outer-close", "focused", true),
    key("Tab"), check("first", "focused", true),
    key("Tab", ["Shift"]), check("outer-close", "focused", true),
    key("Enter"), check("launch", "focused", true), check("outer", "exists", false), capture("outer-closed"),
    key("Tab"), check("after", "focused", true),
    click("launch"), check("first", "focused", true), capture("reopened"),
    click("after"), check("after", "focused", true), check("outer", "exists", false), capture("external-close"),
    click("launch"), check("first", "focused", true),
    key("Tab"), check("inner-launch", "focused", true),
    key("Enter"), check("inner-first", "focused", true), capture("teardown-live"),
  ], "scopes");
  const opened = await tree(out, "outer-open");
  const reopened = await tree(out, "reopened");
  assert.notEqual(get(reopened, "first").token, get(opened, "first").token,
    "a remounted scope must build fresh native nodes");
});
