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
import { createSignal, focus, renderApp, scrollTo, VirtualGrid } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Breadcrumb, BreadcrumbList, BreadcrumbItem, BreadcrumbLink, BreadcrumbPage,
  BreadcrumbSeparator, BreadcrumbEllipsis, ThemeProvider } from "@hewel/zgui/components";
import { createMarqueeSelection, createSelectionModel } from "@hewel/zgui/primitives";

const items = Array.from({ length: 36 }, (_, i) => i);
function App() {
  const [grid, setGrid] = createSignal<NativeNode | null>(null);
  const [page, setPage] = createSignal("Photos");
  const selection = createSelectionModel({ items: () => items });
  const marquee = createMarqueeSelection({
    container: grid,
    selection,
    resolveIntersectingKeys: ({x, y, width, height}) => {
      const keys = [];
      for (let row = Math.floor(y / 30); row < Math.min(12, Math.ceil((y + height) / 30)); row++) {
        for (let col = Math.floor(x / 80); col < Math.min(3, Math.ceil((x + width) / 80)); col++) {
          keys.push(row * 3 + col);
        }
      }
      return keys;
    },
  });
  return <ThemeProvider><column>
    <Breadcrumb testId="crumbs"><BreadcrumbList testId="crumb-list">
      <BreadcrumbItem><BreadcrumbLink testId="home" onClick={() => setPage("Home")}>Home</BreadcrumbLink></BreadcrumbItem>
      <BreadcrumbSeparator />
      <BreadcrumbItem><BreadcrumbEllipsis /></BreadcrumbItem>
      <BreadcrumbSeparator />
      <BreadcrumbItem><BreadcrumbPage testId="page">{page()}</BreadcrumbPage></BreadcrumbItem>
    </BreadcrumbList></Breadcrumb>
    <control tabIndex={0} testId="prepare" onClick={() => {
      const node = grid();
      if (!node) return;
      scrollTo(node, { top: 90, behavior: "instant" });
      focus(node);
      selection.selectSingle(0);
    }}>Prepare</control>
    <VirtualGrid ref={setGrid} testId="grid" each={items} columns={3} itemHeight={30}
      keyed={item => item}
      initialWidth={252} initialHeight={100} overscan={0} tabIndex={0}
      style={{ width: "252px", height: "100px" }} onKeyDown={selection.handleKeyDown}
      onPointerDown={marquee.onPointerDown} onPointerMove={marquee.onPointerMove}
      onPointerUp={marquee.onPointerUp} onPointerCancel={marquee.onPointerCancel}>
      {item => <box testId={"tile-" + item()} selected={selection.isSelected(item())}
        style={{ width: "80px", height: "30px" }}><text>{"Tile " + item()}</text></box>}
    </VirtualGrid>
    <text testId="keys">{[...selection.selectedKeys()].sort((a, b) => a - b).join(",") || "none"}</text>
    <text testId="drag">{marquee.active() ? "dragging" : "idle"}</text>
  </column></ThemeProvider>;
}
export const hostApiVersion = 1;
export const mount = (root: NativeNode) => renderApp(() => <App />, root);
`;

test("breadcrumb activates from the keyboard and marquee extends into scrolled virtual rows", async t => {
  const temp = await createTempWorkspace("wave4c-ui-");
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "wave4c-ui", source, {
    rawSource: true,
  });
  const target = testId => ({ surface: "main", testId });
  const key = (id, key, modifiers) => ({ id, key, modifiers, op: "key.press", surface: "main" });
  const pointer = (id, op, testId) => ({
    id,
    op,
    target: target(testId),
    ...(op === "pointer.move" ? {} : { button: "primary" }),
  });
  const check = (id, testId, equals) => ({
    id,
    op: "assert",
    target: target(testId),
    check: { property: "text", equals },
  });
  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "wave4c-selection",
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "tree.inspect"],
    steps: [
      key("focus-home", "Tab"),
      key("activate-home", "Space"),
      check("navigated", "page", "Home"),
      pointer("prepare", "pointer.click", "prepare"),
      key("range", "ArrowRight", ["Shift"]),
      check("range-result", "keys", "0,1"),
      key("all", { character: "a" }, ["Control"]),
      check("all-result", "keys", itemsText()),
      key("clear", "Escape"),
      check("clear-result", "keys", "none"),
      pointer("drag-start", "pointer.down", "tile-9"),
      pointer("drag-move", "pointer.move", "tile-16"),
      check("drag-active", "drag", "dragging"),
      check("scrolled-result", "keys", "9,10,12,13,15,16"),
      pointer("edge", "pointer.move", "tile-19"),
      { id: "scroll-time", op: "clock.advance", byMs: 80 },
      {
        id: "edge-reached",
        op: "assert",
        target: target("tile-22"),
        check: { property: "exists", equals: true },
      },
      pointer("drag-release", "pointer.up", "tile-22"),
      check("drag-inactive", "drag", "idle"),
      { id: "final", op: "capture", captureId: "final" },
    ],
  });
  const tree = await readTree(out, "final");
  assert.equal(getNode(tree, "crumbs").role, "navigation");
  assert.equal(getNode(tree, "crumbs").name, "Breadcrumb");
  assert.equal(getNode(tree, "crumb-list").role, "list");
  assert.equal(getNode(tree, "page").role, "label");
  const current = tree.semanticProjection.nodes.filter(node => node.ariaCurrent === "page");
  assert.deepEqual(
    current.map(node => node.role),
    ["label"],
  );
  assert.equal(result.teardown.clean, true);
});

function itemsText() {
  return Array.from({ length: 36 }, (_, i) => i).join(",");
}
