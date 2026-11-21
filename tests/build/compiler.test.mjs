import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { mkdtemp, mkdir, writeFile, readFile, readlink, realpath, rm, readdir, symlink } from "node:fs/promises";
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";
import path from "node:path";
import { buildApplication, bundle, contentDigest, publishApplication, publishGeneration, workspaceRoot } from "../../packages/build/src/build.ts";
import { validateModule } from "../../packages/build/src/validate.ts";
import { rebuildQueue, watchApplication } from "../../packages/build/src/watch.ts";
import { typecheckProject, typecheckProjectAsync } from "../../packages/build/src/typecheck.ts";
import { compileBundle, watchBundle } from "../../packages/build/src/compiler.ts";
import { publishBuildStatus } from "../../packages/build/src/publication.ts";
import { validateStylesheet } from "../../packages/build/src/stylex.ts";

const require = createRequire(new URL("../../packages/build/package.json", import.meta.url));
const { TraceMap, originalPositionFor } = require("@jridgewell/trace-mapping");
const validEntry = `import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
function App() {
  const [count] = createSignal(0);
  if (count() < 0) throw new Error("mapped failure");
  return <text>Count: {count()}</text>;
}
export function mount(root: NativeNode): () => void {
  return renderApp(() => <App />, root);
}
`;

async function fixture(t) {
  const directory = await mkdtemp(path.join(workspaceRoot, "tests/build/.fixture-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(path.join(directory, "src"));
  await writeFile(path.join(directory, "package.json"), '{"type":"module"}\n');
  await writeFile(path.join(directory, "tsconfig.json"), JSON.stringify({
    compilerOptions: { target: "ES2022", module: "ESNext", moduleResolution: "Bundler", lib: ["ES2022"], types: [], strict: true, noEmit: true, jsx: "preserve", jsxImportSource: "@hewel/zgui" },
    include: ["src"],
  }));
  await writeFile(path.join(directory, "src/main.tsx"), validEntry);
  return directory;
}

const entry = "export const hostApiVersion = 1; export function mount(root) { return () => {}; }";

test("local TypeScript CLI checks project paths literally, never emits, and preserves diagnostics", async t => {
  const directory = await mkdtemp(path.join(workspaceRoot, "tests/build/.fixture-cli ; "));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(path.join(directory, "src"));
  const config = path.join(directory, "tsconfig.json");
  const source = path.join(directory, "src/main.ts");
  await writeFile(config, JSON.stringify({
    compilerOptions: { target: "ES2022", module: "ESNext", lib: ["ES2022"], types: [], strict: true, noEmit: false, outDir: "dist" },
    include: ["src"],
  }));
  await writeFile(source, "export const value: number = 1;\n");
  assert.equal(typecheckProject(config), undefined);
  assert.ok(!(await readdir(directory)).includes("dist"));

  await writeFile(source, 'export const value: number = "wrong";\n');
  assert.throws(() => typecheckProject(config), error => {
    assert.match(error.message, /main\.ts\(1,\d+\): error TS2322:/);
    assert.match(error.message, /not assignable to type 'number'/);
    return true;
  });
  assert.ok(!(await readdir(directory)).includes("dist"));
  assert.throws(() => typecheckProject(path.join(directory, "missing.json")), /error TS5058:.*missing\.json/);
});

test("AST validation distinguishes nested async work from top-level execution", () => {
  validateModule(`${entry}\nasync function work() { await Promise.resolve(); for await (const x of []) {} }`, { bundled: true, entry: true });
  validateModule(`${entry}\nconst message = 'import("node:fs") and await';`, { bundled: true, entry: true });
  assert.throws(() => validateModule(`${entry}\nawait Promise.resolve();`), /Top-level await/);
  assert.throws(() => validateModule(`${entry}\nfor await (const x of []) {}`), /Top-level await/);
  assert.throws(() => validateModule(`${entry}\nclass C { [await Promise.resolve()]() {} }`), /Top-level await/);
  assert.throws(() => validateModule(`${entry}\nfunction lazy() { return import('./lazy.js'); }`), /Dynamic imports/);
  assert.throws(() => validateModule(`${entry}\nimport 'unresolved';`, { bundled: true }), /unresolved runtime import/);
  assert.throws(() => validateModule(`${entry}\nexport * from 'external';`, { bundled: true }), /external re-export/);
  assert.throws(() => validateModule(`${entry}\nconsole.log(document);`, { bundled: true }), /Unsupported runtime global: document/);
  assert.throws(() => validateModule(`${entry}\nrequire('fs');`, { bundled: true }), /Runtime require/);
  validateModule(`${entry}\nfunction merge() { return (() => arguments[0])(); }`, { bundled: true });
  assert.throws(() => validateModule(`${entry}\nconst args = arguments;`, { bundled: true }), /Unsupported runtime global: arguments/);
  assert.throws(() => validateModule(`${entry}\nconst args = () => arguments;`, { bundled: true }), /Unsupported runtime global: arguments/);
  assert.throws(() => validateModule(`${entry}\nclass C { [arguments[0]]() {} }`, { bundled: true }), /Unsupported runtime global: arguments/);
});

test("host exports must be stable version 1 and a synchronous mount function", () => {
  validateModule("const version=1; const start=() => () => {}; export {version as hostApiVersion, start as mount};", { bundled: true, entry: true });
  for (const code of [
    "export const hostApiVersion=2; export function mount() {}",
    "export let hostApiVersion=1; hostApiVersion=2; export function mount() {}",
    "export const hostApiVersion=1;",
    "export const hostApiVersion=1; export async function mount() {}",
    "export const hostApiVersion=1; export const mount=42;",
  ]) assert.throws(() => validateModule(code, { bundled: true, entry: true }), /Entry must export/);
});

test("handwritten class selectors do not trigger browser-html direction rejection", () => {
  validateStylesheet(".html:not([dir]) { color: red; } .other-html[dir] { color: blue; }", "base.css");
  assert.throws(() => validateStylesheet("html:not([dir='rtl']) .x { color: red }", "app.css"), /directional html/);
  assert.throws(() => validateStylesheet("@media (min-width: 1px) {html[dir='rtl'] .x { color: red }}", "app.css"), /directional html/);
});

test("real TSX bundles have one reactive runtime, composed maps, resources, and verifiable identity", async t => {
  const directory = await fixture(t);
  await writeFile(path.join(directory, "src/app.css"), "text { color: #123456; }\n");
  await mkdir(path.join(directory, "assets/icons"), { recursive: true });
  await writeFile(path.join(directory, "assets/icons/mark.bin"), Buffer.from([0, 255, 1, 42]));
  const { dist, manifest } = await buildApplication("fixture", { appDirectory: directory });
  assert.equal(manifest.hostApiVersion, 1);
  assert.equal(manifest.schemaVersion, 2);
  assert.deepEqual(manifest.styles, ["base.css", "app.css"]);
  assert.deepEqual(manifest.assets, ["assets/icons/mark.bin"]);
  const root = await realpath(dist);
  const names = Object.keys(manifest.artifactDigests).sort();
  const files = new Map(await Promise.all(names.map(async name => [name, await readFile(path.join(root, name))])));
  for (const name of names) {
    assert.equal(manifest.artifactDigests[name], `sha256:${createHash("sha256").update(files.get(name)).digest("hex")}`);
  }
  // Serialize independently, with recursively sorted keys and stable array order.
  const sort = value => Array.isArray(value) ? value.map(sort) : value && typeof value === "object"
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, sort(value[key])])) : value;
  const { buildId, ...identity } = manifest;
  assert.equal(buildId, `sha256:${createHash("sha256").update(JSON.stringify(sort(identity))).digest("hex")}`);
  assert.ok(names.includes("style-origins.json"));
  assert.ok(names.includes("build-info.json"));
  const module = await import(pathToFileURL(path.join(root, manifest.entry)));
  assert.equal(module.hostApiVersion, 1);
  assert.equal(typeof module.mount, "function");
  const code = files.get("app.mjs").toString();
  const marker = code.indexOf('throw new Error("mapped failure")');
  assert.ok(marker >= 0);
  const prefix = code.slice(0, marker);
  const generated = { line: prefix.split("\n").length, column: prefix.length - prefix.lastIndexOf("\n") - 1 };
  const mapJson = JSON.parse(files.get("app.mjs.map"));
  const original = originalPositionFor(new TraceMap(mapJson), generated);
  assert.ok(original.source.endsWith("src/main.tsx"), JSON.stringify(original));
  assert.equal(original.line, 6);
  assert.ok(mapJson.sourcesContent.includes(validEntry));
  const info = JSON.parse(await readFile(path.join(root, "build-info.json"), "utf8"));
  assert.equal(info.bun, Bun.version);
  assert.ok(!Object.hasOwn(info, "node"));
  assert.equal(info.packages["solid-js"], "2.0.1");
  assert.equal(info.packages["@solidjs/universal"], "2.0.1");
  assert.equal(info.packages["@solidjs/babel-plugin"], "2.0.1");
  assert.equal(info.packages.tsrs, "7.1.0-beta.tsrs2");
  assert.equal(info.engine.name, "QuickJS-NG");
  assert.equal(info.packages.rolldown, "1.3.2");
  assert.equal(info.graph.chunks.length, 1);
  assert.deepEqual(info.graph.chunks[0].imports, []);
  assert.deepEqual(info.graph.chunks[0].dynamicImports, []);
  assert.match(info.engine.revision, /^[a-f0-9]{40}$/);
  assert.equal(info.lockfile.path, "bun.lock");
  assert.equal(info.lockfile.digest, `sha256:${createHash("sha256").update(await readFile(path.join(workspaceRoot, "bun.lock"))).digest("hex")}`);
  assert.equal(contentDigest(files, manifest), manifest.buildId);
  assert.throws(() => contentDigest(new Map([...files, ["build-info.json", Buffer.from("tampered")]]), manifest), /Artifact digest mismatch/);
  const firstTarget = await readlink(dist);
  const second = await buildApplication("fixture", { appDirectory: directory });
  assert.equal(second.manifest.buildId, manifest.buildId);
  assert.notEqual(await readlink(dist), firstTarget);
  assert.equal(await readFile(path.join(root, "app.mjs"), "utf8"), code, "old reader remains pinned to a complete generation");
});

