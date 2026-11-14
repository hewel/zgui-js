import { watch } from "node:fs";
import { realpath, rename, rm, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { buildApplication, workspaceRoot } from "./build.mjs";

// Serial execution plus a dirty bit retains edits arriving during a build without overlapping publication.
export function rebuildQueue(build, { delay = 80, onSuccess = () => {}, onError = () => {} } = {}) {
  let running;
  let dirty = false;
  let closed = false;
  let timer;
  async function drain() {
    if (running || closed) return;
    running = (async () => {
      while (dirty && !closed) {
        dirty = false;
        try { onSuccess(await build()); }
        catch (error) { onError(error); }
      }
    })();
    try { await running; }
    finally { running = undefined; }
  }
  return {
    request() {
      if (closed) return;
      dirty = true;
      clearTimeout(timer);
      timer = setTimeout(() => { void drain(); }, delay);
    },
    async close() {
      closed = true;
      clearTimeout(timer);
      await running;
    },
  };
}

const statusFilename = ".zgui-build-status.json";

async function publishBuildStatus(appDirectory, status) {
  const bytes = JSON.stringify(status) + "\n";
  if (Buffer.byteLength(bytes) > 16 * 1024) throw new Error("Build status exceeds 16 KiB");
  const temporary = path.join(appDirectory, `${statusFilename}.${randomUUID()}.tmp`);
  try {
    await writeFile(temporary, bytes, { flag: "wx" });
    await rename(temporary, path.join(appDirectory, statusFilename));
  } finally {
    await rm(temporary, { force: true });
  }
}

async function buildAttempt(app, { root, appDirectory }) {
  let previousDirectory;
  try { previousDirectory = await realpath(path.join(appDirectory, "dist")); }
  catch (error) { if (error.code !== "ENOENT") throw error; previousDirectory = null; }
  const status = {
    schemaVersion: 1, attemptId: randomUUID(), app, status: "building",
    previousDirectory, publicationDirectory: null, buildId: null, error: null,
  };
  await publishBuildStatus(appDirectory, status);
  let result;
  try {
    result = await buildApplication(app, { root, appDirectory });
  } catch (error) {
    await publishBuildStatus(appDirectory, {
      ...status, status: "failed",
      error: { code: "E_BUILD", message: String(error.message ?? error).slice(0, 2048).toWellFormed() },
    });
    throw error;
  }
  await publishBuildStatus(appDirectory, {
    ...status, status: "published",
    publicationDirectory: await realpath(result.dist), buildId: result.manifest.buildId,
  });
  return result;
}

export function watchApplication(app, { root = workspaceRoot, appDirectory = path.join(root, "apps/examples", app), onSuccess, onError = error => console.error(error.message) } = {}) {
  const queue = rebuildQueue(() => buildAttempt(app, { root, appDirectory }), {
    onSuccess: onSuccess ?? (({ dist, manifest }) => console.log(`Published ${dist} ${manifest.buildId}`)), onError,
  });
  const watchers = [];
  try {
    for (const directory of [appDirectory, path.join(root, "packages/ui"), path.join(root, "packages/services"), path.join(root, "packages/build/src")]) {
      watchers.push(watch(directory, { recursive: true }, (_event, filename) => {
        const parts = filename?.toString().split(/[\\/]/) ?? [];
        if (parts.some(part => part === "dist" || part === ".zgui-generations" || part === "node_modules" || part.startsWith(".dist-") || part === statusFilename || part.startsWith(`${statusFilename}.`))) return;
        queue.request();
      }));
    }
    watchers.push(watch(root, (_event, filename) => {
      if (["pnpm-lock.yaml", "Cargo.lock", "Cargo.toml", "package.json", "pnpm-workspace.yaml"].includes(filename?.toString())) queue.request();
    }));
    for (const watcher of watchers) watcher.on("error", onError);
    queue.request();
  } catch (error) {
    for (const watcher of watchers) watcher.close();
    void queue.close();
    throw error;
  }
  return {
    async close() {
      for (const watcher of watchers) watcher.close();
      await queue.close();
    },
  };
}
