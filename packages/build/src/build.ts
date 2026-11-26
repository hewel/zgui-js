import type {
  BuildApplicationOptions,
  BuildInfo,
  Manifest,
  PublishedApplication,
  ResolveNativeHostOptions,
  ResolvedNativeHost,
  WorkerDeclaration,
} from "./index.ts";
import { readFileSync, realpathSync, type Dirent } from "node:fs";
import { lstat, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { compileBundle, compileWorkerBundle, type CompilationResult } from "./compiler.ts";
import { typecheckProject } from "./typecheck.ts";
import { canonicalJson, sha256, moduleIdentity } from "./identity.ts";
import { compileModuleBytecode, resolveNativeHost } from "./host.ts";
import { publishGeneration, resourcePath } from "./publication.ts";
import { readManifestPolicy } from "./manifest-policy.ts";
import { syncCssModuleTypes, validateStylesheet } from "./css.ts";
import { workspaceRoot } from "./sdk.ts";
import { ensureSdkPackagesBuilt } from "./sdk-packages.ts";

export { publishGeneration };
export { workspaceRoot };
const require = createRequire(import.meta.url);

type ManifestIdentity = Omit<Manifest, "buildId">;

function readPackage(directory: string): { name?: unknown; version?: unknown } | undefined {
  try {
    return JSON.parse(readFileSync(path.join(directory, "package.json"), "utf8"));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    return undefined;
  }
}

function resolveModule(from: NodeJS.Require, id: string): string | undefined {
  try {
    return from.resolve(id);
  } catch (error) {
    const { code } = error as NodeJS.ErrnoException;
    if (code !== "MODULE_NOT_FOUND" && code !== "ERR_PACKAGE_PATH_NOT_EXPORTED") throw error;
    return undefined;
  }
}

function packageVersion(from: NodeJS.Require, id: string): string | undefined {
  const manifest = resolveModule(from, `${id}/package.json`);
  if (manifest) return manifestVersion(JSON.parse(readFileSync(manifest, "utf8")), id);
  const entry = resolveModule(from, id);
  if (!entry) return undefined;
  let directory = path.dirname(realpathSync(entry));
  while (true) {
    const pkg = readPackage(directory);
    if (pkg?.name === id) return manifestVersion(pkg, id);
    const parent = path.dirname(directory);
    if (parent === directory) throw new Error(`Cannot locate package.json for ${id}`);
    directory = parent;
  }
}

const runtimePackages = new Set(["solid-js", "@solidjs/universal", "@solidjs/signals"]);

function bundledRuntimeVersions(modules: Iterable<string> = []): Map<string, string> {
  const versions = new Map<string, string>();
  const visited = new Set<string>();
  for (const filename of modules) {
    for (
      let directory = path.dirname(filename);
      !visited.has(directory);
      directory = path.dirname(directory)
    ) {
      visited.add(directory);
      const pkg = readPackage(directory);
      if (typeof pkg?.name !== "string" || !pkg.name) continue;
      if (runtimePackages.has(pkg.name) && !versions.has(pkg.name))
        versions.set(pkg.name, manifestVersion(pkg, pkg.name));
      break;
    }
    if (versions.size === runtimePackages.size) break;
  }
  return versions;
}

export async function optionalFile(filename: string): Promise<Buffer | undefined> {
  try {
    return await readFile(filename);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
}

export async function bundle(
  entry: string,
  {
    root = workspaceRoot,
    appDirectory = path.dirname(entry),
    hostEntry = true,
    filename = "app.mjs",
    development = false,
  }: BuildApplicationOptions & { hostEntry?: boolean; filename?: string } = {},
): Promise<CompilationResult> {
  await ensureSdkPackagesBuilt(root);
  return compileBundle(entry, {
    root,
    appDirectory,
    hostEntry,
    filename,
    development,
  });
}

function manifestVersion(manifest: unknown, id: string): string {
  if (
    manifest &&
    typeof manifest === "object" &&
    "version" in manifest &&
    typeof manifest.version === "string"
  ) {
    return manifest.version;
  }
  throw new Error(`Missing package.json version for ${id}`);
}

export function dependencyVersions({
  root = workspaceRoot,
  appDirectory = root,
  modules,
}: Pick<BuildApplicationOptions, "root" | "appDirectory"> & {
  modules?: Iterable<string>;
} = {}): Record<string, string> {
  const bundled = bundledRuntimeVersions(modules);
  const appRequire = createRequire(path.resolve(appDirectory, "package.json"));
  const rootRequire = createRequire(path.resolve(root, "package.json"));
  const universalFromUi = (from: NodeJS.Require) => {
    const ui = resolveModule(from, "@hewel/zgui");
    return ui && packageVersion(createRequire(realpathSync(ui)), "@solidjs/universal");
  };
  const signalsFromSolid = (from: NodeJS.Require) => {
    const solid = resolveModule(from, "solid-js/package.json");
    return solid && packageVersion(createRequire(solid), "@solidjs/signals");
  };
  const solid =
    bundled.get("solid-js") ??
    packageVersion(appRequire, "solid-js") ??
    manifestVersion(require("solid-js/package.json"), "solid-js");
  const universal =
    bundled.get("@solidjs/universal") ??
    packageVersion(appRequire, "@solidjs/universal") ??
    universalFromUi(appRequire) ??
    packageVersion(rootRequire, "@solidjs/universal") ??
    universalFromUi(rootRequire);
  const signals =
    bundled.get("@solidjs/signals") ??
    packageVersion(appRequire, "@solidjs/signals") ??
    signalsFromSolid(appRequire) ??
    signalsFromSolid(require);
  if (signals === undefined) throw new Error("Cannot locate @solidjs/signals");
  const solidCompiler = packageVersion(require, "@solidjs/compiler");
  if (solidCompiler === undefined) throw new Error("Cannot locate @solidjs/compiler");
  return Object.fromEntries([
    ["solid-js", solid],
    ["@solidjs/signals", signals],
    ["@solidjs/compiler", solidCompiler],
    ...["lightningcss", "rolldown", "tsrs"].map(
      name => [name, manifestVersion(require(`${name}/package.json`), name)] as const,
    ),
    ...(universal === undefined ? [] : [["@solidjs/universal", universal] as const]),
  ]);
}

export async function buildInfo(
  root: string,
  result: CompilationResult,
  { appDirectory = root, development = false } = {},
): Promise<BuildInfo> {
  const cargoLock = await Bun.file(path.join(root, "Cargo.lock")).text();
  const revision = cargoLock.match(
    /name = "rquickjs-sys"\nversion = "([^"]+)"\nsource = "[^"\n]+#([a-f0-9]+)"/,
  );
  // This gitlink was verified against the pinned rquickjs checkout, not inferred from its crate version.
  const engines: Record<string, string | undefined> = {
    "73fcd6062ee615aeb2a4675cd32be67a56e93973": "6d46d07d04041b40f4f49eaa7fdebe44c314c699",
  };
  const engineRevision = revision && engines[revision[2]];
  if (!revision || !engineRevision)
    throw new Error(
      "Unknown resolved rquickjs engine; audit its QuickJS-NG gitlink before packaging",
    );
  const packageJson = (await Bun.file(path.join(root, "package.json")).json()) as {
    packageManager: string;
  };
  const identity = (filename: string) => moduleIdentity(filename, { root, appDirectory });
  return {
    schemaVersion: 2,
    hostApiVersion: 1,
    bun: Bun.version,
    packageManager: packageJson.packageManager,
    packages: dependencyVersions({ root, appDirectory, modules: result.graph.modules }),
    lockfile: {
      path: "bun.lock",
      digest: sha256(await Bun.file(path.join(root, "bun.lock")).bytes()),
    },
    cargoLock: { path: "Cargo.lock", digest: sha256(cargoLock) },
    engine: {
      name: "QuickJS-NG",
      revision: engineRevision,
      rquickjsVersion: revision[1],
      rquickjsRevision: revision[2],
    },
    conditions: development ? ["development", "browser"] : ["browser", "production"],
    target: "es2022",
    solidRuntime: identity(result.solidRuntime),
    graph: {
      modules: result.graph.modules.map(identity).sort(),
      chunks: result.graph.chunks.map(chunk => ({
        ...chunk,
        modules: chunk.modules.map(identity).sort(),
      })),
    },
    styleSystem: result.styleSystem,
  };
}

async function collectAssets(directory: string, prefix = "assets"): Promise<Map<string, Buffer>> {
  let entries: Dirent[];
  try {
    if (!(await lstat(directory)).isDirectory())
      throw new Error(`Assets must be regular files/directories, not links: ${prefix}`);
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return new Map();
    throw error;
  }
  const files = new Map<string, Buffer>();
  for (const entry of entries) {
    const relative = resourcePath(`${prefix}/${entry.name}`);
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      for (const item of await collectAssets(absolute, relative)) files.set(...item);
    } else if (entry.isFile()) files.set(relative, await readFile(absolute));
    else throw new Error(`Assets must be regular files, not links/devices: ${relative}`);
  }
  return files;
}

