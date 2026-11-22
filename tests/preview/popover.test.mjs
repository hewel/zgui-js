import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode as get,
  readPng as pngRgba,
  readTree as tree,
  runScenario,
} from "./harness.mjs";

let temp;
const bundles = {};
before(async () => {
  temp = await createTempWorkspace("preview-popover-");
  for (const [variant, fixture] of [
    ["geometry", "popover-geometry.tsx"],
    ["main", "popover.tsx"],
    ["initial", "popover-initial.tsx"],
    ["sides", "popover-sides.tsx"],
  ]) {
    bundles[variant] = await buildFixtureApp(
      path.join(temp.dir, variant),
      `popover-${variant}`,
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

function run(steps, name, variant = "geometry") {
  return runScenario({
    projectDir: temp.dir,
    bundle: bundles[variant],
    name,
    id: `popover-${name}`,
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
      "surface.scale",
    ],
    steps,
  });
}
const contains = (bounds, x, y) =>
  x >= bounds.x && x < bounds.x + bounds.width && y >= bounds.y && y < bounds.y + bounds.height;
const parentOf = (t, testId) => {
  const child = get(t, testId),
    parent = t.nodes.find(n => n.token === child.parent);
  assert.ok(parent?.boundsLogical, `missing parent for ${testId}`);
  return parent;
};

test("popover positions before first paint, caps, scrolls and rescales native surfaces", async () => {
  const { out } = await run(
    [
      capture("initial", 0),
      advance(400),
      check("cap-surface", "role", "dialog"),
      capture("opened"),
      {
        id: id(),
        op: "wheel",
        target: { surface: "main", testId: "cap-surface" },
        delta: { x: 0, y: 180, unit: "pixels" },
      },
      advance(400),
      capture("scrolled"),
      { id: id(), op: "surface.resize", surface: "main", width: 420, height: 480 },
      advance(400),
      capture("narrow"),
      { id: id(), op: "surface.scale", surface: "main", scale: 2 },
      advance(20),
      capture("dense"),
    ],
    "geometry",
  );
  const frames = {};
  for (const name of ["initial", "opened", "scrolled", "narrow", "dense"])
    frames[name] = await tree(out, name);
  const positioner = (t, testId) => {
    const child = get(t, testId),
      parent = t.nodes.find(n => n.token === child.parent);
    assert.ok(parent?.boundsLogical, `missing positioner for ${testId}`);
    return parent;
  };
  for (const [name, width, height, scale] of [
    ["initial", 800, 600, 1.75],
    ["opened", 800, 600, 1.75],
    ["scrolled", 800, 600, 1.75],
    ["narrow", 420, 480, 1.75],
    ["dense", 420, 480, 2],
  ]) {
    const t = frames[name],
      a = get(t, "cap-anchor").boundsLogical,
      p = positioner(t, "cap-surface").boundsLogical;
    const tolerance = 1 / scale + 0.001;
    assert.ok(p.height > 100, `${name}: surface unexpectedly collapsed`);
    assert.ok(
      p.height <= a.y - 4 - 8 + tolerance,
      `${name}: surface exceeds available height ${JSON.stringify({ a, p })}`,
    );
    assert.ok(
      p.x >= 8 - tolerance && p.x + p.width <= width - 8 + tolerance,
      `${name}: horizontal viewport overflow ${JSON.stringify(p)}`,
    );
    assert.ok(
      p.y >= 8 - tolerance && p.y + p.height <= height - 8 + tolerance,
      `${name}: vertical viewport overflow ${JSON.stringify(p)}`,
    );
    assert.ok(
      Math.abs(p.y + p.height + 4 - a.y) <= tolerance,
      `${name}: panel not anchored above trigger ${JSON.stringify({ a, p })}`,
    );
    const square = get(t, "sized").boundsLogical,
      source = get(t, "size-anchor").boundsLogical;
    assert.ok(square, `${name}: anchor-sized content never obtained layout`);
    const expected = Math.round(source.width * scale) / scale;
    assert.ok(expected > 90, `${name}: anchor control collapsed`);
    assert.ok(
      Math.abs(square.width - expected) < 0.05 && Math.abs(square.height - expected) < 0.05,
      `${name}: anchor-width bootstrap failed ${JSON.stringify({ square, expected })}`,
    );
    const wrapper = positioner(t, "sized").boundsLogical;
    assert.ok(
      Math.abs(wrapper.x * scale - Math.round(wrapper.x * scale)) < 0.001,
      `${name}: x is not device snapped`,
    );
    assert.ok(
      Math.abs(wrapper.y * scale - Math.round(wrapper.y * scale)) < 0.001,
      `${name}: y is not device snapped`,
    );
    assert.ok(
      Math.abs(wrapper.y - (source.y + source.height + 4)) <= tolerance,
      `${name}: sized content lost its anchor`,
    );
    for (const testId of ["cap-surface", "cap-input", "sized"])
      assert.equal(
        get(t, testId).token,
        get(frames.initial, testId).token,
        `${name}: ${testId} remounted during geometry change`,
      );
  }
  assert.ok(
    get(frames.scrolled, "tall-content").boundsLogical.y <
      get(frames.opened, "tall-content").boundsLogical.y,
    "capped popover must scroll its content",
  );
  assert.deepEqual(
    get(frames.scrolled, "cap-anchor").boundsLogical,
    get(frames.opened, "cap-anchor").boundsLogical,
    "inner scrolling moved trigger",
  );
  assert.notEqual(
    positioner(frames.dense, "sized").boundsLogical.y,
    positioner(frames.narrow, "sized").boundsLogical.y,
    "density-only change did not resnap origin",
  );
});

test("popover lifecycle keeps focus, exit state and caller animation handlers", async () => {
  const { out } = await run(
    [
      capture("closed-initial", 0),
      check("surface", "exists", false),
      click("launch"),
      advance(400),
      check("name", "focused", true),
      check("surface", "role", "dialog"),
      capture("opened"),
      key("Tab", ["Shift"]),
      check("cancel", "focused", true),
      key("Tab"),
      check("name", "focused", true),
      key("Home"),
      key("End", ["Shift"]),
      text("Beta"),
      check("name", "value", "Beta"),
      click("counter"),
      check("counter", "text", "Count 1"),
      key("Escape"),
      check("requested", "text", "closed"),
      check("surface", "exists", true),
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
    "main",
  );
  const initial = await tree(out, "closed-initial"),
    opened = await tree(out, "opened");
  const leaving = await tree(out, "leaving"),
    reopened = await tree(out, "reopened"),
    fresh = await tree(out, "fresh");
  assert.equal(
    get(opened, "surface").name,
    "Dimensions",
    "native dialog name must come from the title relation",
  );
  for (const testId of ["surface", "name", "counter"]) {
    const token = get(opened, testId).token;
    assert.equal(get(leaving, testId).token, token);
    assert.equal(get(reopened, testId).token, token);
    assert.notEqual(get(fresh, testId).token, token);
  }
  assert.ok(Number(get(opened, "ends").text) > 0, "caller must observe the native enter end");
  assert.ok(
    Number(get(fresh, "ends").text) > Number(get(opened, "ends").text),
    "caller animation events stopped after reopening",
  );
  assert.deepEqual(
    get(opened, "page").boundsLogical,
    get(initial, "page").boundsLogical,
    "opening changed page scroll/layout",
  );
  const scope = get(opened, "anchor-scope").boundsLogical,
    panel = parentOf(opened, "surface").boundsLogical;
  assert.ok(
    panel.y + panel.height <= scope.y + 4,
    "surface did not escape its clipped source ancestor",
  );
  const title = get(opened, "title");
  assert.ok(
    title.boundsLogical.y + title.boundsLogical.height < scope.y,
    "title probe must lie outside the source clip",
  );
  const base = await pngRgba(path.join(out, "captures/closed-initial.png"));
  const image = await pngRgba(path.join(out, "captures/opened.png"));
  assert.equal(image.width, base.width);
  assert.equal(image.height, base.height);
  const r = title.boundsDevice;
  const left = Math.max(0, Math.floor(r.x)),
    top = Math.max(0, Math.floor(r.y));
  const right = Math.min(image.width, Math.ceil(r.x + r.width)),
    bottom = Math.min(image.height, Math.ceil(r.y + r.height));
  assert.ok(right > left && bottom > top);
  let painted = 0;
  for (let y = top; y < bottom; y++)
    for (let x = left; x < right; x++) {
      const offset = (y * image.width + x) * 4;
      if (
        [0, 1, 2].every(
          c => image.data[offset + c] < 180 && base.data[offset + c] - image.data[offset + c] > 30,
        )
      )
        painted += 1;
    }
  assert.ok(painted >= 20, "popover title did not paint outside its clipped source");
});

test("popover follows scrolling and transforms, keeps local theme and retires its source", async () => {
  const { out } = await run(
    [
      click("launch"),
      advance(400),
      check("name", "focused", true),
      capture("before-scroll"),
      wheelAt(760, 540, 80),
      advance(400),
      check("surface", "exists", true),
      capture("after-scroll"),
      click("move"),
      advance(400),
      capture("translated"),
      click("theme"),
      advance(400),
      capture("dark"),
      at(40, 40),
      check("background-count", "text", "1"),
      advance(400),
      check("surface", "exists", false),
      check("background", "focused", true),
      capture("outside"),
      click("launch"),
      advance(400),
      click("policy"),
      key("Escape"),
      check("requested", "text", "open"),
      at(40, 40),
      check("background-count", "text", "2"),
      check("surface", "exists", true),
      click("policy"),
      key("Escape"),
      advance(400),
      check("surface", "exists", false),
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
    ],
    "ownership",
    "main",
  );
  const before = await tree(out, "before-scroll"),
    scrolled = await tree(out, "after-scroll"),
    moved = await tree(out, "translated"),
    dark = await tree(out, "dark");
  const removed = await tree(out, "before-removal"),
    restored = await tree(out, "restored");
  const anchor = t => get(t, "launch").boundsLogical,
    panel = t => parentOf(t, "surface").boundsLogical;
  const tolerance = 1 / 1.75 + 0.001;
  const close = (a, b, message) =>
    assert.ok(Math.abs(a - b) <= tolerance, `${message}: ${a} vs ${b}`);
  assert.ok(
    get(scrolled, "page").boundsLogical.y < get(before, "page").boundsLogical.y,
    "popover incorrectly locked background scrolling",
  );
  assert.ok(anchor(scrolled).y < anchor(before).y, "scroll control did not move the source");
  close(
    panel(scrolled).y - panel(before).y,
    anchor(scrolled).y - anchor(before).y,
    "surface did not follow root scroll",
  );
  close(anchor(moved).x - anchor(scrolled).x, 30, "transform did not move trigger x");
  close(anchor(moved).y - anchor(scrolled).y, -20, "transform did not move trigger y");
  close(panel(moved).x - panel(scrolled).x, 30, "surface did not follow transform x");
  close(panel(moved).y - panel(scrolled).y, -20, "surface did not follow transform y");
  for (const t of [before, scrolled, moved, dark]) {
    close(panel(t).y + panel(t).height + 4, anchor(t).y, "top anchor gap changed");
    for (const testId of ["surface", "name", "counter"])
      assert.equal(
        get(t, testId).token,
        get(before, testId).token,
        `${testId} remounted on scroll/transform/theme`,
      );
  }
  assert.ok(
    contains(get(before, "background").boundsLogical, 40, 40),
    "outside-click control point misses the background button",
  );
  assert.ok(!contains(panel(dark), 40, 40), "outside-click control point is inside the popup");
  for (const testId of ["surface", "name", "counter"])
    assert.notEqual(
      get(restored, testId).token,
      get(removed, testId).token,
      `${testId} reused a retired native handle`,
    );
});

test("initial popovers elevate inside dialog and restore nested native focus", async () => {
  const { out } = await run(
    [
      advance(400),
      check("dialog", "exists", true),
      check("outer", "exists", true),
      check("inner", "exists", true),
      check("inner-name", "focused", true),
      capture("nested"),
      key("Tab"),
      check("inner-close", "focused", true),
      key("Tab"),
      check("inner-name", "focused", true),
      key("Escape"),
      advance(400),
      check("inner", "exists", false),
      check("outer", "exists", true),
      capture("parent"),
      key("Tab"),
      check("outer-name", "focused", true),
      key("Tab"),
      check("inner-trigger", "focused", true),
      key("Enter"),
      advance(400),
      check("inner-name", "focused", true),
      capture("nested-fresh"),
      key("Escape"),
      advance(400),
      check("inner", "exists", false),
      check("inner-trigger", "focused", true),
      key("Enter"),
      advance(400),
      check("inner-name", "focused", true),
      key("Escape"),
      key("Escape"),
      advance(400),
      check("inner", "exists", false),
      check("outer", "exists", false),
      check("dialog", "exists", true),
      capture("dialog-only"),
      key("Tab"),
      check("dialog-name", "focused", true),
      click("dialog-close"),
      advance(400),
      check("dialog", "exists", false),
      capture("all-closed"),
    ],
    "initial",
    "initial",
  );
  const nested = await tree(out, "nested"),
    parent = await tree(out, "parent"),
    fresh = await tree(out, "nested-fresh"),
    dialog = await tree(out, "dialog-only");
  assert.equal(get(nested, "outer").name, "Item details");
  assert.equal(get(nested, "inner").name, "Nested details");
  assert.equal(
    parent.nodes.some(n => n.state?.focused),
    false,
    "initial child had no prior focused node to restore",
  );
  assert.equal(
    dialog.nodes.some(n => n.state?.focused),
    false,
    "initial parent had no prior focused node to restore",
  );
  assert.equal(get(parent, "outer").token, get(nested, "outer").token);
  assert.equal(get(fresh, "outer").token, get(nested, "outer").token);
  assert.notEqual(get(fresh, "inner").token, get(nested, "inner").token);
  assert.equal(get(dialog, "dialog").token, get(nested, "dialog").token);
});

test("popover places left and right surfaces and flips horizontal placement natively", async () => {
  const { out } = await run(
    [
      capture("initial", 0),
      advance(400),
      capture("sides"),
      click("flip"),
      check("requested-side", "text", "right"),
      advance(400),
      capture("flipped"),
    ],
    "sides",
    "sides",
  );
  const frames = {};
  for (const name of ["initial", "sides", "flipped"]) frames[name] = await tree(out, name);
  const tolerance = 1 / 1.75 + 0.001;
  const close = (actual, expected, label) =>
    assert.ok(Math.abs(actual - expected) <= tolerance, `${label}: ${actual} vs ${expected}`);
  for (const [name, frame] of Object.entries(frames)) {
    const left = get(frame, "left-anchor").boundsLogical,
      right = get(frame, "right-anchor").boundsLogical;
    const toRight = parentOf(frame, "right-surface").boundsLogical,
      toLeft = parentOf(frame, "left-surface").boundsLogical;
    close(toRight.x, left.x + left.width + 6, `${name}: right-side offset`);
    close(toRight.y, left.y, `${name}: start alignment`);
    close(toLeft.x + toLeft.width + 10, right.x, `${name}: left-side offset`);
    close(toLeft.y + toLeft.height, right.y + right.height, `${name}: end alignment`);
    for (const testId of ["right-surface", "left-surface"])
      assert.equal(
        get(frame, testId).token,
        get(frames.initial, testId).token,
        `${testId} remounted on placement change`,
      );
  }
  assert.equal(get(frames.flipped, "requested-side").text, "right");
  assert.deepEqual(
    parentOf(frames.flipped, "left-surface").boundsLogical,
    parentOf(frames.sides, "left-surface").boundsLogical,
    "requesting right near the right window edge must flip back to the same left placement",
  );
});
