import assert from "node:assert/strict";
import { test } from "node:test";
import { watch } from "node:fs";
import { mkdir, mkdtemp, readFile, readdir, realpath, rename, rm, symlink, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";
import { watchApplication } from "../../packages/build/src/watch.mjs";

const json = async file => JSON.parse(await readFile(file, "utf8"));
const source = (label, blocked = false) => `
import { Button, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [value, setValue] = createSignal(${JSON.stringify(label)});
    return <column><text testId="value">{value()}</text><Button testId="go" onClick={() => {
      ${blocked ? "while (true) {}" : `setValue(${JSON.stringify(label + " clicked")});`}
    }}>Go</Button></column>;
  }, root);
}
`;
function channel() {
  const rows = [], waiters = new Set();
  return {
    rows,
    push(row) { rows.push(row); for (const waiter of waiters) if (waiter.predicate(row)) waiter.done(row); },
    wait(predicate, milliseconds = 15000) {
      const found = rows.find(predicate); if (found) return Promise.resolve(found);
      return new Promise((resolve, reject) => {
        const waiter = { predicate, done(value) { clearTimeout(timer); waiters.delete(waiter); resolve(value); } };
        const timer = setTimeout(() => { waiters.delete(waiter); reject(new Error(`event deadline; observed ${JSON.stringify(rows)}`)); }, milliseconds);
        waiters.add(waiter);
      });
    },
  };
}
async function fixture(t) {
  const directory = await mkdtemp(path.join(workspaceRoot, "target/preview-watch-test-"));
  const cleanup = [];
  t.after(async () => { for (const close of cleanup.reverse()) await close(); await rm(directory, { recursive: true, force: true }); });
  const app = path.join(directory, "app"); await mkdir(path.join(app, "src"), { recursive: true });
  await writeFile(path.join(app, "tsconfig.json"), JSON.stringify({ extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"] }));
  const profile = await json(path.join(workspaceRoot, "preview/profiles/default.json"));
  profile.fonts.manifest = path.relative(directory, path.resolve(workspaceRoot, "preview/profiles", profile.fonts.manifest));
  profile.limits.maxJsEntryWallMs = 1000;
  await writeFile(path.join(directory, "profile.json"), JSON.stringify(profile));
  await writeFile(path.join(directory, "scenario.json"), JSON.stringify({ schemaVersion: 1, id: "watch-regression", requiredCapabilities: ["pixels", "virtual-time", "input.pointer"], steps: [
    { id: "before", op: "capture", captureId: "before" },
    { id: "click", op: "pointer.click", target: { testId: "go", surface: "main" }, button: "primary" },
    { id: "advance", op: "clock.advance", byMs: 20 },
    { id: "after", op: "capture", captureId: "after" },
  ] }));
  const edit = (label, blocked) => writeFile(path.join(app, "src/main.tsx"), source(label, blocked));
  const build = async (label, blocked) => {
    await edit(label, blocked);
    const result = await buildApplication("watch-regression", { appDirectory: app });
    return { ...result, directory: await realpath(result.dist) };
  };
  return { directory, app, cleanup, edit, build };
}
function start(f, maximum = 64, consume = true) {
  const out = path.join(f.directory, "watch");
  const child = spawn(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "watch", "--bundle", path.join(f.app, "dist"),
    "--profile", path.join(f.directory, "profile.json"), "--fixture", "preview/fixtures/empty.json",
    "--scenario", path.join(f.directory, "scenario.json"), "--out", out, "--max-runs", String(maximum)],
  { cwd: workspaceRoot, stdio: ["ignore", "pipe", "pipe"] });
  const events = channel(); let buffer = "", stderr = "", ended = false;
  if (consume) child.stdout.on("data", bytes => {
    buffer += bytes;
    for (;;) {
      const newline = buffer.indexOf("\n"); if (newline < 0) break;
      const line = buffer.slice(0, newline); buffer = buffer.slice(newline + 1);
      assert.ok(Buffer.byteLength(line) + 1 <= 8192);
      events.push(JSON.parse(line));
    }
  });
  child.stderr.on("data", bytes => { stderr += bytes; });
  child.once("exit", () => { if (!consume) child.stdout.destroy(); });
  const closed = new Promise((resolve, reject) => { child.once("error", reject); child.once("close", (code, signal) => { ended = true; resolve({ code, signal, stderr }); }); });
  async function exit(milliseconds = 15000) {
    let timer;
    try { return await Promise.race([closed, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`watch did not terminate: ${stderr}`)), milliseconds); })]); }
    finally { clearTimeout(timer); }
  }
  f.cleanup.push(async () => {
    if (!ended) { child.kill("SIGCONT"); child.kill("SIGTERM"); try { await exit(3000); } catch (error) { child.kill("SIGKILL"); await closed; throw error; } }
  });
  return { child, out, events, exit, closed };
}
async function publication(app, directory) {
  const temporary = path.join(app, `.dist-${randomUUID()}`);
  await symlink(path.relative(app, directory), temporary); await rename(temporary, path.join(app, "dist"));
}
async function status(app, previousDirectory, message = "controlled failed build attempt") {
  const value = { schemaVersion: 1, attemptId: randomUUID(), app: "watch-regression", status: "failed", previousDirectory,
    publicationDirectory: null, buildId: null, error: { code: "E_BUILD", message } };
  const temporary = path.join(app, `.zgui-build-status.json.${randomUUID()}.tmp`);
  await writeFile(temporary, JSON.stringify(value)); await rename(temporary, path.join(app, ".zgui-build-status.json"));
  return value;
}
async function observedFile(root, file) {
  let watcher, timer;
  return new Promise((resolve, reject) => {
    const check = async () => { try { await readFile(file); finish(); } catch (error) { if (error.code !== "ENOENT") finish(error); } };
    function finish(error) { watcher?.close(); clearTimeout(timer); error ? reject(error) : resolve(); }
    watcher = watch(root, { recursive: true }, () => void check());
    watcher.on("error", finish); timer = setTimeout(() => finish(new Error(`file deadline: ${file}`)), 10000); void check();
  });
}

