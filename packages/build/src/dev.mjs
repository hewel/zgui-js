import path from "node:path";
import { workspaceRoot } from "./build.mjs";
import { watchApplication } from "./watch.mjs";
import { prepareNativeHost, runNativeHost } from "./native-process.mjs";

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

/** Own one publication watcher and one native session; return only after both have stopped.
 * The optional third argument supplies lifecycle boundaries for deterministic embedding/tests.
 */
export async function devApplication(app, {
  root = workspaceRoot,
  appDirectory = path.join(root, "apps/examples", app),
  hostArgs = [],
  mode = "desktop",
} = {}, {
  watch = watchApplication,
  prepare = prepareNativeHost,
  run = runNativeHost,
  signals = process,
  log = message => console.error(message),
} = {}) {
  root = path.resolve(root);
  appDirectory = path.resolve(appDirectory);
  const controller = new AbortController();
  const published = deferred();
  const stopped = deferred();
  let exitCode;
  let watcher;
  let startup;
  const stop = (code, reason) => {
    if (exitCode !== undefined) return;
    exitCode = code;
    controller.abort(reason);
    stopped.resolve();
  };
  const interrupt = () => stop(130, "SIGINT");
  const terminate = () => stop(143, "SIGTERM");
  const fail = error => {
    if (controller.signal.aborted) return;
    log(error.message ?? String(error));
    stop(error.exitCode ?? 1);
  };
  signals.on("SIGINT", interrupt);
  signals.on("SIGTERM", terminate);
  try {
    if (mode !== "desktop" && mode !== "preview") throw new Error(`Unknown development mode: ${mode}`);
    watcher = watch(app, {
      root, appDirectory, signal: controller.signal,
      onSuccess(result) {
        if (controller.signal.aborted) return;
        log(`Published ${result.dist} ${result.manifest.buildId}`);
        published.resolve(result.dist);
      },
      onError(error) {
        if (!controller.signal.aborted) log(error.message ?? String(error));
      },
    });
    startup = (async () => {
      try {
        if (controller.signal.aborted) return;
        const executable = await prepare({ root, signal: controller.signal });
        if (controller.signal.aborted) return;
        const dist = await Promise.race([published.promise, stopped.promise]);
        if (controller.signal.aborted) return;
        // Launch from this single startup path, never from a rebuild callback.
        const args = mode === "preview"
          ? ["preview", "watch", "--bundle", dist, ...hostArgs]
          : ["--bundle", dist, "--watch", ...hostArgs];
        stop(await run(executable, args, { root, signal: controller.signal }));
      } catch (error) {
        fail(error);
      }
    })();
    await stopped.promise;
  } catch (error) {
    fail(error);
  } finally {
    controller.abort();
    const cleanup = await Promise.allSettled([startup, Promise.resolve().then(() => watcher?.close())]);
    for (const result of cleanup) {
      if (result.status === "rejected") {
        log(result.reason.message ?? String(result.reason));
        if (exitCode === 0 || exitCode === undefined) exitCode = 1;
      }
    }
    signals.removeListener("SIGINT", interrupt);
    signals.removeListener("SIGTERM", terminate);
  }
  return exitCode ?? 1;
}