test("StyleX compiles static TSX and imported token modules before tree shaking", async t => {
  const directory = await fixture(t);
  await writeFile(path.join(directory, "src/palette.stylex.ts"), `import * as stylex from "@stylexjs/stylex";
export const palette = stylex.defineVars({ foreground: "#135790" });`);
  await writeFile(path.join(directory, "src/extra.ts"), `import * as stylex from "@stylexjs/stylex";
export const unused = stylex.create({ root: { opacity: 0.321 } });`);
  await writeFile(path.join(directory, "src/main.tsx"), `import * as stylex from "@stylexjs/stylex";
import { palette } from "./palette.stylex";
import "./extra";
const styles = stylex.create({ root: {
  color: palette.foreground, fontSize: 23,
  boxShadow: "0 2px 4px black", textShadow: "1px 1px black", filter: "brightness(0.5)",
  backdropFilter: "blur(4px)", mixBlendMode: "multiply", isolation: "isolate",
  fontVariationSettings: '"wght" 650', fontStyle: "oblique 12deg", fontStretch: "85%",
  whiteSpace: "pre-wrap", textOverflow: "ellipsis", pointerEvents: "none", userSelect: "none",
  translate: "10px 20%", rotate: "15deg", scale: 1.25,
} });
${validEntry.replace("<text>", "<text {...stylex.props(styles.root)}>")}`);
  const { dist, manifest } = await buildApplication("fixture", { appDirectory: directory });
  assert.equal(manifest.schemaVersion, 2);
  assert.deepEqual(manifest.styles, ["app.css"]);
  const css = await readFile(path.join(dist, "app.css"), "utf8");
  assert.match(css, /:root/);
  assert.match(css, /#135790/);
  assert.match(css, /font-size:23px/);
  assert.match(css, /[;{]box-shadow:0 2px 4px black[;}]/);
  assert.match(css, /[;{]text-shadow:1px 1px black[;}]/);
  assert.match(css, /[;{]filter:brightness\(0?\.5\)[;}]/);
  assert.match(css, /[;{]backdrop-filter:blur\(4px\)[;}]/);
  assert.match(css, /[;{]mix-blend-mode:multiply[;}]/);
  assert.match(css, /[;{]isolation:isolate[;}]/);
  assert.match(css, /[;{]font-variation-settings:["']wght["'] 650[;}]/);
  assert.match(css, /[;{]font-style:oblique 12deg[;}]/);
  assert.match(css, /[;{]font-stretch:85%[;}]/);
  assert.match(css, /[;{]white-space:pre-wrap[;}]/);
  assert.match(css, /[;{]text-overflow:ellipsis[;}]/);
  assert.match(css, /[;{]pointer-events:none[;}]/);
  assert.match(css, /[;{]user-select:none[;}]/);
  assert.match(css, /[;{]translate:10px 20%[;}]/);
  assert.match(css, /[;{]rotate:15deg[;}]/);
  assert.match(css, /[;{]scale:1\.25[;}]/);
  assert.match(css, /opacity:0?\.321/);
  assert.doesNotMatch(css, /@layer/);
  const code = await readFile(path.join(dist, "app.mjs"), "utf8");
  assert.doesNotMatch(code, /stylex\.create\(/);
  const origins = JSON.parse(await readFile(path.join(dist, "style-origins.json"), "utf8"));
  const modules = new Set(origins.rules.flatMap(rule => rule.origins.map(origin => origin.module)));
  assert.ok(modules.has("app/src/main.tsx"));
  assert.ok(modules.has("app/src/palette.stylex.ts"));
  assert.ok(modules.has("app/src/extra.ts"));
  for (const rule of origins.rules) {
    assert.ok(rule.generated.end.offset > rule.generated.start.offset);
    assert.ok(css.slice(rule.generated.start.offset, rule.generated.end.offset).includes("{"));
    for (const origin of rule.origins) {
      assert.equal(origin.original, undefined, "module metadata does not prove an exact original position");
      assert.ok(!path.isAbsolute(origin.module));
    }
  }
});

test("reachable dependency sources contribute StyleX rules even when their JavaScript is tree shaken", async t => {
  const directory = await fixture(t);
  const dependency = path.join(directory, "node_modules/stylex-fixture");
  await mkdir(dependency, { recursive: true });
  await writeFile(path.join(dependency, "package.json"), JSON.stringify({
    name: "stylex-fixture", type: "module", sideEffects: false, exports: "./index.ts",
  }));
  await writeFile(path.join(dependency, "index.ts"), `import * as stylex from "@stylexjs/stylex";
export const styles = stylex.create({ root: { opacity: 0.246 } });`);
  const source = path.join(directory, "src/main.tsx");
  await writeFile(source, `import "stylex-fixture";\n${validEntry}`);
  const result = await bundle(source, { appDirectory: directory });
  assert.match(result.files.get("app.css").toString(), /opacity:0?\.246/);
  const origins = JSON.parse(result.files.get("style-origins.json"));
  assert.ok(origins.rules.some(rule => rule.origins.some(origin => origin.module === "packages/stylex-fixture/index.ts")));
  assert.ok(!result.graph.chunks[0].modules.includes(path.join(dependency, "index.ts")));
});

test("StyleX dynamic values, themes and keyframes retain the official runtime merge contract", async t => {
  const directory = await fixture(t);
  await writeFile(path.join(directory, "src/colors.stylex.ts"), `import * as stylex from "@stylexjs/stylex";
export const colors = stylex.defineVars({ foreground: "red" });`);
  await writeFile(path.join(directory, "src/main.tsx"), `import * as stylex from "@stylexjs/stylex";
import { colors } from "./colors.stylex";
const animation = stylex.keyframes({ from: { opacity: 0 }, to: { opacity: 1 } });
const theme = stylex.createTheme(colors, { foreground: "blue" });
const styles = stylex.create({
  base: { color: colors.foreground, animationName: animation, animationDuration: "var(--motion-duration)" },
  dynamic: (width: number) => ({ width }),
});
export function inspect(width: number) { return stylex.props(styles.base, styles.dynamic(width), theme); }
${validEntry}`);
  const result = await bundle(path.join(directory, "src/main.tsx"), { appDirectory: directory });
  await writeFile(path.join(directory, "app.mjs"), result.files.get("app.mjs"));
  const module = await import(pathToFileURL(path.join(directory, "app.mjs")));
  const props = module.inspect(37);
  assert.equal(typeof props.className, "string");
  assert.ok(Object.values(props.style).includes("37px"));
  const css = result.files.get("app.css").toString();
  assert.match(css, /@keyframes/);
  assert.match(css, /animation-name:/);
  assert.match(css, /animation-duration:var\(--motion-duration\)/);
  assert.match(css, /\.([a-z0-9]+)\.\1[^{}]*:root/);
  assert.match(css, /:not\(#\\#\)/);
  assert.doesNotMatch(css, /@layer|html\[/);
  assert.match(css, /@property --[^{}]+\{\s*syntax: "\*"; inherits: false;/);
  assert.equal(result.styleSystem.name, "stylex");
});

test("native compilation rejects unsupported StyleX APIs and host imports without banning harmless text", async t => {
  const directory = await fixture(t);
  const source = path.join(directory, "src/main.tsx");
  const cases = [
    ['import "@stylexjs/stylex/lib/stylex-inject";', /Unavailable native StyleX host import/],
    ['import { defineConsts } from "@stylexjs/stylex"; const x = defineConsts({ x: 1 });', /Unsupported native StyleX API/],
    ['import * as sx from "@stylexjs/stylex"; const x = sx.types.color("red");', /Unsupported native StyleX API/],
    ['import * as sx from "@stylexjs/stylex"; const call = sx.create; const x = call({root:{color:"red"}});', /Untransformed StyleX compile-time API/],
    ['import * as sx from "@stylexjs/stylex"; export const x = sx.create({root:{backgroundImage:"url(image.png)"}});', /Unsupported native CSS/],
  ];
  for (const [prefix, pattern] of cases) {
    await writeFile(source, `${prefix}\n${validEntry}`);
    await assert.rejects(bundle(source, { appDirectory: directory }), pattern);
  }
  await writeFile(source, `export const message = "document stylex.create() stylex-inject";\n${validEntry}`);
  assert.ok((await bundle(source, { appDirectory: directory })).files.has("app.css"));
});

test("missing Babel maps and missing StyleX output cannot publish a candidate", async t => {
  const directory = await fixture(t);
  const source = path.join(directory, "src/main.tsx");
  await assert.rejects(
    compileBundle(source, { root: workspaceRoot, appDirectory: directory, babelTransform: async () => ({ code: entry, map: null }) }),
    /no JavaScript or composed source map/,
  );
  let pluginRan = false;
  const result = await compileBundle(source, {
    root: workspaceRoot, appDirectory: directory,
    plugins: [{ name: "test-plugin-hook", buildStart() { pluginRan = true; } }],
  });
  assert.equal(pluginRan, true);
  const withoutStylex = new Map([...result.files].filter(([name]) => name !== "app.css" && name !== "style-origins.json"));
  await assert.rejects(
    publishApplication("fixture", { ...result, files: withoutStylex }, { root: workspaceRoot, appDirectory: directory }),
    /Missing StyleX compilation output/,
  );
});

test("manifest v2 fonts and build identity are reproducible across application directories", async t => {
  const first = await fixture(t);
  const second = await fixture(t);
  const source = `import * as stylex from "@stylexjs/stylex";
const styles = stylex.create({ root: { fontFamily: "Fixture Font", color: "#2468ac" } });
${validEntry.replace("<text>", "<text {...stylex.props(styles.root)}>")}`;
  for (const directory of [first, second]) {
    await mkdir(path.join(directory, "assets"));
    await writeFile(path.join(directory, "assets/font.ttf"), Buffer.from([0, 1, 2, 3]));
    await writeFile(path.join(directory, "package.json"), JSON.stringify({
      name: "reproducible-fixture", type: "module", zgui: { fonts: [{ family: "Fixture Font", path: "assets/font.ttf" }] },
    }));
    await writeFile(path.join(directory, "src/main.tsx"), source);
  }
  const a = await buildApplication("fixture", { appDirectory: first });
  const b = await buildApplication("fixture", { appDirectory: second });
  assert.deepEqual(a.manifest, b.manifest);
  assert.deepEqual(a.manifest.fonts, [{ family: "Fixture Font", path: "assets/font.ttf" }]);
  assert.match(a.manifest.styleSystem.configDigest, /^sha256:[a-f0-9]{64}$/);
  const descriptor = JSON.parse(await readFile(path.join(workspaceRoot, "schemas/stylex-profile.json"), "utf8"));
  const sort = value => Array.isArray(value) ? value.map(sort) : value && typeof value === "object"
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, sort(value[key])])) : value;
  const expectedProfileDigest = `sha256:${createHash("sha256").update(JSON.stringify(sort(descriptor))).digest("hex")}`;
  assert.equal(a.manifest.styleSystem.nativeProfileDigest, expectedProfileDigest);
  assert.notEqual(a.manifest.styleSystem.nativeProfileDigest, a.manifest.styleSystem.configDigest);
  for (const name of ["build-info.json", "style-origins.json"]) {
    const metadata = JSON.parse(await readFile(path.join(a.dist, name), "utf8"));
    assert.deepEqual(metadata.styleSystem, a.manifest.styleSystem);
  }
  for (const name of ["build-info.json", "style-origins.json", "app.mjs.map"]) {
    const bytes = await readFile(path.join(a.dist, name), "utf8");
    assert.ok(!bytes.includes(first));
    assert.ok(!bytes.includes(workspaceRoot));
  }
  const diagnostic = await buildApplication("fixture", { appDirectory: first, stylexDiagnostics: true });
  assert.notEqual(diagnostic.manifest.styleSystem.configDigest, a.manifest.styleSystem.configDigest);
  assert.equal(diagnostic.manifest.styleSystem.nativeProfileDigest, expectedProfileDigest);
  assert.notEqual(diagnostic.manifest.buildId, a.manifest.buildId);
  assert.match(await readFile(path.join(diagnostic.dist, "app.mjs"), "utf8"), /data-style-src/);
  await writeFile(path.join(first, "package.json"), JSON.stringify({ zgui: { fonts: [{ family: "Missing", path: "assets/missing.ttf" }] } }));
  const previous = await readlink(a.dist);
  await assert.rejects(buildApplication("fixture", { appDirectory: first }), /Font path must be a declared asset/);
  assert.equal(await readlink(a.dist), previous);
});

test("native profile identity follows canonical workspace descriptor contents independently of compiler diagnostics", async t => {
  const directory = await fixture(t);
  const root = path.join(directory, "workspace");
  const descriptorPath = path.join(root, "schemas/stylex-profile.json");
  await mkdir(path.dirname(descriptorPath), { recursive: true });
  const descriptor = JSON.parse(await readFile(path.join(workspaceRoot, "schemas/stylex-profile.json"), "utf8"));
  await writeFile(descriptorPath, JSON.stringify(descriptor));
  const source = path.join(directory, "src/main.tsx");
  const first = await bundle(source, { root, appDirectory: directory });
  await writeFile(descriptorPath, JSON.stringify(Object.fromEntries(Object.entries(descriptor).reverse()), null, 4) + "\n");
  const reordered = await bundle(source, { root, appDirectory: directory });
  assert.deepEqual(reordered.styleSystem, first.styleSystem);
  descriptor.selectors.hover = !descriptor.selectors.hover;
  await writeFile(descriptorPath, JSON.stringify(descriptor));
  const changed = await bundle(source, { root, appDirectory: directory });
  assert.notEqual(changed.styleSystem.nativeProfileDigest, first.styleSystem.nativeProfileDigest);
  assert.notEqual(changed.styleSystem.configDigest, first.styleSystem.configDigest);
  assert.deepEqual(changed.files.get("app.css"), first.files.get("app.css"));
  await rm(descriptorPath);
  await assert.rejects(bundle(source, { root, appDirectory: directory }), /ENOENT/);
});

test("StyleX watch replaces changed rules and prunes detached and deleted modules", { timeout: 30000 }, async t => {
  const directory = await fixture(t);
  const dependency = path.join(directory, "src/extra.ts");
  const main = path.join(directory, "src/main.tsx");
  const extra = opacity => `import * as stylex from "@stylexjs/stylex"; export const styles = stylex.create({ root: { opacity: ${opacity} } });`;
  const imported = `import "./extra";\n${validEntry}`;
  await writeFile(dependency, extra(0.123));
  await writeFile(main, imported);
  let next = Promise.withResolvers();
  const take = async () => {
    const result = await next.promise;
    next = Promise.withResolvers();
    return result;
  };
  const watcher = watchApplication("fixture", {
    appDirectory: directory, onSuccess(result) { next.resolve(result); }, onError(error) { next.reject(error); },
  });
  const css = async result => readFile(path.join(result.dist, "app.css"), "utf8");
  try {
    assert.match(await css(await take()), /opacity:0?\.123/);
    await writeFile(dependency, extra(0.456));
    const changed = await css(await take());
    assert.match(changed, /opacity:0?\.456/);
    assert.doesNotMatch(changed, /opacity:0?\.123/);
    await writeFile(main, validEntry);
    const detached = await take();
    assert.equal(await css(detached), "");
    const origins = JSON.parse(await readFile(path.join(detached.dist, "style-origins.json"), "utf8"));
    assert.deepEqual(origins.rules, []);
    await rm(dependency);
    await writeFile(main, validEntry.replace("Count:", "removed"));
    assert.equal(await css(await take()), "");
    await writeFile(dependency, extra(0.789));
    await writeFile(main, imported);
    let added;
    do { added = await css(await take()); } while (!/opacity:0?\.789/.test(added));
    assert.doesNotMatch(added, /opacity:0?\.456/);
  } finally { await watcher.close(); }
});

test("native services bundle lazily with one Solid runtime and declared text assets", async t => {
  const directory = await fixture(t);
  const source = `import { readTextAsset } from "@hewel/zgui-services";\n` + validEntry.replace(
    "return renderApp(() => <App />, root);",
    'const request = readTextAsset("assets/message.txt"); request.cancel(); return renderApp(() => <App />, root);',
  );
  await writeFile(path.join(directory, "src/main.tsx"), source);
  await mkdir(path.join(directory, "assets"));
  await writeFile(path.join(directory, "assets/message.txt"), "Native text asset\n");
  const { dist, manifest } = await buildApplication("fixture", { appDirectory: directory });
  assert.deepEqual(manifest.assets, ["assets/message.txt"]);
  assert.equal(await readFile(path.join(dist, manifest.assets[0]), "utf8"), "Native text asset\n");
  const info = JSON.parse(await readFile(path.join(dist, "build-info.json"), "utf8"));
  assert.equal(info.graph.modules.filter(input => /\/solid-js\/dist\/(?:solid|dev)\.js$/.test(input)).length, 1);
  const code = await readFile(path.join(dist, manifest.entry), "utf8");
  assert.match(code, /globalThis\.__zguiServices\.readTextAsset/);
  validateModule(code, { bundled: true, entry: true });
  assert.equal(globalThis.__zguiServices, undefined);
  const module = await import(pathToFileURL(path.join(await realpath(dist), manifest.entry)));
  assert.equal(typeof module.mount, "function", "service binding is not accessed before mount");
});

test("type errors, bad imports, and failed publication preserve the last complete generation", async t => {
  const directory = await fixture(t);
  const { dist, manifest } = await buildApplication("fixture", { appDirectory: directory });
  const target = await readlink(dist);
  await writeFile(path.join(directory, "src/main.tsx"), validEntry + '\nconst wrong: number = "not a number";\n');
  await assert.rejects(buildApplication("fixture", { appDirectory: directory }), /not assignable/);
  assert.equal(await readlink(dist), target);
  for (const specifier of ["solid-js/web", "solid-js/web/dist/server.js", "solid-js/dist/server.js", "node:fs", "fs", "./app.css"]) {
    await writeFile(path.join(directory, "src/main.tsx"), `import ${JSON.stringify(specifier)};\n${validEntry}`);
    await assert.rejects(bundle(path.join(directory, "src/main.tsx")), /Incompatible runtime import|Runtime resource imports/);
  }
  await assert.rejects(publishGeneration(directory, new Map([["app.mjs", Buffer.from("partial")], ["../escaped", Buffer.from("bad")]])), /Non-canonical resource/);
  assert.equal(await readlink(dist), target);
  assert.equal(JSON.parse(await readFile(path.join(dist, "manifest.json"), "utf8")).buildId, manifest.buildId);
  assert.ok((await readdir(path.join(directory, ".zgui-generations"))).every(name => !name.startsWith(".building-")));
});

test("asset symlinks are rejected before publication", async t => {
  const directory = await fixture(t);
  await mkdir(path.join(directory, "assets"));
  await symlink("../src/main.tsx", path.join(directory, "assets/link"));
  await assert.rejects(buildApplication("fixture", { appDirectory: directory }), /regular files/);
  await assert.rejects(readlink(path.join(directory, "dist")), { code: "ENOENT" });
});

test("app-local dependencies resolve browser/production exports without external imports", async t => {
  const directory = await fixture(t);
  const dependency = path.join(directory, "node_modules/native-fixture");
  await mkdir(dependency, { recursive: true });
  await writeFile(path.join(dependency, "package.json"), JSON.stringify({
    name: "native-fixture", type: "module",
    exports: { browser: { production: "./native.js", default: "./wrong.js" }, default: "./wrong.js" },
  }));
  await writeFile(path.join(dependency, "native.js"), 'export const message = "browser-production-branch";');
  await writeFile(path.join(dependency, "wrong.js"), 'throw new Error("wrong-resolution-branch");');
  const source = path.join(directory, "src/main.tsx");
  await writeFile(source, `import { message } from "native-fixture";\n${validEntry.replace("Count:", "{message}")}`);
  const result = await bundle(source, { appDirectory: directory });
  assert.match(result.files.get("app.mjs").toString(), /browser-production-branch/);
  assert.ok(result.graph.modules.includes(path.join(dependency, "native.js")));
  assert.ok(!result.graph.modules.includes(path.join(dependency, "wrong.js")));
  assert.deepEqual(result.graph.chunks[0].imports, []);
});

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

test("watch rebuild queue coalesces bursts and serializes edits arriving during a build", async () => {
  const firstStarted = deferred();
  const releaseFirst = deferred();
  const secondFinished = deferred();
  let calls = 0;
  let active = 0;
  const queue = rebuildQueue(async () => {
    assert.equal(++active, 1);
    calls++;
    if (calls === 1) { firstStarted.resolve(); await releaseFirst.promise; }
    active--;
    return calls;
  }, { delay: 0, onSuccess: result => { if (result === 2) secondFinished.resolve(); } });
  try {
    queue.request(); queue.request(); queue.request();
    await firstStarted.promise;
    queue.request(); queue.request();
    releaseFirst.resolve();
    await secondFinished.promise;
    assert.equal(calls, 2);
  } finally { await queue.close(); }
});

test("development watcher retains the good generation on errors and publishes recovery", { timeout: 30000 }, async t => {
  const directory = await fixture(t);
  const statusPath = path.join(directory, ".zgui-build-status.json");
  const readStatus = () => {
    const bytes = readFileSync(statusPath);
    assert.ok(bytes.length <= 16 * 1024);
    return JSON.parse(bytes);
  };
  const initial = deferred();
  const failed = deferred();
  const recovered = deferred();
  let firstBuildId;
  const watcher = watchApplication("fixture", {
    appDirectory: directory,
    onSuccess(result) {
      if (!firstBuildId) {
        firstBuildId = result.manifest.buildId;
        initial.resolve({ result, status: readStatus() });
      } else if (result.manifest.buildId !== firstBuildId) recovered.resolve({ result, status: readStatus() });
    },
    onError(error) { failed.resolve({ error, status: readStatus() }); },
  });
  try {
    const { result: { dist }, status: published } = await initial.promise;
    const target = await readlink(dist);
    const previousDirectory = await realpath(dist);
    assert.match(published.attemptId, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    assert.deepEqual(published, {
      schemaVersion: 1, attemptId: published.attemptId, app: "fixture", status: "published",
      previousDirectory: null, publicationDirectory: previousDirectory, buildId: firstBuildId, error: null,
    });
    const invalidDeclarations = Array.from({ length: 64 }, (_, index) => `const invalid${index}: number = "bad";`).join("\n");
    await writeFile(path.join(directory, "src/main.tsx"), `${validEntry}\n${invalidDeclarations}\n`);
    const { error, status: failure } = await failed.promise;
    assert.match(error.message, /not assignable/);
    assert.ok(error.message.length > 2048);
    assert.notEqual(failure.attemptId, published.attemptId);
    assert.match(failure.attemptId, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    assert.deepEqual(failure, {
      schemaVersion: 1, attemptId: failure.attemptId, app: "fixture", status: "failed",
      previousDirectory, publicationDirectory: null, buildId: null,
      error: { code: "E_BUILD", message: error.message.slice(0, 2048) },
    });
    assert.equal(await readlink(dist), target);
    assert.equal(JSON.parse(await readFile(path.join(dist, "manifest.json"), "utf8")).buildId, firstBuildId);
    await writeFile(path.join(directory, "src/main.tsx"), validEntry.replace("Count:", "Total:"));
    const { result, status: recovery } = await recovered.promise;
    assert.notEqual(result.manifest.buildId, firstBuildId);
    assert.notEqual(await readlink(dist), target);
    assert.equal(JSON.parse(await readFile(path.join(dist, "manifest.json"), "utf8")).buildId, result.manifest.buildId);
    assert.notEqual(recovery.attemptId, failure.attemptId);
    assert.deepEqual(recovery, {
      schemaVersion: 1, attemptId: recovery.attemptId, app: "fixture", status: "published",
      previousDirectory, publicationDirectory: await realpath(dist), buildId: result.manifest.buildId, error: null,
    });
    assert.deepEqual((await readdir(directory)).filter(name => name.startsWith(".zgui-build-status.json.")), []);
  } finally { await watcher.close(); }
});

test("development watcher surfaces status publication failure instead of reporting success", { timeout: 30000 }, async t => {
  const directory = await fixture(t);
  await mkdir(path.join(directory, ".zgui-build-status.json"));
  const failed = deferred();
  let successes = 0;
  const watcher = watchApplication("fixture", {
    appDirectory: directory,
    onSuccess() { successes++; },
    onError(error) { failed.resolve(error); },
  });
  try {
    const error = await failed.promise;
    assert.equal(error.syscall, "rename");
    assert.match(error.message, /\.zgui-build-status\.json/);
    assert.equal(successes, 0);
    await assert.rejects(realpath(path.join(directory, "dist")), { code: "ENOENT" });
    assert.deepEqual((await readdir(directory)).filter(name => name.startsWith(".zgui-build-status.json.")), []);
  } finally { await watcher.close(); }
});

test("native acceptance status files do not trigger another publication", { timeout: 30000 }, async t => {
  const directory = await fixture(t);
  const initial = deferred();
  let successes = 0;
  const watcher = watchApplication("fixture", {
    appDirectory: directory,
    onSuccess() { successes++; initial.resolve(); },
    onError(error) { throw error; },
  });
  try {
    await initial.promise;
    await writeFile(path.join(directory, ".zgui-native-status.json"), "{}\n");
    await writeFile(path.join(directory, ".zgui-native-status.json.123.tmp"), "{}\n");
    await new Promise(resolve => setTimeout(resolve, 300));
    assert.equal(successes, 1);
  } finally { await watcher.close(); }
});

test("persistent compiler caches real Babel transforms, prunes modules, and recovers after syntax errors", { timeout: 30000 }, async t => {
  const directory = await fixture(t);
  const source = path.join(directory, "src/main.tsx");
  const dependency = path.join(directory, "src/message.ts");
  const dependencySource = value => `export const message: string = ${JSON.stringify(value)};`;
  const mainSource = suffix => `import { message } from "./message";\n${validEntry.replace("Count:", `{message}${suffix}`)}`;
  await writeFile(dependency, dependencySource("incremental-one"));
  await writeFile(source, mainSource("first"));
  const { transformAsync } = require("@babel/core");
  let transforms = 0;
  let next = Promise.withResolvers();
  const take = async () => {
    const result = await next.promise;
    next = Promise.withResolvers();
    return result;
  };
  const start = () => watchBundle(source, {
    root: workspaceRoot,
    appDirectory: directory,
    temporaryDirectory: path.join(directory, "compiler-output"),
    buildDelay: 20,
    babelTransform(source, options) {
      if (options.filename === dependency) transforms++;
      return transformAsync(source, options);
    },
    onCompile(result) { next.resolve({ result }); },
    onError(error) { next.resolve({ error }); },
  });
  let compiler = start();
  try {
    assert.ok((await take()).result);
    assert.equal(transforms, 1);
    await writeFile(source, mainSource("second"));
    assert.match((await take()).result.files.get("app.mjs").toString(), /second/);
    assert.equal(transforms, 1, "the persistent compiler must retain unchanged Babel transforms");
    await writeFile(dependency, dependencySource("incremental-two"));
    assert.match((await take()).result.files.get("app.mjs").toString(), /incremental-two/);
    assert.equal(transforms, 2);
    await writeFile(source, "export const broken = ;");
    assert.ok((await take()).error);
    await writeFile(source, mainSource("recovered"));
    assert.match((await take()).result.files.get("app.mjs").toString(), /recovered/);
    const beforePruning = transforms;
    await writeFile(source, validEntry.replace("Count:", "detached"));
    assert.ok(!(await take()).result.graph.modules.includes(dependency));
    await writeFile(source, mainSource("reattached"));
    assert.match((await take()).result.files.get("app.mjs").toString(), /reattached/);
    assert.equal(transforms, beforePruning + 1, "removed modules must not retain cached transforms");
    await compiler.close();
    const beforeRestart = transforms;
    compiler = start();
    assert.ok((await take()).result);
    assert.equal(transforms, beforeRestart + 1, "closing a compiler clears its transform cache");
  } finally { await compiler.close(); }
});

test("watchApplication orchestrates deterministic status transitions and errors through its adapter seam", async t => {
  const directory = await fixture(t);
  const statuses = [], successes = [], errors = [];
  let handlers, rejectTypecheck = false, publishCount = 0;
  let driverReady = Promise.withResolvers();
  const fakeWatchBundle = (_entry, options) => {
    handlers = options;
    driverReady.resolve();
    return { close: async () => {} };
  };
  const runCycle = async result => {
    try {
      await handlers.onAttempt();
      await handlers.onCompile(result);
    } catch (error) {
      await handlers.onError(error);
    }
  };
  const fakePublish = async (app, result, { appDirectory }) => {
    publishCount += 1;
    const dist = await publishGeneration(appDirectory, result.files);
    return { dist, manifest: { buildId: `sha256:${String(publishCount).padStart(64, "0")}`, app } };
  };
  const fakeTypecheck = async () => {
    if (rejectTypecheck) throw new Error("synthetic TS2322 type failure");
  };
  const watcher = watchApplication("fixture", {
    appDirectory: directory,
    onSuccess: value => successes.push(value),
    onError: error => errors.push(error),
  }, {
    watchBundle: fakeWatchBundle,
    typecheck: fakeTypecheck,
    publish: fakePublish,
    publishStatus: async (appDirectory, value) => {
      statuses.push(value);
      await publishBuildStatus(appDirectory, value);
    },
  });
  try {
    await driverReady.promise;
    const compiled = {
      files: new Map([["app.mjs", Buffer.from("export const hostApiVersion = 1;")]]),
      graph: { modules: [path.join(directory, "src/main.tsx")], chunks: [] },
    };
    await runCycle(compiled);
    const firstDist = await realpath(path.join(directory, "dist"));
    assert.equal(successes.length, 1);
    assert.deepEqual(statuses.map(entry => [entry.status, entry.previousDirectory]), [
      ["building", null],
      ["published", null],
    ]);

    rejectTypecheck = true;
    await runCycle(compiled);
    assert.equal(successes.length, 1);
    assert.equal(publishCount, 1, "failed typecheck must not publish a package");
    assert.equal(errors.at(-1)?.message, "synthetic TS2322 type failure");
    assert.equal(await realpath(path.join(directory, "dist")), firstDist);
    assert.deepEqual(statuses.slice(2).map(entry => [entry.status, entry.previousDirectory]), [
      ["building", firstDist],
      ["failed", firstDist],
    ]);

    rejectTypecheck = false;
    await runCycle({ ...compiled, files: new Map([["app.mjs", Buffer.from("export const hostApiVersion = 1; // v2")]]) });
    const recoveredDist = await realpath(path.join(directory, "dist"));
    assert.notEqual(recoveredDist, firstDist);
    assert.equal(successes.length, 2);
    assert.equal(statuses.at(-1).status, "published");
    assert.equal(statuses.at(-1).previousDirectory, firstDist);

    await rm(path.join(directory, ".zgui-build-status.json"), { force: true });
    await mkdir(path.join(directory, ".zgui-build-status.json"));
    await runCycle(compiled);
    assert.equal(successes.length, 2, "status publication failure must not report success");
    assert.equal(errors.at(-1)?.syscall, "rename");
  } finally {
    await watcher.close();
  }
});

test("development watcher publishes added resources and external dependency changes", { timeout: 30000 }, async t => {
  const directory = await fixture(t);
  const external = await mkdtemp(path.join(workspaceRoot, "tests/build/.dependency-"));
  t.after(() => rm(external, { recursive: true, force: true }));
  const dependency = path.join(external, "message.ts");
  await writeFile(dependency, 'export const message = "external-one";');
  const specifier = path.relative(path.join(directory, "src"), dependency).replace(/\.ts$/, "");
  await writeFile(path.join(directory, "src/main.tsx"), `import { message } from ${JSON.stringify(specifier)};\n${validEntry.replace("Count:", "{message}")}`);
  let next = Promise.withResolvers();
  const take = async () => {
    const result = await next.promise;
    next = Promise.withResolvers();
    return result;
  };
  const watcher = watchApplication("fixture", {
    appDirectory: directory,
    onSuccess(result) { next.resolve(result); },
    onError(error) { next.reject(error); },
  });
  try {
    const initial = await take();
    assert.match(await readFile(path.join(initial.dist, "app.mjs"), "utf8"), /external-one/);
    await writeFile(dependency, 'export const message = "external-two";');
    const changed = await take();
    assert.match(await readFile(path.join(changed.dist, "app.mjs"), "utf8"), /external-two/);
    await mkdir(path.join(directory, "assets"));
    await writeFile(path.join(directory, "assets/new.txt"), "new resource");
    let resources;
    do { resources = await take(); } while (!resources.manifest.assets.includes("assets/new.txt"));
    assert.equal(await readFile(path.join(resources.dist, "assets/new.txt"), "utf8"), "new resource");
    await writeFile(path.join(directory, "src/app.css"), "text { color: red; }");
    let styled;
    do { styled = await take(); } while (!styled.manifest.styles.includes("base.css"));
    assert.deepEqual(styled.manifest.styles, ["base.css", "app.css"]);
    assert.equal(await readFile(path.join(styled.dist, "base.css"), "utf8"), "text { color: red; }");
  } finally { await watcher.close(); }
  await watcher.close();
});

test("async development typechecking preserves diagnostics and reaps cancellation", async t => {
  const directory = await fixture(t);
  const config = path.join(directory, "tsconfig.json");
  await typecheckProjectAsync(config);
  await writeFile(path.join(directory, "src/main.tsx"), `${validEntry}\nconst wrong: number = "bad";`);
  await assert.rejects(typecheckProjectAsync(config), /not assignable/);
  const controller = new AbortController();
  const checking = typecheckProjectAsync(config, { signal: controller.signal });
  controller.abort();
  await assert.rejects(checking, { name: "AbortError" });
});

test("text controls reject children introduced by a reactive spread before evaluating them", async t => {
  const directory = await fixture(t);
  await writeFile(path.join(directory, "src/main.tsx"), `
    import { TextField, TextArea, createSignal, flush } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { mergeProps, render } from "@hewel/zgui/renderer";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) { return render(() => undefined, root); }
    export function probe(root: NativeNode, multiline: boolean) {
      let reads = 0;
      let replace = () => {};
      const dispose = render(() => {
        const [props, setProps] = createSignal({ value: "seed" });
        const Control = multiline ? TextArea : TextField;
        Control(mergeProps(props));
        replace = () => setProps(Object.defineProperty({ value: "next" }, "children", {
          enumerable: true, get() { reads++; return "forbidden"; }
        }));
        return undefined;
      }, root);
      let rejected = false;
      try { replace(); flush(); } catch (error) { rejected = error instanceof TypeError; }
      finally { dispose(); }
      return { rejected, reads };
    }
  `);
  const { dist } = await buildApplication("fixture", { appDirectory: directory });
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zgui");
  try {
    // Only the SDK's shape guard is under test; no native editing is simulated.
    Object.defineProperty(globalThis, "__zgui", { configurable: true, value: {
      createElement: tag => ({ tag }),
      setProperty: (node, name, value) => { node[name] = value; },
    } });
    const module = await import(pathToFileURL(path.join(await realpath(dist), "app.mjs")));
    for (const multiline of [false, true]) {
      assert.deepEqual(module.probe({}, multiline), { rejected: true, reads: 0 });
    }
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zgui", previous);
    else delete globalThis.__zgui;
  }
});

test("static component stylesheets hoist into packaged components.css in layer order without breaking non-host bundles", async t => {
  const directory = await fixture(t);
  await writeFile(path.join(directory, "src/app.css"), "text { color: #222; }\n");
  await writeFile(path.join(directory, "src/main.tsx"), `
    import { renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { Button, ThemeProvider } from "@hewel/zgui/components";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => <ThemeProvider><Button>Save</Button></ThemeProvider>, root);
    }
  `);

  const published = await buildApplication("hoisted-components", { appDirectory: directory });
  assert.deepEqual(published.manifest.styles, ["components.css", "base.css", "app.css"]);
  const componentsCss = await readFile(path.join(published.dist, "components.css"), "utf8");
  assert.match(componentsCss, /^@layer zgui\.components \{/);
  assert.match(componentsCss, /\/\* ButtonStyle \*\//);
  assert.doesNotMatch(componentsCss, /\/\* CalendarStyle \*\//, "unreachable component styles must stay excluded");
  const appJs = await readFile(path.join(published.dist, "app.mjs"), "utf8");
  assert.doesNotMatch(appJs, /--zui-button-ring:/, "ButtonStyle CSS text must be hoisted out of app.mjs");

  const nonHost = await bundle(path.join(directory, "src/main.tsx"), {
    appDirectory: directory,
    hostEntry: false,
  });
  assert.equal(nonHost.files.has("components.css"), false, "hostEntry: false keeps component styles inline for unit harnesses");
  assert.match(nonHost.files.get("app.mjs").toString(), /--zui-button-ring:/);
});

