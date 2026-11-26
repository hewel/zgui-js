import type { Dirent } from "node:fs";
import { readdir, access } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { syncCssModuleTypes } from "./css.ts";
import { ensureSdkPackagesBuilt } from "./sdk-packages.ts";

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

async function fileExists(filePath: string): Promise<boolean> {
  try {
    await access(filePath);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
    throw error;
  }
}

async function readSortedEntries(directory: string): Promise<Dirent[]> {
  try {
    const entries = await readdir(directory, { withFileTypes: true });
    return entries.sort((a, b) => a.name.localeCompare(b.name));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
}

async function discoverPackageConfigs(root: string): Promise<string[]> {
  const packagesDir = path.join(root, "packages");
  const configs: string[] = [];
  for (const entry of await readSortedEntries(packagesDir)) {
    if (!entry.isDirectory()) continue;
    const pkgDir = path.join(packagesDir, entry.name);
    for (const candidate of ["tsconfig.json", "tsconfig.native.json"]) {
      const configPath = path.join(pkgDir, candidate);
      if (await fileExists(configPath)) {
        configs.push(configPath);
        break;
      }
    }
    const oxlintConfigPath = path.join(pkgDir, "tsconfig.oxlint.json");
    if (await fileExists(oxlintConfigPath)) {
      configs.push(oxlintConfigPath);
    }
  }
  return configs;
}

async function discoverTypeTestConfigs(root: string): Promise<string[]> {
  const typesDir = path.join(root, "tests/types");
  return (await readSortedEntries(typesDir))
    .filter(entry => entry.isFile() && /^tsconfig.*\.json$/.test(entry.name))
    .map(entry => path.join(typesDir, entry.name));
}

async function discoverAppConfigs(root: string, appFilter?: string): Promise<string[]> {
  const configs: string[] = [];
  for (const group of ["apps/devtools", "apps/examples"]) {
    const groupDir = path.join(root, group);
    for (const entry of await readSortedEntries(groupDir)) {
      if (!entry.isDirectory()) continue;
      if (appFilter && entry.name !== appFilter && `${group}/${entry.name}` !== appFilter) {
        continue;
      }
      const appDir = path.join(groupDir, entry.name);
      const config = path.join(appDir, "tsconfig.json");
      if (!(await fileExists(config))) continue;
      // Run outside ENOENT guard so CSS Module type generation failures surface instead of silently skipping.
      await syncCssModuleTypes(appDir, { root });
      configs.push(config);
    }
  }
  if (appFilter && configs.length === 0) {
    const directDir = path.resolve(root, appFilter);
    const directConfig = path.join(directDir, "tsconfig.json");
    if (await fileExists(directConfig)) {
      await syncCssModuleTypes(directDir, { root });
      configs.push(directConfig);
    } else {
      throw new Error(`No app tsconfig.json found for --app ${appFilter}`);
    }
  }
  return configs;
}

export async function typecheckWorkspace(
  root = fileURLToPath(new URL("../../../", import.meta.url)),
  { app }: { app?: string } = {},
): Promise<void> {
  await ensureSdkPackagesBuilt(root);
  const configs = app
    ? await discoverAppConfigs(root, app)
    : [
        ...(await discoverPackageConfigs(root)),
        ...(await discoverTypeTestConfigs(root)),
        ...(await discoverAppConfigs(root)),
      ];
  for (const config of configs) {
    typecheckProject(config);
    console.log(`Typechecked ${path.relative(root, config)}`);
  }
}

if (import.meta.main) {
  try {
    const args = process.argv.slice(2);
    let app: string | undefined;
    for (let i = 0; i < args.length; i++) {
      if (args[i] === "--app") {
        app = args[++i];
        if (!app) throw new Error("--app requires an application name or directory");
      } else {
        throw new Error(`Unknown option: ${args[i]}`);
      }
    }
    await typecheckWorkspace(undefined, { app });
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
