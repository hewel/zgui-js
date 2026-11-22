import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode as get,
  readTree as tree,
  runScenario,
} from "./harness.mjs";

let temp;
const bundles = {};
before(async () => {
  temp = await createTempWorkspace("preview-dialog-");
  for (const [variant, fixture] of [
    ["main", "dialog.tsx"],
    ["initial", "dialog-initial.tsx"],
  ]) {
    bundles[variant] = await buildFixtureApp(
      path.join(temp.dir, variant),
      `dialog-${variant}`,
      new URL(`./fixtures/${fixture}`, import.meta.url),
    );
  }
});
after(async () => {
  await temp?.cleanup();
});

let serial = 0;
const id = () => `step-${++serial}`;
const click = testId => ({
  id: id(),
  op: "pointer.click",
  target: { surface: "main", testId },
  button: "primary",
});
const key = (name, modifiers) => ({
  id: id(),
  op: "key.press",
  surface: "main",
  key: name,
  ...(modifiers ? { modifiers } : {}),
});
const text = value => ({ id: id(), op: "text.commit", surface: "main", text: value });
const at = (x, y) => ({
  id: id(),
  op: "pointer.click",
  surface: "main",
  position: { x, y },
  button: "primary",
});
const wheelAt = (x, y, dy) => ({
  id: id(),
  op: "wheel",
  surface: "main",
  position: { x, y },
  delta: { x: 0, y: dy, unit: "pixels" },
});
const wheel = testId => ({
  id: id(),
  op: "wheel",
  target: { surface: "main", testId },
  delta: { x: 0, y: 3, unit: "lines" },
});
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
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

async function run(steps, name, variant = "main") {
  const res = await runScenario({
    projectDir: temp.dir,
    bundle: bundles[variant],
    name,
    id: `dialog-${name}`,
    requiredCapabilities: [
      "pixels",
      "virtual-time",
      "selectors.test-id",
      "input.pointer",
      "input.keyboard",
      "input.text",
      "input.wheel",
      "tree.inspect",
      "surface.resize",
    ],
    steps,
    assertEmptyResources: false,
  });
  assert.equal(res.result.teardown.resources.focusTraps, 0);
  assert.equal(res.result.teardown.resources.scrollLocks, 0);
  assert.equal(res.result.teardown.resources.frameCallbacks, 0);
  assert.equal(res.result.teardown.resources.timers, 0);
  return res;
}
const contains = (bounds, x, y) =>
  x >= bounds.x && x < bounds.x + bounds.width && y >= bounds.y && y < bounds.y + bounds.height;
const focused = tree => tree.nodes.filter(node => node.state?.focused === true);

test("dialog opens lazily, traps focus, exits on Escape and remounts fresh after removal", async () => {
  const { out } = await run(
    [
      capture("initial", 0),
      check("surface", "exists", false),
      click("launch"),
      advance(400),
      check("name", "focused", true),
      check("surface", "role", "dialog"),
      capture("opened"),
      key("Tab", ["Shift"]),
      capture("backtab"),
      key("Tab"),
      check("name", "focused", true),
      key("Home"),
      key("End", ["Shift"]),
      text("Beta"),
      check("name", "value", "Beta"),
      click("counter"),
      check("counter", "text", "Count 1"),
      key("Escape"),
      check("surface", "exists", true),
      check("requested", "text", "closed"),
      advance(80),
      capture("leaving"),
      click("launch"),
      advance(400),
      check("name", "value", "Beta"),
      check("counter", "text", "Count 1"),
      capture("reopened"),
      key("Escape"),
      advance(400),
      check("surface", "exists", false),
      check("launch", "focused", true),
      capture("closed"),
      key("Enter"),
      advance(400),
      check("name", "focused", true),
      check("name", "value", "Alpha"),
      check("counter", "text", "Count 0"),
      capture("fresh"),
      click("cancel"),
      advance(400),
      check("surface", "exists", false),
      check("launch", "focused", true),
    ],
    "lifecycle",
  );
  const initial = await tree(out, "initial");
  const opened = await tree(out, "opened");
  const backtab = await tree(out, "backtab");
  const leaving = await tree(out, "leaving");
  const reopened = await tree(out, "reopened");
  const fresh = await tree(out, "fresh");
  for (const testId of ["surface", "name", "counter"]) {
    const token = get(opened, testId).token;
    assert.equal(get(leaving, testId).token, token, `${testId} remounted while leaving`);
    assert.equal(get(reopened, testId).token, token, `${testId} remounted on reopen`);
    assert.notEqual(get(fresh, testId).token, token, `${testId} not rebuilt after full removal`);
  }
  for (const testId of ["after", "page"]) {
    assert.deepEqual(
      get(opened, testId).boundsLogical,
      get(initial, testId).boundsLogical,
      `opening shifted ${testId} layout or scroll`,
    );
  }
  const backtabFocus = focused(backtab);
  assert.equal(backtabFocus.length, 1, "Shift+Tab must land on exactly one node");
  assert.equal(backtabFocus[0].role, "button");
  assert.equal(backtabFocus[0].name, "Close");
  assert.notEqual(backtabFocus[0].token, get(backtab, "name").token);
  assert.ok(
    contains(get(initial, "background").boundsLogical, 40, 40),
    "the outside-click control point must sit inside the background action",
  );
  assert.ok(
    !contains(get(opened, "surface").boundsLogical, 40, 40),
    "the outside-click control point must stay outside the dialog surface",
  );
});

