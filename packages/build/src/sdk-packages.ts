import { createHash, randomUUID } from "node:crypto";
import { existsSync, type Dirent } from "node:fs";
import {
  copyFile,
  mkdir,
  open,
  readdir,
  readFile,
  rename,
  rm,
  rmdir,
  stat,
  utimes,
  writeFile,
} from "node:fs/promises";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { extractStaticComponentSheet } from "./compiler.ts";
import { packagedSdkRoot, workspaceRoot } from "./sdk.ts";

export type SdkPackageName = "ui" | "services";

interface SdkPackageSpec {
  readonly name: SdkPackageName;
  readonly dir: string;
  readonly handwrittenDts: readonly string[];
  readonly verifyGeneratedStyles: boolean;
}

const SDK_PACKAGE_SPECS: Readonly<Record<SdkPackageName, SdkPackageSpec>> = {
  ui: {
    name: "ui",
    dir: "packages/ui",
    handwrittenDts: ["solid-native.d.ts", "events.d.ts", "styles.d.ts"],
    verifyGeneratedStyles: true,
  },
  services: {
    name: "services",
    dir: "packages/services",
    handwrittenDts: ["solid.d.ts"],
    verifyGeneratedStyles: false,
  },
};

const ALL_SDK_PACKAGES: readonly SdkPackageName[] = ["ui", "services"];
const OXLINT_OUTPUT_FILES = [
  "oxlint/plugin.js",
  "oxlint/solid.js",
  "oxlint/ast.js",
  "oxlint/scope.js",
  "oxlint/style-analysis.js",
] as const;

const compilerCli = path.join(
  path.dirname(createRequire(import.meta.url).resolve("tsrs/package.json")),
  "bin/tsrs",
);

const inFlight = new Map<string, Promise<void>>();

export function hasCheckoutSdkSources(root: string): boolean {
  const resolved = path.resolve(root);
  if (resolved === packagedSdkRoot) return false;
  return ALL_SDK_PACKAGES.every(pkg =>
    existsSync(path.join(resolved, SDK_PACKAGE_SPECS[pkg].dir, "tsconfig.build.json")),
  );
}

async function listFilesRecursive(dir: string, prefix = ""): Promise<string[]> {
  let entries: Dirent[];
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
  const files: string[] = [];
  for (const entry of entries) {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listFilesRecursive(full, rel)));
    } else if (entry.isFile()) {
      files.push(rel);
    }
  }
  return files;
}

async function newestInputMtimeMs(pkgDir: string): Promise<number> {
  let newest = 0;
  for (const cfg of ["tsconfig.json", "tsconfig.build.json", "tsconfig.oxlint.json"]) {
    const full = path.join(pkgDir, cfg);
    if (existsSync(full)) {
      const info = await stat(full);
      if (info.mtimeMs > newest) newest = info.mtimeMs;
    }
  }
  for (const inputDir of [path.join(pkgDir, "src"), path.join(pkgDir, "oxlint/src")]) {
    for (const rel of await listFilesRecursive(inputDir)) {
      const info = await stat(path.join(inputDir, rel));
      if (info.mtimeMs > newest) newest = info.mtimeMs;
    }
  }
  return newest;
}

async function isPackageDistFresh(pkgDir: string, spec: SdkPackageSpec): Promise<boolean> {
  const distDir = path.join(pkgDir, "dist");
  const hasOxlintBuild = existsSync(path.join(pkgDir, "tsconfig.oxlint.json"));
  const required = [
    "index.js",
    "index.d.ts",
    ...spec.handwrittenDts,
    ...(hasOxlintBuild ? OXLINT_OUTPUT_FILES : []),
  ];
  let stampMtime = Number.POSITIVE_INFINITY;
  for (const rel of required) {
    const full = path.join(distDir, rel);
    try {
      const info = await stat(full);
      if (
        rel === "index.js" ||
        rel === "index.d.ts" ||
        (hasOxlintBuild && (OXLINT_OUTPUT_FILES as readonly string[]).includes(rel))
      ) {
        stampMtime = Math.min(stampMtime, info.mtimeMs);
      }
    } catch {
      return false;
    }
  }
  if (!Number.isFinite(stampMtime)) return false;

  if (spec.verifyGeneratedStyles) {
    const srcGenDir = path.join(pkgDir, "src/components/generated");
    const distGenDir = path.join(distDir, "components/generated");
    try {
      const srcStyles = (await readdir(srcGenDir)).filter(name => name.endsWith("Style.ts"));
      const distStyles = (await readdir(distGenDir)).filter(name => name.endsWith("Style.js"));
      if (srcStyles.length === 0 || distStyles.length < srcStyles.length) return false;
    } catch {
      return false;
    }
  }

  const newestInput = await newestInputMtimeMs(pkgDir);
  return stampMtime >= newestInput;
}

