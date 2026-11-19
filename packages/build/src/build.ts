import { readFileSync, type Dirent } from "node:fs";
import { readFile, readdir, lstat } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { compileBundle, type CompilationResult } from "./compiler.ts";
import { typecheckProject } from "./typecheck.ts";
import { canonicalJson, sha256, moduleIdentity } from "./identity.ts";
import { publishGeneration, resourcePath } from "./publication.ts";
import { validateStylesheet } from "./stylex.ts";

export { publishGeneration };
export const workspaceRoot = fileURLToPath(new URL("../../../", import.meta.url));
const require = createRequire(import.meta.url);

export type FontDeclaration = { family: string; path: string };

type Capabilities = {
  shellSurfaces?: boolean;
  sessionLock?: boolean;
  imagePrefixes?: string[];
  unixSockets?: string[];
  commands?: string[];
  controlChannel?: boolean;
};

type ManifestIdentity = {
  schemaVersion: 2;
  id: string;
  hostApiVersion: 1;
  entry: string;
  sourceMap: string;
  styles: string[];
  assets: string[];
  fonts: FontDeclaration[];
  capabilities?: Capabilities;
  styleSystem: CompilationResult["styleSystem"];
  artifactDigests: Record<string, string>;
};

export type Manifest = ManifestIdentity & { buildId: string };
export type PublishedApplication = { dist: string; manifest: Manifest };

export type BuildInfo = {
  schemaVersion: 2;
  hostApiVersion: 1;
  bun: string;
  packageManager: string;
  packages: Record<string, string>;
  lockfile: { path: "bun.lock"; digest: string };
  cargoLock: { path: "Cargo.lock"; digest: string };
  engine: {
    name: "QuickJS-NG";
    revision: string;
    rquickjsVersion: string;
    rquickjsRevision: string;
  };
  conditions: string[];
  target: "es2022";
  solidRuntime: string;
  graph: CompilationResult["graph"];
  styleSystem: CompilationResult["styleSystem"];
};

type BuildOptions = {
  root?: string;
  appDirectory?: string;
  stylexDiagnostics?: boolean;
  development?: boolean;
};

