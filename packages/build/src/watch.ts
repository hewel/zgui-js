import { watch, type FSWatcher } from "node:fs";
import { access, realpath, rm, writeFile, mkdtemp } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { tmpdir } from "node:os";
import { publishApplication, workspaceRoot, type PublishedApplication } from "./build.ts";
import { watchBundle } from "./compiler.ts";
import {
  BUILD_STATUS_FILE,
  NATIVE_STATUS_FILE,
  publishBuildStatus,
  type BuildStatus,
} from "./publication.ts";
import { typecheckProjectAsync } from "./typecheck.ts";

interface WatchOptions {
  root?: string;
  appDirectory?: string;
  stylexDiagnostics?: boolean;
  development?: boolean;
  onSuccess?: (result: PublishedApplication) => void;
  onError?: (error: unknown) => void;
  signal?: AbortSignal;
}

interface WatchDependencies {
  watchBundle?: typeof watchBundle;
  typecheck?: typeof typecheckProjectAsync;
  publish?: typeof publishApplication;
  publishStatus?: typeof publishBuildStatus;
}

export interface ApplicationWatcher {
  close(): Promise<void>;
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

export function watchApplication(
  app: string,
  {
    root = workspaceRoot,
    appDirectory = path.join(root, "apps/examples", app),
    stylexDiagnostics = false,
    development = false,
    onSuccess = ({ dist, manifest }) => console.log(`Published ${dist} ${manifest.buildId}`),
    onError = error =>
      console.error(
        error !== null && typeof error === "object" && "message" in error
          ? error.message
          : String(error),
      ),
    signal,
  }: WatchOptions = {},
  {
    watchBundle: startWatchBundle = watchBundle,
    typecheck = typecheckProjectAsync,
    publish = publishApplication,
    publishStatus = publishBuildStatus,
  }: WatchDependencies = {},
): ApplicationWatcher {
  if (!/^[a-z][a-z0-9-]*$/.test(app)) throw new Error(`Invalid application name: ${app}`);
  const controller = new AbortController();
  const abort = () => controller.abort(signal?.reason);
  signal?.addEventListener("abort", abort, { once: true });
  if (signal?.aborted) abort();
  let compiler: ApplicationWatcher | undefined;
  let temporary: string;
  let status: Extract<BuildStatus, { status: "building" }> | undefined;
  let published: PublishedApplication | undefined;
  let closed = false;
  let closing: Promise<void> | undefined;
  let restart = false;
  const watchers: FSWatcher[] = [];
  const dependencyWatchers = new Map<string, FSWatcher>();
  let graphModules = new Set<string>();
  const report = (error: unknown) => {
    if (!closed && !controller.signal.aborted) onError(error);
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
      stylexDiagnostics,
      development,
      watchFiles: [path.join(temporary, "invalidate")],
      async onAttempt() {
        published = undefined;
        status = undefined;
        controller.signal.throwIfAborted();
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
        const nextStatus: Extract<BuildStatus, { status: "building" }> = {
          schemaVersion: 1,
          attemptId: randomUUID(),
          app,
          status: "building",
          previousDirectory,
          publicationDirectory: null,
          buildId: null,
          error: null,
        };
        await publishStatus(appDirectory, nextStatus);
        status = nextStatus;
      },
      async onCompile(result) {
        graphModules = new Set(result.graph.modules);
        await watchDependencyManifests(result.graph.modules);
        await typecheck(path.join(appDirectory, "tsconfig.json"), { signal: controller.signal });
        // Observers leave building/failed states via publication identity even on byte-identical recovery.
        published = await publish(app, result, {
          root,
          appDirectory,
          development,
          signal: controller.signal,
        });
        await publishStatus(appDirectory, {
          ...status!,
          status: "published",
          publicationDirectory: await realpath(published.dist),
          buildId: published.manifest.buildId,
        });
        if (!closed && published) onSuccess(published);
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
          part.startsWith(`${NATIVE_STATUS_FILE}.`),
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
      watchers.push(
        watch(directory, { recursive: true }, (_event, filename) => changed(filename, directory)),
      );
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
        controller.abort();
        signal?.removeEventListener("abort", abort);
        for (const watcher of dependencyWatchers.values()) watcher.close();
        for (const watcher of watchers) watcher.close();
        await initialized.catch(() => {});
        await queue.close();
        await compiler?.close();
        if (temporary) await rm(temporary, { recursive: true, force: true });
      })());
    },
  };
}
