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
  project = await mkdtemp(path.join(workspaceRoot, "target/preview-menubar-"));
  const app = path.join(project, "main");
  await mkdir(path.join(app, "src"), { recursive: true });
  const source = await readFile(fileURLToPath(new URL("./fixtures/menubar.tsx", import.meta.url)), "utf8");
  await writeFile(path.join(app, "src/main.tsx"), source);
  await writeFile(path.join(app, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
    include: ["src/**/*.tsx"],
  }));
  await buildApplication("menubar", { appDirectory: app });
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
const moveTo = testId => ({ id: id(), op: "pointer.move", target: { surface: "main", testId } });
const key = (name, modifiers) => ({ id: id(), op: "key.press", surface: "main", key: name, ...(modifiers ? { modifiers } : {}) });
const char = text => key({ character: text });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const emptyResources = {
  nodes: 0, wrappers: 0, callbacks: 0, focusTraps: 0, scrollLocks: 0, timers: 0,
  frameCallbacks: 0, requests: 0, engineAlive: false,
};

async function run(steps, name) {
  const scenario = path.join(project, `${name}.json`);
  const out = path.join(project, `run-${name}`);
  await writeFile(scenario, JSON.stringify({
    schemaVersion: 1,
    id: `menubar-${name}`,
    requiredCapabilities: [
      "pixels", "virtual-time", "selectors.test-id", "selectors.semantic",
      "input.pointer", "input.keyboard", "tree.inspect", "surface.scale",
    ],
    steps,
  }));
  const process = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), [
    "preview", "run",
    "--bundle", path.join(project, "main/dist"),
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
const get = (t, testId) => {
  const node = t.nodes.find(entry => entry.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
};
const positionerBounds = (t, testId) => {
  const surface = get(t, testId);
  const positioner = t.nodes.find(entry => entry.token === surface.parent);
  assert.ok(positioner?.boundsLogical, `missing positioner for ${testId}`);
  return positioner.boundsLogical;
};

test("menubar horizontal bar, arrow/hover swaps, item choices, submenu, and dismiss focus", async () => {
  const { out } = await run([
    capture("closed", 0),
    check("bar", "role", "menuBar"),
    check("file-trigger", "role", "menuItem"),
    check("tools-trigger", "enabled", false),
    check("file-content", "exists", false),
    // Closed hover never opens menus
    moveTo("edit-trigger"),
    moveTo("view-trigger"),
    check("edit-content", "exists", false),
    check("view-content", "exists", false),

    // Clicking File opens anchored below File trigger and focuses first enabled item
    click("file-trigger"),
    advance(250),
    check("file-content", "exists", true),
    check("file-content", "role", "menu"),
    check("file-new", "focused", true),
    capture("file-open"),

    // Typeahead + ArrowDown + Submenu inside MenubarContent
    char("o"),
    check("file-open", "focused", true),
    key("ArrowDown"),
    check("file-export", "focused", true),
    key("ArrowRight"),
    advance(150),
    check("file-export-sub", "exists", true),
    capture("submenu-open"),
    key("Escape"),
    advance(300),
    check("file-export-sub", "exists", false),
    check("file-content", "exists", true),
    key("Escape"),
    check("file-content", "exists", false),
    check("file-trigger", "focused", true),

    // Keyboard bar navigation skips disabled Tools; ArrowDown opens View
    key("ArrowRight"),
    check("edit-trigger", "focused", true),
    check("edit-content", "exists", false),
    key("ArrowDown"),
    advance(250),
    check("edit-content", "exists", true),
    check("edit-undo", "focused", true),

    // ArrowRight inside Edit swaps past disabled Tools directly to View
    key("ArrowRight"),
    advance(250),
    check("edit-content", "exists", false),
    check("tools-content", "exists", false),
    check("view-content", "exists", true),
    check("view-wrap", "focused", true),
    capture("view-swapped"),

    // Checkbox defaults closeOnSelect=false; Radio defaults closeOnSelect=true & restores focus
    key("Space"),
    check("wrap-state", "text", "true"),
    check("view-content", "exists", true),
    capture("view-checked"),
    key("ArrowDown"),
    key("ArrowDown"),
    key("ArrowDown"),
    check("layout-split", "focused", true),
    key("Enter"),
    advance(250),
    check("layout-state", "text", "split"),
    check("view-content", "exists", false),
    check("view-trigger", "focused", true),

    // Hover swap along an open bar, then outside press closes without stealing focus from Outside action
    click("file-trigger"),
    advance(250),
    check("file-content", "exists", true),
    moveTo("edit-trigger"),
    advance(250),
    check("file-content", "exists", false),
    check("edit-content", "exists", true),
    check("edit-undo", "focused", true),
    capture("hover-edit"),

    click("outside"),
    advance(250),
    check("edit-content", "exists", false),
    check("outside", "focused", true),
    check("outside-clicks", "text", "1"),
    check("last-action", "text", "outside"),

    // Directly clicking View trigger opens View cleanly and toggles closed on second click
    click("view-trigger"),
    advance(250),
    check("view-content", "exists", true),
    check("view-wrap", "focused", true),
    click("view-trigger"),
    advance(250),
    check("view-content", "exists", false),
  ], "main");

  const frames = {};
  for (const name of ["file-open", "view-swapped", "hover-edit"]) {
    frames[name] = await tree(out, name);
  }
  const tolerance = 1 / 1.75 + 0.001;
  for (const [frameName, triggerId, contentId] of [
    ["file-open", "file-trigger", "file-content"],
    ["view-swapped", "view-trigger", "view-content"],
    ["hover-edit", "edit-trigger", "edit-content"],
  ]) {
    const t = frames[frameName];
    const anchor = get(t, triggerId).boundsLogical;
    const panel = positionerBounds(t, contentId);
    assert.ok(Math.abs(panel.x - anchor.x) <= tolerance,
      `${frameName}: start alignment ${JSON.stringify({ anchor, panel })}`);
    assert.ok(Math.abs(panel.y - anchor.y - anchor.height - 4) <= tolerance,
      `${frameName}: bottom offset gap ${JSON.stringify({ anchor, panel })}`);
  }
});
