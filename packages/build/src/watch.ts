import type { ApplicationWatcher, PublishedApplication, WatchApplicationOptions } from "./index.ts";
import { readlinkSync, realpathSync, statSync, watch, type FSWatcher } from "node:fs";
import { access, realpath, rm, writeFile, mkdtemp } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { tmpdir } from "node:os";
import { publishApplication } from "./build.ts";
import { syncCssModuleTypes } from "./css.ts";
import { workspaceRoot } from "./sdk.ts";
import { watchBundle } from "./compiler.ts";
import {
  BUILD_STATUS_FILE,
  NATIVE_STATUS_FILE,
  publishBuildStatus,
  type BuildStatus,
} from "./publication.ts";
import { typecheckProjectAsync } from "./typecheck.ts";

interface WatchDependencies {
  watchBundle?: typeof watchBundle;
  typecheck?: typeof typecheckProjectAsync;
  publish?: typeof publishApplication;
  publishStatus?: typeof publishBuildStatus;
}

// Serial execution plus a dirty bit retains edits arriving during a build without overlapping publication.
export function rebuildQueue<T>(
  build: () => T | Promise<T>,
  {
    delay = 80,
    onSuccess = () => {},
    onError = () => {},
  }: {
    delay?: number;
    onSuccess?: (result: T) => void;
    onError?: (error: unknown) => void;
  } = {},
) {
  let running: Promise<void> | undefined;
  let dirty = false;
  let closed = false;
  let timer: NodeJS.Timeout | undefined;
  async function drain() {
    if (running || closed) return;
    running = (async () => {
      while (dirty && !closed) {
        dirty = false;
        try {
          onSuccess(await build());
        } catch (error) {
          onError(error);
        }
      }
    })();
    try {
      await running;
    } finally {
      running = undefined;
    }
  }
  return {
    request() {
      if (closed) return;
      dirty = true;
      clearTimeout(timer);
      timer = setTimeout(() => {
        void drain();
      }, delay);
    },
    async close() {
      closed = true;
      clearTimeout(timer);
      await running;
    },
  };
}