async function removeEmptyDirs(dir: string): Promise<void> {
  let entries: Dirent[];
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const child = path.join(dir, entry.name);
    await removeEmptyDirs(child);
    try {
      const remaining = await readdir(child);
      if (remaining.length === 0) await rmdir(child);
    } catch {
      // Ignore concurrent deletion.
    }
  }
}

async function syncDirectoryInPlace(stagingDir: string, distDir: string): Promise<void> {
  await mkdir(distDir, { recursive: true });
  const stagedFiles = await listFilesRecursive(stagingDir);
  const stagedSet = new Set(stagedFiles);

  for (const rel of stagedFiles) {
    const srcPath = path.join(stagingDir, rel);
    const destPath = path.join(distDir, rel);
    const nextBytes = await readFile(srcPath);
    let unchanged = false;
    try {
      const currentBytes = await readFile(destPath);
      unchanged = currentBytes.equals(nextBytes);
    } catch {
      unchanged = false;
    }
    if (unchanged) continue;
    await mkdir(path.dirname(destPath), { recursive: true });
    const tmpPath = `${destPath}.tmp-${process.pid}-${randomUUID()}`;
    await writeFile(tmpPath, nextBytes);
    await rename(tmpPath, destPath);
  }

  const existingFiles = await listFilesRecursive(distDir);
  for (const rel of existingFiles) {
    if (stagedSet.has(rel)) continue;
    if (path.basename(rel).includes(".tmp-")) continue;
    await rm(path.join(distDir, rel), { force: true });
  }
  await removeEmptyDirs(distDir);

  const now = new Date();
  for (const sentinel of ["index.js", "index.d.ts", ...OXLINT_OUTPUT_FILES]) {
    const target = path.join(distDir, sentinel);
    if (existsSync(target)) {
      await utimes(target, now, now).catch(() => {});
    }
  }
}

async function verifyGeneratedStyleSheets(stagingDir: string): Promise<number> {
  const genDir = path.join(stagingDir, "components/generated");
  const entries = (await readdir(genDir)).filter(name => name.endsWith("Style.js")).sort();
  if (entries.length === 0) {
    throw new Error(`No generated component style modules emitted under ${genDir}`);
  }
  for (const filename of entries) {
    const fullPath = path.join(genDir, filename);
    const source = await readFile(fullPath, "utf8");
    const sheet = extractStaticComponentSheet(source, fullPath);
    if (!sheet) {
      throw new Error(
        `Generated component style ${filename} failed single-statement static sheet hoisting verification`,
      );
    }
  }
  return entries.length;
}

async function withSdkBuildLock<T>(root: string, fn: () => Promise<T>): Promise<T> {
  const key = createHash("sha256").update(path.resolve(root)).digest("hex").slice(0, 16);
  const lockPath = path.join(os.tmpdir(), `zgui-sdk-build-${key}.lock`);
  const deadline = Date.now() + 60_000;
  let handle: Awaited<ReturnType<typeof open>> | undefined;

  while (!handle) {
    try {
      handle = await open(lockPath, "wx");
      await handle.writeFile(`${process.pid}\n`);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      try {
        const info = await stat(lockPath);
        const ownerPid = Number.parseInt((await readFile(lockPath, "utf8")).trim(), 10);
        let ownerAlive = false;
        if (Number.isInteger(ownerPid) && ownerPid > 0) {
          try {
            process.kill(ownerPid, 0);
            ownerAlive = true;
          } catch {
            ownerAlive = false;
          }
        }
        if (!ownerAlive || Date.now() - info.mtimeMs > 60_000) {
          await rm(lockPath, { force: true });
          continue;
        }
      } catch {
        continue;
      }
      if (Date.now() > deadline) {
        throw new Error(`Timed out waiting for SDK build lock ${lockPath}`);
      }
      await Bun.sleep(25);
    }
  }

  try {
    return await fn();
  } finally {
    await handle.close().catch(() => {});
    await rm(lockPath, { force: true }).catch(() => {});
  }
}