for (const restoreOriginal of [false, true]) {
test(`real compile failure preserves historical success and recovers with ${restoreOriginal ? "original" : "changed"} source`, { timeout: 30000 }, async t => {
  const f = await fixture(t); await f.edit("initial");
  const builds = channel();
  const builder = watchApplication("watch-regression", { appDirectory: f.app,
    onSuccess: value => builds.push({ success: value }), onError: error => builds.push({ error }) });
  f.cleanup.push(() => builder.close());
  const firstBuild = (await builds.wait(row => row.success)).success;
  const running = start(f, 2);
  const first = await running.events.wait(row => row.event === "run.completed");
  assert.equal(first.exitCode, 0);
  assert.equal(first.buildId, firstBuild.manifest.buildId);
  const originalPng = await readFile(path.join(running.out, first.output, "captures/after.png"));
  await writeFile(path.join(f.app, "src/main.tsx"), source("invalid") + '\nconst broken: number = "type error";\n');
  const failure = await running.events.wait(row => row.event === "build.failed");
  const failedIndex = await json(path.join(running.out, "index.json"));
  assert.equal(failedIndex.latestAttempt.buildAttemptId, failure.buildAttemptId);
  assert.equal(failedIndex.latestAttempt.buildId, null);
  assert.equal(failedIndex.latestAttempt.output, null);
  assert.equal(failedIndex.lastSuccess.runId, first.runId);
  assert.equal(failedIndex.lastSuccess.historical, true);
  assert.deepEqual(await readFile(path.join(running.out, first.output, "captures/after.png")), originalPng);
  await f.edit(restoreOriginal ? "initial" : "recovered");
  const second = await running.events.wait(row => row.event === "run.completed" && row.runId !== first.runId);
  if (restoreOriginal) assert.equal(second.buildId, first.buildId);
  assert.equal(second.exitCode, 0);
  assert.notEqual(second.nativeRunId, first.nativeRunId);
  assert.notEqual(second.output, first.output);
  assert.equal((await running.exit()).code, 0);
  const final = await json(path.join(running.out, "index.json"));
  assert.equal(final.latestAttempt.runId, second.runId);
  assert.equal(final.lastSuccess.historical, false);
  const tree = await json(path.join(running.out, second.output, "captures/after.tree.json"));
  assert.equal(tree.nodes.find(node => node.testId === "value").text, restoreOriginal ? "initial clicked" : "recovered clicked");
});
}

