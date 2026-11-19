import { access, realpath } from "node:fs/promises";
import path from "node:path";
import { workspaceRoot } from "./build.ts";

export type ResolvedApplication = { app: string; appDirectory: string; root: string };

async function exists(filename: string): Promise<boolean> {
  try {
    await access(filename);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
    throw error;
  }
}

export async function resolveApplication(
  selector?: string,
  { cwd = process.cwd(), root = workspaceRoot } = {},
): Promise<ResolvedApplication> {
  let directory: string;
  if (selector !== undefined) {
    const local = path.resolve(cwd, selector);
    // Explicit paths never fall back to a similarly named repository example.
    const explicitPath =
      path.isAbsolute(selector) ||
      selector.includes("/") ||
      selector.includes("\\") ||
      selector === "." ||
      selector === "..";
    directory =
      explicitPath || (await exists(local)) ? local : path.join(root, "apps/examples", selector);
  } else {
    directory = (await Bun.file(path.join(cwd, "src/main.tsx")).exists())
      ? cwd
      : path.resolve(cwd) === path.resolve(root)
        ? path.join(root, "apps/examples/counter")
        : cwd;
  }
  const appDirectory = await realpath(directory).catch((error: Error) => {
    throw new Error(`Cannot open application directory ${directory}: ${error.message}`, {
      cause: error,
    });
  });
  for (const filename of ["src/main.tsx", "tsconfig.json"]) {
    if (!(await Bun.file(path.join(appDirectory, filename)).exists()))
      throw new Error(`Application ${appDirectory} requires ${filename}`);
  }
  let name = path.basename(appDirectory);
  try {
    const manifest = (await Bun.file(path.join(appDirectory, "package.json")).json()) as {
      name?: unknown;
    };
    if (
      path.dirname(appDirectory) !== path.join(root, "apps/examples") &&
      typeof manifest.name === "string" &&
      manifest.name
    ) {
      name = manifest.name.slice(manifest.name.lastIndexOf("/") + 1);
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  // The native manifest identity uses a portable, deterministic identifier.
  let app = name
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  if (!/^[a-z]/.test(app)) app = `app-${app || "local"}`;
  return { app, appDirectory, root };
}
