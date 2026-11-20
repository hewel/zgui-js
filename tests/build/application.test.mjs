import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFile, spawn } from "node:child_process";
import { cp, mkdtemp, mkdir, writeFile, readFile, rename, rm, symlink } from "node:fs/promises";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import { test } from "node:test";
import { resolveApplication } from "../../packages/build/src/application.ts";
import { bundle, publishApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import { typecheckProject } from "../../packages/build/src/typecheck.ts";

const execute = promisify(execFile);
const cli = path.join(workspaceRoot, "packages/build/src/cli.ts");

async function publicBuild(directory) {
  const modules = path.join(directory, "node_modules/@hewel");
  await mkdir(modules, { recursive: true });
  await symlink(path.join(workspaceRoot, "packages/build"), path.join(modules, "zgui-build"), "dir");
  const consumer = path.join(directory, "build-consumer.mjs");
  await writeFile(consumer, `export * from "@hewel/zgui-build";\n`);
  return import(consumer);
}

test("the build package exposes only its programmatic API, without CLI side effects", async t => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "zgui-build-api-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const listeners = process.listeners("SIGINT");
  const api = await publicBuild(directory);
  assert.deepEqual(Object.keys(api).sort(), [
    "buildApplication", "devApplication", "prepareNativeHost", "previewApplication",
    "resolveApplication", "runNativeHost", "watchApplication",
  ]);
  assert.deepEqual(process.listeners("SIGINT"), listeners);
});

async function externalApplication(t, prefix = path.join(os.tmpdir(), "zgui app ")) {
  const directory = await mkdtemp(prefix);
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(path.join(directory, "src"));
  const modules = path.join(directory, "node_modules/@hewel");
  await mkdir(modules, { recursive: true });
  await symlink(path.join(workspaceRoot, "packages/ui"), path.join(modules, "zgui"), "dir");
  await symlink(path.join(workspaceRoot, "packages/services"), path.join(modules, "zgui-services"), "dir");
  await writeFile(path.join(directory, "package.json"), JSON.stringify({ name: "@consumer/custom-app", type: "module" }));
  await writeFile(path.join(directory, "tsconfig.json"), JSON.stringify({
    extends: "@hewel/zgui/tsconfig.native.json", include: ["src"],
  }));
  await writeFile(path.join(directory, "src/main.tsx"), `import { renderApp, type NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) { return renderApp(() => <text>External application</text>, root); }
`);
  return directory;
}

const strictConsumerOptions = { strict: true, noUncheckedIndexedAccess: true };

test("external consumers typecheck the source-linked build package without packing it", async t => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "zgui-build-types-consumer-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const modules = path.join(directory, "node_modules/@hewel");
  await mkdir(modules, { recursive: true });
  await symlink(path.join(workspaceRoot, "packages/build"), path.join(modules, "zgui-build"), "dir");
  await writeFile(path.join(directory, "package.json"), JSON.stringify({ type: "module" }));
  await writeFile(path.join(directory, "public-api.ts"),
    await readFile(path.join(workspaceRoot, "packages/build/tests/public-api.ts")));
  const config = path.join(directory, "tsconfig.json");
  await writeFile(config, JSON.stringify({
    compilerOptions: {
      ...strictConsumerOptions, target: "ESNext", module: "ESNext", moduleResolution: "Bundler", types: [],
    },
    include: ["public-api.ts"],
  }));
  typecheckProject(config);
});

test("external consumers typecheck all linked SDK entry points with unchecked indexed access", async t => {
  const directory = await externalApplication(t);
  await symlink(path.join(workspaceRoot, "packages/build"), path.join(directory, "node_modules/@hewel/zgui-build"), "dir");
  await writeFile(path.join(directory, "src/build-api.ts"), `export { buildApplication } from "@hewel/zgui-build";\n`);
  await writeFile(path.join(directory, "src/main.tsx"), `import { renderApp, type NativeNode } from "@hewel/zgui";
import { createElement } from "@hewel/zgui/renderer";
import { Button } from "@hewel/zgui/components";
import { CHECK, Icon } from "@hewel/zgui/icons";
import { getResolvedCapabilities } from "@hewel/zgui-services";

export function mount(root: NativeNode) {
  const label = getResolvedCapabilities().imagePrefixes[0] ?? "Available";
  return renderApp(() => [
    createElement("text"),
    Button({ children: Icon({ icon: CHECK, label }) }),
  ], root);
}
`);
  const config = path.join(directory, "tsconfig.json");
  await writeFile(config, JSON.stringify({
    extends: "@hewel/zgui/tsconfig.native.json",
    compilerOptions: { ...strictConsumerOptions, skipLibCheck: true },
    include: ["src"],
  }));
  typecheckProject(config);
});

