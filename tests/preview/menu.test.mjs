import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";
import { pngRgba } from "./png.mjs";

let project;
before(async () => {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-menu-"));
  for (const [variant, fixture] of [["main", "menu.tsx"], ["controlled", "menu-controlled.tsx"], ["initial", "menu-initial.tsx"],
    ["descendants", "menu-descendants.tsx"]]) {
    const app = path.join(project, variant);
    await mkdir(path.join(app, "src"), { recursive: true });
    const source = await readFile(fileURLToPath(new URL(`./fixtures/${fixture}`, import.meta.url)), "utf8");
    await writeFile(path.join(app, "src/main.tsx"), source);
    await writeFile(path.join(app, "tsconfig.json"), JSON.stringify({
      extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
    }));
    await buildApplication(`menu-${variant}`, { appDirectory: app });
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
const at = (op, where) => ({ id: id(), op,
  ...(typeof where === "string" ? { target: { surface: "main", testId: where } } : { surface: "main", position: where }),
  button: "primary" });
const down = where => at("pointer.down", where);
const up = where => at("pointer.up", where);
const wheel = delta => ({ id: id(), op: "wheel", surface: "main", position: { x: 760, y: 580 }, delta: { x: 0, y: delta, unit: "pixels" } });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const emptyResources = { nodes: 0, wrappers: 0, callbacks: 0, focusTraps: 0, scrollLocks: 0, timers: 0,
  frameCallbacks: 0, requests: 0, engineAlive: false };

async function run(steps, name, variant = "main") {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1, id: `menu-${name}`,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "selectors.semantic", "input.pointer", "input.keyboard",
      "input.wheel", "tree.inspect", "surface.scale"],
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
const leafTexts = (t, testId, value) => {
  const children = childMap(t);
  return descendants(t, testId).filter(node => node.text === value && !children.get(node.token)?.length);
};
const images = (t, testId) => descendants(t, testId).filter(node => node.role === "image");

test("native dropdown roving, typeahead, activation, state and removal", async () => {
  const measured = await run([
    capture("page-before"), wheel(80), advance(400), capture("page-after"),
    key("Tab"), key("ArrowDown"), advance(300), capture("open"),
  ], "measure");
  const frozen = await tree(measured.out, "open");
  const r = get(frozen, "disabled").boundsLogical;
  assert.ok(r.width > 0 && r.height > 0, "disabled row must have measurable bounds");
  assert.equal(get(frozen, "disabled").state.enabled, false);
  const disabledX = r.x + r.width / 2, disabledY = r.y + r.height / 2;
  const pageBefore = await tree(measured.out, "page-before"), pageAfter = await tree(measured.out, "page-after");
  assert.ok(get(pageAfter, "page").boundsLogical.y < get(pageBefore, "page").boundsLogical.y - 1,
    "closed-menu control must produce genuine background scrolling");

  const { out } = await run([
    capture("initial", 0),
    check("menu", "exists", false),
    key("Tab"), check("trigger", "focused", true),
    key("ArrowUp"), check("print", "focused", true),
    check("menu", "role", "menu"), check("menu", "name", "Actions"),
    check("print", "role", "menuItem"), check("print", "name", "Print"),
    check("upload", "role", "menuItemCheckBox"), check("shortcuts", "role", "menuItemCheckBox"),
    check("name", "role", "menuItemRadio"), check("date", "role", "menuItemRadio"),
    check("disabled", "enabled", false), check("selections", "text", "0"),
    capture("first"), advance(300), capture("light"),
    key("ArrowDown"), check("properties", "focused", true),
    key("Home"), check("print", "focused", true),
    key("ArrowUp"), check("delete", "focused", true),
    key("ArrowDown"), check("print", "focused", true),
    key("End"), check("delete", "focused", true),
    key("Home"), check("print", "focused", true),
    check("selections", "text", "0"),
    char("p"), check("properties", "focused", true),
    char("p"), check("print", "focused", true),
    advance(1000),
    char("p"), check("properties", "focused", true),
    char("r"), check("print", "focused", true),
    char("o"), check("properties", "focused", true),
    advance(999),
    char("s"), check("properties", "focused", true),
    advance(1000),
    char("s"), check("save", "focused", true),
    char("s"), check("send", "focused", true),
    key("Tab"), check("send", "focused", true),
    key("Tab", ["Shift"]), check("send", "focused", true),
    advance(1000),
    char("p", ["Control"]), check("print", "focused", true), check("selections", "text", "0"),
    advance(1000),
    char("s"), check("save", "focused", true),
    char("s"), check("send", "focused", true),
    key("Space"), check("last", "text", "send"), check("selections", "text", "1"), check("open", "text", "open"),
    advance(1000), char(" "), check("selections", "text", "1"),
    char("Enter"), check("selections", "text", "1"),
    key("Enter"), check("selections", "text", "2"), check("open", "text", "open"),
    moveTo("print"), check("print", "focused", true),
    moveTo("label"), check("menu", "focused", true),
    key("Tab"), check("print", "focused", true),
    move(disabledX, disabledY), down({ x: disabledX, y: disabledY }), up({ x: disabledX, y: disabledY }),
    check("selections", "text", "2"), check("open", "text", "open"),
    down("print"), check("selections", "text", "3"), check("print-count", "text", "1"), check("open", "text", "open"),
    up("print"), check("selections", "text", "3"),
    click("shortcuts"), check("shortcut-state", "text", "false"), check("print-count", "text", "1"), capture("no-shortcut"),
    click("print"), check("selections", "text", "4"), check("print-count", "text", "2"),
    click("shortcuts"), check("shortcut-state", "text", "true"), check("print-count", "text", "2"),
    click("upload"), check("upload-state", "text", "true"), check("check-count", "text", "1"), check("send", "text", "Upload"),
    advance(1000), char("u"), check("send", "focused", true), capture("checked"),
    click("upload"), click("upload"), check("upload-state", "text", "true"), check("check-count", "text", "3"), capture("rechecked"),
    click("name"), check("radio-changes", "text", "0"),
    key("ArrowDown"), check("date", "focused", true), check("radio-changes", "text", "0"),
    key("Space"), check("sort", "text", "date"), check("radio-changes", "text", "1"),
    key("Space"), check("radio-changes", "text", "1"),
    click("name"), check("radio-changes", "text", "2"),
    click("date"), check("radio-changes", "text", "3"), capture("choices"),
    click("theme"), advance(300), check("open", "text", "open"),
    check("print-count", "text", "2"), check("check-count", "text", "3"), capture("dark"),
    { id: id(), op: "surface.scale", surface: "main", scale: 2 }, advance(50), capture("dense"),
    check("open", "text", "open"), check("print-count", "text", "2"), check("check-count", "text", "3"),
    advance(1000), char("s"), check("save", "focused", true),
    key("Enter"), check("selections", "text", "5"), check("last", "text", "save"), check("open", "text", "closed"),
    check("menu", "exists", true),
    advance(40), capture("exit"),
    click("trigger"), advance(300), check("open", "text", "open"),
    check("print-count", "text", "2"), check("check-count", "text", "3"), capture("reopened"),
    click("save"), check("selections", "text", "6"), advance(300),
    check("menu", "exists", false), check("trigger", "focused", true), capture("closed"),
    move(760, 580),
    key("Space"), check("print", "focused", true), advance(300),
    check("print-count", "text", "0"), check("check-count", "text", "0"),
    check("upload-state", "text", "true"), check("sort", "text", "date"), capture("fresh"),
    click("toggle-source"), check("trigger", "exists", false), check("menu", "exists", false),
    check("toggle-source", "focused", true), capture("removed"),
    click("toggle-source"), click("trigger"), advance(300),
    check("print-count", "text", "0"), check("check-count", "text", "0"), capture("remounted"),
    char("p"), check("properties", "focused", true),
  ], "main");

  const { out: wheelOut } = await run([
    click("trigger"), advance(300),
    check("menu", "exists", true), check("open", "text", "open"), check("print", "focused", true),
    capture("before-wheel"), wheel(80), advance(400),
    check("open", "text", "open"), check("menu", "exists", true), capture("after-wheel"),
    click("outside"), check("open", "text", "closed"), advance(300),
    check("menu", "exists", false), check("outside", "focused", true), capture("outside"),
    click("trigger"), char("p"), check("properties", "focused", true),
  ], "wheel");

  const wheelFrames = {};
  for (const name of ["before-wheel", "after-wheel", "outside"]) wheelFrames[name] = await tree(wheelOut, name);
  assert.ok(get(wheelFrames["after-wheel"], "page").boundsLogical.y < get(wheelFrames["before-wheel"], "page").boundsLogical.y - 1,
    "an open menu must not lock background scrolling");
  assert.equal(get(wheelFrames["after-wheel"], "page").token, get(wheelFrames["before-wheel"], "page").token,
    "background scrolling must not remount the page");
  assert.equal(get(wheelFrames["after-wheel"], "menu").token, get(wheelFrames["before-wheel"], "menu").token,
    "background scrolling must not remount the surface");
  assert.deepEqual(get(wheelFrames["after-wheel"], "trigger").boundsLogical, get(wheelFrames["before-wheel"], "trigger").boundsLogical,
    "the fixed anchor must not move under background scrolling");
  assert.deepEqual(positionerBounds(wheelFrames["after-wheel"], "menu"), positionerBounds(wheelFrames["before-wheel"], "menu"),
    "the fixed-anchor positioner must not move under background scrolling");

  const frames = {};
  for (const name of ["first", "light", "no-shortcut", "checked", "rechecked", "choices", "dark", "dense",
    "exit", "reopened", "closed", "fresh", "remounted"])
    frames[name] = await tree(out, name);

  assert.deepEqual(positionerBounds(frames.first, "menu"), get(frames.light, "menu").boundsLogical,
    "first-paint positioner must already match the settled surface");
  for (const [name, scale] of [["light", 1.75], ["dark", 1.75], ["dense", 2]]) {
    const anchor = get(frames[name], "trigger").boundsLogical;
    const panel = positionerBounds(frames[name], "menu");
    const tolerance = 1 / scale + 0.001;
    assert.ok(Math.abs(panel.x - anchor.x) <= tolerance, `${name}: start-aligned x ${JSON.stringify({ anchor, panel })}`);
    assert.ok(Math.abs(panel.y - anchor.y - anchor.height - 4) <= tolerance,
      `${name}: offset gap ${JSON.stringify({ anchor, panel })}`);
    assert.ok(panel.x >= 8 - tolerance && panel.y >= 8 - tolerance
      && panel.x + panel.width <= 800 - 8 + tolerance && panel.y + panel.height <= 600 - 8 + tolerance,
      `${name}: viewport padding ${JSON.stringify(panel)}`);
  }

  for (const testId of ["menu", "print", "print-count", "check-count", "upload", "send"])
    for (const name of ["checked", "rechecked", "choices", "dark", "dense", "reopened"])
      assert.equal(get(frames[name], testId).token, get(frames.light, testId).token,
        `${name}: ${testId} remounted across interactions`);
  assert.notEqual(get(frames.fresh, "menu").token, get(frames.light, "menu").token,
    "a completed exit must produce a fresh surface");
  assert.notEqual(get(frames.remounted, "trigger").token, get(frames.fresh, "trigger").token,
    "remounted source creates a fresh trigger");
  assert.notEqual(get(frames.remounted, "menu").token, get(frames.fresh, "menu").token,
    "remounted source creates a fresh surface");
  assert.deepEqual(get(frames.light, "disabled").boundsLogical, r,
    "disabled row geometry differs from the frozen measurement");

  const checkedIcons = images(frames.checked, "upload"), recheckedIcons = images(frames.rechecked, "upload");
  assert.equal(checkedIcons.length, 1, "checked upload row shows one tick");
  assert.equal(recheckedIcons.length, 1, "rechecked upload row shows one tick");
  assert.notEqual(recheckedIcons[0].token, checkedIcons[0].token, "a fresh tick paints per check");
  assert.equal(recheckedIcons[0].parent, checkedIcons[0].parent, "the indicator container survives toggles");
  assert.equal(get(frames.rechecked, "check-count").token, get(frames.checked, "check-count").token,
    "the body counter survives indicator swaps");
  assert.equal(images(frames.choices, "date").length, 1, "chosen radio shows its dot");
  assert.equal(images(frames.choices, "name").length, 0, "unchosen radio shows no dot");
  assert.equal(images(frames.light, "name").length, 1, "initial radio shows its dot");
  assert.equal(images(frames.light, "date").length, 0);
  assert.equal(leafTexts(frames["no-shortcut"], "print", "Ctrl+P").length, 0,
    "removed shortcut leaves no stale text");
  assert.equal(leafTexts(frames.light, "print", "Ctrl+P").length, 1,
    "visible shortcut text renders once");
  for (const testId of ["print-body", "print-count"])
    assert.equal(get(frames["no-shortcut"], testId).token, get(frames.light, testId).token,
      `${testId} survives the shortcut swap`);

  const removedImage = await pngRgba(path.join(out, "captures/removed.png"));
  assert.equal(removedImage.width, 1600);
  assert.equal(removedImage.height, 1200);
  const pixel = (x, y) => {
    const offset = (y * removedImage.width + x) * 4;
    return [...removedImage.data.subarray(offset, offset + 4)];
  };
  const background = pixel(16, 16);
  const menuDevice = get(frames.fresh, "menu").boundsDevice;
  const left = Math.max(0, Math.floor(menuDevice.x - 48)), top = Math.max(0, Math.floor(menuDevice.y - 48));
  const right = Math.min(removedImage.width, Math.ceil(menuDevice.x + menuDevice.width + 48));
  const bottom = Math.min(removedImage.height, Math.ceil(menuDevice.y + menuDevice.height + 48));
  assert.ok(right < 720, `probe region must end before the right-hand controls: ${right}`);
  for (let y = top; y < bottom; y += 8) for (let x = left; x < right; x += 8)
    assert.deepEqual(pixel(x, y), background, `stale paint at device ${x},${y}`);
});

test("controlled dropdown only requests open, checked and value changes", async () => {
  const { out } = await run([
    capture("initial", 0), check("menu", "exists", false),
    click("trigger"), check("requests", "text", "1"), check("menu", "exists", false),
    click("accept"), check("open", "text", "open"), advance(300),
    check("flag", "focused", true), capture("open"),
    key("Escape"), check("requests", "text", "2"), check("open", "text", "open"), check("menu", "exists", true),
    click("flag"), check("checks", "text", "1"), check("requests", "text", "3"), check("menu", "exists", true),
    click("radio-b"), check("radios", "text", "1"), check("requests", "text", "4"), check("menu", "exists", true),
    capture("denied"),
    click("accept"), check("requests", "text", "5"), check("open", "text", "closed"), advance(300),
    check("menu", "exists", false), check("accept", "focused", true), capture("closed"),
  ], "controlled", "controlled");
  const open = await tree(out, "open"), denied = await tree(out, "denied");
  for (const testId of ["menu", "flag", "radio-a", "radio-b"])
    assert.equal(get(denied, testId).token, get(open, testId).token, `${testId} remounted under controlled refusal`);
  for (const name of ["open", "denied"]) {
    const t = name === "open" ? open : denied;
    assert.equal(images(t, "flag").length, 1, `${name}: controlled checked stays ticked`);
    assert.equal(images(t, "radio-a").length, 1, `${name}: controlled value keeps Alpha`);
    assert.equal(images(t, "radio-b").length, 0, `${name}: denied radio stays unchosen`);
    assert.equal(get(t, "flag").role, "menuItemCheckBox");
    assert.equal(get(t, "radio-a").role, "menuItemRadio");
    assert.equal(get(t, "radio-b").role, "menuItemRadio");
  }
});

test("default-open dropdown inside a dialog shares elevation and restores focus", async () => {
  await run([
    capture("initial", 0),
    check("dialog", "exists", true), check("menu", "exists", true), check("keep", "focused", true),
    advance(300), capture("open"),
    key("Escape"), advance(300), check("menu", "exists", false), check("dialog", "exists", true),
    click("prepare"), check("trigger", "focused", true),
    key("Enter"), check("keep", "focused", true),
    click("keep"), check("count", "text", "1"), check("menu", "exists", true),
    key("Escape"), advance(300), check("menu", "exists", false), check("trigger", "focused", true),
    check("dialog", "exists", true), capture("restored"),
    key("Enter"), check("keep", "focused", true),
    key("Escape"), key("Escape"), advance(300),
    check("menu", "exists", false), check("dialog", "exists", false), capture("closed"),
  ], "initial", "initial");
});

test("native menu navigates from focused descendants and respects propagation", async () => {
  await run([
    key("Tab"), key("Enter"), advance(300), check("row-a", "focused", true),
    moveTo("row-a"), key("Tab"), check("child", "focused", true), capture("inside"),
    moveTo("label"), check("menu", "focused", true), capture("leave"),
    key("Tab"), check("row-a", "focused", true),
    key("Tab"), check("child", "focused", true),
    key("Tab"), check("loose", "focused", true),
    key("Tab"), check("blocked", "focused", true),
    key("ArrowDown"), check("blocked", "focused", true),
    moveTo("row-a"), key("Tab"), check("child", "focused", true),
    key("ArrowDown"), check("row-b", "focused", true), capture("navigated"),
    key("Tab"), check("loose", "focused", true),
    key("Home"), check("row-a", "focused", true),
    key("Tab"), key("Tab"), check("loose", "focused", true),
    key("End"), check("row-c", "focused", true),
    key("Tab"), check("loose", "focused", true),
    char("ArrowDown"), check("loose", "focused", true),
    key("ArrowUp"), check("row-b", "focused", true),
  ], "descendants", "descendants");
});
