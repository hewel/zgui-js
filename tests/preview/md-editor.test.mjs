import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
import {
  buildExampleApp,
  createTempWorkspace,
  getNode,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

const PNG_1X1_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

const WORKSPACE_WELCOME_MD = [
  "# Welcome to Studio",
  "",
  "Intro line with 你好🚀世界 UTF-16 test.",
  "",
  "```ts",
  'export const studio = "ready";',
  "```",
  "",
  ...Array.from({ length: 20 }, (_, i) => [
    `Spacer paragraph ${i + 1} for scroll testing.`,
    "",
  ]).flat(),
  "## Deep Target Section",
  "",
  "Reached via programmatic scrollIntoView.",
].join("\n");

const DEEP_HEADING_LINE = WORKSPACE_WELCOME_MD.split("\n").indexOf("## Deep Target Section");

const WORKSPACE_ARCH_MD = [
  "# Architecture Notes",
  "",
  "WaylandPipeline renders native Wayland surfaces with Vello and Stylo.",
  "",
  "- [ ] Verify FTS5 search hit",
].join("\n");

let temp;
let bundle;
let fixturePath;

before(async () => {
  temp = await createTempWorkspace("preview-md-editor-");
  bundle = await buildExampleApp("md-editor");
  fixturePath = path.join(temp.dir, "fixture.json");
  await writeJson(fixturePath, {
    schemaVersion: 1,
    id: "md-editor-wave1",
    services: {
      clipboard: {
        imageBase64: PNG_1X1_BASE64,
      },
      files: {
        dirs: ["/home/preview/notes"],
        files: {
          "/home/preview/notes/welcome.md": WORKSPACE_WELCOME_MD,
          "/home/preview/notes/architecture.md": WORKSPACE_ARCH_MD,
        },
        dialogs: [
          { action: "openDirectory", path: "/home/preview/notes" },
          { action: "openFile", path: "/home/preview/notes/architecture.md" },
          { action: "saveFile", path: "/home/preview/notes/exported.md" },
        ],
      },
    },
    events: [],
  });
});

after(async () => {
  await temp?.cleanup();
});

let serial = 0;
const id = () => `step-${++serial}`;
const click = (testId, surface = "main") => ({
  id: id(),
  op: "pointer.click",
  target: { surface, testId },
  button: "primary",
});
const key = (name, modifiers, surface = "main") => ({
  id: id(),
  op: "key.press",
  surface,
  key: name,
  ...(modifiers ? { modifiers } : {}),
});
const commitText = (text, surface = "main") => ({
  id: id(),
  op: "text.commit",
  surface,
  text,
});
const check = (testId, property, equals, surface = "main") => ({
  id: id(),
  op: "assert",
  target: { surface, testId },
  check: { property, equals },
});
const capture = (captureId, surface = "main") => ({
  id: id(),
  op: "capture",
  captureId,
  surface,
});
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });

const CAPS = [
  "pixels",
  "virtual-time",
  "selectors.test-id",
  "input.pointer",
  "input.wheel",
  "input.keyboard",
  "input.text",
  "tree.inspect",
  "surface.resize",
];

function run(name, steps) {
  return runScenario({
    projectDir: temp.dir,
    bundle,
    fixture: fixturePath,
    name,
    id: `md-editor-${name}`,
    width: 960,
    height: 600,
    scale: 1,
    requiredCapabilities: CAPS,
    steps,
  });
}

