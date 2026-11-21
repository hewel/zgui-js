import { randomUUID } from "node:crypto";
import { lstat, mkdir, rename, rm, symlink, writeFile } from "node:fs/promises";
import path from "node:path";

export const BUILD_STATUS_FILE = ".zgui-build-status.json";
export const NATIVE_STATUS_FILE = ".zgui-native-status.json";

export type TypecheckStatus =
  | { status: "running"; diagnostics: null }
  | { status: "passed"; diagnostics: null }
  | { status: "failed"; diagnostics: string };

type BuildAttempt = {
  schemaVersion: 1;
  attemptId: string;
  app: string;
  previousDirectory: string | null;
  typecheck?: TypecheckStatus;
};

export type BuildStatus = BuildAttempt &
  (
    | { status: "building"; publicationDirectory: null; buildId: null; error: null }
    | { status: "published"; publicationDirectory: string; buildId: string; error: null }
    | {
        status: "failed";
        publicationDirectory: null;
        buildId: null;
        error: { code: "E_BUILD"; message: string };
      }
  );

export function resourcePath(name: string): string {
  if (
    !name ||
    name.includes("\\") ||
    name.includes("\0") ||
    name.startsWith("/") ||
    name.split("/").some(part => !part || part === "." || part === "..")
  ) {
    throw new Error(`Non-canonical resource path: ${name}`);
  }
  return name;
}

export async function activatePublication(
  appDirectory: string,
  packageDirectory: string,
): Promise<string> {
  const token = randomUUID();
  const link = path.join(appDirectory, `.dist-${token}`);
  const dist = path.join(appDirectory, "dist");
  try {
    try {
      const existing = await lstat(dist);
      if (!existing.isSymbolicLink())
        throw new Error("dist must be absent or a package symlink; refusing to overwrite it");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    await symlink(path.relative(appDirectory, packageDirectory), link, "dir");
    // Linux rename replaces the symlink atomically. Prior candidates remain pinned for active readers.
    await rename(link, dist);
    return dist;
  } finally {
    await rm(link, { force: true });
  }
}

export async function publishGeneration(
  appDirectory: string,
  files: ReadonlyMap<string, Uint8Array>,
): Promise<string> {
  const generations = path.join(appDirectory, ".zgui-generations");
  await mkdir(generations, { recursive: true });
  const token = randomUUID();
  const staging = path.join(generations, `.building-${token}`);
  const generation = path.join(generations, token);
  let published = false;
  try {
    await mkdir(staging);
    for (const [name, bytes] of files) {
      resourcePath(name);
      const destination = path.join(staging, name);
      await mkdir(path.dirname(destination), { recursive: true });
      await writeFile(destination, bytes, { flag: "wx" });
    }
    await rename(staging, generation);
    const dist = await activatePublication(appDirectory, generation);
    published = true;
    return dist;
  } finally {
    await rm(staging, { recursive: true, force: true });
    if (!published) await rm(generation, { recursive: true, force: true });
  }
}

export async function publishBuildStatus(appDirectory: string, status: BuildStatus): Promise<void> {
  const bytes = JSON.stringify(status) + "\n";
  if (Buffer.byteLength(bytes) > 16 * 1024) throw new Error("Build status exceeds 16 KiB");
  const temporary = path.join(appDirectory, `${BUILD_STATUS_FILE}.${randomUUID()}.tmp`);
  try {
    await writeFile(temporary, bytes, { flag: "wx" });
    await rename(temporary, path.join(appDirectory, BUILD_STATUS_FILE));
  } finally {
    await rm(temporary, { force: true });
  }
}
