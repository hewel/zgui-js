import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
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
  temp = await createTempWorkspace("preview-workbench-components-");
  bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "workbench-components",
    new URL("./fixtures/workbench-components.tsx", import.meta.url),
  );
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
const commitText = value => ({ id: id(), op: "text.commit", surface: "main", text: value });
const check = (testId, property, equals) => ({
  id: id(),
  op: "assert",
  target: { surface: "main", testId },
  check: { property, equals },
});
const capture = captureId => ({ id: id(), op: "capture", captureId });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const resize = (width, height) => ({
  id: id(),
  op: "surface.resize",
  surface: "main",
  width,
  height,
});

const CAPS = [
  "pixels",
  "virtual-time",
  "selectors.test-id",
  "input.pointer",
  "input.keyboard",
  "input.text",
  "tree.inspect",
  "surface.resize",
];

function run(name, steps) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `workbench-${name}`,
    requiredCapabilities: CAPS,
    steps,
  });
}

test(
  "SplitPane supports keyboard resize, collapse/expand, and pointer-capture drag",
  { timeout: 30000 },
  async () => {
    const { out, result } = await run("split-pane", [
      resize(900, 660),
      advance(60),
      check("wb-split-readout", "text", "left:40,collapsed:false"),
      capture("initial"),

      // Focus handle and resize right with ArrowRight (+5%)
      click("wb-split-handle"),
      advance(20),
      key("ArrowRight"),
      advance(30),
      check("wb-split-readout", "text", "left:45,collapsed:false"),
      check("wb-split-right-readout", "text", "right:55"),

      // Collapse via Enter, then restore via Enter
      key("Enter"),
      advance(30),
      check("wb-split-readout", "text", "left:0,collapsed:true"),
      capture("collapsed"),

      key("Enter"),
      advance(30),
      check("wb-split-readout", "text", "left:45,collapsed:false"),

      // Pointer-capture drag handle right by ~130px
      {
        id: id(),
        op: "pointer.down",
        target: { surface: "main", testId: "wb-split-handle" },
        button: "primary",
      },
      { id: id(), op: "pointer.move", surface: "main", position: { x: 540, y: 210 } },
      {
        id: id(),
        op: "pointer.up",
        surface: "main",
        position: { x: 540, y: 210 },
        button: "primary",
      },
      advance(40),
      capture("dragged"),
    ]);

    const tInitial = await readTree(out, "initial");
    const tDragged = await readTree(out, "dragged");
    const leftBefore = getNode(tInitial, "wb-split-left").boundsLogical.width;
    const leftAfter = getNode(tDragged, "wb-split-left").boundsLogical.width;
    assert.ok(
      leftAfter > leftBefore + 60,
      `expected left panel to widen after drag: before=${leftBefore}, after=${leftAfter}`,
    );

    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "Select & Combobox support keyboard navigation, scrollIntoView, filtering, and empty state",
  { timeout: 30000 },
  async () => {
    const { out, result } = await run("select-combobox", [
      resize(900, 660),
      advance(60),

      // Open Select and jump to last option (opt-24) via End + Enter (scrolls listbox)
      click("wb-select"),
      advance(60),
      check("wb-select-list", "exists", true),
      key("End"),
      advance(40),
      capture("select-open-end"),
      key("Enter"),
      advance(40),
      check("wb-select-val", "text", "sel:opt-24"),
      check("wb-select-list", "exists", false),

      // Combobox filter -> no match empty state -> refine -> select opt-15
      click("wb-combo-input"),
      advance(40),
      check("wb-combo-list", "exists", true),
      key({ character: "a" }, ["Control"]),
      commitText("zzzz-no-match"),
      advance(40),
      capture("combo-empty"),
      key({ character: "a" }, ["Control"]),
      commitText("Option 15"),
      advance(40),
      key("Enter"),
      advance(40),
      check("wb-combo-val", "text", "combo:opt-15"),
      check("wb-combo-list", "exists", false),
    ]);

    const emptyTree = await readTree(out, "combo-empty");
    const emptyStatus = emptyTree.nodes.find(n => n.text === "Nothing matched");
    assert.ok(emptyStatus, "expected Combobox empty state message 'Nothing matched'");

    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "Toast supports OverlayLayer::Toast stack limit, action button, hover-pause, auto-dismiss, promise, and reduced motion",
  { timeout: 30000 },
  async () => {
    const { out, result } = await run("toast", [
      resize(900, 660),
      advance(60),

      // Max stacking cap (maxVisible = 2) when pushing 3 toasts
      click("btn-toast-stack"),
      advance(50),
      check("toast-count", "text", "toasts:2"),
      check("wb-toast-region", "exists", true),
      capture("toast-stacked"),

      // Clear all
      click("btn-toast-clear"),
      advance(40),
      check("toast-count", "text", "toasts:0"),

      // Trigger auto-dismissing toast (200ms), hover to pause, advance 300ms (still visible), then leave and advance 300ms (auto-dismissed)
      click("btn-toast-success"),
      advance(40),
      check("toast-count", "text", "toasts:1"),
      { id: id(), op: "pointer.move", target: { surface: "main", testId: "wb-toast-region" } },
      advance(300),
      check("toast-count", "text", "toasts:1"),
      { id: id(), op: "pointer.move", surface: "main", position: { x: 10, y: 10 } },
      advance(300),
      check("toast-count", "text", "toasts:0"),

      // Trigger toast with action and click Undo button
      click("btn-toast-success"),
      advance(50),
      click("toast-action-save-1"),
      advance(40),
      check("toast-action-log", "text", "undo-clicked"),
      check("toast-count", "text", "toasts:0"),

      // Reduced motion + toast.promise transitions to success
      click("toggle-reduced"),
      click("btn-toast-promise"),
      advance(50),
      check("toast-count", "text", "toasts:1"),
      capture("toast-promise-reduced"),
    ]);

    const promiseTree = await readTree(out, "toast-promise-reduced");
    const syncedLabel = promiseTree.nodes.find(n => n.text === "Synced:ok");
    assert.ok(syncedLabel, "expected toast.promise to resolve into 'Synced:ok'");

    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "TreeView supports expand/collapse, keyboard navigation, virtualization, and dark theme",
  { timeout: 30000 },
  async () => {
    const { out, result } = await run("tree-view", [
      resize(900, 660),
      advance(60),

      // Navigate nested tree via keyboard: workspace -> ArrowDown (notes) -> ArrowRight (expand notes) -> ArrowDown (alpha.md) -> Enter (activate)
      click("tree-item-workspace"),
      advance(30),
      // Clicking expanded workspace toggles it closed; ArrowRight re-expands it cleanly
      key("ArrowRight"),
      advance(30),
      key("ArrowDown"),
      advance(30),
      key("ArrowRight"),
      advance(30),
      key("ArrowDown"),
      advance(30),
      key("Enter"),
      advance(30),
      check(
        "wb-tree-state",
        "text",
        "sel:workspace/notes/alpha.md,act:workspace/notes/alpha.md,exp:workspace|workspace/notes",
      ),

      // ArrowLeft jumps to parent (workspace/notes), second ArrowLeft collapses it
      key("ArrowLeft"),
      advance(20),
      key("ArrowLeft"),
      advance(30),
      check(
        "wb-tree-state",
        "text",
        "sel:workspace/notes/alpha.md,act:workspace/notes/alpha.md,exp:workspace",
      ),

      // Virtualized 120-item tree: verify windowed rendering + End key selects last item
      click("tree-item-node-0"),
      advance(30),
      capture("vtree-top"),
      key("End"),
      advance(40),
      key("Enter"),
      advance(40),
      check("wb-vtree-state", "text", "vsel:node-119"),

      // Switch to dark theme and capture
      click("toggle-scheme"),
      advance(60),
      capture("dark-theme"),
    ]);

    const topTree = await readTree(out, "vtree-top");
    const mountedVirtNodes = topTree.nodes.filter(
      n => typeof n.testId === "string" && n.testId.startsWith("tree-item-node-"),
    );
    assert.ok(
      mountedVirtNodes.length > 0 && mountedVirtNodes.length < 30,
      `expected virtualized TreeView to mount a small window of rows, got ${mountedVirtNodes.length}`,
    );

    assert.equal(result.teardown?.clean, true);
  },
);