test(
  "Markdown Studio exercises workspace TreeView, SplitPane, UTF-16 CJK/emoji formatting, scrollIntoView, and clipboard+Toast (#63)",
  { timeout: 60000 },
  async () => {
    const { out, result } = await run("editor-core", [
      advance(32),
      check("md-editor-root", "exists", true),
      check("view-editor", "exists", true),

      // 1. Open local workspace folder into TreeView (#61, #62)
      click("btn-open-folder"),
      advance(32),
      check("workspace-tree", "exists", true),
      check("tree-item-ws:welcome.md", "exists", true),
      check("tree-item-ws:architecture.md", "exists", true),
      click("tree-item-ws:welcome.md"),
      advance(32),
      capture("workspace-loaded"),

      // 2. Programmatic scrollIntoView down to deep heading, then back to top (#55)
      click(`tree-item-heading-${DEEP_HEADING_LINE}`),
      advance(120),
      capture("after-outline-scroll"),
      click("tree-item-heading-0"),
      advance(120),

      // 3. Clipboard paste seeded PNG ImageHandle -> dismiss -> copy fenced code block + Toast feedback (#59, #62)
      click("btn-paste-image"),
      advance(32),
      check("pasted-image-card", "exists", true),
      check("pasted-image-el", "exists", true),
      click("btn-clear-pasted-img"),
      advance(24),
      check("pasted-image-card", "exists", false),
      click("copy-code-2"),
      advance(32),
      check("studio-toast-region", "exists", true),

      // 4. UTF-16 selection formatting with CJK + emoji (#56) + live dirty window title (#58)
      click("md-source-editor"),
      advance(20),
      key({ character: "a" }, ["Control"]),
      commitText("你好🚀世界"),
      advance(20),
      key({ character: "a" }, ["Control"]),
      click("fmt-bold"),
      advance(24),
      check("save-state-badge", "text", "Unsaved *"),
      check("window-title-readout", "text", "welcome.md * — Markdown Studio"),
      check("selection-readout", "text", "sel 2:8"),

      // 5. SplitPane keyboard resize (#62)
      click("split-handle-sidebar"),
      advance(16),
      key("ArrowRight"),
      key("ArrowRight"),
      advance(24),
      capture("after-split-resize"),
    ]);

    const tLoaded = await readTree(out, "workspace-loaded");
    const tScrolled = await readTree(out, "after-outline-scroll");
    const tResized = await readTree(out, "after-split-resize");

    const scrollText = getNode(tScrolled, "preview-scroll-readout").text ?? "";
    const statusText = getNode(tScrolled, "status-msg").text ?? "";
    const scroller = getNode(tScrolled, "md-preview-scroll");
    const headingNode = getNode(tScrolled, `preview-heading-${DEEP_HEADING_LINE}`);
    const matchY = scrollText.match(/scrollY=(\d+)/);
    assert.ok(
      matchY && Number(matchY[1]) > 40,
      `expected outline scrollIntoView to scroll preview, got "${scrollText}" (status="${statusText}", metrics=${JSON.stringify(scroller.scrollMetrics)}, heading=${JSON.stringify(headingNode.boundsLogical)})`,
    );

    const sideBefore = getNode(tLoaded, "studio-sidebar-panel").boundsLogical.width;
    const sideAfter = getNode(tResized, "studio-sidebar-panel").boundsLogical.width;
    assert.ok(
      sideAfter > sideBefore + 20,
      `expected sidebar SplitPane to widen: before=${sideBefore}, after=${sideAfter}`,
    );

    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "Markdown Studio exercises router + useBeforeLeave guard, SQLite FTS5 + KV Settings, auxiliary WindowSurface, and close veto (#63)",
  { timeout: 60000 },
  async () => {
    const { out, result } = await run("router-fts-window", [
      advance(32),
      // 1. Open workspace folder so FTS5 indexes workspace notes and bookmark is persisted (#60, #61)
      click("btn-open-folder"),
      advance(32),
      click("tree-item-ws:welcome.md"),
      advance(24),

      // 2. Dirty doc -> attempt to leave to /search -> useBeforeLeave blocks and shows guard banner (#57)
      click("ins-h2"),
      advance(24),
      check("save-state-badge", "text", "Unsaved *"),
      click("nav-search"),
      advance(24),
      check("route-readout", "text", "/editor"),
      check("nav-guard-banner", "exists", true),

      // 3. Click Save & Leave -> saves atomically (#61), clears dirty flag, routes to /search (#57)
      click("btn-guard-save-leave"),
      advance(32),
      check("route-readout", "text", "/search"),
      check("view-search", "exists", true),

      // 4. SQLite FTS5 full-text search (#60) -> find WaylandPipeline in architecture.md -> open hit
      click("fts-search-input"),
      commitText("WaylandPipeline"),
      advance(32),
      check("fts-hit-0", "exists", true),
      capture("fts-results"),
      click("fts-open-0"),
      advance(32),
      check("route-readout", "text", "/editor"),
      check("window-title-readout", "text", "architecture.md — Markdown Studio"),

      // 5. History back/forward (#57) + Settings route with Select & persisted Bookmarks (#60, #62)
      click("nav-back"),
      advance(24),
      check("route-readout", "text", "/search"),
      click("nav-forward"),
      advance(24),
      check("route-readout", "text", "/editor"),
      click("nav-settings"),
      advance(32),
      check("view-settings", "exists", true),
      check("bookmark-row-0", "exists", true),
      click("settings-preset-select"),
      advance(24),
      key("ArrowDown"),
      key("Enter"),
      advance(24),

      // 6. Auxiliary WindowSurface for About & Quick Preferences (#58)
      click("btn-about-window"),
      advance(32),
      check("aux-about-root", "exists", true, "about-prefs"),
      check("aux-theme-readout", "text", "Active Theme: ocean / dark", "about-prefs"),
      click("aux-dark-switch", "about-prefs"),
      advance(24),
      check("aux-theme-readout", "text", "Active Theme: ocean / light", "about-prefs"),
      click("aux-close-btn", "about-prefs"),
      advance(24),

      // 7. Return to editor, make dirty edit, fire surface.closeRequest -> veto opens confirmation Dialog -> Discard & Close (#58)
      click("nav-editor"),
      advance(32),
      click("ins-task"),
      advance(24),
      check("save-state-badge", "text", "Unsaved *"),
      { id: id(), op: "surface.closeRequest", surface: "main" },
      advance(24),
      check("unsaved-close-dialog", "exists", true),
      click("btn-close-discard"),
    ]);

    const tFts = await readTree(out, "fts-results");
    assert.ok(
      tFts.nodes.some(n => typeof n.text === "string" && n.text.includes("architecture.md")),
      "expected FTS5 search results to include architecture.md",
    );

    assert.equal(result.teardown?.clean, true);
  },
);

test(
  "Textarea scrolls vertically on wheel and Preview/Editor modes collapse hidden SplitPane panels (#77)",
  { timeout: 60000 },
  async () => {
    const { out, result } = await run("textarea-scroll-and-panes", [
      advance(32),
      check("md-source-editor", "exists", true),
      // Open workspace welcome.md (50+ lines) so Textarea overflows vertically in the 600px window.
      click("btn-open-folder"),
      advance(32),
      click("tree-item-ws:welcome.md"),
      advance(48),
      capture("before-wheel"),
      {
        id: id(),
        op: "wheel",
        target: { surface: "main", testId: "md-source-editor" },
        delta: { x: 0, y: 240, unit: "pixels" },
      },
      advance(200),
      capture("after-wheel"),

      // Switch to Preview-only mode -> editor panel collapses to 0 width, preview expands.
      click("mode-preview"),
      advance(32),
      capture("mode-preview"),

      // Switch to Editor-only mode -> preview panel collapses to 0 width, editor expands.
      click("mode-editor"),
      advance(32),
      capture("mode-editor"),

      // Switch back to Split mode -> both panels visible again.
      click("mode-split"),
      advance(32),
      capture("mode-split"),
    ]);

    const tBefore = await readTree(out, "before-wheel");
    const tAfter = await readTree(out, "after-wheel");
    const beforeMetrics = getNode(tBefore, "md-source-editor").scrollMetrics;
    const afterMetrics = getNode(tAfter, "md-source-editor").scrollMetrics;
    assert.ok(beforeMetrics, "Textarea must be a vertical scroll container with scrollMetrics");
    assert.ok(afterMetrics, "Textarea must report scrollMetrics after wheel");
    assert.ok(
      afterMetrics.offsetY > beforeMetrics.offsetY,
      `expected Textarea wheel to increase offsetY: before=${beforeMetrics.offsetY}, after=${afterMetrics.offsetY}`,
    );

    const tSplit = await readTree(out, "mode-split");
    const tPreview = await readTree(out, "mode-preview");
    const tEditor = await readTree(out, "mode-editor");

    const splitEditorW = getNode(tSplit, "studio-editor-panel").boundsLogical.width;
    const splitPreviewW = getNode(tSplit, "studio-preview-panel").boundsLogical.width;
    assert.ok(splitEditorW > 200 && splitPreviewW > 200, "split mode shows both panes");

    const previewModeEditorW = getNode(tPreview, "studio-editor-panel").boundsLogical.width;
    const previewModePreviewW = getNode(tPreview, "studio-preview-panel").boundsLogical.width;
    assert.ok(
      previewModeEditorW <= 1,
      `preview mode must collapse editor panel to 0px, got ${previewModeEditorW}`,
    );
    assert.ok(
      previewModePreviewW > splitPreviewW + 150,
      `preview mode must expand preview panel (${previewModePreviewW} vs split ${splitPreviewW})`,
    );

    const editorModeEditorW = getNode(tEditor, "studio-editor-panel").boundsLogical.width;
    const editorModePreviewW = getNode(tEditor, "studio-preview-panel").boundsLogical.width;
    assert.ok(
      editorModePreviewW <= 1,
      `editor mode must collapse preview panel to 0px, got ${editorModePreviewW}`,
    );
    assert.ok(
      editorModeEditorW > splitEditorW + 150,
      `editor mode must expand editor panel (${editorModeEditorW} vs split ${splitEditorW})`,
    );

    assert.equal(result.teardown?.clean, true);
  },
);
