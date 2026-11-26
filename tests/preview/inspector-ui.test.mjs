import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, resolveApplication } from "../../packages/build/src/index.ts";
import { workspaceRoot } from "../../packages/build/src/build.ts";
import { createTempWorkspace, getNode, readTree, runScenario, writeJson } from "./harness.mjs";

async function runInspectorUiCase({ development }) {
  const work = await createTempWorkspace(development ? "inspector-ui-dev-" : "inspector-ui-");
  try {
    const sockPath = "/zgui-preview-only/dev-inspect.sock";
    const initialSnapshot = {
      readOnly: true,
      picker: {
        active: false,
        hovered: null,
        selected: { surface: "main", token: "node-2", source: "app/src/main.tsx:42:7" },
      },
      session: {
        generation: 3,
        resources: { nodes: 14, callbacks: 4, timers: 0, unixSockets: 0 },
      },
      profiler: {
        frameCount: 1,
        frames: [{ ordinal: 7, styleUs: 120, layoutUs: 240, paintUs: 410, totalUs: 770 }],
      },
      surfaces: [
        {
          surface: "main",
          coherent: true,
          coherenceStatus: "coherent",
          excluded: false,
          frame: {
            frameOrdinal: 7,
            phases: { styleUs: 120, layoutUs: 240, paintUs: 410, totalUs: 770 },
          },
          nodes: [
            {
              token: "node-1",
              parent: null,
              kind: "box",
              testId: "root-box",
              classes: ["app-root"],
              source: "app/src/main.tsx:10:5",
              role: "Generic",
              redacted: false,
              boxModel: {
                borderBox: { x: 0, y: 0, width: 800, height: 600 },
                contentBox: { x: 16, y: 16, width: 768, height: 568 },
                padding: { top: 16, right: 16, bottom: 16, left: 16 },
                border: { top: 0, right: 0, bottom: 0, left: 0 },
                margin: { top: 0, right: 0, bottom: 0, left: 0 },
              },
              computedStyle: {
                display: "Flex",
                position: "Static",
                visibility: "Visible",
                opacity: 1,
                fontSize: 14,
              },
            },
            {
              token: "node-2",
              parent: "node-1",
              kind: "input",
              testId: "secret-field",
              classes: ["password-input"],
              source: "app/src/main.tsx:42:7",
              role: "TextInput",
              text: "",
              redacted: true,
              boxModel: {
                borderBox: { x: 24, y: 80, width: 260, height: 36 },
                contentBox: { x: 32, y: 86, width: 244, height: 24 },
                padding: { top: 6, right: 8, bottom: 6, left: 8 },
                border: { top: 1, right: 1, bottom: 1, left: 1 },
                margin: { top: 4, right: 0, bottom: 4, left: 0 },
              },
              computedStyle: {
                display: "Block",
                position: "Relative",
                visibility: "Visible",
                opacity: 1,
                fontSize: 13,
              },
            },
          ],
        },
      ],
    };

    const toggleReply = {
      ok: true,
      picker: {
        active: true,
        hovered: null,
        selected: { surface: "main", token: "node-2", source: "app/src/main.tsx:42:7" },
      },
    };

    const fixturePath = path.join(work.dir, "fixture.json");
    await writeJson(fixturePath, {
      schemaVersion: 1,
      id: "inspector-ui-fixture",
      events: [],
      services: {
        sockets: [
          {
            path: sockPath,
            framing: "line",
            send: [JSON.stringify({ op: "snapshot" })],
            events: [{ atMs: 1, kind: "message", text: JSON.stringify(initialSnapshot) }],
          },
          {
            path: sockPath,
            framing: "line",
            send: [JSON.stringify({ op: "pick", action: "toggle" })],
            events: [{ atMs: 1, kind: "message", text: JSON.stringify(toggleReply) }],
          },
        ],
      },
    });

    const inspectorDir = path.join(workspaceRoot, "apps/devtools/inspector");
    const resolved = await resolveApplication(inspectorDir);
    const built = await buildApplication(resolved.app, {
      ...resolved,
      development,
      extraCapabilities: { unixSockets: [sockPath] },
    });

    const viewportHeight = 780;
    const { out } = await runScenario({
      projectDir: work.dir,
      bundle: built.dist,
      fixture: fixturePath,
      name: development ? "inspector-ui-dev" : "inspector-ui",
      width: 1240,
      height: viewportHeight,
      forbidJsDiagnostics: development,
      requiredCapabilities: ["services.fixtures", "pixels", "tree.inspect"],
      steps: [
        { id: "wait-socket", op: "clock.advance", byMs: 20 },
        { id: "capture-loaded", op: "capture", captureId: "loaded" },
        {
          id: "toggle-pick",
          op: "pointer.click",
          target: { surface: "main", testId: "inspector-pick-toggle" },
          button: "primary",
        },
        { id: "wait-pick", op: "clock.advance", byMs: 20 },
        { id: "capture-picked", op: "capture", captureId: "picked" },
      ],
    });

    const loadedTree = await readTree(out, "loaded");
    assert.equal(getNode(loadedTree, "inspector-status").text, "Connected");
    assert.equal(getNode(loadedTree, "inspector-node-count").text, "2 nodes");
    assert.equal(getNode(loadedTree, "inspector-node-source").text, "app/src/main.tsx:42:7");
    assert.match(getNode(loadedTree, "inspector-node-redacted").text, /REDACTED/);
    assert.equal(getNode(loadedTree, "inspector-frame-phases").text, "120us / 240us / 410us");
    assert.equal(getNode(loadedTree, "inspector-frame-total").text, "770us");
    assert.equal(getNode(loadedTree, "inspector-session-stats").text, "gen 3 | 14 nodes");

    const toolbar = getNode(loadedTree, "inspector-toolbar").boundsLogical;
    const workspaceTop = toolbar.y + toolbar.height;
    for (const id of ["inspector-tree-pane", "inspector-details-pane", "inspector-divider"]) {
      const { y, height } = getNode(loadedTree, id).boundsLogical;
      assert.ok(Math.abs(y - workspaceTop) < 1, `${id} starts at ${y}, expected ${workspaceTop}`);
      assert.ok(
        Math.abs(y + height - viewportHeight) < 1,
        `${id} ends at ${y + height}, expected ${viewportHeight}`,
      );
    }

    const pickedTree = await readTree(out, "picked");
    assert.equal(getNode(pickedTree, "inspector-status").text, "Picker active");
  } finally {
    await work.cleanup();
  }
}

test("self-hosted Inspector UI renders native tree, source tag, box model, frame timings, redaction, and pick toggle", async () => {
  await runInspectorUiCase({ development: false });
});

test("self-hosted Inspector UI runs cleanly under development: true without Solid reactive diagnostics", async () => {
  await runInspectorUiCase({ development: true });
});
