import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { EventEmitter, getEventListeners } from "node:events";
import path from "node:path";
import { test } from "node:test";
import { devApplication } from "../../packages/build/src/dev.mjs";
import { prepareNativeHost, runNativeHost } from "../../packages/build/src/native-process.mjs";
import { workspaceRoot } from "../../packages/build/src/build.mjs";

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

function lifecycle(overrides = {}) {
  const signals = new EventEmitter();
  const started = deferred();
  const exited = deferred();
  const logs = [];
  let callbacks;
  let closes = 0;
  let launches = 0;
  const dependencies = {
    signals,
    log: message => logs.push(message),
    watch(_app, options) {
      callbacks = options;
      return { async close() { closes++; } };
    },
    async prepare() { return process.execPath; },
    run(executable, args, options) {
      launches++;
      started.resolve({ executable, args, ...options });
      options.signal.addEventListener("abort", () => exited.resolve(143), { once: true });
      return exited.promise;
    },
    ...overrides,
  };
  return {
    dependencies, signals, started, exited, logs,
    get callbacks() { return callbacks; },
    get closes() { return closes; },
    get launches() { return launches; },
    publish(id = "first") {
      callbacks.onSuccess({ dist: path.join(workspaceRoot, "apps/examples/counter/dist"), manifest: { buildId: id } });
    },
  };
}

for (const mode of ["desktop", "preview"]) {
  test(`${mode} recovers from initial build errors and retains a single host across rebuilds`, { timeout: 5000 }, async t => {
    const prepared = deferred();
    const fixture = lifecycle({ prepare: () => prepared.promise });
    t.after(() => fixture.signals.emit("SIGTERM"));
    const result = devApplication("counter", { mode, hostArgs: ["--exit-after-ms", "50"] }, fixture.dependencies);
    fixture.callbacks.onError(new Error("initial TSX failure"));
    assert.equal(fixture.launches, 0);
    fixture.publish();
    fixture.publish("second");
    assert.equal(fixture.launches, 0, "publication alone must not bypass native preparation");
    prepared.resolve(process.execPath);
    const host = await fixture.started.promise;
    assert.equal(host.args[host.args.indexOf("--bundle") + 1], path.join(workspaceRoot, "apps/examples/counter/dist"));
    assert.ok(host.args.includes(mode === "desktop" ? "--watch" : "watch"));
    assert.deepEqual(host.args.slice(-2), ["--exit-after-ms", "50"]);
    fixture.callbacks.onError(new Error("later TSX failure"));
    fixture.publish("third");
    assert.equal(host.signal.aborted, false, "failed rebuilds must retain the current native session");
    assert.equal(fixture.launches, 1);
    fixture.exited.resolve(23);
    assert.equal(await result, 23);
    assert.equal(fixture.closes, 1);
    assert.equal(fixture.signals.listenerCount("SIGINT"), 0);
    assert.equal(fixture.signals.listenerCount("SIGTERM"), 0);
    assert.ok(fixture.logs.includes("initial TSX failure"));
    assert.ok(fixture.logs.includes("later TSX failure"));
  });
}

test("shutdown during preparation waits for cleanup and never launches a late host", { timeout: 5000 }, async () => {
  const aborted = deferred();
  const reaped = deferred();
  const fixture = lifecycle({
    async prepare({ signal }) {
      signal.addEventListener("abort", () => aborted.resolve(signal.reason), { once: true });
      await reaped.promise;
      return process.execPath;
    },
  });
  let finished = false;
  const result = devApplication("counter", {}, fixture.dependencies).then(code => { finished = true; return code; });
  fixture.signals.emit("SIGINT");
  assert.equal(await aborted.promise, "SIGINT");
  fixture.publish("too-late");
  fixture.signals.emit("SIGTERM");
  assert.equal(finished, false, "shutdown must await the child owner, not just request cancellation");
  reaped.resolve();
  assert.equal(await result, 130);
  assert.equal(fixture.launches, 0);
  assert.equal(fixture.closes, 1);
  assert.equal(fixture.signals.listenerCount("SIGINT"), 0);
  assert.equal(fixture.signals.listenerCount("SIGTERM"), 0);
});

