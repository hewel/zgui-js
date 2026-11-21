import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  readTree,
  runScenario,
} from "./harness.mjs";

const source = `
import { VirtualGrid, VirtualList, mountStylesheet, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

const rows = Array.from({ length: 1000 }, (_, i) => ({ id: i, title: "Row " + i }));
const tiles = Array.from({ length: 800 }, (_, i) => ({ id: i, title: "Tile " + i }));

export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet("reset", ":root { padding: 0; background: #10141c; color: #f8fafc; }");
    return (
      <row style={{ width: "640px", height: "240px", gap: "12px", padding: "8px" }}>
        <VirtualList
          testId="vlist"
          each={rows}
          itemHeight={32}
          gap={4}
          overscan={2}
          initialHeight={224}
          style={{ width: "260px", height: "224px" }}
        >
          {(item, index) => (
            <box testId={"row-" + index()} style={{ height: "32px", flexShrink: 0, backgroundColor: "#1e293b", padding: "4px" }}>
              <text>{item.title}</text>
            </box>
          )}
        </VirtualList>
        <VirtualGrid
          testId="vgrid"
          each={tiles}
          itemHeight={48}
          columns={4}
          gap={6}
          overscan={1}
          initialWidth={340}
          initialHeight={224}
          style={{ width: "340px", height: "224px" }}
        >
          {(tile, index) => (
            <box testId={"tile-" + index()} style={{ flex: 1, height: "48px", backgroundColor: "#0f766e", padding: "4px" }}>
              <text>{tile.title}</text>
            </box>
          )}
        </VirtualGrid>
      </row>
    );
  }, root);
}
`;

test("VirtualList (1,000 rows) and VirtualGrid (800 tiles) mount within default QuickJS turn budget and scroll cleanly", async t => {
  const temp = await createTempWorkspace("preview-virtual-list-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "virtual-list", source, {
    rawSource: true,
  });

  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "virtual-list",
    requiredCapabilities: [],
    width: 640,
    height: 240,
    scale: 1,
    steps: [
      { id: "settle-initial", op: "clock.advance", byMs: 32 },
      { id: "cap-initial", op: "capture", captureId: "initial" },
      {
        id: "wheel-list",
        op: "wheel",
        target: { surface: "main", testId: "vlist" },
        delta: { x: 0, y: 1800, unit: "pixels" },
      },
      { id: "settle-scrolled", op: "clock.advance", byMs: 500 },
      { id: "cap-scrolled", op: "capture", captureId: "scrolled" },
    ],
  });

  const findNode = (tree, testId) => tree.nodes.find(entry => entry.testId === testId);
  const initial = await readTree(out, "initial");
  assert.ok(getNode(initial, "row-0"), "initial row-0 must be mounted");
  assert.equal(findNode(initial, "row-500"), undefined, "offscreen row-500 must not be mounted");
  assert.ok(getNode(initial, "tile-0"), "initial tile-0 must be mounted");
  assert.equal(findNode(initial, "tile-799"), undefined, "offscreen tile-799 must not be mounted");

  const vlistInitial = getNode(initial, "vlist");
  assert.ok(vlistInitial?.scrollMetrics, "VirtualList should expose scrollMetrics");
  // 1000 rows * 32px + 999 * 4px = 35996px
  assert.ok(
    Math.abs(vlistInitial.scrollMetrics.contentHeight - 35996) <= 4,
    `expected ~35996px virtual list contentHeight, got ${vlistInitial.scrollMetrics.contentHeight}`,
  );

  const vgridInitial = getNode(initial, "vgrid");
  assert.ok(vgridInitial?.scrollMetrics, "VirtualGrid should expose scrollMetrics");
  // 200 rows * 48px + 199 * 6px = 10794px
  assert.ok(
    Math.abs(vgridInitial.scrollMetrics.contentHeight - 10794) <= 4,
    `expected ~10794px virtual grid contentHeight, got ${vgridInitial.scrollMetrics.contentHeight}`,
  );

  const scrolled = await readTree(out, "scrolled");
  const vlistScrolled = getNode(scrolled, "vlist");
  assert.ok(
    vlistScrolled.scrollMetrics.offsetY > 1000,
    `expected wheel to scroll vlist past y=1000, got ${vlistScrolled.scrollMetrics.offsetY}`,
  );
  assert.equal(findNode(scrolled, "row-0"), undefined, "row-0 must unmount after scrolling down 1800px");
  assert.ok(getNode(scrolled, "row-50"), "row-50 must mount around y=1800 (stride=36px)");
});
