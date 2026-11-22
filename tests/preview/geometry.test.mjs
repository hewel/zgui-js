import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  readTree,
  runScenario,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-geometry-");
  bundle = await buildFixtureApp(
    temp.dir,
    "geometry",
    new URL("./fixtures/geometry.tsx", import.meta.url),
  );
});
after(async () => {
  await temp?.cleanup();
});

const report = (anchor, follower, late, calls) =>
  "geometry:" +
  JSON.stringify({
    anchor: `${anchor}x40`,
    follower: `${follower}x40`,
    zero: "0x0",
    late: late === null ? "pending" : `${late}x40`,
    anchorCalls: calls,
  });

test("geometry scenario delivers first-paint observations and lifecycle", async () => {
  const click = (id, suffix = "") => ({
    id: `click-${id}${suffix}`,
    op: "pointer.click",
    target: { surface: "main", testId: id },
    button: "primary",
  });
  const capture = (id, requiredFrameTimeUs) => ({
    id: `capture-${id}`,
    op: "capture",
    captureId: id,
    ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }),
  });
  const expect = (id, equals) => ({
    id: `report-${id}`,
    op: "assert",
    target: { surface: "main", testId: "geometry-report" },
    check: { property: "text", equals },
  });
  const viewport = (id, width, height, scale) => ({
    id: `viewport-${id}`,
    op: "assert",
    target: { surface: "main", testId: "viewport-report" },
    check: { property: "text", equals: `viewport:${width}x${height}@${scale}` },
  });
  const steps = [
    capture("initial", 0),
    expect("initial", report(100, 100, null, 1)),
    viewport("initial", 800, 600, 1.75),
    click("resize"),
    capture("resized"),
    expect("resized", report(160, 160, null, 2)),
    click("late"),
    capture("late"),
    expect("late", report(160, 160, 160, 2)),
    click("cancel"),
    click("resize", "-after-cancel"),
    capture("cancelled"),
    expect("cancelled", report(160, 160, 220, 2)),
    click("toggle"),
    capture("removed"),
    {
      id: "assert-pair-removed",
      op: "assert",
      target: { surface: "main", testId: "geometry-pair" },
      check: { property: "exists", equals: false },
    },
    click("resize", "-hidden"),
    click("toggle", "-show"),
    capture("remounted"),
    expect("remounted", report(280, 280, null, 1)),
    { id: "window-resize", op: "surface.resize", surface: "main", width: 760, height: 560 },
    { id: "resize-frame", op: "clock.advance", byMs: 20 },
    capture("viewport-resized"),
    viewport("resized", 760, 560, 1.75),
    { id: "density", op: "surface.scale", surface: "main", scale: 1.25 },
    { id: "density-frame", op: "clock.advance", byMs: 20 },
    capture("viewport-scaled"),
    viewport("scaled", 760, 560, 1.25),
    click("toggle", "-viewport-hide"),
    capture("viewport-removed"),
    {
      id: "viewport-owner-removed",
      op: "assert",
      target: { surface: "main", testId: "geometry-pair" },
      check: { property: "exists", equals: false },
    },
    { id: "hidden-window-resize", op: "surface.resize", surface: "main", width: 800, height: 600 },
    { id: "hidden-window-frame", op: "clock.advance", byMs: 20 },
    click("toggle", "-viewport-show"),
    capture("viewport-remounted"),
    viewport("remounted", 800, 600, 1.25),
  ];
  const { out, stderr } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "geometry",
    id: "geometry",
    requiredCapabilities: [
      "pixels",
      "virtual-time",
      "selectors.test-id",
      "input.pointer",
      "tree.inspect",
      "surface.resize",
      "surface.scale",
    ],
    steps,
    stderrPath: `${temp.dir}/native.stderr`,
    assertEmptyResources: false,
  });

  const firstFrame = stderr.split("\n").find(line => line.startsWith("first-native-frame:"));
  assert.ok(firstFrame, `first-native-frame log missing: ${stderr}`);
  assert.ok(
    firstFrame.includes(report(100, 100, null, 1)),
    `first paint must already report anchor/follower/zero measurements: ${firstFrame}`,
  );
  assert.ok(
    firstFrame.includes("viewport:800x600@1.75"),
    `first paint must already report the window viewport: ${firstFrame}`,
  );

  const tree = name => readTree(out, name);
  const node = (capture, id) => getNode(capture, id);
  const bounds = (capture, id) => node(capture, id).boundsLogical;
  const captures = {};
  for (const name of ["initial", "resized", "late", "cancelled", "removed", "remounted"])
    captures[name] = await tree(name);
  for (const [name, anchorWidth, followerWidth] of [
    ["initial", 100, 100],
    ["resized", 160, 160],
    ["late", 160, 160],
    ["cancelled", 220, 160],
    ["remounted", 280, 280],
  ]) {
    for (const [id, expected] of [
      ["anchor", anchorWidth],
      ["follower", followerWidth],
    ]) {
      const measured = bounds(captures[name], id);
      assert.ok(
        measured &&
          Math.abs(measured.width - expected) < 0.05 &&
          Math.abs(measured.height - 40) < 0.05,
        `${name} ${id} expected ${expected}x40: ${JSON.stringify(measured)}`,
      );
    }
  }
  for (const id of ["anchor", "follower", "zero"]) {
    const token = node(captures.initial, id).token;
    for (const name of ["resized", "late", "cancelled"]) {
      assert.equal(node(captures[name], id).token, token, `${id} token changed in ${name}`);
    }
    assert.equal(
      captures.removed.nodes.find(candidate => candidate.testId === id),
      undefined,
      `${id} survived the hidden capture`,
    );
    assert.notEqual(
      node(captures.remounted, id).token,
      token,
      `${id} reused a stale token after remount`,
    );
  }
});
