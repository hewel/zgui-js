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
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-menubar-");
  bundle = await buildFixtureApp(path.join(temp.dir, "main"), "menubar", new URL("./fixtures/menubar.tsx", import.meta.url));
});
after(async () => { await temp?.cleanup(); });

let serial = 0;
const id = () => `step-${++serial}`;
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const moveTo = testId => ({ id: id(), op: "pointer.move", target: { surface: "main", testId } });
const key = (name, modifiers) => ({ id: id(), op: "key.press", surface: "main", key: name, ...(modifiers ? { modifiers } : {}) });
const char = text => key({ character: text });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const capture = (captureId, requiredFrameTimeUs) => ({ id: id(), op: "capture", captureId, ...(requiredFrameTimeUs === undefined ? {} : { requiredFrameTimeUs }) });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

function run(steps, name) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    id: `menubar-${name}`,
    requiredCapabilities: [
      "pixels", "virtual-time", "selectors.test-id", "selectors.semantic",
      "input.pointer", "input.keyboard", "tree.inspect", "surface.scale",
    ],
    steps,
  });
}
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