function packageVersion(req: NodeJS.Require, id: string): string {
  let dir = path.dirname(req.resolve(id));
  while (true) {
    try {
      const pkg = JSON.parse(readFileSync(path.join(dir, "package.json"), "utf8")) as {
        name?: unknown;
        version?: unknown;
      };
      if (pkg.name === id && typeof pkg.version === "string") return pkg.version;
    } catch {}
    const parent = path.dirname(dir);
    if (parent === dir) throw new Error(`Cannot locate package.json for ${id}`);
    dir = parent;
  }
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
    stylexDiagnostics = false,
    development = false,
  }: BuildOptions & { hostEntry?: boolean; filename?: string } = {},
): Promise<CompilationResult> {
  return compileBundle(entry, {
    root,
    appDirectory,
    hostEntry,
    filename,
    stylexDiagnostics,
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

export function dependencyVersions(): Record<string, string> {
  const signalsPkgPath = createRequire(require.resolve("solid-js/package.json")).resolve(
    "@solidjs/signals/package.json",
  );
  const babelPluginRequire = createRequire(require.resolve("@solidjs/babel-plugin"));
  const uiRequire = createRequire(path.join(workspaceRoot, "packages/ui/package.json"));
  return Object.fromEntries([
    ...[
      "solid-js",
      "@solidjs/babel-plugin",
      "@babel/core",
      "@babel/parser",
      "@babel/traverse",
      "@babel/preset-typescript",
      "rolldown",
      "tsrs",
      "@stylexjs/stylex",
      "@stylexjs/babel-plugin",
      "styleq",
    ].map(name => [name, manifestVersion(require(`${name}/package.json`), name)] as const),
    ["@solidjs/universal", packageVersion(uiRequire, "@solidjs/universal")],
    [
      "@solidjs/signals",
      manifestVersion(JSON.parse(readFileSync(signalsPkgPath, "utf8")), "@solidjs/signals"),
    ],
    [
      "@babel/plugin-syntax-jsx",
      manifestVersion(
        babelPluginRequire("@babel/plugin-syntax-jsx/package.json"),
        "@babel/plugin-syntax-jsx",
      ),
    ],
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
    packages: dependencyVersions(),
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
    stylexDiagnostics = false,
    development = false,
  }: BuildOptions = {},
): Promise<PublishedApplication> {
  if (!/^[a-z][a-z0-9-]*$/.test(app)) throw new Error(`Invalid application name: ${app}`);
  typecheckProject(path.join(appDirectory, "tsconfig.json"));
  const result = await bundle(path.join(appDirectory, "src/main.tsx"), {
    root,
    appDirectory,
    stylexDiagnostics,
    development,
  });
  return publishApplication(app, result, { root, appDirectory, development });
}

function isFontDeclaration(font: unknown): font is FontDeclaration {
  return (
    !!font &&
    typeof font === "object" &&
    "family" in font &&
    typeof font.family === "string" &&
    !!font.family.trim() &&
    font.family === font.family.trim() &&
    "path" in font &&
    typeof font.path === "string" &&
    Object.keys(font).every(key => key === "family" || key === "path")
  );
}

function capabilityString(value: unknown): value is string {
  if (typeof value !== "string") return false;
  let nonspace = false;
  for (const char of value) {
    const code = char.charCodeAt(0);
    if (
      code < 32 ||
      (code >= 127 && code <= 159) ||
      (char.length === 1 && code >= 0xd800 && code <= 0xdfff)
    )
      return false;
    nonspace ||= code !== 32;
  }
  return nonspace;
}

function absoluteCapabilityPath(value: string): boolean {
  return (
    value === "/" ||
    (value.startsWith("/") &&
      !value.includes("\\") &&
      value
        .slice(1)
        .split("/")
        .every(part => part && part !== "." && part !== ".."))
  );
}

function parseCapabilities(input: unknown): Capabilities | undefined {
  if (input === undefined) return undefined;
  if (input === null || typeof input !== "object" || Array.isArray(input))
    throw new Error("package.json zgui.capabilities must be an object");

  const capabilities: Capabilities = {};
  for (const [key, value] of Object.entries(input) as [string, unknown][]) {
    const field = `package.json zgui.capabilities.${key}`;
    switch (key) {
      case "shellSurfaces":
      case "sessionLock":
      case "controlChannel":
        if (typeof value !== "boolean") throw new Error(`${field} must be a boolean`);
        if (value) capabilities[key] = true;
        break;
      case "imagePrefixes":
      case "unixSockets":
      case "commands": {
        const paths = key !== "commands";
        if (
          !Array.isArray(value) ||
          !value.every(
            (item: unknown) => capabilityString(item) && (!paths || absoluteCapabilityPath(item)),
          )
        )
          throw new Error(
            `${field} must be an array of ${paths ? "canonical absolute paths" : "nonempty strings"}`,
          );
        if (new Set(value).size !== value.length) throw new Error(`${field} contains duplicates`);
        if (value.length) capabilities[key] = value;
        break;
      }
      default:
        throw new Error(`Unknown capability: ${field}`);
    }
  }
  return Object.keys(capabilities).length ? capabilities : undefined;
}

export async function publishApplication(
  app: string,
  result: CompilationResult,
  {
    root,
    appDirectory,
    signal,
    development = false,
  }: { root: string; appDirectory: string; signal?: AbortSignal; development?: boolean },
): Promise<PublishedApplication> {
  signal?.throwIfAborted();
  const files = new Map(result.files);
  const stylesheet = files.get("app.css");
  if (!stylesheet || !files.has("style-origins.json") || !result.styleSystem)
    throw new Error("Missing StyleX compilation output");
  validateStylesheet(stylesheet.toString(), "app.css", { allowDynamicProperties: true });
  const css = await optionalFile(path.join(appDirectory, "src/app.css"));
  if (css !== undefined) {
    validateStylesheet(css.toString(), "base.css");
    files.set("base.css", css);
  }
  const assets = await collectAssets(path.join(appDirectory, "assets"));
  for (const item of assets) files.set(...item);
  const pkg = await optionalFile(path.join(appDirectory, "package.json"));
  const packageJson: { zgui?: { fonts?: unknown; capabilities?: unknown } } | undefined =
    pkg && JSON.parse(pkg.toString("utf8"));
  const capabilities = parseCapabilities(packageJson?.zgui?.capabilities);
  const declarations = packageJson?.zgui?.fonts ?? [];
  if (!Array.isArray(declarations))
    throw new Error("package.json zgui.fonts must be an array of {family,path}");
  const fonts: FontDeclaration[] = [];
  const families = new Set<string>();
  for (const font of declarations as unknown[]) {
    if (!isFontDeclaration(font))
      throw new Error("Invalid application font mapping; expected {family,path}");
    resourcePath(font.path);
    if (!assets.has(font.path)) throw new Error(`Font path must be a declared asset: ${font.path}`);
    const family = font.family.toLowerCase();
    if (families.has(family)) throw new Error(`Duplicate application font family: ${font.family}`);
    families.add(family);
    fonts.push(font);
  }
  files.set(
    "build-info.json",
    Buffer.from(
      JSON.stringify(await buildInfo(root, result, { appDirectory, development }), null, 2) + "\n",
    ),
  );
  const identity: ManifestIdentity = {
    schemaVersion: 2,
    id: `dev.example.${app}`,
    hostApiVersion: 1,
    entry: "app.mjs",
    sourceMap: "app.mjs.map",
    styles: css === undefined ? ["app.css"] : ["base.css", "app.css"],
    assets: [...assets.keys()].sort(),
    fonts,
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
