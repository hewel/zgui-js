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
  assert.equal(
    findNode(scrolled, "row-0"),
    undefined,
    "row-0 must unmount after scrolling down 1800px",
  );
  assert.ok(getNode(scrolled, "row-50"), "row-50 must mount around y=1800 (stride=36px)");
});

const imperativeScrollSource = `
import { Button, VirtualList, createSignal, mountStylesheet, observeGeometry, renderApp, scrollBy, scrollIntoView, scrollTo } from "@hewel/zgui";
import type { NativeNode, VirtualListController } from "@hewel/zgui";

export const hostApiVersion = 1;

const rows = Array.from({ length: 400 }, (_, i) => ({ id: i, title: "Item " + i }));

export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet("reset", ":root { padding: 0; background: #10141c; color: #f8fafc; }");
    const [docScroll, setDocScroll] = createSignal<NativeNode | null>(null);
    const [targetHeading, setTargetHeading] = createSignal<NativeNode | null>(null);
    const [docY, setDocY] = createSignal(0);
    let vlistCtrl: VirtualListController | null = null;
    observeGeometry(docScroll, "scrollPosition", pos => setDocY(Math.round(pos.offset.y)));

    return (
      <column style={{ width: "640px", height: "300px", padding: "8px", gap: "8px" }}>
        <row style={{ gap: "6px", height: "32px", flexShrink: 0 }}>
          <Button testId="btn-scroll-to" onClick={() => scrollTo(docScroll, { top: 420, behavior: "instant" })}>To 420</Button>
          <Button testId="btn-scroll-by" onClick={() => scrollBy(docScroll, { top: 80, behavior: "instant" })}>By +80</Button>
          <Button testId="btn-into-view" onClick={() => scrollIntoView(targetHeading, { block: "start", behavior: "instant" })}>Heading</Button>
          <Button testId="btn-jump-list" onClick={() => vlistCtrl?.scrollToIndex(120, { align: "start", behavior: "instant" })}>Jump 120</Button>
          <text testId="doc-y">{"y=" + docY()}</text>
        </row>
        <row style={{ flex: 1, gap: "12px" }}>
          <scroll testId="doc-scroll" ref={setDocScroll} style={{ width: "300px", height: "240px" }}>
            <column style={{ width: "100%", gap: "0px" }}>
              <box style={{ height: "900px", flexShrink: 0, backgroundColor: "#1e293b" }}><text>Top spacer</text></box>
              <box testId="target-heading" ref={setTargetHeading} style={{ height: "40px", flexShrink: 0, backgroundColor: "#0f766e" }}><text>Target Heading</text></box>
              <box style={{ height: "600px", flexShrink: 0, backgroundColor: "#1e293b" }}><text>Bottom spacer</text></box>
            </column>
          </scroll>
          <VirtualList
            testId="jump-vlist"
            each={rows}
            itemHeight={32}
            gap={4}
            overscan={2}
            initialHeight={240}
            controller={ctrl => { vlistCtrl = ctrl; }}
            style={{ width: "300px", height: "240px" }}
          >
            {(item, index) => (
              <box testId={"jump-row-" + index()} style={{ height: "32px", flexShrink: 0, backgroundColor: "#334155", padding: "4px" }}>
                <text>{item.title}</text>
              </box>
            )}
          </VirtualList>
        </row>
      </column>
    );
  }, root);
}
`;

test("scrollTo, scrollBy, scrollIntoView, and VirtualList.scrollToIndex update real scrollPosition and rendered tree (#55)", async t => {
  const temp = await createTempWorkspace("preview-imperative-scroll-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "imperative-scroll",
    imperativeScrollSource,
    {
      rawSource: true,
    },
  );

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "imperative-scroll",
    requiredCapabilities: [],
    width: 640,
    height: 300,
    scale: 1,
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      {
        id: "click-to",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "btn-scroll-to" },
      },
      { id: "settle-to", op: "clock.advance", byMs: 32 },
      { id: "cap-to", op: "capture", captureId: "after-to" },
      {
        id: "click-by",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "btn-scroll-by" },
      },
      { id: "settle-by", op: "clock.advance", byMs: 32 },
      { id: "cap-by", op: "capture", captureId: "after-by" },
      {
        id: "click-into",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "btn-into-view" },
      },
      {
        id: "click-jump",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "btn-jump-list" },
      },
      { id: "settle-final", op: "clock.advance", byMs: 32 },
      { id: "cap-final", op: "capture", captureId: "final" },
    ],
  });
  assert.equal(result.teardown?.clean, true);

  const afterTo = await readTree(out, "after-to");
  assert.ok(Math.abs(getNode(afterTo, "doc-scroll").scrollMetrics.offsetY - 420) <= 2);

  const afterBy = await readTree(out, "after-by");
  assert.ok(Math.abs(getNode(afterBy, "doc-scroll").scrollMetrics.offsetY - 500) <= 2);

  const finalTree = await readTree(out, "final");
  assert.ok(
    Math.abs(getNode(finalTree, "doc-scroll").scrollMetrics.offsetY - 900) <= 4,
    `scrollIntoView(start) should align heading at y=900, got ${getNode(finalTree, "doc-scroll").scrollMetrics.offsetY}`,
  );
  // VirtualList row 120 with stride 36 => y = 4320.
  assert.ok(
    Math.abs(getNode(finalTree, "jump-vlist").scrollMetrics.offsetY - 4320) <= 4,
    `VirtualList.scrollToIndex(120) should scroll to y=4320, got ${getNode(finalTree, "jump-vlist").scrollMetrics.offsetY}`,
  );
  assert.ok(getNode(finalTree, "jump-row-120"), "jump-row-120 must be mounted after scrollToIndex");
  assert.equal(
    finalTree.nodes.find(entry => entry.testId === "jump-row-0"),
    undefined,
  );
});

