import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

let project;
before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-submenu-"));
  for (const [variant, fixture] of [["main", "submenu.tsx"], ["controlled", "submenu-controlled.tsx"],
    ["initial", "submenu-initial.tsx"]]) {
    const app = path.join(project, variant);
    await mkdir(path.join(app, "src"), { recursive: true });
    const source = await readFile(fileURLToPath(new URL(`./fixtures/${fixture}`, import.meta.url)), "utf8");
    await writeFile(path.join(app, "src/main.tsx"), source);
    await writeFile(path.join(app, "tsconfig.json"), JSON.stringify({
      extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
    }));
    await buildApplication(`submenu-${variant}`, { appDirectory: app });
  }
});
after(async () => {
  if (!project) return;
  if (process.env.ZGUI_KEEP_PREVIEW_OUTPUT === "1") {
    console.log(`retained preview project: ${project}`);
  } else {
    await rm(project, { recursive: true, force: true });
  }
});

let serial = 0;
const id = () => `step-${++serial}`;
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const key = (name, modifiers) => ({ id: id(), op: "key.press", surface: "main", key: name, ...(modifiers ? { modifiers } : {}) });
const char = (text, modifiers) => key({ character: text }, modifiers);
const moveTo = testId => ({ id: id(), op: "pointer.move", target: { surface: "main", testId } });
const move = (x, y) => ({ id: id(), op: "pointer.move", surface: "main", position: { x, y } });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const emptyResources = { nodes: 0, wrappers: 0, callbacks: 0, focusTraps: 0, scrollLocks: 0, timers: 0,
  frameCallbacks: 0, requests: 0, engineAlive: false };

