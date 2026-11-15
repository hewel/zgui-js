import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { mkdtemp, mkdir, writeFile, readFile, readlink, realpath, rm, readdir, symlink } from "node:fs/promises";
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";
import path from "node:path";
import { buildApplication, bundle, contentDigest, publishGeneration, workspaceRoot } from "../../packages/build/src/build.mjs";
import { validateModule } from "../../packages/build/src/validate.mjs";
import { rebuildQueue, watchApplication } from "../../packages/build/src/watch.mjs";
import { typecheckProject, typecheckProjectAsync } from "../../packages/build/src/typecheck.mjs";
import { compilerOptions, outputOptions, compilationResult } from "../../packages/build/src/compiler.mjs";

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
  validateModule(`${entry}\nconst message = 'import(\"node:fs\") and await';`, { bundled: true, entry: true });
  assert.throws(() => validateModule(`${entry}\nawait Promise.resolve();`), /Top-level await/);
  assert.throws(() => validateModule(`${entry}\nfor await (const x of []) {}`), /Top-level await/);
  assert.throws(() => validateModule(`${entry}\nclass C { [await Promise.resolve()]() {} }`), /Top-level await/);
  assert.throws(() => validateModule(`${entry}\nfunction lazy() { return import('./lazy.js'); }`), /Dynamic imports/);
  assert.throws(() => validateModule(`${entry}\nimport 'unresolved';`, { bundled: true }), /unresolved runtime import/);
  assert.throws(() => validateModule(`${entry}\nexport * from 'external';`, { bundled: true }), /external re-export/);
  assert.throws(() => validateModule(`${entry}\nconsole.log(document);`, { bundled: true }), /Unsupported runtime global: document/);
  assert.throws(() => validateModule(`${entry}\nrequire('fs');`, { bundled: true }), /Runtime require/);
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

test("real TSX bundles have one reactive runtime, composed maps, resources, and verifiable identity", async t => {
  const directory = await fixture(t);
  await writeFile(path.join(directory, "src/app.css"), "text { color: #123456; }\n");
  await mkdir(path.join(directory, "assets/icons"), { recursive: true });
  await writeFile(path.join(directory, "assets/icons/mark.bin"), Buffer.from([0, 255, 1, 42]));
  const { dist, manifest } = await buildApplication("fixture", { appDirectory: directory });
  assert.equal(manifest.hostApiVersion, 1);
  assert.deepEqual(manifest.styles, ["app.css"]);
  assert.deepEqual(manifest.assets, ["assets/icons/mark.bin"]);
  const root = await realpath(dist);
  const names = [manifest.entry, manifest.sourceMap, ...manifest.styles, ...manifest.assets].sort();
  const files = new Map(await Promise.all(names.map(async name => [name, await readFile(path.join(root, name))])));
  // Recompute independently, as the Rust loader does, rather than trusting the compiler helper.
  const digest = createHash("sha256");
  for (const name of names) digest.update(name).update("\0").update(files.get(name)).update("\0");
  assert.equal(manifest.buildId, `sha256:${digest.digest("hex")}`);
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
  assert.equal(info.packages["solid-js"], "1.9.19");
  assert.equal(info.packages["babel-preset-solid"], "1.9.19");
  assert.equal(info.engine.name, "QuickJS-NG");
  assert.equal(info.packages.rolldown, "1.3.2");
  assert.equal(info.graph.chunks.length, 1);
  assert.deepEqual(info.graph.chunks[0].imports, []);
  assert.deepEqual(info.graph.chunks[0].dynamicImports, []);
  assert.match(info.engine.revision, /^[a-f0-9]{40}$/);
  assert.match(info.lockfile.digest, /^sha256:[a-f0-9]{64}$/);
  assert.equal(contentDigest(new Map([...files, ["build-info.json", Buffer.from("not a resource")]]), manifest), manifest.buildId);
  const firstTarget = await readlink(dist);
  const second = await buildApplication("fixture", { appDirectory: directory });
  assert.equal(second.manifest.buildId, manifest.buildId);
  assert.notEqual(await readlink(dist), firstTarget);
  assert.equal(await readFile(path.join(root, "app.mjs"), "utf8"), code, "old reader remains pinned to a complete generation");
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

test("persistent compiler caches real Babel transforms, prunes modules, and recovers after syntax errors", { timeout: 30000 }, async t => {
  const directory = await fixture(t);
  const source = path.join(directory, "src/main.tsx");
  const dependency = path.join(directory, "src/message.ts");
  const dependencySource = value => `export const message: string = ${JSON.stringify(value)};`;
  const mainSource = suffix => `import { message } from "./message";\n${validEntry.replace("Count:", `{message}${suffix}`)}`;
  await writeFile(dependency, dependencySource("incremental-one"));
  await writeFile(source, mainSource("first"));
  const { watch } = await import(require.resolve("rolldown"));
  const { transformAsync } = require("@babel/core");
  let transforms = 0;
  let next = Promise.withResolvers();
  const take = async () => {
    const result = await next.promise;
    next = Promise.withResolvers();
    return result;
  };
  const options = compilerOptions(source, {
    root: workspaceRoot, appDirectory: directory,
    babelTransform(source, options) {
      if (options.filename === dependency) transforms++;
      return transformAsync(source, options);
    },
    plugins: [{
      name: "observe-incremental-output",
      generateBundle(_options, output) {
        const result = compilationResult(Object.values(output), this.getModuleIds());
        next.resolve({ result });
        for (const filename of Object.keys(output)) delete output[filename];
      },
    }],
  });
  const start = () => watch({
    ...options,
    output: { ...outputOptions(), dir: path.join(directory, "compiler-output") },
    watch: { buildDelay: 20 },
  }).on("event", async event => {
    if (event.code === "ERROR") next.resolve({ error: event.error });
    if (event.code === "ERROR" || event.code === "BUNDLE_END") await event.result?.close();
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
    do { styled = await take(); } while (!styled.manifest.styles.includes("app.css"));
    assert.equal(await readFile(path.join(styled.dist, "app.css"), "utf8"), "text { color: red; }");
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
    import { TextField, TextArea, createSignal } from "@hewel/zgui";
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
      try { replace(); } catch (error) { rejected = error instanceof TypeError; }
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
