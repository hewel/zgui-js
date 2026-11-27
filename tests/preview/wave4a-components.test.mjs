import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  functionalProfile,
  getNode,
  pixelAt,
  readPng,
  readTree,
  runScenario,
} from "./harness.mjs";

async function preview(t, name, source, steps) {
  const temp = await createTempWorkspace(`preview-${name}-`);
  t.after(() => temp.cleanup());
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), name, source, {
    rawSource: true,
  });
  return runScenario({
    projectDir: temp.dir,
    bundle,
    name,
    profile: functionalProfile,
    requiredCapabilities: [],
    forbidJsDiagnostics: true,
    width: 640,
    height: 480,
    scale: 1,
    steps: steps.map((step, index) => ({ id: `step-${index}`, ...step })),
  });
}

const main = { surface: "main" };
const target = testId => ({ ...main, testId });
const click = testId => ({ op: "pointer.click", target: target(testId), button: "primary" });
const key = key => ({ op: "key.press", ...main, key });
const advance = (byMs = 400) => ({ op: "clock.advance", byMs });
const capture = captureId => ({ op: "capture", captureId });
const check = (testId, property, equals) => ({
  op: "assert",
  target: target(testId),
  check: { property, equals },
});

test("AlertDialog blocks backdrop presses and restores focus after Escape, Cancel, and Action", async t => {
  const { out } = await preview(
    t,
    "alert-confirmation",
    `
    import { createSignal, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { AlertDialog, AlertDialogTrigger, AlertDialogContent, AlertDialogHeader,
      AlertDialogMedia, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter,
      AlertDialogCancel, AlertDialogAction, ThemeProvider } from "@hewel/zgui/components";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      const [confirmed, setConfirmed] = createSignal(false);
      return renderApp(() => <ThemeProvider><column style={{ padding: "16px", alignItems: "start" }}>
        <text testId="result">{confirmed() ? "Deleted" : "Kept"}</text>
        <AlertDialog>
          <AlertDialogTrigger testId="launch">Delete item</AlertDialogTrigger>
          <AlertDialogContent testId="confirmation" size="sm">
            <AlertDialogHeader>
              <AlertDialogMedia>!</AlertDialogMedia>
              <AlertDialogTitle>Delete item?</AlertDialogTitle>
              <AlertDialogDescription>This cannot be undone.</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel testId="cancel">Cancel</AlertDialogCancel>
              <AlertDialogAction testId="confirm" onClick={() => setConfirmed(true)}>Delete</AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </column></ThemeProvider>, root);
    }
  `,
    [
      click("launch"),
      advance(),
      { op: "pointer.click", ...main, button: "primary", position: { x: 10, y: 10 } },
      advance(),
      check("confirmation", "exists", true),
      check("cancel", "focused", true),
      capture("open"),
      key("Escape"),
      advance(),
      check("confirmation", "exists", false),
      check("launch", "focused", true),
      key("Enter"),
      advance(),
      click("cancel"),
      advance(),
      check("confirmation", "exists", false),
      check("launch", "focused", true),
      check("result", "text", "Kept"),
      key("Enter"),
      advance(),
      click("confirm"),
      advance(),
      check("confirmation", "exists", false),
      check("launch", "focused", true),
      check("result", "text", "Deleted"),
    ],
  );
  const open = await readTree(out, "open");
  const dialog = getNode(open, "confirmation");
  assert.equal(dialog.role, "alertDialog");
  assert.equal(dialog.name, "Delete item?");
  assert.ok(dialog.boundsLogical.x > 10, "the click point was on the backdrop");
  assert.ok(dialog.boundsLogical.width <= 321, "small alert uses its stylesheet width");
});

