import { watch } from "node:fs";
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

export function watchApplication(app, { root = workspaceRoot, appDirectory = path.join(root, "apps/examples", app), onSuccess, onError = error => console.error(error.message) } = {}) {
  const queue = rebuildQueue(() => buildApplication(app, { root, appDirectory }), {
    onSuccess: onSuccess ?? (({ dist, manifest }) => console.log(`Published ${dist} ${manifest.buildId}`)), onError,
  });
  const watchers = [];
  try {
    for (const directory of [appDirectory, path.join(root, "packages/ui"), path.join(root, "packages/build/src")]) {
      watchers.push(watch(directory, { recursive: true }, (_event, filename) => {
        const parts = filename?.toString().split(/[\\/]/) ?? [];
        if (parts.some(part => part === "dist" || part === ".zgui-generations" || part === "node_modules" || part.startsWith(".dist-"))) return;
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
