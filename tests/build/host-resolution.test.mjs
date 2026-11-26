import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const workspace = fileURLToPath(new URL("../../", import.meta.url));
const buildApi = new URL("../../packages/build/src/index.ts", import.meta.url).href;
const cli = path.join(workspace, "packages/cli/src/cli.ts");
const abi = {
  name: "zgui-host",
  version: "0.9.3-test",
  hostApiVersion: 1,
  schemaVersion: 2,
  platform: `${process.platform}-${process.arch}`,
  bytecodeFormat: "qjs-bytecode-v1",
  quickjsAbi: "qjs-ng-fixture",
};

async function checkout(t) {
  const cwd = await mkdtemp(path.join(tmpdir(), "zgui-host-resolution-"));
  t.after(() => rm(cwd, { recursive: true, force: true }));
  const cargoRoot = path.join(cwd, "host checkout");
  const executable = path.join(cargoRoot, "custom-target/zgui-host");
  const log = path.join(cwd, "calls");
  const bin = path.join(cwd, "bin");
  await mkdir(path.dirname(executable), { recursive: true });
  await mkdir(path.join(cargoRoot, "crates/zgui-host"), { recursive: true });
  await mkdir(bin);
  await writeFile(path.join(cargoRoot, "Cargo.toml"), "[workspace]\n");
  await writeFile(path.join(cargoRoot, "crates/zgui-host/Cargo.toml"), "");
  await writeFile(
    path.join(bin, "cargo"),
    `#!${process.execPath}
import { appendFileSync } from "node:fs";
if (process.cwd() !== ${JSON.stringify(cargoRoot)}) process.exit(72);
appendFileSync(${JSON.stringify(log)}, "build\\n");
console.log(JSON.stringify({ reason: "compiler-artifact",
  target: { name: "zgui-host", kind: ["bin"] }, executable: "custom-target/zgui-host" }));
`,
    { mode: 0o755 },
  );
  await writeFile(
    executable,
    `#!${process.execPath}
import { appendFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";
const args = process.argv.slice(2);
if (args.join(" ") === "--version --json") {
  appendFileSync(${JSON.stringify(log)}, "probe\\n");
  console.log(${JSON.stringify(JSON.stringify(abi))});
} else if (args[0] === "compile-bytecode") {
  const flag = name => args[args.indexOf(name) + 1];
  appendFileSync(${JSON.stringify(log)}, "compile:" + flag("--module-name") + "\\n");
  mkdirSync(dirname(flag("--output")), { recursive: true });
  await Bun.write(flag("--output"), "fixture bytecode");
} else process.exit(64);
`,
    { mode: 0o755 },
  );
  const env = { ...process.env, PATH: bin, ZGUI_JS_HOST_EXECUTABLE: "" };
  return { cwd, cargoRoot, executable, log, env };
}

async function application(cwd) {
  const appDirectory = path.join(cwd, "app");
  await mkdir(path.join(appDirectory, "src"), { recursive: true });
  const modules = path.join(appDirectory, "node_modules/@hewel");
  await mkdir(modules, { recursive: true });
  await symlink(path.join(workspace, "packages/ui"), path.join(modules, "zgui"), "dir");
  await writeFile(
    path.join(appDirectory, "package.json"),
    '{"name":"fixture-app","type":"module"}',
  );
  await writeFile(
    path.join(appDirectory, "tsconfig.json"),
    '{"extends":"@hewel/zgui/tsconfig.native.json","include":["src"]}',
  );
  await writeFile(
    path.join(appDirectory, "src/main.tsx"),
    `import { renderApp, type NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) { return renderApp(() => <text>Fixture</text>, root); }
`,
  );
  return appDirectory;
}

async function invoke({ cwd, env }, args) {
  const child = Bun.spawn([process.execPath, ...args], {
    cwd,
    env,
    stdout: "pipe",
    stderr: "pipe",
  });
  const [code, stdout, stderr] = await Promise.all([
    child.exited,
    child.stdout.text(),
    child.stderr.text(),
  ]);
  assert.equal(code, 0, stderr);
  return stdout;
}

test("a Cargo-built host reports its verified ABI and working directory", async t => {
  const fixture = await checkout(t);
  const resolved = JSON.parse(
    await invoke(fixture, [
      "-e",
      `
import { resolveNativeHost } from ${JSON.stringify(buildApi)};
console.log(JSON.stringify(await resolveNativeHost({ hostRoot: "host checkout", allowCargoBuild: true, verifyAbi: true })));
`,
    ]),
  );
  assert.deepEqual(resolved, {
    source: "cargo",
    executable: fixture.executable,
    cargoRoot: fixture.cargoRoot,
    runRoot: fixture.cargoRoot,
    abi,
  });
  assert.equal(await readFile(fixture.log, "utf8"), "build\nprobe\n");
});

test("bytecode build resolves its compiler from --host-root without --host-executable", async t => {
  const fixture = await checkout(t);
  const app = await application(fixture.cwd);
  await invoke(fixture, [cli, "build", app, "--bytecode", "--host-root", "host checkout"]);
  const manifest = JSON.parse(await readFile(path.join(app, "dist/manifest.json"), "utf8"));
  assert.equal(manifest.entry, "app.qjsb");
  assert.equal(manifest.quickjsAbi, abi.quickjsAbi);
  assert.equal(await readFile(fixture.log, "utf8"), "build\nprobe\ncompile:app.mjs\n");
});

test("bytecode packaging uses one Cargo build and ABI probe for compiler and packaged host", async t => {
  const fixture = await checkout(t);
  const app = await application(fixture.cwd);
  const out = path.join(fixture.cwd, "release");
  await invoke(fixture, [
    cli,
    "package",
    app,
    "--bytecode",
    "--host-root",
    "host checkout",
    "--out",
    out,
  ]);
  const manifest = JSON.parse(await readFile(path.join(out, "package-manifest.json"), "utf8"));
  assert.equal(manifest.hostVersion, abi.version);
  assert.equal(manifest.hostSource, "cargo");
  assert.equal(await readFile(path.join(out, "bundle/app.qjsb"), "utf8"), "fixture bytecode");
  assert.equal(await readFile(fixture.log, "utf8"), "build\nprobe\ncompile:app.mjs\n");
});