async function compileSinglePackage(
  root: string,
  spec: SdkPackageSpec,
  { silent = false }: { silent?: boolean } = {},
): Promise<void> {
  const pkgDir = path.join(root, spec.dir);
  const tsconfigPath = path.join(pkgDir, "tsconfig.build.json");
  if (!existsSync(tsconfigPath)) {
    throw new Error(`Missing SDK build tsconfig: ${tsconfigPath}`);
  }
  const stagingDir = path.join(
    os.tmpdir(),
    `zgui-dist-staging-${spec.name}-${process.pid}-${randomUUID()}`,
  );
  try {
    const child = Bun.spawn(
      [
        process.execPath,
        compilerCli,
        "--project",
        tsconfigPath,
        "--outDir",
        stagingDir,
        "--pretty",
        "false",
      ],
      {
        cwd: pkgDir,
        stdin: "ignore",
        stdout: "pipe",
        stderr: "pipe",
      },
    );
    const [code, stdout, stderr] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ]);
    if (code !== 0 || child.signalCode) {
      const detail = `${stdout}${stderr}`.trim();
      throw new Error(
        `SDK build failed for ${spec.dir} (exit ${child.signalCode ?? code})${detail ? `:\n${detail}` : ""}`,
      );
    }

    for (const rel of spec.handwrittenDts) {
      const srcPath = path.join(pkgDir, "src", rel);
      const destPath = path.join(stagingDir, rel);
      await mkdir(path.dirname(destPath), { recursive: true });
      await copyFile(srcPath, destPath);
    }

    const oxlintTsconfigPath = path.join(pkgDir, "tsconfig.oxlint.json");
    if (existsSync(oxlintTsconfigPath)) {
      const oxlintChild = Bun.spawn(
        [
          process.execPath,
          compilerCli,
          "--project",
          oxlintTsconfigPath,
          "--outDir",
          path.join(stagingDir, "oxlint"),
          "--pretty",
          "false",
        ],
        {
          cwd: pkgDir,
          stdin: "ignore",
          stdout: "pipe",
          stderr: "pipe",
        },
      );
      const [oxCode, oxStdout, oxStderr] = await Promise.all([
        oxlintChild.exited,
        new Response(oxlintChild.stdout).text(),
        new Response(oxlintChild.stderr).text(),
      ]);
      if (oxCode !== 0 || oxlintChild.signalCode) {
        const detail = `${oxStdout}${oxStderr}`.trim();
        throw new Error(
          `SDK oxlint build failed for ${spec.dir} (exit ${oxlintChild.signalCode ?? oxCode})${detail ? `:\n${detail}` : ""}`,
        );
      }
    }

    let verifiedSheets = 0;
    if (spec.verifyGeneratedStyles) {
      verifiedSheets = await verifyGeneratedStyleSheets(stagingDir);
    }

    await syncDirectoryInPlace(stagingDir, path.join(pkgDir, "dist"));
    if (!silent) {
      const detail = verifiedSheets > 0 ? ` (${verifiedSheets} static style sheets verified)` : "";
      console.log(`Built ${spec.dir}/dist${detail}`);
    }
  } finally {
    await rm(stagingDir, { recursive: true, force: true }).catch(() => {});
  }
}

export async function buildSdkPackages({
  root = workspaceRoot,
  packages = ALL_SDK_PACKAGES,
  silent = false,
}: {
  root?: string;
  packages?: readonly SdkPackageName[];
  silent?: boolean;
} = {}): Promise<void> {
  const resolvedRoot = path.resolve(root);
  const uniquePackages = [...new Set(packages)];
  await withSdkBuildLock(resolvedRoot, async () => {
    for (const name of uniquePackages) {
      await compileSinglePackage(resolvedRoot, SDK_PACKAGE_SPECS[name], { silent });
    }
  });
}

export async function ensureSdkPackagesBuilt(
  root: string = workspaceRoot,
  {
    silent = true,
    packages = ALL_SDK_PACKAGES,
  }: { silent?: boolean; packages?: readonly SdkPackageName[] } = {},
): Promise<void> {
  const resolvedRoot = path.resolve(root);
  if (!hasCheckoutSdkSources(resolvedRoot)) return;

  const targetPackages = [...new Set(packages)];
  const cacheKey = `${resolvedRoot}:${[...targetPackages].sort().join(",")}`;
  const existing = inFlight.get(cacheKey);
  if (existing) return existing;

  const run = (async () => {
    const stale: SdkPackageName[] = [];
    for (const name of targetPackages) {
      const spec = SDK_PACKAGE_SPECS[name];
      if (!(await isPackageDistFresh(path.join(resolvedRoot, spec.dir), spec))) {
        stale.push(name);
      }
    }
    if (stale.length === 0) return;

    await withSdkBuildLock(resolvedRoot, async () => {
      for (const name of stale) {
        const spec = SDK_PACKAGE_SPECS[name];
        if (await isPackageDistFresh(path.join(resolvedRoot, spec.dir), spec)) continue;
        await compileSinglePackage(resolvedRoot, spec, { silent });
      }
    });
  })();

  inFlight.set(cacheKey, run);
  try {
    await run;
  } finally {
    if (inFlight.get(cacheKey) === run) inFlight.delete(cacheKey);
  }
}
