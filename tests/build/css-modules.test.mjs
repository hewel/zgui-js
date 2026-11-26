import assert from "node:assert/strict";
import { cp, mkdir, mkdtemp, readFile, rm, stat, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, watchApplication } from "../../packages/build/src/index.ts";
import { workspaceRoot } from "../../packages/build/src/internal.ts";
import {
  syncCssModuleTypes,
  transformStylesheet,
  validateStylesheet,
} from "../../packages/build/src/css.ts";
import { typecheckProject } from "../../packages/build/src/typecheck.ts";

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
  await syncCssModuleTypes({ appDirectory: dirA });
  const afterStat = await stat(dtsPath);
  assert.equal(
    afterStat.mtimeMs,
    beforeStat.mtimeMs,
    "unchanged .module.css.d.ts must not be rewritten",
  );

  await rm(path.join(dirA, "src/card.module.css"));
  await syncCssModuleTypes({ appDirectory: dirA });
  await assert.rejects(() => stat(dtsPath), { code: "ENOENT" });
});

test("multi-root tsconfig and cssModuleInclude sync, typecheck, build, and watch external SDK/plugin CSS Modules", async t => {
  const ws = await mkdtemp(path.join(os.tmpdir(), "zgui-multi-root-css-"));
  t.after(() => rm(ws, { recursive: true, force: true }));

  const appDir = path.join(ws, "apps/shell");
  const sdkSrcDir = path.join(ws, "packages/sdk/src");
  const pluginAlphaSrc = path.join(ws, "plugins/alpha/src");
  const pluginBetaSub = path.join(ws, "plugins/beta/src/sub");
  const externalNoPkgDir = path.join(ws, "external-plugin/src");

  for (const dir of [
    path.join(appDir, "src"),
    sdkSrcDir,
    pluginAlphaSrc,
    pluginBetaSub,
    externalNoPkgDir,
  ]) {
    await mkdir(dir, { recursive: true });
  }
  await symlink(path.join(workspaceRoot, "node_modules"), path.join(ws, "node_modules"), "dir");
  await writeFile(
    path.join(ws, "package.json"),
    JSON.stringify({ name: "multi-root-workspace", private: true, type: "module" }),
  );
  await writeFile(
    path.join(ws, "packages/sdk/package.json"),
    JSON.stringify({ name: "@multi/sdk", version: "0.1.0", type: "module" }),
  );
  await writeFile(
    path.join(ws, "plugins/alpha/package.json"),
    JSON.stringify({ name: "@multi/alpha", version: "0.1.0", type: "module" }),
  );
  await writeFile(
    path.join(ws, "plugins/beta/package.json"),
    JSON.stringify({ name: "@multi/beta", version: "0.1.0", type: "module" }),
  );
  // Deliberately omit package.json in externalNoPkgDir's hierarchy to prove standalone syncCssModuleTypes
  // does not require moduleIdentity / package.json.
  await writeFile(
    path.join(appDir, "package.json"),
    JSON.stringify({
      name: "shell",
      version: "0.1.0",
      type: "module",
      zgui: { id: "dev.test.shell" },
    }),
  );
  await writeFile(
    path.join(appDir, "tsconfig.base.json"),
    JSON.stringify({
      extends: "@hewel/zgui/tsconfig.native.json",
      compilerOptions: {
        strict: true,
        noUncheckedIndexedAccess: true,
        allowImportingTsExtensions: true,
      },
    }),
  );
  await writeFile(
    path.join(appDir, "tsconfig.json"),
    `{
      // JSONC comments must be accepted
      "extends": "./tsconfig.base.json",
      "include": [
        "src/**/*.ts",
        "src/**/*.tsx",
        "../../packages/sdk/src/zgui-*.tsx",
        "../../plugins/*/src/*.tsx"
      ]
    }\n`,
  );

  await writeFile(
    path.join(sdkSrcDir, "zgui-primitives.module.css"),
    `.pill { display: flex; padding: 4px; }\n`,
  );
  await writeFile(
    path.join(sdkSrcDir, "zgui-widget.tsx"),
    `import styles from "./zgui-primitives.module.css";
export function SdkWidget() {
  return <box class={styles.pill}><text>SDK</text></box>;
}
`,
  );
  await writeFile(path.join(pluginAlphaSrc, "alpha.module.css"), `.panel { display: flex; }\n`);
  await writeFile(
    path.join(pluginAlphaSrc, "index.tsx"),
    `import styles from "./alpha.module.css";
export function AlphaPlugin() {
  return <box class={styles.panel}><text>Alpha</text></box>;
}
`,
  );
  await writeFile(path.join(pluginBetaSub, "nested.module.css"), `.badge { color: #fff; }\n`);
  await writeFile(path.join(externalNoPkgDir, "extra.module.css"), `.extraBox { gap: 8px; }\n`);
  await writeFile(path.join(appDir, "src/shell.module.css"), `.root { display: flex; }\n`);
  await writeFile(
    path.join(appDir, "src/main.tsx"),
    `import { renderApp, type NativeNode } from "@hewel/zgui";
import { SdkWidget } from "../../../packages/sdk/src/zgui-widget.tsx";
import { AlphaPlugin } from "../../../plugins/alpha/src/index.tsx";
import betaStyles from "../../../plugins/beta/src/sub/nested.module.css";
import shellStyles from "./shell.module.css";

export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  return renderApp(
    () => (
      <box class={shellStyles.root}>
        <SdkWidget />
        <AlphaPlugin />
        <text class={betaStyles.badge}>Beta</text>
      </box>
    ),
    root,
  );
}
`,
  );

  // 1. Cold syncCssModuleTypes + typecheckProject before any build.
  const synced = await syncCssModuleTypes({
    project: path.join(appDir, "tsconfig.json"),
    include: ["external-plugin/src"],
    cwd: ws,
  });
  assert.deepEqual(synced, [
    path.join(appDir, "src/shell.module.css"),
    path.join(externalNoPkgDir, "extra.module.css"),
    path.join(sdkSrcDir, "zgui-primitives.module.css"),
    path.join(pluginAlphaSrc, "alpha.module.css"),
    path.join(pluginBetaSub, "nested.module.css"),
  ]);
  typecheckProject(path.join(appDir, "tsconfig.json"));

  const extraDts = path.join(externalNoPkgDir, "extra.module.css.d.ts");
  const sdkDts = path.join(sdkSrcDir, "zgui-primitives.module.css.d.ts");
  const extraBefore = await stat(extraDts);
  await new Promise(resolve => setTimeout(resolve, 25));
  await syncCssModuleTypes({
    project: path.join(appDir, "tsconfig.json"),
    include: ["external-plugin/src"],
    cwd: ws,
  });
  const extraAfter = await stat(extraDts);
  assert.equal(
    extraAfter.mtimeMs,
    extraBefore.mtimeMs,
    "unchanged external .d.ts must not rewrite",
  );

  // Stale .d.ts in external root is pruned when CSS Module is removed.
  await rm(path.join(externalNoPkgDir, "extra.module.css"));
  await syncCssModuleTypes({
    project: path.join(appDir, "tsconfig.json"),
    include: ["external-plugin/src"],
    cwd: ws,
  });
  await assert.rejects(() => stat(extraDts), { code: "ENOENT" });

  // 2. buildApplication syncs updated external CSS Module types before tsrs typecheck.
  await writeFile(
    path.join(sdkSrcDir, "zgui-primitives.module.css"),
    `.pill { display: flex; padding: 4px; }\n.pillActive { opacity: 1; }\n`,
  );
  await writeFile(
    path.join(sdkSrcDir, "zgui-widget.tsx"),
    `import { cx } from "@hewel/zgui";
import styles from "./zgui-primitives.module.css";
export function SdkWidget() {
  return <box class={cx(styles.pill, styles.pillActive)}><text>SDK</text></box>;
}
`,
  );
  const built = await buildApplication("shell", {
    appDirectory: appDir,
    cssModuleInclude: [path.relative(process.cwd(), externalNoPkgDir)],
  });
  assert.match(await readFile(sdkDts, "utf8"), /readonly pillActive: string;/);
  assert.match(await readFile(path.join(built.dist, "app.css"), "utf8"), /_pillActive/);

  // 3. watchApplication watches external tsconfig directories and emits .d.ts for new CSS Modules.
  let initialPublishedResolve;
  const initialPublished = new Promise(resolve => {
    initialPublishedResolve = resolve;
  });
  const watcher = watchApplication("shell", {
    appDirectory: appDir,
    onSuccess: () => initialPublishedResolve(),
    onError: error => {
      throw error;
    },
  });
  t.after(() => watcher.close());
  await initialPublished;

  const freshCssPath = path.join(pluginAlphaSrc, "fresh.module.css");
  const freshDtsPath = `${freshCssPath}.d.ts`;
  await writeFile(freshCssPath, `.freshTag { color: #0f0; }\n`);

  const deadline = Date.now() + 10000;
  let generatedDts = "";
  while (Date.now() < deadline) {
    try {
      generatedDts = await readFile(freshDtsPath, "utf8");
      if (generatedDts.includes("readonly freshTag: string;")) break;
    } catch {
      // Wait for external watcher debounced sync.
    }
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.match(generatedDts, /readonly freshTag: string;/);
});