// Identity covers every packaged byte through artifactDigests, including provenance/build info.
export function contentDigest(
  files: ReadonlyMap<string, Uint8Array>,
  manifest: ManifestIdentity & { buildId?: string },
): string {
  const { buildId: _buildId, ...identity } = manifest;
  const names = Object.keys(manifest.artifactDigests).sort();
  const packaged = [...files.keys()].filter(name => name !== "manifest.json").sort();
  if (JSON.stringify(names) !== JSON.stringify(packaged))
    throw new Error("Artifact digest inventory does not match packaged files");
  for (const name of names) {
    resourcePath(name);
    const bytes = files.get(name);
    if (!bytes) throw new Error(`Missing package resource: ${name}`);
    if (sha256(bytes) !== manifest.artifactDigests[name])
      throw new Error(`Artifact digest mismatch: ${name}`);
  }
  return sha256(canonicalJson(identity));
}

export async function buildApplication(
  app: string,
  {
    root = workspaceRoot,
    appDirectory = path.join(root, "apps/examples", app),
    development = false,
    bytecode = false,
    hostRoot,
    hostExecutable,
    hostSearchFrom,
    extraCapabilities,
    capabilitiesFile,
    cssModuleInclude,
  }: BuildApplicationOptions = {},
  {
    signal,
    nativeHost,
  }: { signal?: AbortSignal | undefined; nativeHost?: ResolvedNativeHost | undefined } = {},
): Promise<PublishedApplication> {
  signal?.throwIfAborted();
  if (!/^[a-z][a-z0-9-]*$/.test(app)) throw new Error(`Invalid application name: ${app}`);
  root = path.resolve(root);
  appDirectory = path.resolve(appDirectory);
  await ensureSdkPackagesBuilt(root);
  await syncCssModuleTypes({
    appDirectory,
    include: cssModuleInclude,
  });
  typecheckProject(path.join(appDirectory, "tsconfig.json"));
  const result = await bundle(path.join(appDirectory, "src/main.tsx"), {
    root,
    appDirectory,
    development,
  });
  return publishApplication(app, result, {
    root,
    appDirectory,
    development,
    bytecode,
    hostRoot,
    hostExecutable,
    hostSearchFrom,
    nativeHost,
    signal,
    extraCapabilities,
    capabilitiesFile,
  });
}