async function run(steps, name, variant = "main") {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `submenu-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "selectors.semantic", "input.pointer", "input.keyboard",
      "tree.inspect", "surface.scale"],
    steps,
  }));
  const process = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "run",
    "--bundle", path.join(project, variant, "dist"),
    "--profile", path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario", scenario,
    "--out", out,
  ], { cwd: workspaceRoot, encoding: "utf8", timeout: 60000 });
  if (process.stderr) await writeFile(path.join(project, `${name}.stderr`), process.stderr);
  assert.ifError(process.error);
  const result = JSON.parse(process.stdout);
  assert.equal(process.signal, null, process.stderr);
  assert.equal(process.status, result.exitCode);
  assert.equal(result.exitCode, 0, process.stdout);
  assert.equal(result.status, "passed", process.stdout);
  assert.equal(result.assertions.failed, 0);
  assert.equal(result.assertions.passed, steps.filter(step => step.op === "assert").length);
  assert.equal(result.teardown.clean, true);
  assert.deepEqual(result.teardown.resources, emptyResources);
  return { result, out };
}

const tree = async (out, name) => JSON.parse(await readFile(path.join(out, `captures/${name}.tree.json`), "utf8"));
const get = (tree, testId) => {
  const node = tree.nodes.find(node => node.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};
const positionerBounds = (t, testId) => {
  const surface = get(t, testId);
  const positioner = t.nodes.find(node => node.token === surface.parent);
  assert.ok(positioner?.boundsLogical, `missing positioner for ${testId}`);
  return positioner.boundsLogical;
};
const childMap = t => {
  const children = new Map();
  for (const node of t.nodes) {
    if (!node.parent) continue;
    if (!children.has(node.parent)) children.set(node.parent, []);
    children.get(node.parent).push(node);
  }
  return children;
};
const descendants = (t, testId) => {
  const children = childMap(t);
  const found = [];
  const walk = token => { for (const child of children.get(token) ?? []) { found.push(child); walk(child.token); } };
  walk(get(t, testId).token);
  return found;
};
const images = (t, testId) => descendants(t, testId).filter(node => node.role === "image");
const pointer = (op, position, button = "primary") => ({ id: id(), op, surface: "main", position, button });
const movePoint = p => move(p.x, p.y);
const trajectory = (t, left) => {
  const r = get(t, "branch").boundsLogical, s = get(t, "submenu").boundsLogical;
  const a = get(t, "after").boundsLogical, d = get(t, "disabled-branch").boundsLogical;
  const from = { x: left ? Math.floor(r.x + r.width - 20) : Math.ceil(r.x + 20), y: Math.ceil(r.y + 4) };
  const toward = { x: Math.round(r.x + r.width * (left ? 0.3 : 0.7)), y: Math.floor(a.y + a.height / 2) };
  const away = { x: from.x, y: toward.y };
  const inside = (p, b) => b.x < p.x && p.x < b.x + b.width && b.y < p.y && p.y < b.y + b.height;
  const near = left ? s.x + s.width : s.x;
  const u = (toward.x - from.x) / (near - from.x);
  assert.ok(inside(from, r) && inside(toward, a) && !inside(toward, r) && !inside(toward, s));
  assert.ok(0 < u && u < 1);
  assert.ok(from.y + u * (s.y - from.y) < toward.y && toward.y < from.y + u * (s.y + s.height - from.y));
  assert.ok(inside(away, a) && !inside(away, s));
  return { from, toward, away, disabled: { x: d.x + d.width / 2, y: d.y + d.height / 2 } };
};

test("native submenu keyboard, nested selection, theme and lifecycle", async () => {
  const probeRight = await run([
    key("Tab"), key("ArrowDown"), key("ArrowDown"), key("ArrowRight"), advance(300), capture("right"),
  ], "probe-right");
  const probeLeft = await run([
    click("edge"), click("root-trigger"), key("ArrowDown"), key("ArrowRight"), advance(300), capture("left"),
  ], "probe-left");
  const submenu = await run([
    check("submenu", "exists", false),
    key("Tab"), key("ArrowDown"), check("before", "focused", true),
    key("ArrowDown"), check("branch", "focused", true),
    char("ArrowRight"), char("Enter"), char(" "), check("submenu", "exists", false),
    key("ArrowRight"), check("keep", "focused", true),
    check("submenu", "role", "menu"), check("submenu", "name", "Export"),
    capture("first", 0), advance(300), capture("light"),
    key("Space"), check("sub-count", "text", "1"),
    key("ArrowDown"), check("flag", "focused", true), key("Space"), check("pinned", "text", "true"),
    char("m"), check("deep-trigger", "focused", true), check("deep", "exists", false),
    key("ArrowRight"), check("leaf", "focused", true), check("deep", "role", "menu"),
    advance(300), capture("nested"),
    key("Escape"), advance(300), check("deep", "exists", false),
    check("submenu", "exists", true), check("deep-trigger", "focused", true),
    key("ArrowLeft"), check("branch", "focused", true), advance(40), capture("exit"),
    key("ArrowRight"), advance(300), check("submenu", "exists", true),
    check("sub-count", "text", "1"), capture("reopened"),
    moveTo("keep"), key("ArrowLeft"), advance(300), check("submenu", "exists", false),
    check("menu", "exists", true),
    key("Space"), check("keep", "focused", true), advance(300),
    check("sub-count", "text", "0"), check("pinned", "text", "true"), capture("fresh"),
    char("t"), check("theme", "focused", true), key("Enter"), advance(300), capture("dark"),
    { id: id(), op: "surface.scale", surface: "main", scale: 2 },
    advance(300), check("submenu", "exists", true), check("sub-count", "text", "0"),
    check("pinned", "text", "true"), capture("dense"),
    click("toggle-source"), advance(300), check("menu", "exists", false),
    check("submenu", "exists", false), check("deep", "exists", false),
    check("toggle-source", "focused", true), capture("removed"),
    click("toggle-source"), check("menu", "exists", false),
    click("root-trigger"), key("ArrowDown"), key("Enter"), advance(300),
    check("keep", "focused", true), check("sub-count", "text", "0"),
    check("pinned", "text", "true"), capture("remounted"),
    char("k"), check("keep", "focused", true),
  ], "submenu");
  const selection = await run([
    click("root-trigger"), key("ArrowDown"), key("ArrowRight"), char("m"), key("ArrowRight"),
    key("Enter"),
    check("selections", "text", "1"), check("selected", "text", "PDF"),
    check("root-state", "text", "closed"),
    advance(300), check("menu", "exists", false), check("submenu", "exists", false),
    check("deep", "exists", false), check("root-trigger", "focused", true), capture("selected"),
  ], "selection");

  const frames = {};
  for (const name of ["first", "light", "nested", "exit", "reopened", "fresh", "dark", "dense", "removed", "remounted"]) {
    frames[name] = await tree(submenu.out, name);
  }
  const selectedTree = await tree(selection.out, "selected");
  const rightTree = await tree(probeRight.out, "right"), leftTree = await tree(probeLeft.out, "left");
  {
    const anchor = get(rightTree, "branch").boundsLogical;
    const panel = positionerBounds(rightTree, "submenu");
    const tolerance = 1 / 1.75 + 0.001;
    assert.ok(Math.abs(panel.x - (anchor.x + anchor.width - 4)) <= tolerance,
      "right probe submenu hugs the trigger's right edge with the -4px offset");
    assert.ok(Math.abs(panel.y - anchor.y) <= tolerance, "right probe submenu aligns to the trigger top");
    assert.ok(panel.x >= 8 - tolerance && panel.y >= 8 - tolerance &&
      panel.x + panel.width <= 800 - 8 + tolerance && panel.y + panel.height <= 600 - 8 + tolerance,
      "right probe submenu stays inside the padded viewport");
  }
  {
    const anchor = get(leftTree, "branch").boundsLogical;
    const panel = positionerBounds(leftTree, "submenu");
    const tolerance = 1 / 1.75 + 0.001;
    assert.ok(Math.abs(panel.x - (anchor.x - panel.width + 4)) <= tolerance,
      "left probe submenu hugs the trigger's left edge after the viewport-edge flip");
    assert.ok(Math.abs(panel.y - anchor.y) <= tolerance, "left probe submenu aligns to the trigger top");
    assert.ok(panel.x >= 8 - tolerance && panel.y >= 8 - tolerance &&
      panel.x + panel.width <= 800 - 8 + tolerance && panel.y + panel.height <= 600 - 8 + tolerance,
      "left probe submenu stays inside the padded viewport");
  }
  const firstAnchor = get(frames.first, "branch").boundsLogical;
  const firstPanel = positionerBounds(frames.first, "submenu");
  const firstTolerance = 1 / 1.75 + 0.001;
  assert.ok(Math.abs(firstPanel.x - (firstAnchor.x + firstAnchor.width - 4)) <= firstTolerance);
  assert.ok(Math.abs(firstPanel.y - firstAnchor.y) <= firstTolerance);
  const settledPanel = get(frames.light, "submenu").boundsLogical;
  assert.deepEqual([firstPanel.width, firstPanel.height], [settledPanel.width, settledPanel.height]);
  for (const testId of ["submenu", "keep", "sub-count"]) {
    assert.equal(get(frames.reopened, testId).token, get(frames.light, testId).token,
      `${testId}: reopening a busy exit reuses the live panel`);
    assert.notEqual(get(frames.fresh, testId).token, get(frames.light, testId).token,
      `${testId}: completed exit remounts the panel`);
  }
  assert.equal(get(frames.reopened, "sub-count").text, "1", "reopened panel keeps its live body counter");
  assert.equal(get(frames.fresh, "sub-count").text, "0", "fresh panel mounts a new body counter");
  for (const testId of ["branch", "trigger-count"]) {
    assert.equal(get(frames.fresh, testId).token, get(frames.light, testId).token,
      `${testId}: trigger row survives panel remounts`);
  }
  for (const name of ["dark", "dense"]) {
    for (const testId of ["submenu", "keep", "sub-count"]) {
      assert.equal(get(frames[name], testId).token, get(frames.fresh, testId).token,
        `${name}: ${testId} survives theme and scale changes`);
    }
  }
  for (const testId of ["branch", "root-trigger"]) {
    assert.notEqual(get(frames.remounted, testId).token, get(frames.dense, testId).token,
      `${testId}: source removal remounts the whole menu`);
  }
  for (const name of ["fresh", "dark", "dense", "remounted"]) {
    assert.equal(images(frames[name], "flag").length, 1,
      `${name}: checked submenu checkbox keeps its indicator`);
  }
  for (const [name, scale] of [["light", 1.75], ["dark", 1.75], ["dense", 2]]) {
    const anchor = get(frames[name], "branch").boundsLogical;
    const panel = positionerBounds(frames[name], "submenu");
    const tolerance = 1 / scale + 0.001;
    assert.ok(Math.abs(panel.x - (anchor.x + anchor.width - 4)) <= tolerance,
      `${name}: submenu hugs the trigger's right edge`);
    assert.ok(Math.abs(panel.y - anchor.y) <= tolerance, `${name}: submenu aligns to the trigger top`);
  }
  assert.equal(selectedTree.nodes.find(node => node.testId === "menu"), undefined,
    "leaf selection removes the whole stack");

  const insideRect = (p, b) => b.x < p.x && p.x < b.x + b.width && b.y < p.y && p.y < b.y + b.height;
  const right = trajectory(rightTree, false);
  const hoverRight = await run([
    click("root-trigger"), advance(300),
    movePoint(right.from), advance(119), check("submenu", "exists", false),
    advance(1), check("submenu", "exists", true), advance(300),
    movePoint(right.from), movePoint(right.toward), check("after", "focused", true),
    advance(500), check("submenu", "exists", true), check("sub-state", "text", "open"), capture("corridor"),
    moveTo("keep"), check("keep", "focused", true),
    movePoint(right.from), movePoint(right.away), advance(100),
    moveTo("keep"), advance(200), check("submenu", "exists", true), check("keep", "focused", true),
    movePoint(right.from), movePoint(right.away), advance(100),
    movePoint(right.from), advance(200), check("submenu", "exists", true),
    movePoint(right.away), advance(179), check("sub-state", "text", "open"),
    advance(1), check("sub-state", "text", "closed"), advance(300),
    check("submenu", "exists", false), check("menu", "exists", true),
    movePoint(right.from), advance(120), advance(300), check("submenu", "exists", true),
    capture("before-scale"),
    { id: id(), op: "surface.scale", surface: "main", scale: 2 }, advance(500),
    check("submenu", "exists", true), check("sub-state", "text", "open"), capture("dpi-grace"),
    click("outside"), advance(300), check("submenu", "exists", false), check("menu", "exists", true),
    check("outside", "focused", true),
    click("outside"), advance(300), check("menu", "exists", false), check("outside", "focused", true),
    capture("dismissed"),
  ], "hover-right");
  const beforeScale = await tree(hoverRight.out, "before-scale"), dpiGrace = await tree(hoverRight.out, "dpi-grace");
  assert.equal(get(dpiGrace, "submenu").token, get(beforeScale, "submenu").token,
    "DPI change keeps the same submenu surface");
  assert.equal(get(dpiGrace, "branch").token, get(beforeScale, "branch").token,
    "DPI change keeps the same submenu trigger");
  assert.ok(insideRect(right.from, get(beforeScale, "branch").boundsLogical),
    "before-scale branch still covers the hover point");
  const stationary = { x: right.from.x * 1.75 / 2, y: right.from.y * 1.75 / 2 };
  assert.ok(!insideRect(stationary, get(dpiGrace, "branch").boundsLogical) &&
    !insideRect(stationary, get(dpiGrace, "submenu").boundsLogical),
    "the unmoved device point leaves both surfaces at scale 2");
  const left = trajectory(leftTree, true);
  await run([
    click("edge"), click("root-trigger"), advance(300),
    movePoint(left.from), advance(119), check("submenu", "exists", false),
    advance(1), check("submenu", "exists", true), advance(300),
    movePoint(left.from), movePoint(left.toward), check("after", "focused", true),
    advance(500), check("submenu", "exists", true), check("sub-state", "text", "open"), capture("corridor"),
    moveTo("keep"), check("keep", "focused", true),
    movePoint(left.from), movePoint(left.away), advance(100),
    moveTo("keep"), advance(200), check("submenu", "exists", true), check("keep", "focused", true),
    movePoint(left.from), movePoint(left.away), advance(100),
    movePoint(left.from), advance(200), check("submenu", "exists", true),
    movePoint(left.away), advance(179), check("sub-state", "text", "open"),
    advance(1), check("sub-state", "text", "closed"), advance(300),
    check("submenu", "exists", false), check("menu", "exists", true),
    movePoint(left.from), advance(120), advance(300), check("submenu", "exists", true),
    capture("before-scale"),
    moveTo("keep"), check("keep", "focused", true), capture("left-panel"),
  ], "hover-left");
  assert.ok(insideRect(right.disabled, get(rightTree, "disabled-branch").boundsLogical),
    "timing probe presses the disabled row's raw center");
  await run([
    click("root-trigger"), advance(300),
    movePoint(right.disabled), pointer("pointer.down", right.disabled), pointer("pointer.up", right.disabled),
    advance(200),
    check("disabled-submenu", "exists", false), check("submenu", "exists", false), check("menu", "exists", true),
    movePoint(right.from), advance(119), check("submenu", "exists", false),
    movePoint(right.away), advance(200), check("submenu", "exists", false),
    movePoint(right.from), pointer("pointer.down", right.from), check("submenu", "exists", false),
    pointer("pointer.up", right.from), check("submenu", "exists", true),
    move(760, 580), advance(180), advance(300), check("submenu", "exists", false), check("menu", "exists", true),
    movePoint(right.from), pointer("pointer.down", right.from, "secondary"), pointer("pointer.up", right.from, "secondary"),
    check("submenu", "exists", false),
    movePoint(right.away), advance(200), check("submenu", "exists", false),
    movePoint(right.from), click("toggle-source"), advance(300),
    check("root-trigger", "exists", false), check("menu", "exists", false), check("submenu", "exists", false),
    capture("removed"),
  ], "timing");
});

