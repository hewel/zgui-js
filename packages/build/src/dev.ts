import path from "node:path";
import type { EventEmitter } from "node:events";
import { workspaceRoot } from "./build.ts";
import { watchApplication, type ApplicationWatcher } from "./watch.ts";
import { prepareNativeHost, runNativeHost } from "./native-process.ts";

export type DevMode = "desktop" | "preview";

interface DevOptions {
  root?: string;
  appDirectory?: string;
  hostArgs?: string[];
  mode?: DevMode;
  stylexDiagnostics?: boolean;
}

interface DevDependencies {
  watch?: typeof watchApplication;
  prepare?: typeof prepareNativeHost;
  run?: typeof runNativeHost;
  signals?: Pick<EventEmitter, "on" | "removeListener">;
  log?: (message: unknown) => void;
}

function errorMessage(error: unknown) {
  return (
    (error !== null && typeof error === "object" && "message" in error
      ? error.message
      : undefined) ?? String(error)
  );
}

/** Own one publication watcher and one native session; return only after both have stopped.
 * The optional third argument supplies lifecycle boundaries for deterministic embedding/tests.
 */
export async function devApplication(
  app: string,
  {
    root = workspaceRoot,
    appDirectory = path.join(root, "apps/examples", app),
    hostArgs = [],
    mode = "desktop",
    stylexDiagnostics = mode === "preview",
  }: DevOptions = {},
  {
    watch = watchApplication,
    prepare = prepareNativeHost,
    run = runNativeHost,
    signals = process,
    log = message => console.error(message),
  }: DevDependencies = {},
) {
  root = path.resolve(root);
  appDirectory = path.resolve(appDirectory);
  const controller = new AbortController();
  const published = Promise.withResolvers<string>();
  const stopped = Promise.withResolvers<void>();
  let exitCode: number | undefined;
  let watcher: ApplicationWatcher | undefined;
  let startup: Promise<void> | undefined;
  const stop = (code: number, reason?: unknown) => {
    if (exitCode !== undefined) return;
    exitCode = code;
    controller.abort(reason);
    stopped.resolve();
  };
  const interrupt = () => stop(130, "SIGINT");
  const terminate = () => stop(143, "SIGTERM");
  const fail = (error: unknown) => {
    if (controller.signal.aborted) return;
    log(errorMessage(error));
    stop(
      error !== null &&
        typeof error === "object" &&
        "exitCode" in error &&
        typeof error.exitCode === "number"
        ? error.exitCode
        : 1,
    );
  };
  signals.on("SIGINT", interrupt);
  signals.on("SIGTERM", terminate);
  try {
    if (mode !== "desktop" && mode !== "preview")
      throw new Error(`Unknown development mode: ${String(mode)}`);
    watcher = watch(app, {
      root,
      appDirectory,
      stylexDiagnostics,
      development: mode === "desktop",
      signal: controller.signal,
      onSuccess(result) {
        if (controller.signal.aborted) return;
        log(`Published ${result.dist} ${result.manifest.buildId}`);
        published.resolve(result.dist);
      },
      onError(error) {
        if (!controller.signal.aborted) log(errorMessage(error));
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
        const args =
          mode === "preview"
            ? ["preview", "watch", "--bundle", dist!, ...hostArgs]
            : ["--bundle", dist!, "--watch", ...hostArgs];
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
    const cleanup = await Promise.allSettled([
      startup,
      Promise.resolve().then(() => watcher?.close()),
    ]);
    for (const result of cleanup) {
      if (result.status === "rejected") {
        log(errorMessage(result.reason));
        if (exitCode === 0 || exitCode === undefined) exitCode = 1;
      }
    }
    signals.removeListener("SIGINT", interrupt);
    signals.removeListener("SIGTERM", terminate);
  }
  return exitCode ?? 1;
}
