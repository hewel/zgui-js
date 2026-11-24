import { createHash } from "node:crypto";
import { open, readFile, rm, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { workspaceRoot } from "../../packages/build/src/internal.ts";

let queue = Promise.resolve();

async function acquireFileLock(lockPath) {
  const deadline = Date.now() + 60_000;
  while (true) {
    try {
      const handle = await open(lockPath, "wx");
      await handle.writeFile(`${process.pid}\n`);
      return handle;
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      try {
        const info = await stat(lockPath);
        const pid = Number.parseInt((await readFile(lockPath, "utf8")).trim(), 10);
        let alive = false;
        if (Number.isInteger(pid) && pid > 0) {
          try {
            process.kill(pid, 0);
            alive = true;
          } catch {
            alive = false;
          }
        }
        if (!alive || Date.now() - info.mtimeMs > 60_000) {
          await rm(lockPath, { force: true });
          continue;
        }
      } catch {
        continue;
      }
      if (Date.now() > deadline) {
        throw new Error(`Timed out waiting for pack lock ${lockPath}`);
      }
      await Bun.sleep(25);
    }
  }
}

export function withPackLock(fn, root = workspaceRoot) {
  const key = createHash("sha256").update(path.resolve(root)).digest("hex").slice(0, 16);
  const lockPath = path.join(os.tmpdir(), `zgui-pack-lock-${key}.lock`);
  const run = async () => {
    const handle = await acquireFileLock(lockPath);
    try {
      return await fn();
    } finally {
      await handle.close().catch(() => {});
      await rm(lockPath, { force: true }).catch(() => {});
    }
  };
  const next = queue.then(run, run);
  queue = next.then(
    () => {},
    () => {},
  );
  return next;
}
