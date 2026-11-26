import assert from "node:assert/strict";
import { realpathSync } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";
import { buildApplication } from "../../packages/build/src/build.ts";
import { createClock, createHost } from "./native-fixture.mjs";

const root = path.resolve(import.meta.dirname, "../..");

test("development: true bundle mounts ThemeProvider, Tabs, Card, Item, Collapsible, and overlays without Solid dev-mode scope errors", async t => {
  const dir = await fs.mkdtemp(path.join(import.meta.dirname, ".dev-mode-"));
  t.after(async () => {
    await fs.rm(dir, { recursive: true, force: true });
  });
  await fs.mkdir(path.join(dir, "src"), { recursive: true });
  await fs.writeFile(
    path.join(dir, "package.json"),
    JSON.stringify({
      name: "dev-mode-smoke",
      private: true,
      zgui: { id: "dev.zgui.dev-mode-smoke", entry: "src/main.tsx" },
    }),
  );
  await fs.writeFile(
    path.join(dir, "tsconfig.json"),
    JSON.stringify({
      extends: "../../../packages/ui/tsconfig.native.json",
      include: ["src"],
    }),
  );
  await fs.writeFile(
    path.join(dir, "src/main.tsx"),
    `
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
  `,
  );

  const built = await buildApplication("dev-mode-smoke", {
    root,
    appDirectory: dir,
    outDir: path.join(dir, "dist"),
    development: true,
  });
  const entryPath = realpathSync(path.join(built.dist, built.manifest.entry));
  const mod = await import(`${pathToFileURL(entryPath).href}?t=${Date.now()}`);
  const h = createHost({ strictStyles: true });
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  h.flush();
  assert.ok(
    h.nodes.size > 10,
    "expected full themed component tree to mount under development: true",
  );
});