test("application-local build supports directories outside the workspace with spaces", async t => {
  const directory = await externalApplication(t);
  const { stdout } = await execute(process.execPath, [cli, "build"], { cwd: directory, timeout: 30000 });
  const manifest = JSON.parse(await readFile(path.join(directory, "dist/manifest.json"), "utf8"));
  assert.equal(manifest.id, "dev.example.custom-app");
  assert.equal(manifest.schemaVersion, 2);
  assert.deepEqual(manifest.styles, ["app.css"]);
  assert.deepEqual(manifest.fonts, []);
  assert.equal(manifest.styleSystem.profile, "zgui-stylex-v1");
  assert.match(manifest.styleSystem.nativeProfileDigest, /^sha256:[a-f0-9]{64}$/);
  assert.ok(manifest.artifactDigests["style-origins.json"]);
  assert.match(stdout, new RegExp(manifest.buildId));
  assert.match(await readFile(path.join(directory, "dist/app.mjs"), "utf8"), /External application/);
  assert.equal(Object.hasOwn(manifest, "capabilities"), false);
});

test("build provenance reports the Solid packages bundled from the consumer", async t => {
  const directory = await externalApplication(t);
  const modules = path.join(directory, "node_modules");
  const ui = path.join(modules, "@hewel/zgui");
  const sdk = path.join(workspaceRoot, "packages/ui");
  await rm(ui);
  await mkdir(ui);
  await cp(path.join(sdk, "src"), path.join(ui, "src"), { recursive: true });
  for (const filename of ["package.json", "tsconfig.native.json"])
    await cp(path.join(sdk, filename), path.join(ui, filename));

  const sdkRequire = createRequire(path.join(sdk, "package.json"));
  const solidRequire = createRequire(sdkRequire.resolve("solid-js/package.json"));
  const versions = {
    "solid-js": "2.0.81",
    "@solidjs/universal": "2.0.82",
    "@solidjs/signals": "2.0.83",
  };
  for (const [name, version] of Object.entries(versions)) {
    const resolve = name === "@solidjs/signals" ? solidRequire : sdkRequire;
    const installed = path.join(modules, name);
    await mkdir(path.dirname(installed), { recursive: true });
    await cp(path.dirname(resolve.resolve(`${name}/package.json`)), installed, { recursive: true });
    const filename = path.join(installed, "package.json");
    await writeFile(filename, JSON.stringify({ ...JSON.parse(await readFile(filename, "utf8")), version }));
  }

  const result = await bundle(path.join(directory, "src/main.tsx"), { appDirectory: directory });
  const { dist } = await publishApplication("custom-app", result, { root: workspaceRoot, appDirectory: directory });
  const { packages } = JSON.parse(await readFile(path.join(dist, "build-info.json"), "utf8"));
  for (const [name, version] of Object.entries(versions)) assert.equal(packages[name], version, name);
});

