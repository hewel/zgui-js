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
