import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

function countSubtree(node) {
  let count = 1;
  for (const child of node.children) count += countSubtree(child);
  return count;
}

test("VirtualList mounts only the visible slice for 10,000 items and slides cleanly as scrollPosition changes", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(t, `
    import { VirtualList, createSignal, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";

    const [items, setItems] = createSignal(Array.from({ length: 10000 }, (_, i) => ({ id: i, label: "Row " + i })));
    export { setItems };

    export function mount(root: NativeNode) {
      return renderApp(() => (
        <VirtualList
          testId="vlist"
          each={items()}
          itemHeight={40}
          gap={0}
          overscan={2}
          initialHeight={200}
          keyed={item => item.id}
          fallback={<text testId="empty">No items</text>}
        >
          {(item, index) => <text testId={"row-" + index()}>{item().label}</text>}
        </VirtualList>
      ), root);
    }
  `);
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try { dispose?.(); }
    finally { delete globalThis.__zgui; }
  });

  const root = h.makeNode("root");
  dispose = h.runTurn(() => mod.mount(root));
  // 200px viewport / 40px row = 5 visible rows + 1 + 2*overscan(2) = 10 rows mounted out of 10,000!
  assert.ok(h.byTestId("row-0"), "row 0 should be mounted");
  assert.ok(h.byTestId("row-5"), "row 5 should be mounted");
  assert.equal(h.byTestId("row-500"), undefined, "far offscreen row 500 must NOT be mounted");
  assert.ok(countSubtree(root) < 40, `expected <40 attached nodes for 10,000 items, got ${countSubtree(root)}`);

  // Simulate scrolling down to y = 4000 (row 100).
  const scroller = h.byTestId("vlist");
  const observer = h.geometry.find(record => record.active && record.node === scroller && record.kind === "scrollPosition");
  assert.ok(observer, "VirtualList must subscribe to scrollPosition geometry");
  h.runTurn(() => observer.callback({
    offset: { x: 0, y: 4000 },
    contentSize: { width: 400, height: 400000 },
    scrollport: { width: 400, height: 200 },
  }));
  assert.equal(h.byTestId("row-0"), undefined, "row 0 unmounts after scrolling to row 100");
  assert.ok(h.byTestId("row-100"), "row 100 mounts after scrolling to y=4000");
  assert.ok(h.byTestId("row-104"), "row 104 mounts in visible window");
  assert.ok(countSubtree(root) < 40, `attached node count stays bounded after scrolling: ${countSubtree(root)}`);

  // Updating an item with the same key reactively updates its mounted text without losing scroll window.
  h.runTurn(() => mod.setItems(prev => prev.map((it, idx) => idx === 100 ? { id: 100, label: "Updated 100" } : it)));
  assert.equal(h.host.getTextContent(h.byTestId("row-100")), "Updated 100");

  // Shrinking the list while scrolled down clamps immediately into the valid slice.
  h.runTurn(() => mod.setItems([{ id: 1, label: "Only 1" }, { id: 2, label: "Only 2" }]));
  assert.ok(h.byTestId("row-0"), "row 0 should appear immediately when list shrinks");
  assert.equal(h.host.getTextContent(h.byTestId("row-0")), "Only 1");

  // Emptying items renders fallback cleanly.
  h.runTurn(() => mod.setItems([]));
  assert.ok(h.byTestId("empty"), "empty list renders fallback");
});

test("VirtualGrid mounts only visible rows of columns for 2,000 tiles and reacts to scrollport resize", async t => {
  const h = createHost({ strictStyles: true });
  const mod = await moduleFor(t, `
    import { VirtualGrid, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";

    const tiles = Array.from({ length: 2000 }, (_, i) => i);

    export function mount(root: NativeNode) {
      return renderApp(() => (
        <VirtualGrid
          testId="vgrid"
          each={tiles}
          itemHeight={50}
          minColumnWidth={100}
          gap={10}
          overscan={1}
          initialWidth={320}
          initialHeight={150}
        >
          {(tile, index) => <box testId={"tile-" + index()}><text>{String(tile)}</text></box>}
        </VirtualGrid>
      ), root);
    }
  `);
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try { dispose?.(); }
    finally { delete globalThis.__zgui; }
  });

  const root = h.makeNode("root");
  dispose = h.runTurn(() => mod.mount(root));
  // 320px width with 100px minWidth + 10px gap => 3 columns; ~6 visible rows => ~18 tiles out of 2,000.
  assert.ok(h.byTestId("tile-0"));
  assert.equal(h.byTestId("tile-1000"), undefined);
  assert.ok(countSubtree(root) < 80, `expected <80 attached nodes for 2,000 tiles, got ${countSubtree(root)}`);

  // Scroll to row 100 (stride = 60px -> y = 6000 -> tile 300).
  const scroller = h.byTestId("vgrid");
  const observer = h.geometry.find(record => record.active && record.node === scroller && record.kind === "scrollPosition");
  assert.ok(observer);
  h.runTurn(() => observer.callback({
    offset: { x: 0, y: 6000 },
    contentSize: { width: 320, height: 40000 },
    scrollport: { width: 320, height: 150 },
  }));
  assert.equal(h.byTestId("tile-0"), undefined);
  assert.ok(h.byTestId("tile-300"), "tile 300 mounts when scrolled to row 100");
  assert.ok(countSubtree(root) < 80, `attached node count stays bounded after scrolling: ${countSubtree(root)}`);
});