test("native controlled submenu requests and denied returns", async () => {
  const { out } = await run([
    check("branch", "focused", true), check("submenu", "exists", false),
    key("ArrowRight"), check("requests", "text", "1"), check("submenu", "exists", false),
    key("Space"), check("requests", "text", "2"),
    char("Enter"), check("requests", "text", "2"),
    key("Enter"), check("requests", "text", "3"),
    click("accept"), check("open", "text", "open"), advance(300),
    check("keep", "focused", true), capture("open"),
    char("ArrowLeft"), check("requests", "text", "3"), check("submenu", "exists", true),
    key("ArrowLeft"), check("requests", "text", "4"), check("last", "text", "closed"),
    check("submenu", "exists", true), check("branch", "focused", true),
    key("Escape"), check("requests", "text", "5"), check("submenu", "exists", true),
    click("outside"), check("requests", "text", "6"), check("submenu", "exists", true),
    capture("denied"),
    click("accept"), check("requests", "text", "7"), check("open", "text", "closed"),
    advance(300), check("submenu", "exists", false), check("menu", "exists", true),
    check("accept", "focused", true), capture("closed"),
  ], "controlled", "controlled");
  const open = await tree(out, "open"), denied = await tree(out, "denied");
  for (const testId of ["submenu", "keep"]) {
    assert.equal(get(denied, testId).token, get(open, testId).token,
      `${testId}: denied close requests keep the live controlled panel`);
  }
});

test("native initially open submenu stack returns one layer at a time", async () => {
  await run([
    capture("initial", 0),
    check("dialog", "exists", true), check("menu", "exists", true),
    check("submenu", "exists", true), check("deep", "exists", true),
    check("leaf", "focused", true),
    advance(300), capture("open"),
    key("ArrowLeft"), check("deep-trigger", "focused", true), advance(300),
    check("deep", "exists", false), check("submenu", "exists", true),
    check("menu", "exists", true), check("dialog", "exists", true),
    key("ArrowLeft"), check("branch", "focused", true), advance(300),
    check("submenu", "exists", false), check("menu", "exists", true),
    check("dialog", "exists", true), capture("returned"),
    key("Escape"), advance(300), check("menu", "exists", false), check("dialog", "exists", true),
    key("Escape"), advance(300), check("dialog", "exists", false), capture("closed"),
  ], "initial", "initial");
  await run([
    advance(300), check("dialog", "exists", true), check("menu", "exists", true),
    check("submenu", "exists", true), check("deep", "exists", true),
    check("leaf", "focused", true), capture("active"),
  ], "initial-active", "initial");
});