test("shutdown after preparation while waiting for a first publication closes the watcher", { timeout: 5000 }, async () => {
  const prepared = deferred();
  const fixture = lifecycle({ async prepare() { prepared.resolve(); return process.execPath; } });
  const result = devApplication("counter", {}, fixture.dependencies);
  await prepared.promise;
  fixture.signals.emit("SIGTERM");
  assert.equal(await result, 143);
  assert.equal(fixture.launches, 0);
  assert.equal(fixture.closes, 1);
});

test("a preparation-completion signal race cannot spawn a host", { timeout: 5000 }, async () => {
  const prepared = deferred();
  const fixture = lifecycle({ prepare: () => prepared.promise });
  const result = devApplication("counter", {}, fixture.dependencies);
  fixture.publish();
  prepared.resolve(process.execPath);
  fixture.signals.emit("SIGTERM");
  assert.equal(await result, 143);
  assert.equal(fixture.launches, 0);
});

test("native preparation failure is fatal even before any successful publication", { timeout: 5000 }, async () => {
  const fixture = lifecycle({ async prepare() { throw Object.assign(new Error("cargo failed"), { exitCode: 101 }); } });
  assert.equal(await devApplication("counter", {}, fixture.dependencies), 101);
  assert.equal(fixture.launches, 0);
  assert.equal(fixture.closes, 1);
  assert.deepEqual(fixture.logs, ["cargo failed"]);
  assert.equal(fixture.signals.listenerCount("SIGTERM"), 0);
});

test("native spawn errors close the watcher without retrying", { timeout: 5000 }, async () => {
  let attempts = 0;
  const fixture = lifecycle({ async run() { attempts++; throw new Error("native executable is unavailable"); } });
  const result = devApplication("counter", {}, fixture.dependencies);
  fixture.publish();
  assert.equal(await result, 1);
  assert.equal(attempts, 1);
  assert.equal(fixture.closes, 1);
  assert.deepEqual(fixture.logs.filter(message => message === "native executable is unavailable"), ["native executable is unavailable"]);
});

test("watcher startup failure removes signal listeners and skips cargo", { timeout: 5000 }, async () => {
  let preparations = 0;
  const fixture = lifecycle({
    watch() { throw new Error("watch unavailable"); },
    async prepare() { preparations++; return process.execPath; },
  });
  assert.equal(await devApplication("counter", {}, fixture.dependencies), 1);
  assert.equal(preparations, 0);
  assert.equal(fixture.signals.listenerCount("SIGINT"), 0);
  assert.equal(fixture.signals.listenerCount("SIGTERM"), 0);
});

test("native executable comes from Cargo artifacts, including incremental fresh builds", { timeout: 5000 }, async () => {
  const executable = path.join(workspaceRoot, "custom-cargo-target/debug/native-host");
  let builds = 0;
  const spawnProcess = (_command, _args, options) => {
    builds++;
    return spawn(process.execPath, ["-e", `
      console.log(JSON.stringify({ reason: 'compiler-artifact', target: { name: 'dependency', kind: ['bin'] }, executable: '/wrong/dependency' }));
      console.log(JSON.stringify({ reason: 'compiler-artifact', target: { name: 'native-host', kind: ['bin'] }, fresh: true, executable: ${JSON.stringify(executable)} }));
    `], options);
  };
  assert.equal(await prepareNativeHost({}, { spawnProcess }), executable);
  assert.equal(await prepareNativeHost({}, { spawnProcess }), executable);
  assert.equal(builds, 2, "every dev run must let Cargo check incremental freshness");
});

test("Cargo nonzero exit preserves its status and rejects missing executable metadata", { timeout: 5000 }, async () => {
  await assert.rejects(prepareNativeHost({}, {
    spawnProcess: (_command, _args, options) => spawn(process.execPath, ["-e", "process.exit(101)"], options),
  }), error => error.exitCode === 101);
  await assert.rejects(prepareNativeHost({}, {
    spawnProcess: (_command, _args, options) => spawn(process.execPath, ["-e", ""], options),
  }), /did not report a native-host executable/);
});

