import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile, rm, symlink } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import { test } from "node:test";
import { resolveApplication } from "../../packages/build/src/application.ts";
import { workspaceRoot } from "../../packages/build/src/build.ts";

const execute = promisify(execFile);
const cli = path.join(workspaceRoot, "packages/build/src/cli.ts");

async function externalApplication(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "zgui app "));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(path.join(directory, "src"));
  await symlink(path.join(workspaceRoot, "node_modules"), path.join(directory, "node_modules"), "dir");
  await writeFile(path.join(directory, "package.json"), JSON.stringify({ name: "@consumer/custom-app", type: "module" }));
  await writeFile(path.join(directory, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src"],
  }));
  await writeFile(path.join(directory, "src/main.tsx"), `import { renderApp, type NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) { return renderApp(() => <text>External application</text>, root); }
`);
  return directory;
}

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