function capabilitiesTarget(filename: string): string | undefined {
  try {
    return realpathSync(filename);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  // A dangling link still has a target directory to watch for recreation.
  try {
    return path.resolve(path.dirname(filename), readlinkSync(filename));
  } catch (error) {
    if (!["ENOENT", "EINVAL"].includes((error as NodeJS.ErrnoException).code ?? "")) throw error;
    return undefined;
  }
}

export function watchApplication(
  app: string,
  {
    root = workspaceRoot,
    appDirectory = path.join(root, "apps/examples", app),
    development = false,
    strictTypecheck = false,
    extraCapabilities,
    capabilitiesFile,
    onSuccess = ({ dist, manifest }) => console.log(`Published ${dist} ${manifest.buildId}`),
    onError = error =>
      console.error(
        error !== null && typeof error === "object" && "message" in error
          ? error.message
          : String(error),
      ),
    onTypecheck,
    signal,
  }: WatchApplicationOptions = {},
  {
    watchBundle: startWatchBundle = watchBundle,
    typecheck = typecheckProjectAsync,
    publish = publishApplication,
    publishStatus = publishBuildStatus,
  }: WatchDependencies = {},
): ApplicationWatcher {
  if (!/^[a-z][a-z0-9-]*$/.test(app)) throw new Error(`Invalid application name: ${app}`);
  root = path.resolve(root);
  appDirectory = path.resolve(appDirectory);
  if (capabilitiesFile !== undefined) capabilitiesFile = path.resolve(capabilitiesFile);
  const asyncTypecheck = development && !strictTypecheck;
  const controller = new AbortController();
  const abort = () => controller.abort(signal?.reason);
  signal?.addEventListener("abort", abort, { once: true });
  if (signal?.aborted) abort();
  let compiler: ApplicationWatcher | undefined;
  let temporary: string;
  let status: Extract<BuildStatus, { status: "building" }> | undefined;
  let published: PublishedApplication | undefined;
  let activeAttemptId: string | undefined;
  let activeTypecheckAbort: AbortController | undefined;
  let activeTypecheckRun: Promise<void> | undefined;
  let closed = false;
  let closing: Promise<void> | undefined;
  let restart = false;
  const watchers: FSWatcher[] = [];
  let targetWatcher: FSWatcher | undefined;
  const dependencyWatchers = new Map<string, FSWatcher>();
  let graphModules = new Set<string>();
  const report = (error: unknown) => {
    if (!closed && !controller.signal.aborted) onError(error);
  };
  const cancelActiveTypecheck = () => {
    activeTypecheckAbort?.abort();
    activeTypecheckAbort = undefined;
  };

  async function watchDependencyManifests(modules: Iterable<string>) {
    const directories = new Set<string>();
    for (const filename of modules) {
      let directory = path.dirname(filename);
      while (true) {
        try {
          await access(path.join(directory, "package.json"));
          directories.add(directory);
          break;
        } catch (error) {
          if (
            !(
              error !== null &&
              typeof error === "object" &&
              "code" in error &&
              error.code === "ENOENT"
            )
          )
            throw error;
        }
        const parent = path.dirname(directory);
        if (parent === directory) break;
        directory = parent;
      }
    }
    for (const [directory, watcher] of dependencyWatchers) {
      if (!directories.has(directory)) {
        watcher.close();
        dependencyWatchers.delete(directory);
      }
    }
    for (const directory of directories) {
      if (dependencyWatchers.has(directory)) continue;
      if (closed || controller.signal.aborted) return;
      const watcher = watch(directory, (_event, filename) => {
        if (filename?.toString() === "package.json") {
          restart = true;
          queue.request();
        }
      });
      watcher.on("error", report);
      dependencyWatchers.set(directory, watcher);
    }
  }

  function startCompiler() {
    status = undefined;
    compiler = startWatchBundle(path.join(appDirectory, "src/main.tsx"), {
      root,
      appDirectory,
      temporaryDirectory: path.join(temporary, "output"),
      buildDelay: 80,
      development,
      watchFiles: [path.join(temporary, "invalidate")],
      async onAttempt() {
        published = undefined;
        status = undefined;
        cancelActiveTypecheck();
        controller.signal.throwIfAborted();
        await syncCssModuleTypes(appDirectory, { root });
        let previousDirectory: string | null;
        try {
          previousDirectory = await realpath(path.join(appDirectory, "dist"));
        } catch (error) {
          if (
            !(
              error !== null &&
              typeof error === "object" &&
              "code" in error &&
              error.code === "ENOENT"
            )
          )
            throw error;
          previousDirectory = null;
        }
        const attemptId = randomUUID();
        activeAttemptId = attemptId;
        const nextStatus: Extract<BuildStatus, { status: "building" }> = {
          schemaVersion: 1,
          attemptId,
          app,
          status: "building",
          previousDirectory,
          publicationDirectory: null,
          buildId: null,
          error: null,
          ...(asyncTypecheck ? { typecheck: { status: "running", diagnostics: null } } : {}),
        };
        await publishStatus(appDirectory, nextStatus);
        status = nextStatus;
      },
      async onCompile(result) {
        graphModules = new Set(result.graph.modules);
        await watchDependencyManifests(result.graph.modules);
        await syncCssModuleTypes(appDirectory, { root });
        if (!asyncTypecheck) {
          await typecheck(path.join(appDirectory, "tsconfig.json"), { signal: controller.signal });
        }
        // Observers leave building/failed states via publication identity even on byte-identical recovery.
        published = await publish(app, result, {
          root,
          appDirectory,
          development,
          extraCapabilities,
          capabilitiesFile,
          signal: controller.signal,
        });
        const publicationDirectory = await realpath(published.dist);
        const publishedStatus: Extract<BuildStatus, { status: "published" }> = {
          ...status!,
          status: "published",
          publicationDirectory,
          buildId: published.manifest.buildId,
          ...(asyncTypecheck ? { typecheck: { status: "running", diagnostics: null } } : {}),
        };
        await publishStatus(appDirectory, publishedStatus);
        if (!closed && published) onSuccess(published);

        if (asyncTypecheck && !closed && !controller.signal.aborted) {
          const attemptId = publishedStatus.attemptId;
          cancelActiveTypecheck();
          const tcController = new AbortController();
          const onParentAbort = () => tcController.abort(controller.signal.reason);
          controller.signal.addEventListener("abort", onParentAbort, { once: true });
          activeTypecheckAbort = tcController;
          const run = (async () => {
            try {
              await typecheck(path.join(appDirectory, "tsconfig.json"), {
                signal: tcController.signal,
              });
              if (
                closed ||
                tcController.signal.aborted ||
                controller.signal.aborted ||
                activeAttemptId !== attemptId
              ) {
                return;
              }
              await publishStatus(appDirectory, {
                ...publishedStatus,
                typecheck: { status: "passed", diagnostics: null },
              });
              onTypecheck?.({ status: "passed", attemptId });
            } catch (error) {
              if (
                closed ||
                tcController.signal.aborted ||
                controller.signal.aborted ||
                activeAttemptId !== attemptId
              ) {
                return;
              }
              const diagnostics = String(
                (error !== null && typeof error === "object" && "message" in error
                  ? error.message
                  : undefined) ?? error,
              )
                .slice(0, 2048)
                .toWellFormed();
              try {
                await publishStatus(appDirectory, {
                  ...publishedStatus,
                  typecheck: { status: "failed", diagnostics },
                });
              } catch (statusError) {
                report(statusError);
              }
              onTypecheck?.({ status: "failed", attemptId, diagnostics });
            } finally {
              controller.signal.removeEventListener("abort", onParentAbort);
              if (activeTypecheckAbort === tcController) activeTypecheckAbort = undefined;
            }
          })();
          activeTypecheckRun = run;
        }
      },
      async onError(error) {
        try {
          if (status && !controller.signal.aborted)
            await publishStatus(appDirectory, {
              ...status,
              status: "failed",
              error: {
                code: "E_BUILD",
                message: String(
                  (error !== null && typeof error === "object" && "message" in error
                    ? error.message
                    : undefined) ?? error,
                )
                  .slice(0, 2048)
                  .toWellFormed(),
              },
            });
          report(error);
        } catch (failure) {
          report(failure);
        }
      },
    });
  }

  // Rolldown watches the actual dependency graph, including external app dependencies.
  // The sentinel adds missing imports, new resources, and config changes to that loop.
  const queue = rebuildQueue(
    async () => {
      await initialized;
      if (closed || controller.signal.aborted) return;
      if (restart) {
        restart = false;
        await compiler?.close();
        if (!closed) startCompiler();
      } else await writeFile(path.join(temporary, "invalidate"), randomUUID());
    },
    { onError: report },
  );
  const changed = (filename: string | Buffer | null, directory = root) => {
    if (filename && capabilitiesFile === path.resolve(directory, filename.toString())) return;
    const parts = filename?.toString().split(/[\\/]/) ?? [];
    if (
      parts.some(
        part =>
          part === "dist" ||
          part === ".zgui-generations" ||
          part === "node_modules" ||
          part.startsWith(".dist-") ||
          part === BUILD_STATUS_FILE ||
          part.startsWith(`${BUILD_STATUS_FILE}.`) ||
          part === NATIVE_STATUS_FILE ||
          part.startsWith(`${NATIVE_STATUS_FILE}.`) ||
          part.endsWith(".module.css.d.ts"),
      )
    )
      return;
    if (
      parts.some(part => part === "package.json" || /^tsconfig.*\.json$/.test(part)) ||
      parts.some(part => ["bun.lock", "bunfig.toml"].includes(part))
    )
      restart = true;
    else if (filename && graphModules.has(path.resolve(directory, filename.toString()))) return;
    queue.request();
  };
  try {
    for (const directory of new Set([
      appDirectory,
      path.join(root, "packages/ui"),
      path.join(root, "packages/services"),
    ])) {
      if (
        directory !== appDirectory &&
        !statSync(directory, { throwIfNoEntry: false })?.isDirectory()
      )
        continue;
      watchers.push(
        watch(directory, { recursive: true }, (_event, filename) => changed(filename, directory)),
      );
    }
    if (capabilitiesFile !== undefined) {
      const configured = capabilitiesFile;
      let watchedTarget: string | undefined;
      const watchFile = (file: string, onChange: () => void) => {
        const basename = path.basename(file);
        return watch(path.dirname(file), (_event, filename) => {
          if (filename === null || filename.toString() === basename) onChange();
        });
      };
      const refreshTarget = () => {
        const target = capabilitiesTarget(configured);
        if (target === watchedTarget) return;
        targetWatcher?.close();
        targetWatcher = undefined;
        watchedTarget = undefined;
        if (target === undefined || target === configured) return;
        try {
          targetWatcher = watchFile(target, onChange);
          targetWatcher.on("error", report);
          watchedTarget = target;
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
        }
      };
      const onChange = () => {
        try {
          refreshTarget();
        } catch (error) {
          report(error);
        }
        queue.request();
      };
      watchers.push(watchFile(configured, onChange));
      refreshTarget();
    }
    watchers.push(
      watch(root, (_event, filename) => {
        if (
          ["bun.lock", "Cargo.lock", "Cargo.toml", "package.json", "bunfig.toml"].includes(
            filename?.toString() ?? "",
          ) ||
          /^tsconfig.*\.json$/.test(filename?.toString() ?? "")
        )
          changed(filename);
      }),
    );
    for (const watcher of watchers) watcher.on("error", report);
  } catch (error) {
    targetWatcher?.close();
    for (const watcher of watchers) watcher.close();
    signal?.removeEventListener("abort", abort);
    throw error;
  }
  const initialized = (async () => {
    temporary = await mkdtemp(path.join(tmpdir(), "zgui-compiler-"));
    await writeFile(path.join(temporary, "invalidate"), "");
    if (!closed && !controller.signal.aborted) startCompiler();
  })();
  void initialized.catch(report);
  return {
    close() {
      return (closing ??= (async () => {
        closed = true;
        cancelActiveTypecheck();
        controller.abort();
        signal?.removeEventListener("abort", abort);
        for (const watcher of dependencyWatchers.values()) watcher.close();
        targetWatcher?.close();
        for (const watcher of watchers) watcher.close();
        await initialized.catch(() => {});
        await queue.close();
        await compiler?.close();
        await activeTypecheckRun?.catch(() => {});
        if (temporary) await rm(temporary, { recursive: true, force: true });
      })());
    },
  };
}