test("native child crash returns its exit status and releases cancellation listeners", { timeout: 5000 }, async () => {
  const controller = new AbortController();
  assert.equal(await runNativeHost(process.execPath, ["-e", "process.exit(37)"], { signal: controller.signal }), 37);
  assert.equal(getEventListeners(controller.signal, "abort").length, 0);
});

test("spawn failures are reaped and cancellation listeners are released", { timeout: 5000 }, async () => {
  const controller = new AbortController();
  await assert.rejects(runNativeHost(path.join(workspaceRoot, "missing-native-executable"), [], { signal: controller.signal }), /Cannot run/);
  assert.equal(getEventListeners(controller.signal, "abort").length, 0);
});

test("already-aborted operations never spawn", { timeout: 5000 }, async () => {
  const controller = new AbortController();
  controller.abort("SIGINT");
  let spawns = 0;
  const processOptions = { spawnProcess() { spawns++; throw new Error("unexpected spawn"); } };
  assert.equal(await runNativeHost(process.execPath, [], { signal: controller.signal }, processOptions), 130);
  await assert.rejects(prepareNativeHost({ signal: controller.signal }, processOptions), error => error.exitCode === 130);
  assert.equal(spawns, 0);
});

test("abort during spawn setup is forwarded and the child is reaped", { timeout: 5000 }, async () => {
  const controller = new AbortController();
  let child;
  const result = runNativeHost(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { signal: controller.signal }, {
    spawnProcess(command, args, options) {
      child = spawn(command, args, options);
      controller.abort("SIGINT");
      return child;
    },
  });
  assert.equal(await result, 130);
  assert.ok(child.signalCode);
  assert.equal(getEventListeners(controller.signal, "abort").length, 0);
});

test("a native process ignoring SIGTERM is killed and reaped after the grace period", { timeout: 5000 }, async t => {
  const controller = new AbortController();
  const ready = deferred();
  const terminating = deferred();
  let child;
  const result = runNativeHost(process.execPath, ["-e", `
    process.on('SIGTERM', () => process.send('terminating'));
    setInterval(() => {}, 1000);
    process.send('ready');
  `], { signal: controller.signal }, {
    killAfter: 100,
    spawnProcess(command, args, options) {
      child = spawn(command, args, { ...options, stdio: [...options.stdio, "ipc"] });
      child.on("message", message => {
        if (message === "ready") ready.resolve();
        if (message === "terminating") terminating.resolve();
      });
      return child;
    },
  });
  t.after(async () => { controller.abort(); await result; });
  await ready.promise;
  controller.abort("SIGTERM");
  await terminating.promise;
  assert.equal(await result, 143);
  assert.equal(child.signalCode, "SIGKILL");
  assert.equal(getEventListeners(controller.signal, "abort").length, 0);
});

test("cancellation during Cargo compilation rejects only after the compiler is reaped", { timeout: 5000 }, async t => {
  const controller = new AbortController();
  const ready = deferred();
  let child;
  const result = prepareNativeHost({ signal: controller.signal }, {
    spawnProcess(_command, _args, options) {
      child = spawn(process.execPath, ["-e", `
        setInterval(() => {}, 1000);
        process.send('ready');
      `], { ...options, stdio: [...options.stdio, "ipc"] });
      child.once("message", () => ready.resolve());
      return child;
    },
  });
  t.after(() => controller.abort());
  await ready.promise;
  controller.abort("SIGINT");
  await assert.rejects(result, error => error.exitCode === 130);
  assert.ok(child.signalCode);
  assert.equal(getEventListeners(controller.signal, "abort").length, 0);
});

test("a clean native close ends dev and closes its publication watcher", { timeout: 5000 }, async () => {
  const fixture = lifecycle();
  const result = devApplication("counter", {}, fixture.dependencies);
  fixture.publish();
  await fixture.started.promise;
  fixture.exited.resolve(0);
  assert.equal(await result, 0);
  assert.equal(fixture.closes, 1);
  assert.equal(fixture.signals.listenerCount("SIGINT"), 0);
  assert.equal(fixture.signals.listenerCount("SIGTERM"), 0);
});