test("Table aligns its native columns; virtual DataTable sorts, resizes and keeps its header and keyboard focus", async t => {
  const { out } = await preview(
    t,
    "table-workbench",
    `
    import { createSignal, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { DataTable, Table, TableBody, TableCell, TableFooter, TableHead, TableHeader, TableRow, ThemeProvider } from "@hewel/zgui/components";
    import type { DataTableColumn } from "@hewel/zgui/components";
    export const hostApiVersion = 1;
    const rows = Array.from({ length: 1000 }, (_, i) => ({ id: 999 - i, rank: ((999 - i) * 7) % 1000 }));
    const columns: readonly DataTableColumn<(typeof rows)[number]>[] = [
      { id: "rank", header: () => <text testId="sort-rank">Rank</text>, sortable: true, resizable: true,
        width: 112, minWidth: 80, maxWidth: 172, sortAccessor: row => row.rank,
        cell: (row, index) => <text testId={"rank-" + index()}>{row.rank}</text> },
      { id: "id", header: () => <text testId="id-header">ID</text>, width: 140,
        cell: row => <text testId={"row-" + row.id}>{row.id}</text> },
    ];
    export function mount(root: NativeNode) {
      const [activated, setActivated] = createSignal("None");
      return renderApp(() => <ThemeProvider><column style={{ gap: "12px" }}>
        <Table testId="static-table" columns="84px 1fr" stickyHeader style="--zui-color-muted: #ff0000">
          <TableHeader><TableRow><TableHead>Unit</TableHead><TableHead testId="static-head">State</TableHead></TableRow></TableHeader>
          <TableBody><TableRow><TableCell>Network</TableCell><TableCell testId="static-cell">Ready</TableCell></TableRow></TableBody>
          <TableFooter><TableRow>
            <TableCell testId="footer-cell">Total</TableCell>
            <TableCell><Table style="width: 160px">
              <TableBody><TableRow><TableCell testId="nested-cell">
                <box testId="nested-marker" style="width: 8px; height: 8px; background-color: #0000ff" />
              </TableCell></TableRow></TableBody>
            </Table></TableCell>
          </TableRow></TableFooter>
        </Table>
        <DataTable testId="table-grid" data={rows} columns={columns} height={168} itemHeight={28}
          toolbar={<text testId="activated">{activated()}</text>}
          onRowActivate={(row, index) => setActivated(row.id + " at " + index)} />
      </column></ThemeProvider>, root);
    }
  `,
    [
      advance(32),
      check("rank-0", "text", "993"),
      capture("initial"),
      click("sort-rank"),
      check("rank-0", "text", "0"),
      key("Space"),
      check("rank-0", "text", "999"),
      key("Enter"),
      check("rank-0", "text", "993"),
      {
        op: "pointer.down",
        target: { ...main, role: "splitter", name: "Resize rank column" },
        button: "primary",
      },
      { op: "pointer.move", ...main, position: { x: 330, y: 430 } },
      { op: "pointer.up", ...main, position: { x: 330, y: 430 }, button: "primary" },
      advance(32),
      capture("resized"),
      key("ArrowLeft"),
      advance(32),
      capture("nudged"),
      { op: "wheel", target: target("row-997"), delta: { x: 0, y: 850, unit: "pixels" } },
      advance(),
      capture("scrolled"),
      { ...key("Tab"), modifiers: ["Shift"] },
      { ...key("Tab"), modifiers: ["Shift"] },
      check("table-grid", "focused", true),
      key("End"),
      advance(32),
      key("Enter"),
      check("activated", "text", "0 at 999"),
      capture("last"),
    ],
  );
  const initial = await readTree(out, "initial");
  const resized = await readTree(out, "resized");
  const nudged = await readTree(out, "nudged");
  const scrolled = await readTree(out, "scrolled");
  const last = await readTree(out, "last");
  const parent = (tree, id) => tree.nodes.find(node => node.token === getNode(tree, id).parent);
  const width = (tree, id) => parent(tree, id).boundsLogical.width;
  assert.equal(getNode(initial, "static-table").role, "table");
  assert.equal(
    getNode(initial, "static-head").boundsLogical.x,
    getNode(initial, "static-cell").boundsLogical.x,
  );
  const png = await readPng(path.join(out, "captures", "initial.png"));
  const sample = (id, inset = 3) => {
    const { x, y, width, height } = getNode(initial, id).boundsDevice;
    return pixelAt(png, Math.floor(x + width - inset), Math.floor(y + height / 2));
  };
  const footer = sample("footer-cell");
  const [footerRed, footerGreen] = footer;
  const [bodyRed, bodyGreen] = sample("static-cell");
  assert.ok(
    footerRed - footerGreen > bodyRed - bodyGreen + 40,
    "the footer cell paints the muted tint",
  );
  assert.deepEqual(sample("nested-marker"), [0, 0, 255, 255], "the inner table is visible");
  assert.deepEqual(sample("nested-cell", 30), footer, "the inner cell adds no footer tint");
  assert.equal(width(initial, "rank-0"), 112);
  assert.equal(width(resized, "rank-0"), 172);
  assert.equal(width(nudged, "rank-0"), 164);
  for (const tree of [initial, resized, nudged]) {
    assert.equal(
      parent(tree, "id-header").boundsLogical.x,
      parent(tree, "row-999").boundsLogical.x,
    );
  }
  assert.deepEqual(
    getNode(scrolled, "id-header").boundsLogical,
    getNode(nudged, "id-header").boundsLogical,
  );
  assert.ok(!scrolled.nodes.some(node => node.testId === "rank-0"));
  assert.ok(last.nodes.filter(node => node.testId?.startsWith("rank-")).length < 20);
  assert.ok(getNode(last, "row-0").boundsLogical.height > 0);
});

test("DataTable reveals the last measured row without replacing grid focus", async t => {
  const { out } = await preview(
    t,
    "measured-table",
    `
    import { renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { DataTable, ThemeProvider } from "@hewel/zgui/components";
    import type { DataTableColumn } from "@hewel/zgui/components";
    export const hostApiVersion = 1;
    const rows = Array.from({ length: 80 }, (_, row) => row);
    const columns: DataTableColumn<number>[] = [{
      id: "message", header: "Message", width: 200,
      cell: row => <box testId={"message-" + row} style={{ height: row % 2 ? "48px" : "24px" }}>
        <text>Log {row}</text>
      </box>,
    }];
    export function mount(root: NativeNode) {
      return renderApp(() => <ThemeProvider>
        <DataTable testId="measured-grid" data={rows} columns={columns} height={160} estimatedItemHeight={40} />
      </ThemeProvider>, root);
    }
  `,
    [
      advance(200),
      capture("initial"),
      key("Tab"),
      check("measured-grid", "focused", true),
      key("End"),
      advance(200),
      check("measured-grid", "focused", true),
      capture("last"),
    ],
  );
  const initial = await readTree(out, "initial");
  const last = await readTree(out, "last");
  const short = getNode(initial, "message-0").boundsLogical;
  const tall = getNode(initial, "message-1").boundsLogical;
  assert.equal(tall.height - short.height, 24);
  assert.ok(tall.y >= short.y + short.height);
  const grid = getNode(last, "measured-grid").boundsLogical;
  const targetRow = getNode(last, "message-79").boundsLogical;
  assert.ok(targetRow.y >= grid.y + 40);
  assert.ok(targetRow.y + targetRow.height <= grid.y + grid.height);
  assert.ok(!last.nodes.some(node => node.testId === "message-0"));
});