async function workerChildren(pid) {
  const tasks = await readdir(`/proc/${pid}/task`);
  return [...new Set((await Promise.all(tasks.map(async task => {
    try { return (await readFile(`/proc/${pid}/task/${task}/children`, "utf8")).trim().split(/\s+/).filter(Boolean).map(Number); }
    catch (error) { if (error.code === "ENOENT") return []; throw error; }
  }))).flat())];
}

test("superseded frozen native input is reaped before the newest complete package starts", async t => {
  const f = await fixture(t);
  const frozen = await f.build("frozen", true), middle = await f.build("middle"), newest = await f.build("newest");
  await publication(f.app, frozen.directory);
  const running = start(f, 2);
  const first = await running.events.wait(row => row.event === "run.started");
  await observedFile(running.out, path.join(running.out, first.output, "progress/action-000002.json"));
  const workerPids = await workerChildren(running.child.pid);
  assert.equal(workerPids.length, 1);
  // Stop only the owned controller while publishing two already validated builds;
  // its real worker continues. No scheduling sleep decides which build wins.
  running.child.kill("SIGSTOP");
  try { await publication(f.app, middle.directory); await publication(f.app, newest.directory); }
  finally { running.child.kill("SIGCONT"); }
  const canceled = await running.events.wait(row => row.event === "run.canceled");
  assert.equal(canceled.buildId, frozen.manifest.buildId);
  assert.equal(canceled.superseded, true);
  assert.equal(canceled.cancellation.workerStopped, true);
  assert.equal(canceled.teardownVerified, false);
  assert.throws(() => process.kill(workerPids[0], 0), error => error.code === "ESRCH");
  const second = await running.events.wait(row => row.event === "run.started" && row.runId !== first.runId);
  assert.equal(second.buildId, newest.manifest.buildId);
  assert.ok(second.sequence > canceled.sequence);
  const final = await running.events.wait(row => row.event === "run.completed" && row.buildId === newest.manifest.buildId);
  assert.equal(final.exitCode, 0);
  assert.equal((await running.exit()).code, 0);
  assert.equal(running.events.rows.some(row => row.event === "build.accepted" && row.buildId === middle.manifest.buildId), false);
  const index = await json(path.join(running.out, "index.json"));
  assert.equal(index.latestAttempt.buildId, newest.manifest.buildId);
  assert.equal(index.lastSuccess.buildId, newest.manifest.buildId);
});

test("a newer failed attempt cannot promote a late successful worker", async t => {
  const f = await fixture(t), built = await f.build("late");
  const running = start(f, 1);
  const first = await running.events.wait(row => row.event === "run.started");
  await observedFile(running.out, path.join(running.out, first.output, "progress/action-000000.json"));
  const children = await workerChildren(running.child.pid);
  assert.equal(children.length, 1);
  process.kill(children[0], "SIGSTOP");
  try {
    await status(f.app, built.directory);
    await running.events.wait(row => row.event === "build.failed");
  } finally { process.kill(children[0], "SIGCONT"); }
  const completed = await running.events.wait(row => row.event === "run.completed");
  assert.equal(completed.exitCode, 0);
  assert.equal(completed.superseded, true);
  assert.equal((await running.exit()).code, 3);
  const index = await json(path.join(running.out, "index.json"));
  assert.equal(index.latestAttempt.status, "failed");
  assert.equal(index.lastSuccess, null);
});

test("an unread stdout pipe fails within a deadline instead of trapping the controller", async t => {
  const f = await fixture(t), built = await f.build("pressure");
  const running = start(f, 64, false);
  let pending = Promise.resolve(), writing = false;
  const timer = setInterval(() => {
    if (writing) return;
    writing = true;
    pending = status(f.app, built.directory, "bounded pressure diagnostic ".repeat(70)).finally(() => { writing = false; });
  }, 10);
  try {
    const result = await running.exit(8000);
    assert.equal(result.code, 8, result.stderr);
    const terminal = await json(path.join(running.out, "watch.json"));
    assert.equal(terminal.firstError.code, "E_OUTPUT_IO");
    assert.match(terminal.firstError.message, /stdout consumer/);
  } finally { clearInterval(timer); await pending; }
});
