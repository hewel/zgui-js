import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";
import { buildApplication } from "../../packages/build/src/build.ts";
import { createHost } from "./native-fixture.mjs";

const root = path.resolve(import.meta.dirname, "../..");

test("development: true bundle mounts ThemeProvider, Tabs, Card, Item, Collapsible, and overlays without Solid dev-mode scope errors", async t => {
  const dir = await fs.mkdtemp(path.join(import.meta.dirname, ".dev-mode-"));
  t.after(async () => { await fs.rm(dir, { recursive: true, force: true }); });
  await fs.mkdir(path.join(dir, "src"), { recursive: true });
  await fs.writeFile(path.join(dir, "package.json"), JSON.stringify({
    name: "dev-mode-smoke",
    private: true,
    zgui: { id: "dev.zgui.dev-mode-smoke", entry: "src/main.tsx" },
  }));
  await fs.writeFile(path.join(dir, "tsconfig.json"), JSON.stringify({
    extends: "../../../packages/ui/tsconfig.native.json",
    include: ["src"],
  }));
  await fs.writeFile(path.join(dir, "src/main.tsx"), `
    import { renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import {
      Button, Card, CardAction, CardContent, CardHeader, CardTitle,
      Collapsible, CollapsibleContent, CollapsibleTrigger,
      ContextMenu, ContextMenuContent, ContextMenuTrigger,
      DropdownMenu, DropdownMenuContent, DropdownMenuTrigger,
      HoverCard, HoverCardContent, HoverCardTrigger,
      Item, ItemContent, ItemDescription, ItemTitle,
      MenuItem,
      Tabs, TabsContent, TabsList, TabsTrigger,
      ThemeProvider,
    } from "@hewel/zgui/components";

    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => (
        <ThemeProvider>
          <Tabs defaultValue="a">
            <TabsList>
              <TabsTrigger value="a">Tab A</TabsTrigger>
              <TabsTrigger value="b">Tab B</TabsTrigger>
            </TabsList>
            <TabsContent value="a">
              <Card>
                <CardHeader>
                  <CardTitle>Title</CardTitle>
                  <CardAction><Button>Act</Button></CardAction>
                </CardHeader>
                <CardContent>
                  <Item>
                    <ItemContent>
                      <ItemTitle>Item</ItemTitle>
                      <ItemDescription>Desc</ItemDescription>
                    </ItemContent>
                  </Item>
                  <Collapsible defaultOpen>
                    <CollapsibleTrigger>Toggle</CollapsibleTrigger>
                    <CollapsibleContent><text>Body</text></CollapsibleContent>
                  </Collapsible>
                  <DropdownMenu>
                    <DropdownMenuTrigger>Menu</DropdownMenuTrigger>
                    <DropdownMenuContent><MenuItem>One</MenuItem></DropdownMenuContent>
                  </DropdownMenu>
                  <HoverCard>
                    <HoverCardTrigger><text>Hover</text></HoverCardTrigger>
                    <HoverCardContent><text>Tip</text></HoverCardContent>
                  </HoverCard>
                  <ContextMenu>
                    <ContextMenuTrigger><text>Area</text></ContextMenuTrigger>
                    <ContextMenuContent><MenuItem>Ctx</MenuItem></ContextMenuContent>
                  </ContextMenu>
                </CardContent>
              </Card>
            </TabsContent>
            <TabsContent value="b"><text>Second</text></TabsContent>
          </Tabs>
        </ThemeProvider>
      ), root);
    }
  `);

  const built = await buildApplication("dev-mode-smoke", {
    root,
    appDirectory: dir,
    outDir: path.join(dir, "dist"),
    development: true,
  });
  const mod = await import(`${pathToFileURL(path.join(built.dist, built.manifest.entry)).href}?t=${Date.now()}`);
  const h = createHost({ strictStyles: true });
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try { dispose?.(); }
    finally { delete globalThis.__zgui; }
  });
  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  h.flush();
  assert.ok(h.nodes.size > 10, "expected full themed component tree to mount under development: true");
});

