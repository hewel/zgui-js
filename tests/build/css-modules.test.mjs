import assert from "node:assert/strict";
import { cp, mkdir, mkdtemp, readFile, rm, stat, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import {
  syncCssModuleTypes,
  transformStylesheet,
  validateStylesheet,
} from "../../packages/build/src/css.ts";

test("Lightning CSS AST validator rejects unsupported Stylo features and v1 CSS Module ICSS/composes", () => {
  const disallowedNative = [
    ['@import "./other.css";', /@import/],
    [".a { background-image: url(foo.png); }", /url\(/],
    ['.a { background-image: image-set("a.png" 1x); }', /image-set/],
    ['@font-face { font-family: X; src: local("Sans"); }', /@font-face/],
    ["@container (min-width: 100px) { .a { color: red; } }", /@container/],
    ["@media (prefers-reduced-motion: reduce) { .a { opacity: 1; } }", /prefers-reduced-motion/],
    ['@property --x { syntax: "*"; inherits: false; }', /@property/],
    ['html[dir="rtl"] .a { color: red; }', /html\[dir\]/],
  ];
  for (const [css, pattern] of disallowedNative) {
    assert.throws(() => validateStylesheet(css, "test.css"), pattern, css);
  }

  const disallowedModules = [
    [".a { color: red; } .b { composes: a; }", /composes/],
    [":export { token: value; }", /:export/],
    [':import("./a.css") { a: b; }', /:import/],
    ["@value primary: #fff; .a { color: primary; }", /@value|deprecated/i],
  ];
  for (const [css, pattern] of disallowedModules) {
    assert.throws(
      () => transformStylesheet(css, { filename: "app/src/bad.module.css", cssModules: true }),
      pattern,
      css,
    );
  }
});

test("Tabs, Accordion, Collapsible, and TreeView stylesheets hoist statically into @layer zgui.components with adapted selectors intact", async t => {
  const dir = await mkdtemp(path.join(workspaceRoot, "tests/build/.fixture-static-sheets-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await mkdir(path.join(dir, "src"), { recursive: true });
  await writeFile(
    path.join(dir, "tsconfig.json"),
    JSON.stringify({
      extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
      include: ["src/**/*.ts", "src/**/*.tsx"],
    }),
  );
  await writeFile(
    path.join(dir, "src/main.tsx"),
    `
import { renderApp, type NativeNode } from "@hewel/zgui";
import {
  Accordion, AccordionContent, AccordionItem, AccordionTrigger,
  Collapsible, CollapsibleContent, CollapsibleTrigger,
  Tabs, TabsContent, TabsList, TabsTrigger,
  TreeView, ThemeProvider,
} from "@hewel/zgui/components";

export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  return renderApp(() => (
    <ThemeProvider>
      <column>
        <Tabs defaultValue="a">
          <TabsList><TabsTrigger value="a">A</TabsTrigger></TabsList>
          <TabsContent value="a"><label>Tab A</label></TabsContent>
        </Tabs>
        <Accordion defaultValue={["one"]}>
          <AccordionItem value="one">
            <AccordionTrigger>Item</AccordionTrigger>
            <AccordionContent><label>Body</label></AccordionContent>
          </AccordionItem>
        </Accordion>
        <Collapsible defaultOpen>
          <CollapsibleTrigger>Toggle</CollapsibleTrigger>
          <CollapsibleContent><label>Details</label></CollapsibleContent>
        </Collapsible>
        <TreeView items={[{ id: "root", label: "Root" }]} label="Files" />
      </column>
    </ThemeProvider>
  ), root);
}
`,
  );

  const { dist, manifest } = await buildApplication("static-sheets", { appDirectory: dir });
  assert.deepEqual(manifest.styles, ["components.css", "app.css"]);
  const componentsCss = await readFile(path.join(dist, "components.css"), "utf8");
  const appJs = await readFile(path.join(dist, "app.mjs"), "utf8");

  assert.match(componentsCss, /^@layer zgui\.components\s*\{/);
  // Adapted Tabs selectors survive static hoisting (no dead runtime .replaceAll).
  assert.match(
    componentsCss,
    /\.zui-tabs__trigger\[data-variant=["']default["']\]\[data-state=["']active["']\]/,
  );
  assert.match(componentsCss, /\.zui-tabs__trigger\[data-orientation=["']vertical["']\]/);
  assert.doesNotMatch(
    componentsCss,
    /\.zui-tabs__list\[data-variant=["']default["']\]\s+\.zui-tabs__trigger/,
  );
  // Adapted Accordion & Collapsible rules survive static hoisting.
  assert.match(componentsCss, /\.zui-accordion__chevron\[data-state=["']open["']\]/);
  assert.match(componentsCss, /\.zui-accordion__measure/);
  assert.match(componentsCss, /\.zui-collapsible__content\[data-collapsed\]/);
  // TreeView stylesheet is hoisted into components.css and stripped from app.mjs.
  assert.match(componentsCss, /\.zui-tree__row/);
  assert.doesNotMatch(
    appJs,
    /padding-left:\s*calc\(8px \+ var\(--zui-tree-depth/,
    "TreeViewStyle CSS must not remain in app.mjs",
  );
  assert.doesNotMatch(
    appJs,
    /\.zui-collapsible__content\[data-collapsed\]/,
    "Collapsible extra CSS must not remain in app.mjs",
  );
});

test("two separate checkout directories produce byte-identical CSS Modules artifacts and identical buildId, and .d.ts sync skips unchanged writes", async t => {
  const base = await mkdtemp(path.join(os.tmpdir(), "zgui-css-mod-repro-"));
  t.after(() => rm(base, { recursive: true, force: true }));
  const dirA = path.join(base, "checkout-alpha", "app");
  const dirB = path.join(base, "checkout-beta-nested", "app");
  const srcExample = path.join(workspaceRoot, "apps/examples/css-modules");

  for (const dir of [dirA, dirB]) {
    await mkdir(path.join(dir, "src"), { recursive: true });
    await symlink(path.join(workspaceRoot, "node_modules"), path.join(dir, "node_modules"));
    await cp(path.join(srcExample, "package.json"), path.join(dir, "package.json"));
    await cp(path.join(srcExample, "src/app.css"), path.join(dir, "src/app.css"));
    await cp(path.join(srcExample, "src/card.module.css"), path.join(dir, "src/card.module.css"));
    await cp(path.join(srcExample, "src/main.tsx"), path.join(dir, "src/main.tsx"));
    await writeFile(
      path.join(dir, "tsconfig.json"),
      JSON.stringify({
        extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
        include: ["src/**/*.ts", "src/**/*.tsx"],
      }),
    );
  }

  const buildA = await buildApplication("css-modules", { appDirectory: dirA });
  const buildB = await buildApplication("css-modules", { appDirectory: dirB });

  assert.equal(buildA.manifest.buildId, buildB.manifest.buildId);
  assert.deepEqual(buildA.manifest, buildB.manifest);
  for (const artifact of ["components.css", "app.css", "app.mjs", "app.mjs.map"]) {
    const bytesA = await readFile(path.join(buildA.dist, artifact));
    const bytesB = await readFile(path.join(buildB.dist, artifact));
    assert.deepEqual(bytesA, bytesB, `${artifact} must be byte-identical across checkout paths`);
  }

  // Verify syncCssModuleTypes skips rewriting unchanged .d.ts files (watch-loop safe)
  // and prunes stale .d.ts files when a .module.css is deleted.
  const dtsPath = path.join(dirA, "src/card.module.css.d.ts");
  const beforeStat = await stat(dtsPath);
  await new Promise(resolve => setTimeout(resolve, 25));
  await syncCssModuleTypes(dirA, { root: workspaceRoot });
  const afterStat = await stat(dtsPath);
  assert.equal(
    afterStat.mtimeMs,
    beforeStat.mtimeMs,
    "unchanged .module.css.d.ts must not be rewritten",
  );

  await rm(path.join(dirA, "src/card.module.css"));
  await syncCssModuleTypes(dirA, { root: workspaceRoot });
  await assert.rejects(() => stat(dtsPath), { code: "ENOENT" });
});
