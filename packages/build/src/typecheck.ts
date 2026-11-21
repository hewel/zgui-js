import type { Dirent } from "node:fs";
import { readdir, access } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { syncCssModuleTypes } from "./css.ts";

const compilerCli = path.join(
  path.dirname(createRequire(import.meta.url).resolve("tsrs/package.json")),
  "bin/tsrs",
);
const stdio = { stdin: "ignore", stdout: "pipe", stderr: "pipe" } as const;

function command(configPath: string): string[] {
  return [
    process.execPath,
    compilerCli,
    "--project",
    path.resolve(configPath),
    "--noEmit",
    "--pretty",
    "false",
  ];
}

export function typecheckProject(configPath: string): void {
  const child = Bun.spawnSync(command(configPath), stdio);
  if (!child.success) {
    const diagnostics = `${child.stdout.toString()}${child.stderr.toString()}`.trim();
    throw new Error(diagnostics || `TypeScript exited with ${child.signalCode ?? child.exitCode}`);
  }
}

// Dev compilation must leave the event loop free for file changes and shutdown.
export async function typecheckProjectAsync(
  configPath: string,
  { signal }: { signal?: AbortSignal } = {},
): Promise<void> {
  signal?.throwIfAborted();
  const child = Bun.spawn(command(configPath), stdio);
  let killTimer: ReturnType<typeof setTimeout> | undefined;
  const abort = () => {
    child.kill("SIGTERM");
    killTimer = setTimeout(() => child.kill("SIGKILL"), 1000);
    killTimer.unref();
  };
  signal?.addEventListener("abort", abort, { once: true });
  if (signal?.aborted) abort();
  try {
    const [code, stdout, stderr] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ]);
    if (code !== 0 || child.signalCode) {
      throw new Error(
        `${stdout}${stderr}`.trim() || `TypeScript exited with ${child.signalCode ?? code}`,
      );
    }
  } finally {
    clearTimeout(killTimer);
    signal?.removeEventListener("abort", abort);
    signal?.throwIfAborted();
  }
}

export async function typecheckWorkspace(
  root = fileURLToPath(new URL("../../../", import.meta.url)),
): Promise<void> {
  const configs = [
    path.join(root, "packages/build/tsconfig.json"),
    path.join(root, "packages/ui/tsconfig.native.json"),
    path.join(root, "packages/services/tsconfig.json"),
    path.join(root, "tests/types/tsconfig.json"),
    path.join(root, "tests/types/tsconfig.services.json"),
  ];
  const examples = path.join(root, "apps/examples");
  let entries: Dirent[] = [];
  try {
    entries = await readdir(examples, { withFileTypes: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    if (!entry.isDirectory()) continue;
    const exampleDir = path.join(examples, entry.name);
    const config = path.join(exampleDir, "tsconfig.json");
    try {
      await access(config);
      await syncCssModuleTypes(exampleDir, { root });
      configs.push(config);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
  }
  for (const config of configs) {
    typecheckProject(config);
    console.log(`Typechecked ${path.relative(root, config)}`);
  }
}

if (import.meta.main) {
  try {
    await typecheckWorkspace();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