test("observeGeometry, trapFocus, lockScroll, and registerDismissLayer accept ref signal accessors cleanly in development: true (#51)", async t => {
  const dir = await fs.mkdtemp(path.join(import.meta.dirname, ".dev-mode-ref-"));
  t.after(async () => {
    await fs.rm(dir, { recursive: true, force: true });
  });
  await fs.mkdir(path.join(dir, "src"), { recursive: true });
  await fs.writeFile(
    path.join(dir, "package.json"),
    JSON.stringify({
      name: "dev-mode-ref-accessor",
      private: true,
      zgui: { id: "dev.zgui.dev-mode-ref-accessor", entry: "src/main.tsx" },
    }),
  );
  await fs.writeFile(
    path.join(dir, "tsconfig.json"),
    JSON.stringify({
      extends: "../../../packages/ui/tsconfig.native.json",
      include: ["src"],
    }),
  );
  await fs.writeFile(
    path.join(dir, "src/main.tsx"),
    `
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
  `,
  );

  const built = await buildApplication("dev-mode-ref-accessor", {
    root,
    appDirectory: dir,
    outDir: path.join(dir, "dist"),
    development: true,
  });
  const entryPath = realpathSync(path.join(built.dist, built.manifest.entry));
  const mod = await import(`${pathToFileURL(entryPath).href}?t=${Date.now()}`);
  const h = createHost({ strictStyles: true });
  globalThis.__zgui = h.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  const scroller = h.byTestId("ref-scroll");
  assert.ok(scroller, "scroll node mounted");

  const geo = h.geometry.find(
    record => record.active && record.node === scroller && record.kind === "scrollPosition",
  );
  assert.ok(geo, "observeGeometry attached via ref accessor in dev mode");
  h.runTurn(() =>
    geo.callback({
      offset: { x: 0, y: 128 },
      contentSize: { width: 300, height: 1000 },
      scrollport: { width: 300, height: 200 },
    }),
  );
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

test("useWindowState avoids reentrant flush() warnings when setWindowTitle runs inside createEffect in development: true (#77)", async t => {
  const dir = await fs.mkdtemp(path.join(import.meta.dirname, ".dev-mode-window-"));
  t.after(async () => {
    await fs.rm(dir, { recursive: true, force: true });
  });
  await fs.mkdir(path.join(dir, "src"), { recursive: true });
  await fs.writeFile(
    path.join(dir, "package.json"),
    JSON.stringify({
      name: "dev-mode-window-effect",
      private: true,
      zgui: { id: "dev.zgui.dev-mode-window-effect", entry: "src/main.tsx" },
    }),
  );
  await fs.writeFile(
    path.join(dir, "tsconfig.json"),
    JSON.stringify({
      extends: "../../../packages/ui/tsconfig.native.json",
      include: ["src"],
    }),
  );
  await fs.writeFile(
    path.join(dir, "src/main.tsx"),
    `
    import { createEffect, createSignal, renderApp, setWindowTitle, useWindowState } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";

    export const hostApiVersion = 1;
    const [docName, setDocName] = createSignal("welcome.md");
    export { setDocName };

    export function mount(root: NativeNode) {
      return renderApp(() => {
        const winState = useWindowState();
        createEffect(docName, title => {
          setWindowTitle(title + " — Studio");
        });
        return <text testId="win-title">{winState().title}</text>;
      }, root);
    }
  `,
  );

  const built = await buildApplication("dev-mode-window-effect", {
    root,
    appDirectory: dir,
    outDir: path.join(dir, "dist"),
    development: true,
  });
  const entryPath = realpathSync(path.join(built.dist, built.manifest.entry));
  const mod = await import(`${pathToFileURL(entryPath).href}?t=${Date.now()}`);
  const h = createHost({ strictStyles: true });
  globalThis.__zgui = h.host;
  const warnings = [];
  const origWarn = console.warn;
  const origError = console.error;
  console.warn = (...args) => {
    warnings.push(args.map(String).join(" "));
  };
  console.error = (...args) => {
    warnings.push(args.map(String).join(" "));
  };
  let dispose;
  t.after(() => {
    console.warn = origWarn;
    console.error = origError;
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });

  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  await Promise.resolve();
  h.flush();
  assert.equal(h.host.getTextContent(h.byTestId("win-title")), "welcome.md — Studio");

  h.runTurn(() => mod.setDocName("architecture.md"));
  await Promise.resolve();
  h.flush();
  assert.equal(h.host.getTextContent(h.byTestId("win-title")), "architecture.md — Studio");
  assert.deepEqual(
    warnings.filter(w => w.includes("FLUSH_IN_EFFECT_CALLBACK") || w.includes("REACTIVE_WRITE")),
    [],
    `unexpected Solid dev warnings: ${warnings.join("\n")}`,
  );

  h.runTurn(() => dispose());
  dispose = undefined;
});

test("Slider, SliderPrimitive, Accordion, ToggleGroup, RadioGroup, SplitPane, TreeView, CommandPalette, and menus mount and update cleanly in development: true", async t => {
  const dir = await fs.mkdtemp(path.join(import.meta.dirname, ".dev-mode-components-"));
  t.after(async () => {
    await fs.rm(dir, { recursive: true, force: true });
  });
  await fs.mkdir(path.join(dir, "src"), { recursive: true });
  await fs.writeFile(
    path.join(dir, "package.json"),
    JSON.stringify({
      name: "dev-mode-components",
      private: true,
      zgui: { id: "dev.zgui.dev-mode-components", entry: "src/main.tsx" },
    }),
  );
  await fs.writeFile(
    path.join(dir, "tsconfig.json"),
    JSON.stringify({
      extends: "../../../packages/ui/tsconfig.native.json",
      include: ["src"],
    }),
  );
  await fs.writeFile(
    path.join(dir, "src/main.tsx"),
    `
    import { createSignal, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import {
      Accordion, AccordionContent, AccordionItem, AccordionTrigger,
      CommandPalette, CommandPaletteProvider,
      DropdownMenu, DropdownMenuContent, DropdownMenuTrigger,
      MenuCheckboxItem, MenuRadioGroup, MenuRadioItem, MenuSub, MenuSubContent, MenuSubTrigger,
      RadioGroup, RadioGroupItem,
      Slider,
      SplitPane, SplitPaneHandle, SplitPanePanel,
      ThemeProvider,
      ToggleGroup, ToggleGroupItem,
      TreeItem, TreeView,
    } from "@hewel/zgui/components";
    import { RadioGroupPrimitive, RadioItemPrimitive, SliderPrimitive } from "@hewel/zgui/primitives";

    export const hostApiVersion = 1;
    const [sliderVal, setSliderVal] = createSignal(25);
    const [textVal, setTextVal] = createSignal("alpha");
    const [checked, setChecked] = createSignal(true);
    export { setSliderVal, setTextVal, setChecked };

    export function mount(root: NativeNode) {
      const [accVal, setAccVal] = createSignal<readonly string[]>(["item-a"]);
      const [tgVal, setTgVal] = createSignal<readonly string[]>(["alpha"]);
      const [rgVal, setRgVal] = createSignal("r1");
      const [expanded, setExpanded] = createSignal<readonly string[]>(["n1"]);
      const [cmdItems] = createSignal([{ id: "c1", label: "Run", onSelect: () => {} }]);

      return renderApp(() => (
        <ThemeProvider>
          <CommandPaletteProvider commands={cmdItems()}>
            <column>
              <Slider testId="slider" value={sliderVal()} onValueChange={setSliderVal} min={0} max={100} label="Vol" />
              <SliderPrimitive testId="slider-prim" value={sliderVal()} onValueChange={setSliderVal} min={0} max={100}>
                {({ fraction, trackRef }) => <box ref={trackRef} testId="slider-prim-fill" data-frac={fraction().toFixed(2)} />}
              </SliderPrimitive>
              <Accordion value={accVal()} onValueChange={setAccVal} selection={checked() ? "multiple" : "single"}>
                <AccordionItem value="item-a">
                  <AccordionContent testId="acc-content"><text>Body</text></AccordionContent>
                  <AccordionTrigger testId="acc-trigger" level={checked() ? 3 : 4}>Header</AccordionTrigger>
                </AccordionItem>
              </Accordion>
              <ToggleGroup value={tgVal()} onValueChange={setTgVal} selection={checked() ? "multiple" : "single"}>
                <ToggleGroupItem value={textVal()}>Toggle</ToggleGroupItem>
              </ToggleGroup>
              <RadioGroup value={rgVal()} onValueChange={setRgVal} label="RG">
                <RadioGroupItem value={rgVal()} label="One" />
              </RadioGroup>
              <RadioGroupPrimitive value={rgVal()} onValueChange={setRgVal}>
                <RadioItemPrimitive value={rgVal()} />
              </RadioGroupPrimitive>
              <SplitPane defaultSizes={[40, 60]} label="Split">
                <SplitPanePanel minSize={checked() ? 10 : 20}><text>Left</text></SplitPanePanel>
                <SplitPaneHandle label="Resize" />
                <SplitPanePanel minSize={10}><text>Right</text></SplitPanePanel>
              </SplitPane>
              <TreeView expandedIds={expanded()} onExpandedChange={setExpanded} label="Tree">
                <TreeItem testId="tree-root" itemId="n1" label={textVal()} badge={textVal()}>
                  <TreeItem itemId="n2" label="Child" />
                </TreeItem>
              </TreeView>
              <CommandPalette open={checked()} commands={cmdItems()} />
              <DropdownMenu open={checked()}>
                <DropdownMenuTrigger>Menu</DropdownMenuTrigger>
                <DropdownMenuContent>
                  <MenuCheckboxItem checked={checked()} onCheckedChange={setChecked}>Check</MenuCheckboxItem>
                  <MenuRadioGroup value={rgVal()} onValueChange={setRgVal}>
                    <MenuRadioItem value={rgVal()}>Radio</MenuRadioItem>
                  </MenuRadioGroup>
                  <MenuSub open={checked()}>
                    <MenuSubTrigger>Sub</MenuSubTrigger>
                    <MenuSubContent><MenuCheckboxItem checked={checked()}>Nested</MenuCheckboxItem></MenuSubContent>
                  </MenuSub>
                </DropdownMenuContent>
              </DropdownMenu>
            </column>
          </CommandPaletteProvider>
        </ThemeProvider>
      ), root);
    }
  `,
  );

  const built = await buildApplication("dev-mode-components", {
    root,
    appDirectory: dir,
    development: true,
  });
  const entryPath = realpathSync(path.join(built.dist, built.manifest.entry));
  const mod = await import(`${pathToFileURL(entryPath).href}?t=${Date.now()}`);
  const h = createHost({ strictStyles: true });
  createClock(t, h);
  globalThis.__zgui = h.host;
  const diagnostics = [];
  const origWarn = console.warn;
  const origError = console.error;
  console.warn = (...args) => diagnostics.push(args.map(String).join(" "));
  console.error = (...args) => diagnostics.push(args.map(String).join(" "));
  let dispose;
  t.after(() => {
    console.warn = origWarn;
    console.error = origError;
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });

  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  await Promise.resolve();
  h.flush();

  const sliderNode = h.byTestId("slider");
  assert.equal(sliderNode.props.get("--zui-slider-fraction"), "25.0000%");
  assert.equal(h.byTestId("slider-prim-fill").props.get("data-frac"), "0.25");
  assert.equal(
    h.byTestId("acc-content").props.get("ariaLabelledBy"),
    h.byTestId("acc-trigger"),
    "AccordionContent mounted before AccordionTrigger resolves ariaLabelledBy reactively",
  );

  h.runTurn(() => {
    mod.setSliderVal(75);
    mod.setTextVal("beta");
    mod.setChecked(false);
  });
  await Promise.resolve();
  h.flush();

  assert.equal(sliderNode.props.get("--zui-slider-fraction"), "75.0000%");
  assert.equal(h.byTestId("slider-prim-fill").props.get("data-frac"), "0.75");
  const treeRow = h.byTestId("tree-root");
  assert.ok(treeRow, "tree root mounted");
  const childLabels = [];
  for (let child = h.host.getFirstChild(treeRow); child; child = h.host.getNextSibling(child)) {
    if (child.tag === "label") childLabels.push(h.host.getTextContent(child));
  }
  assert.deepEqual(childLabels, ["beta", "beta"]);
  assert.deepEqual(
    diagnostics.filter(
      d =>
        d.includes("REACTIVE_WRITE_IN_OWNED_SCOPE") ||
        d.includes("STRICT_READ_UNTRACKED") ||
        d.includes("REACTIVITY_HALTED") ||
        d.includes("FLUSH_IN_EFFECT_CALLBACK"),
    ),
    [],
    `unexpected Solid dev diagnostics:\n${diagnostics.join("\n")}`,
  );

  h.runTurn(() => dispose());
  dispose = undefined;
});

test("sysmon synchronous spawnCommand failure in development: true surfaces alert instead of REACTIVE_WRITE_IN_OWNED_SCOPE", async t => {
  const sysmonDir = path.join(root, "apps/examples/sysmon");
  const built = await buildApplication("sysmon", {
    root,
    appDirectory: sysmonDir,
    development: true,
  });
  const entryPath = realpathSync(path.join(built.dist, built.manifest.entry));
  const mod = await import(`${pathToFileURL(entryPath).href}?t=${Date.now()}`);
  const h = createHost({ strictStyles: false });
  createClock(t, h);
  globalThis.__zgui = h.host;
  globalThis.__zguiServices = {
    getResolvedCapabilities: () => ({ commands: ["/usr/bin/vmstat"] }),
    onControlMessage: () => () => {},
    spawnCommand(program) {
      if (program === "/usr/bin/vmstat") {
        return { kill() {}, promise: new Promise(() => {}) };
      }
      throw new Error("E_CAPABILITY: spawn disabled");
    },
  };
  const diagnostics = [];
  const origWarn = console.warn;
  const origError = console.error;
  console.warn = (...args) => diagnostics.push(args.map(String).join(" "));
  console.error = (...args) => diagnostics.push(args.map(String).join(" "));
  let dispose;
  t.after(() => {
    console.warn = origWarn;
    console.error = origError;
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
      delete globalThis.__zguiServices;
    }
  });

  dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  await Promise.resolve();
  h.flush();

  const alertNodes = [...h.nodes.values()].filter(node => node.props.get("role") === "alert");
  const alertTexts = alertNodes.map(node => h.host.getTextContent(node));
  assert.ok(
    alertTexts.some(text => text.includes("E_CAPABILITY: spawn disabled")),
    `expected Alert to display synchronous spawnCommand error, got ${JSON.stringify(alertTexts)}`,
  );
  assert.deepEqual(
    diagnostics.filter(
      d =>
        d.includes("REACTIVE_WRITE_IN_OWNED_SCOPE") ||
        d.includes("STRICT_READ_UNTRACKED") ||
        d.includes("REACTIVITY_HALTED"),
    ),
    [],
    `unexpected Solid dev diagnostics:\n${diagnostics.join("\n")}`,
  );

  h.runTurn(() => dispose());
  dispose = undefined;
});