test("an installed build package builds, watches, and launches hosts from a sibling repository", { timeout: 30000 }, async t => {
  const directory = await externalApplication(t, path.resolve(workspaceRoot, "../.zgui-consumer-"));
  const tarball = "zgui-build.tgz";
  await execute(process.execPath, ["pm", "pack", "--filename", path.join(directory, tarball)], {
    cwd: path.join(workspaceRoot, "packages/build"),
  });
  const installed = path.join(directory, "node_modules/@hewel/zgui-build");
  await mkdir(installed);
  await execute("tar", ["-xf", path.join(directory, tarball), "--strip-components=1", "-C", installed]);
  const { dependencies } = JSON.parse(await readFile(path.join(installed, "package.json"), "utf8"));
  for (const name of Object.keys(dependencies)) {
    const target = path.join(installed, "node_modules", name);
    await mkdir(path.dirname(target), { recursive: true });
    await symlink(path.join(workspaceRoot, "packages/build/node_modules", name), target, "dir");
  }
  await writeFile(path.join(directory, "src/main.tsx"), `import { renderApp, type NativeNode } from "@hewel/zgui";
import { getResolvedCapabilities } from "@hewel/zgui-services";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  const { imagePrefixes } = getResolvedCapabilities();
  return renderApp(() => <text>{imagePrefixes.join(",")}</text>, root);
}
`);
  const consumer = path.join(directory, "driver", "index.mjs");
  await mkdir(path.dirname(consumer));
  await writeFile(consumer, `export * from "@hewel/zgui-build";\n`);
  await writeFile(path.join(directory, "driver", "public-api.ts"),
    await readFile(path.join(workspaceRoot, "packages/build/tests/public-api.ts")));
  const driverConfig = path.join(directory, "driver", "tsconfig.json");
  await writeFile(driverConfig, JSON.stringify({
    compilerOptions: { strict: true, target: "ESNext", module: "ESNext", moduleResolution: "Bundler", types: [] },
    include: ["public-api.ts"],
  }));
  typecheckProject(driverConfig);
  const { buildApplication, prepareNativeHost, watchApplication } = await import(consumer);
  const appDirectory = directory;
  const { dist, manifest } = await buildApplication("custom-app", { appDirectory });
  const info = JSON.parse(await readFile(path.join(dist, "build-info.json"), "utf8"));
  assert.equal(manifest.id, "dev.example.custom-app");
  assert.equal(info.packages["@solidjs/universal"], "2.0.1");
  assert.ok(info.graph.modules.some(module => module.startsWith("packages/@hewel/zgui-services/")));
  for (const [lock, filename] of [[info.lockfile, "bun.lock"], [info.cargoLock, "Cargo.lock"]]) {
    const digest = createHash("sha256").update(await readFile(path.join(workspaceRoot, filename))).digest("hex");
    assert.equal(lock.digest, `sha256:${digest}`);
  }

  const hostRoot = path.join(directory, "host checkout");
  const bin = path.join(directory, "bin");
  await mkdir(path.join(hostRoot, "target"), { recursive: true });
  await mkdir(bin);
  await assert.rejects(prepareNativeHost(), /installed SDK has no Cargo workspace/);
  await writeFile(path.join(bin, "cargo"), `#!${process.execPath}
if (process.cwd() !== ${JSON.stringify(hostRoot)}) process.exit(72);
console.log(JSON.stringify({
  reason: "compiler-artifact", target: { name: "native-host", kind: ["bin"] },
  executable: "target/native-host",
}));
`, { mode: 0o755 });
  const hostScript = `#!${process.execPath}
const args = process.argv.slice(2);
const manifest = await Bun.file(args[args.indexOf("--bundle") + 1] + "/manifest.json").json();
console.log(JSON.stringify({ script: import.meta.path, cwd: process.cwd(), args, styleSystem: manifest.styleSystem }));
`;
  const preparedHost = path.join(hostRoot, "target/native-host");
  const prebuiltHost = path.join(bin, "prebuilt host");
  await writeFile(preparedHost, hostScript, { mode: 0o755 });
  await writeFile(prebuiltHost, hostScript, { mode: 0o755 });
  const installedCli = path.join(installed, "src/cli.ts");
  const invoke = async (...args) => JSON.parse((await execute(process.execPath, [installedCli, ...args], {
    cwd: directory,
    env: { ...process.env, PATH: `${bin}${path.delimiter}${process.env.PATH}` },
    timeout: 10000,
  })).stdout);
  const dev = await invoke("dev", "--host-root", path.relative(directory, hostRoot));
  assert.equal(dev.script, preparedHost);
  assert.equal(dev.cwd, hostRoot);
  assert.deepEqual(dev.args, ["--bundle", dist, "--watch"]);
  const oneShot = await invoke("preview", "--host-root", path.relative(directory, hostRoot));
  assert.equal(oneShot.script, preparedHost);
  assert.equal(oneShot.cwd, hostRoot);
  assert.deepEqual(oneShot.args.slice(0, 4), ["preview", "run", "--bundle", dist]);
  const watched = await invoke("preview", "--watch", "--host-root", path.relative(directory, hostRoot),
    "--host-executable", path.relative(directory, prebuiltHost));
  assert.equal(watched.script, prebuiltHost);
  assert.equal(watched.cwd, hostRoot);
  assert.deepEqual(watched.args.slice(0, 4), ["preview", "watch", "--bundle", dist]);
  assert.notEqual(oneShot.styleSystem.configDigest, dev.styleSystem.configDigest);
  assert.deepEqual(watched.styleSystem, oneShot.styleSystem);

  const overlayDirectory = await mkdtemp(path.join(os.tmpdir(), "zgui-overlays-"));
  t.after(() => rm(overlayDirectory, { recursive: true, force: true }));
  const capabilitiesFile = path.join(overlayDirectory, "composed.json");
  const update = async capabilities => {
    await writeFile(`${capabilitiesFile}.tmp`, JSON.stringify(capabilities));
    await rename(`${capabilitiesFile}.tmp`, capabilitiesFile);
  };
  await update({ namedSockets: ["first"] });
  let next = Promise.withResolvers();
  const watcher = watchApplication("custom-app", {
    appDirectory, capabilitiesFile, extraCapabilities: { shellSurfaces: true },
    onSuccess: result => next.resolve(result),
    onError: error => next.reject(error),
  });
  t.after(() => watcher.close());
  try {
    const first = await next.promise;
    assert.deepEqual(first.manifest.capabilities, { namedSockets: ["first"], shellSurfaces: true });
    next = Promise.withResolvers();
    const denied = assert.rejects(next.promise, /capabilitiesFile.*sessionLock/);
    await update({ sessionLock: true });
    await denied;
    assert.equal(JSON.parse(await readFile(path.join(dist, "manifest.json"), "utf8")).buildId, first.manifest.buildId);
    next = Promise.withResolvers();
    await update({ namedSockets: ["second"] });
    const second = await next.promise;
    assert.deepEqual(second.manifest.capabilities, { namedSockets: ["second"], shellSurfaces: true });
    assert.notEqual(second.manifest.buildId, first.manifest.buildId);
  } finally {
    await watcher.close();
  }
});