export async function buildWithResolvedHost(
  app: string,
  options: BuildApplicationOptions & { root: string; appDirectory: string },
  {
    hostRoot,
    hostExecutable,
    signal,
  }: Pick<ResolveNativeHostOptions, "hostRoot" | "hostExecutable" | "signal">,
) {
  let host: Promise<ResolvedNativeHost> | undefined;
  const resolveHost = () =>
    (host ??= resolveNativeHost({
      hostExecutable,
      hostRoot,
      root: options.root,
      cwd: options.appDirectory,
      searchFrom: options.hostSearchFrom,
      signal,
    }));
  // Bytecode needs the compiler early; packaging validates the manifest before resolving a runtime-only host.
  const application = await buildApplication(app, options, {
    signal,
    nativeHost: options.bytecode ? await resolveHost() : undefined,
  });
  return { application, resolveHost };
}

export async function publishApplication(
  app: string,
  result: CompilationResult,
  {
    root,
    appDirectory,
    signal,
    development = false,
    bytecode = false,
    hostRoot,
    hostExecutable,
    hostSearchFrom,
    nativeHost,
    extraCapabilities,
    capabilitiesFile,
  }: BuildApplicationOptions & {
    root: string;
    appDirectory: string;
    signal?: AbortSignal | undefined;
    nativeHost?: ResolvedNativeHost | undefined;
  },
): Promise<PublishedApplication> {
  signal?.throwIfAborted();
  const files = new Map(result.files);
  const moduleStyles = files.get("app.css");
  if (!moduleStyles || !result.styleSystem)
    throw new Error("Missing Lightning CSS compilation output");
  const componentsCss = files.get("components.css");
  if (componentsCss !== undefined) {
    validateStylesheet(componentsCss.toString("utf8"), "components.css");
  }
  const globalCssFile = await optionalFile(path.join(appDirectory, "src/app.css"));
  const globalCss =
    globalCssFile !== undefined
      ? validateStylesheet(globalCssFile.toString("utf8"), "app.css")
      : "";
  const modulesCss = moduleStyles.toString("utf8").trim();
  const mergedAppCssParts = [globalCss, modulesCss].filter(part => part.length > 0);
  const mergedAppCss = mergedAppCssParts.length > 0 ? `${mergedAppCssParts.join("\n\n")}\n` : "";
  files.set("app.css", Buffer.from(mergedAppCss, "utf8"));
  const assets = await collectAssets(path.join(appDirectory, "assets"));
  for (const item of assets) files.set(...item);
  const {
    id,
    window: windowConfig,
    capabilities,
    workerEntries,
    fonts,
  } = await readManifestPolicy(app, appDirectory, assets, { extraCapabilities, capabilitiesFile });
  const workers: WorkerDeclaration[] = [];
  for (const worker of workerEntries) {
    signal?.throwIfAborted();
    const compiled = await compileWorkerBundle(worker.entry, {
      root,
      appDirectory,
      name: worker.name,
      development,
    });
    for (const [fileName, bytes] of compiled.files) {
      files.set(fileName, bytes);
    }
    workers.push({ name: compiled.name, path: compiled.path, sourceMap: compiled.sourceMap });
  }
  files.set(
    "build-info.json",
    Buffer.from(
      JSON.stringify(await buildInfo(root, result, { appDirectory, development }), null, 2) + "\n",
    ),
  );
  const chunks = result.graph.chunks
    .map(chunk => chunk.fileName)
    .filter(fileName => fileName !== "app.mjs")
    .sort()
    .map(chunkPath => ({ path: chunkPath, sourceMap: `${chunkPath}.map` }));
  let entryPath = "app.mjs";
  let bytecodeMeta: { bytecodeFormat: "qjs-bytecode-v1"; quickjsAbi: string } | undefined;
  let finalChunks = chunks;
  let finalWorkers = workers;
  if (bytecode) {
    const host =
      nativeHost ??
      (await resolveNativeHost({
        hostExecutable,
        hostRoot,
        root,
        cwd: appDirectory,
        searchFrom: hostSearchFrom,
        signal,
      }));
    const bytecodeFormat = host.abi?.bytecodeFormat;
    const quickjsAbi = host.abi?.quickjsAbi;
    if (bytecodeFormat !== "qjs-bytecode-v1" || !quickjsAbi) {
      throw new Error(
        `Resolved zgui-host (${host.executable}) does not report qjs-bytecode-v1 ABI metadata for --bytecode compilation`,
      );
    }
    const tempDir = await mkdtemp(path.join(os.tmpdir(), "zgui-bytecode-"));
    try {
      const toBytecodePath = (jsPath: string): string => {
        if (!jsPath.endsWith(".mjs")) {
          throw new Error(`Expected .mjs module path for bytecode compilation: ${jsPath}`);
        }
        return `${jsPath.slice(0, -4)}.qjsb`;
      };
      const sourceDir = path.join(tempDir, "src");
      const outDir = path.join(tempDir, "out");
      const modulePaths = [
        "app.mjs",
        ...chunks.map(chunk => chunk.path),
        ...workers.map(worker => worker.path),
      ];
      for (const jsPath of modulePaths) {
        const jsBytes = files.get(jsPath);
        if (!jsBytes) throw new Error(`Missing compiled JS module for bytecode: ${jsPath}`);
        const dest = path.join(sourceDir, jsPath);
        await mkdir(path.dirname(dest), { recursive: true });
        await writeFile(dest, jsBytes);
      }
      const compileFile = async (jsPath: string): Promise<string> => {
        const bcPath = toBytecodePath(jsPath);
        const inPath = path.join(sourceDir, jsPath);
        const outPath = path.join(outDir, bcPath);
        const bcBytes = await compileModuleBytecode(host, {
          input: inPath,
          output: outPath,
          moduleName: jsPath,
          sourceDir,
          signal,
        });
        files.delete(jsPath);
        files.set(bcPath, bcBytes);
        return bcPath;
      };
      entryPath = await compileFile("app.mjs");
      finalChunks = [];
      for (const chunk of chunks) {
        finalChunks.push({
          path: await compileFile(chunk.path),
          sourceMap: chunk.sourceMap,
        });
      }
      finalWorkers = [];
      for (const worker of workers) {
        finalWorkers.push({
          name: worker.name,
          path: await compileFile(worker.path),
          sourceMap: worker.sourceMap,
        });
      }
      bytecodeMeta = { bytecodeFormat: "qjs-bytecode-v1", quickjsAbi };
    } finally {
      await rm(tempDir, { recursive: true, force: true });
    }
  }
  const identity: ManifestIdentity = {
    schemaVersion: 2,
    id,
    hostApiVersion: 1,
    entry: entryPath,
    sourceMap: "app.mjs.map",
    ...(bytecodeMeta ?? {}),
    ...(finalChunks.length > 0 ? { chunks: finalChunks } : {}),
    ...(finalWorkers.length > 0 ? { workers: finalWorkers } : {}),
    styles: [...(componentsCss !== undefined ? ["components.css"] : []), "app.css"],
    assets: [...assets.keys()].sort(),
    fonts,
    ...(windowConfig ? { window: windowConfig } : {}),
    ...(capabilities ? { capabilities } : {}),
    styleSystem: result.styleSystem,
    artifactDigests: Object.fromEntries(
      [...files]
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([name, bytes]) => [name, sha256(bytes)]),
    ),
  };
  const manifest: Manifest = { ...identity, buildId: contentDigest(files, identity) };
  signal?.throwIfAborted();
  files.set("manifest.json", Buffer.from(JSON.stringify(manifest, null, 2) + "\n"));
  signal?.throwIfAborted();
  const dist = await publishGeneration(appDirectory, files);
  return { dist, manifest };
}