test("measured rows align on jump and keep the viewport steady as upward overscan grows", async t => {
  const temp = await createTempWorkspace("preview-measured-list-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "measured-list",
    `
    import { VirtualList, createSignal, renderApp, scrollBy } from "@hewel/zgui";
    import type { NativeNode, VirtualListController } from "@hewel/zgui";
    const rows = Array.from({ length: 2000 }, (_, i) => i);
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      let handle: VirtualListController;
      let scroller: NativeNode;
      const [rowHeight, setRowHeight] = createSignal(60);
      return renderApp(() => <column>
        <row>
          <control role="button" testId="jump" onClick={() => handle.scrollToIndex(1000, { align: "center" })}>Jump</control>
          <control role="button" testId="up" onClick={() => scrollBy(scroller, { top: -34 })}>Scroll up</control>
          <control role="button" testId="smooth" onClick={() => handle.scrollToIndex(1500, { align: "center", behavior: "smooth" })}>Glide</control>
          <control role="button" testId="revisit" onClick={() => {
            setRowHeight(96);
            handle.scrollToIndex(1000, { align: "center" });
          }}>Revisit</control>
        </row>
        <VirtualList ref={node => { scroller = node; }} testId="measured" each={rows} estimatedItemHeight={30} gap={4} overscan={1}
          initialHeight={160} style={{ height: "160px" }} controller={value => { handle = value; }}>
          {row => <box testId={"row-" + row}
            style={{ height: row === 1000 ? rowHeight() + "px" : row === 996 || (row > 1005 && row % 4 === 0) ? "60px" : row === 1001 ? "20px" : "30px" }}>
            <text>Row {row}</text>
          </box>}
        </VirtualList>
      </column>, root);
    }
    `,
    { rawSource: true },
  );
  const { out } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "measured-list",
    requiredCapabilities: [],
    width: 480,
    height: 260,
    scale: 1,
    steps: [
      {
        id: "jump",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "jump" },
      },
      { id: "settle-jump", op: "clock.advance", byMs: 200 },
      { id: "cap-jump", op: "capture", captureId: "jump" },
      {
        id: "up",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "up" },
      },
      { id: "settle-up", op: "clock.advance", byMs: 200 },
      { id: "cap-up", op: "capture", captureId: "up" },
      {
        id: "smooth",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "smooth" },
      },
      { id: "settle-smooth", op: "clock.advance", byMs: 1500 },
      { id: "cap-smooth", op: "capture", captureId: "smooth" },
      {
        id: "revisit",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "revisit" },
      },
      { id: "settle-revisit", op: "clock.advance", byMs: 200 },
      { id: "cap-revisit", op: "capture", captureId: "revisit" },
    ],
  });
  const jumped = await readTree(out, "jump");
  const up = await readTree(out, "up");
  const viewport = getNode(jumped, "measured").boundsLogical;
  const before = getNode(jumped, "row-1000").boundsLogical;
  const next = getNode(jumped, "row-1001").boundsLogical;
  assert.equal(before.height, 60);
  assert.equal(next.height, 20);
  assert.ok(Math.abs(next.y - before.y - before.height - 4) <= 1, "measured rows do not overlap");
  assert.ok(Math.abs(before.y + 30 - (viewport.y + 80)) <= 1, "target centers after measurement");
  assert.ok(!jumped.nodes.some(node => node.testId === "row-996"));
  assert.ok(getNode(up, "row-996"));
  assert.ok(!up.nodes.some(node => node.testId === "row-0"));
  const after = getNode(up, "row-1000").boundsLogical;
  assert.ok(
    Math.abs(after.y - before.y - 34) <= 2,
    `expected 34px anchor movement, got ${after.y - before.y}`,
  );
  const smooth = await readTree(out, "smooth");
  const smoothTarget = getNode(smooth, "row-1500").boundsLogical;
  assert.ok(
    Math.abs(smoothTarget.y + smoothTarget.height / 2 - (viewport.y + 80)) <= 1,
    "measurements along a glide do not cancel or misalign the destination",
  );
  const revisited = getNode(await readTree(out, "revisit"), "row-1000").boundsLogical;
  assert.equal(revisited.height, 96);
  assert.ok(
    Math.abs(revisited.y + 48 - (viewport.y + 80)) <= 1,
    "a previously measured row is re-aligned if its height changed offscreen",
  );
});