test("dismiss policy, scrim clicks, nesting and source ownership stay native", async () => {
  const { out } = await run(
    [
      click("launch"),
      advance(400),
      at(40, 40),
      check("background-count", "text", "0"),
      advance(400),
      check("surface", "exists", false),
      click("launch"),
      advance(400),
      click("policy"),
      key("Escape"),
      check("requested", "text", "open"),
      at(40, 40),
      check("background-count", "text", "0"),
      check("surface", "exists", true),
      click("policy"),
      click("nested-open"),
      advance(400),
      check("nested-name", "focused", true),
      capture("nested"),
      key("Tab"),
      check("nested-close", "focused", true),
      key("Tab"),
      key("Tab"),
      check("nested-name", "focused", true),
      key("Escape"),
      advance(400),
      check("nested-surface", "exists", false),
      check("surface", "exists", true),
      check("nested-open", "focused", true),
      capture("parent"),
      click("nested-open"),
      advance(400),
      key("Escape"),
      key("Escape"),
      advance(400),
      check("nested-surface", "exists", false),
      check("surface", "exists", false),
      check("launch", "focused", true),
      click("launch"),
      advance(400),
      capture("before-removal"),
      click("remove"),
      check("surface", "exists", false),
      check("launch", "exists", false),
      capture("removed"),
      click("source-toggle"),
      advance(400),
      check("surface", "exists", true),
      check("name", "focused", true),
      check("name", "value", "Alpha"),
      capture("restored"),
      click("nested-open"),
      advance(400),
      check("nested-name", "focused", true),
      capture("active-exit"),
    ],
    "ownership",
  );
  const nested = await tree(out, "nested");
  const parent = await tree(out, "parent");
  const beforeRemoval = await tree(out, "before-removal");
  const restored = await tree(out, "restored");
  assert.equal(
    get(parent, "surface").token,
    get(nested, "surface").token,
    "closing the nested dialog remounted the parent surface",
  );
  for (const testId of ["surface", "name"]) {
    assert.notEqual(
      get(restored, testId).token,
      get(beforeRemoval, testId).token,
      `${testId} was not rebuilt when the source remounted`,
    );
  }
});

test("scroll locks, scoped theme and resize keep native geometry honest", async () => {
  const { out } = await run(
    [
      capture("scroll-initial", 0),
      wheelAt(760, 540, 240),
      advance(400),
      capture("scrolled"),
      click("launch"),
      advance(400),
      capture("locked"),
      wheelAt(760, 540, 120),
      advance(400),
      capture("locked-wheel"),
      wheel("inner-scroll"),
      advance(400),
      capture("inner-scrolled"),
      click("theme"),
      advance(400),
      check("surface", "exists", true),
      capture("dark"),
      { id: id(), op: "surface.resize", surface: "main", width: 420, height: 480 },
      advance(400),
      capture("narrow"),
      { id: id(), op: "surface.resize", surface: "main", width: 800, height: 600 },
      advance(400),
      click("cancel"),
      advance(400),
      wheelAt(760, 540, 120),
      advance(400),
      capture("unlocked"),
    ],
    "geometry",
  );
  const scrollInitial = await tree(out, "scroll-initial");
  const scrolled = await tree(out, "scrolled");
  const locked = await tree(out, "locked");
  const lockedWheel = await tree(out, "locked-wheel");
  const innerScrolled = await tree(out, "inner-scrolled");
  const dark = await tree(out, "dark");
  const narrow = await tree(out, "narrow");
  const unlocked = await tree(out, "unlocked");
  const pageOf = tree => get(tree, "page").boundsLogical;
  assert.ok(
    pageOf(scrolled).y < pageOf(scrollInitial).y,
    "the unlocked control wheel must scroll the page",
  );
  assert.deepEqual(pageOf(locked), pageOf(scrolled), "opening the dialog must not move the page");
  assert.deepEqual(
    pageOf(lockedWheel),
    pageOf(locked),
    "the scroll lock must hold the page against wheel input",
  );
  assert.deepEqual(
    pageOf(innerScrolled),
    pageOf(locked),
    "inner scrolling must not leak to the locked page",
  );
  const innerOf = tree => get(tree, "inner-content").boundsLogical;
  assert.ok(
    innerOf(innerScrolled).y < innerOf(locked).y,
    "the nested scroller must keep working under the lock",
  );
  for (const testId of ["surface", "name"]) {
    const token = get(locked, testId).token;
    assert.equal(get(dark, testId).token, token, `${testId} rebuilt on theme switch`);
    assert.equal(get(narrow, testId).token, token, `${testId} rebuilt on resize`);
  }
  const narrowBounds = get(narrow, "surface").boundsLogical;
  assert.ok(
    Number.isFinite(narrowBounds.x) &&
      Number.isFinite(narrowBounds.y) &&
      Number.isFinite(narrowBounds.width) &&
      Number.isFinite(narrowBounds.height),
  );
  assert.ok(
    narrowBounds.x >= -1 &&
      narrowBounds.y >= -1 &&
      narrowBounds.x + narrowBounds.width <= 421 &&
      narrowBounds.y + narrowBounds.height <= 481,
    `narrow surface must fit the resized window: ${JSON.stringify(narrowBounds)}`,
  );
  assert.ok(
    pageOf(unlocked).y < pageOf(locked).y,
    "releasing the lock must let the page scroll again",
  );
});

test("initially open nested dialogs trap and release natively", async () => {
  await run(
    [
      advance(400),
      check("initial-outer", "exists", true),
      check("initial-inner", "exists", true),
      check("initial-inner-name", "focused", true),
      capture("initial-nested"),
      key("Tab"),
      check("initial-inner-close", "focused", true),
      key("Tab"),
      check("initial-inner-name", "focused", true),
      click("initial-inner-close"),
      advance(400),
      check("initial-inner", "exists", false),
      check("initial-outer", "exists", true),
      click("initial-outer-close"),
      advance(400),
      check("initial-outer", "exists", false),
      capture("initial-closed"),
    ],
    "initial",
    "initial",
  );
});