test("observeGeometry, trapFocus, lockScroll, and registerDismissLayer accept ref signal accessors cleanly in development: true (#51)", async t => {
  const dir = await fs.mkdtemp(path.join(import.meta.dirname, ".dev-mode-ref-"));
  t.after(async () => { await fs.rm(dir, { recursive: true, force: true }); });
  await fs.mkdir(path.join(dir, "src"), { recursive: true });
  await fs.writeFile(path.join(dir, "package.json"), JSON.stringify({
    name: "dev-mode-ref-accessor",
    private: true,
    zgui: { id: "dev.zgui.dev-mode-ref-accessor", entry: "src/main.tsx" },
  }));
  await fs.writeFile(path.join(dir, "tsconfig.json"), JSON.stringify({
    extends: "../../../packages/ui/tsconfig.native.json",
    include: ["src"],
  }));
  await fs.writeFile(path.join(dir, "src/main.tsx"), `
    import { createSignal, lockScroll, observeGeometry, registerDismissLayer, renderApp, trapFocus } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";

    export const hostApiVersion = 1;
    export const log = { scrollY: -1, dismissReasons: [] as string[] };
    let updateDismiss: ((leaving: boolean) => void) | null = null;
    export function markLeaving(leaving: boolean) { updateDismiss?.(leaving); }

    export function mount(root: NativeNode) {
      return renderApp(() => {
        const [scroller, setScroller] = createSignal<NativeNode | null>(null);
        observeGeometry(scroller, "scrollPosition", pos => { log.scrollY = pos.offset.y; });
        trapFocus(scroller, { wrap: true });
        lockScroll(scroller);
        const layer = registerDismissLayer(scroller, reason => { log.dismissReasons.push(reason); }, { layer: "popover" });
        updateDismiss = leaving => layer.update({ layer: "popover", leaving });
        return <scroll testId="ref-scroll" ref={setScroller}><text>Body</text></scroll>;
      }, root);
    }
  `);

  const built = await buildApplication("dev-mode-ref-accessor", {
    root,
    appDirectory: dir,
    outDir: path.join(dir, "dist"),
    development: true,
  });
  const mod = await import(`${pathToFileURL(path.join(built.dist, built.manifest.entry)).href}?t=${Date.now()}`);
  const h = createHost({ strictStyles: true });
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try { dispose?.(); }
    finally { delete globalThis.__zgui; }
  });
  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  const scroller = h.byTestId("ref-scroll");
  assert.ok(scroller, "scroll node mounted");

  const geo = h.geometry.find(record => record.active && record.node === scroller && record.kind === "scrollPosition");
  assert.ok(geo, "observeGeometry attached via ref accessor in dev mode");
  h.runTurn(() => geo.callback({
    offset: { x: 0, y: 128 },
    contentSize: { width: 300, height: 1000 },
    scrollport: { width: 300, height: 200 },
  }));
  assert.equal(mod.log.scrollY, 128);

  assert.equal(h.traps.filter(r => r.active && r.node === scroller).length, 1, "trapFocus active");
  assert.equal(h.locks.filter(r => r.active && r.node === scroller).length, 1, "lockScroll active");
  const dismiss = h.layers.find(r => r.active && r.node === scroller);
  assert.ok(dismiss, "registerDismissLayer active");
  h.runTurn(() => mod.markLeaving(true));
  assert.equal(dismiss.options.leaving, true);

  h.runTurn(() => dispose());
  dispose = undefined;
  assert.equal(h.geometry.filter(r => r.active).length, 0, "geometry observer disposed on unmount");
  assert.equal(h.traps.filter(r => r.active).length, 0, "focus trap disposed on unmount");
  assert.equal(h.locks.filter(r => r.active).length, 0, "scroll lock disposed on unmount");
  assert.equal(h.layers.filter(r => r.active).length, 0, "dismiss layer disposed on unmount");
});