test("capability watches follow dangling links, atomic target replacements, and retargeting", async t => {
  const appDirectory = await externalApplication(t);
  const directory = await mkdtemp(path.join(os.tmpdir(), "zgui-capability-links-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const capabilitiesFile = path.join(directory, "composed.json");
  const first = path.join(directory, "first", "capabilities.json");
  const second = path.join(directory, "second", "other.json");
  for (const target of [first, second]) await mkdir(path.dirname(target));
  const link = target => symlink(path.relative(directory, target), capabilitiesFile);
  const update = async (target, name) => {
    await writeFile(`${target}.tmp`, JSON.stringify({ namedSockets: [name] }));
    await rename(`${target}.tmp`, target);
  };
  await link(first);
  const { watchApplication } = await publicBuild(appDirectory);
  let next = Promise.withResolvers();
  const missing = assert.rejects(next.promise, /capabilitiesFile.*ENOENT/);
  const watcher = watchApplication("custom-app", {
    appDirectory, capabilitiesFile,
    onSuccess: result => next.resolve(result),
    onError: error => next.reject(error),
  });
  t.after(() => watcher.close());
  try {
    await missing;
    const expectSockets = async (name, change) => {
      next = Promise.withResolvers();
      await change();
      assert.deepEqual((await next.promise).manifest.capabilities, { namedSockets: [name] });
    };
    await expectSockets("created", () => update(first, "created"));
    await expectSockets("replaced", () => update(first, "replaced"));
    await update(second, "retargeted");
    await expectSockets("retargeted", async () => {
      await symlink(path.relative(directory, second), `${capabilitiesFile}.tmp`);
      await rename(`${capabilitiesFile}.tmp`, capabilitiesFile);
    });
    next = Promise.withResolvers();
    const removed = assert.rejects(next.promise, /capabilitiesFile.*ENOENT/);
    await rm(second);
    await removed;
    await expectSockets("restored", () => update(second, "restored"));
  } finally {
    await watcher.close();
  }
});

test("zgui.id overrides the default identity and invalid IDs leave the previous publication intact", { timeout: 30000 }, async t => {
  const directory = await externalApplication(t);
  const { buildApplication } = await publicBuild(directory);
  const build = () => buildApplication("custom-app", { appDirectory: directory });
  const manifestPath = path.join(directory, "dist/manifest.json");
  const packagePath = path.join(directory, "package.json");
  const configure = id => writeFile(packagePath, JSON.stringify({ zgui: { id } }));

  const baseline = await build();
  assert.equal(baseline.manifest.id, "dev.example.custom-app");
  await configure("org.example/My App");
  const custom = await build();
  assert.equal(custom.manifest.id, "org.example/My App");
  assert.notEqual(custom.manifest.buildId, baseline.manifest.buildId);
  const published = await readFile(manifestPath);
  for (const id of ["", "  ", "\u00a0test", "test ", "test\n", "test\u009fapp", null]) {
    await configure(id);
    await assert.rejects(build(), /package\.json zgui\.id/);
  }
  assert.deepEqual(await readFile(manifestPath), published);
});

test("capability overlays from code and the CLI seal the same deterministic union", { timeout: 30000 }, async t => {
  const directory = await externalApplication(t);
  const { buildApplication } = await publicBuild(directory);
  const capabilitiesFile = path.join(directory, "plugin-capabilities.json");
  const packagePath = path.join(directory, "package.json");
  const base = {
    shellSurfaces: true, controlChannel: false,
    imagePrefixes: ["$XDG_DATA_HOME/icons", "$HOME"],
    unixSockets: ["$XDG_RUNTIME_DIR/host-${WAYLAND_DISPLAY}.sock", "$HOME/$WAYLAND_DISPLAY"],
    namedSockets: ["niri"], commands: ["status"],
  };
  const overlay = {
    shellSurfaces: false, controlChannel: true, sessionLock: false, imagePrefixes: ["$HOME"],
    unixSockets: ["/run/literal-$NAME", "$HOME/$WAYLAND_DISPLAY"],
    namedSockets: ["niri", "Status_2"], commands: ["status", "plugin"],
  };
  const expected = {
    shellSurfaces: true, controlChannel: true,
    imagePrefixes: ["$HOME", "$XDG_DATA_HOME/icons"],
    unixSockets: ["$HOME/$WAYLAND_DISPLAY", "$XDG_RUNTIME_DIR/host-${WAYLAND_DISPLAY}.sock", "/run/literal-$NAME"],
    namedSockets: ["Status_2", "niri"], commands: ["plugin", "status"],
  };
  await writeFile(packagePath, JSON.stringify({ name: "@consumer/custom-app", zgui: { capabilities: base } }));
  const baseline = await buildApplication("custom-app", { appDirectory: directory });
  const merged = await buildApplication("custom-app", { appDirectory: directory, extraCapabilities: overlay });
  assert.deepEqual(merged.manifest.capabilities, expected);
  assert.notEqual(merged.manifest.buildId, baseline.manifest.buildId);
  const { namedSockets, commands, unixSockets, ...flags } = overlay;
  await writeFile(capabilitiesFile, JSON.stringify({
    ...flags, namedSockets: namedSockets.toReversed(), commands: commands.toReversed(), unixSockets: unixSockets.toReversed(),
  }));
  await execute(process.execPath, [cli, "build", "--capabilities-file", path.basename(capabilitiesFile)], { cwd: directory });
  const manifest = JSON.parse(await readFile(path.join(directory, "dist/manifest.json"), "utf8"));
  assert.deepEqual(manifest, merged.manifest);
  const both = await buildApplication("custom-app", {
    appDirectory: directory, capabilitiesFile, extraCapabilities: { commands: ["status"] },
  });
  assert.equal(both.manifest.buildId, merged.manifest.buildId);
});

test("overlays cannot declare sessionLock and report independent validation failures", { timeout: 30000 }, async t => {
  const directory = await externalApplication(t);
  const { buildApplication, previewApplication } = await publicBuild(directory);
  const packagePath = path.join(directory, "package.json");
  const manifestPath = path.join(directory, "dist/manifest.json");
  const capabilitiesFile = path.join(directory, "plugin-capabilities.json");
  const configure = capabilities => writeFile(packagePath, JSON.stringify({ zgui: { capabilities } }));
  const options = { appDirectory: directory };
  const build = (extraCapabilities, optionsOverride) =>
    buildApplication("custom-app", { ...options, extraCapabilities, ...optionsOverride });

  await assert.rejects(build({ sessionLock: true }), /extraCapabilities\.sessionLock.*only.*package\.json/);
  await assert.rejects(readFile(manifestPath), { code: "ENOENT" });
  await configure({ sessionLock: true });
  const { manifest } = await build({ sessionLock: false });
  assert.deepEqual(manifest.capabilities, { sessionLock: true });
  assert.deepEqual((await build({})).manifest, manifest);
  await assert.rejects(build({ sessionLock: true }), /extraCapabilities\.sessionLock/);
  await assert.rejects(
    previewApplication("custom-app", { ...options, extraCapabilities: { sessionLock: true } }),
    /extraCapabilities\.sessionLock/,
  );
  await assert.rejects(build({ namedSockets: ["niri", "niri"] }), /extraCapabilities\.namedSockets.*duplicates/);
  await writeFile(capabilitiesFile, JSON.stringify({ sessionLock: true }));
  await assert.rejects(
    execute(process.execPath, [cli, "build", "--capabilities-file", capabilitiesFile], { cwd: directory }),
    /capabilitiesFile.*plugin-capabilities\.json.*sessionLock.*only.*package\.json/,
  );
  await writeFile(capabilitiesFile, "{");
  await assert.rejects(build(undefined, { capabilitiesFile }), /capabilitiesFile.*plugin-capabilities\.json.*JSON/);
  await configure({ commands: ["status", "status"] });
  await assert.rejects(build({ commands: ["status"] }), /package\.json zgui\.capabilities\.commands.*duplicates/);
  assert.deepEqual(JSON.parse(await readFile(manifestPath, "utf8")), manifest);
});

test("capabilities are closed declarations and change the sealed build identity", { timeout: 30000 }, async t => {
  const directory = await externalApplication(t);
  const compilation = await bundle(path.join(directory, "src/main.tsx"), { appDirectory: directory });
  const build = async capabilities => {
    await writeFile(path.join(directory, "package.json"), JSON.stringify({
      name: "@consumer/custom-app", type: "module", zgui: { capabilities },
    }));
    const { manifest } = await publishApplication("custom-app", compilation, {
      appDirectory: directory, root: workspaceRoot,
    });
    return manifest;
  };
  const omitted = await build(undefined);
  const omittedBytes = await readFile(path.join(directory, "dist/manifest.json"));
  assert.deepEqual(await build({ shellSurfaces: false, imagePrefixes: [] }), omitted);
  const declared = await build({
    shellSurfaces: true, sessionLock: true, imagePrefixes: ["/var/media"],
    unixSockets: ["/run/user/1000/status.sock"], commands: ["status"], controlChannel: true,
  });
  assert.notEqual(declared.buildId, omitted.buildId);
  assert.deepEqual(declared.artifactDigests, omitted.artifactDigests);
  assert.deepEqual(declared.capabilities, {
    shellSurfaces: true, sessionLock: true, imagePrefixes: ["/var/media"],
    unixSockets: ["/run/user/1000/status.sock"], commands: ["status"], controlChannel: true,
  });
  assert.deepEqual(await build({ namedSockets: [] }), omitted);
  assert.deepEqual(await readFile(path.join(directory, "dist/manifest.json")), omittedBytes);
  const portable = {
    imagePrefixes: ["$HOME", "$XDG_DATA_HOME/icons", "$XDG_RUNTIME_DIR/z-shell/media"],
    unixSockets: [
      "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY",
      "$XDG_RUNTIME_DIR/z-shell/host-${WAYLAND_DISPLAY}.sock",
      "/run/literal-$UNKNOWN",
    ],
    namedSockets: ["Status_2", "niri"],
  };
  const sealed = await build(portable);
  assert.deepEqual(sealed.capabilities, portable);
  assert.notEqual(sealed.buildId, (await build({ ...portable, namedSockets: ["niri"] })).buildId);
  for (const entry of [
    "$XDG_RUNTIME_DIR/../socket", "$HOME//socket", "$HOME/./socket", "$HOME/x\\y",
    "$HOME/$UNKNOWN", "$HOME/${UNKNOWN}", "$HOME/host-$WAYLAND_DISPLAY.sock",
    "$HOME/${WAYLAND_DISPLAY}\n", "$HOME_OTHER/socket",
  ]) {
    await assert.rejects(build({ unixSockets: [entry] }), /zgui\.capabilities\.unixSockets/);
  }
  for (const names of [["niri", "niri"], ["2niri"], ["niri\n"], ["x".repeat(65)]]) {
    await assert.rejects(build({ namedSockets: names }), /zgui\.capabilities\.namedSockets/);
  }
  await assert.rejects(build({ imagePrefixes: ["/var/../media"] }), /zgui\.capabilities\.imagePrefixes/);
  await assert.rejects(build({ unknown: true }), /zgui\.capabilities\.unknown/);
});

test("explicit missing application paths never select a repository example", async t => {
  const directory = await externalApplication(t);
  await assert.rejects(resolveApplication("./counter", { cwd: directory }), /Cannot open application directory/);
  const selected = await resolveApplication(directory, { cwd: workspaceRoot });
  assert.equal(selected.appDirectory, directory);
  assert.equal(selected.app, "custom-app");
});

test("preview preserves Cargo failures and repeated interrupts wait for child cleanup", { timeout: 30000 }, async t => {
  const directory = await externalApplication(t);
  const bin = path.join(directory, "bin");
  await mkdir(bin);
  const cargo = path.join(bin, "cargo");
  const env = { ...process.env, PATH: `${bin}${path.delimiter}${process.env.PATH}` };
  await writeFile(cargo, `#!${process.execPath}\nprocess.exit(101);\n`, { mode: 0o755 });
  await assert.rejects(execute(process.execPath, [cli, "preview"], { cwd: directory, env }), error => error.code === 101);

  await writeFile(cargo, `#!${process.execPath}
process.on('SIGINT', () => console.log('cargo-stopping'));
process.on('SIGTERM', () => console.log('cargo-stopping'));
setInterval(() => {}, 1000);
console.log('cargo-ready ' + process.pid);
`);
  const child = spawn(process.execPath, [cli, "preview"], { cwd: directory, env, stdio: ["ignore", "pipe", "pipe"] });
  let log = "", cargoPid;
  const completion = new Promise(resolve => child.once("close", (code, signal) => resolve({ code, signal })));
  t.after(async () => {
    if (cargoPid) { try { process.kill(-cargoPid, "SIGKILL"); } catch (error) { if (error.code !== "ESRCH") throw error; } }
    child.kill("SIGKILL");
    await completion;
  });
  child.stderr.on("data", chunk => { log += chunk; });
  const waitFor = pattern => new Promise((resolve, reject) => {
    const check = () => {
      const match = log.match(pattern);
      if (match) { cleanup(); resolve(match); }
    };
    const stopped = () => { cleanup(); reject(new Error(`Preview exited before ${pattern}: ${log}`)); };
    const cleanup = () => { child.stderr.off("data", check); child.off("close", stopped); };
    child.stderr.on("data", check);
    child.once("close", stopped);
    check();
  });
  cargoPid = Number((await waitFor(/cargo-ready (\d+)/))[1]);
  child.kill("SIGINT");
  await waitFor(/cargo-stopping/);
  child.kill("SIGINT");
  assert.deepEqual(await completion, { code: 130, signal: null });
  assert.throws(() => process.kill(cargoPid, 0), { code: "ESRCH" });
});
