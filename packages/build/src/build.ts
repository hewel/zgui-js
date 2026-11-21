import type {
  BuildApplicationOptions,
  BuildInfo,
  Capabilities,
  FontDeclaration,
  Manifest,
  PublishedApplication,
  WindowDeclaration,
  WorkerDeclaration,
} from "./index.ts";
import { readFileSync, realpathSync, type Dirent } from "node:fs";
import { readFile, readdir, lstat } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { compileBundle, compileWorkerBundle, type CompilationResult } from "./compiler.ts";
import { typecheckProject } from "./typecheck.ts";
import { canonicalJson, sha256, moduleIdentity } from "./identity.ts";
import { publishGeneration, resourcePath } from "./publication.ts";
import { validateStylesheet } from "./stylex.ts";
import { workspaceRoot } from "./sdk.ts";

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
    stylexDiagnostics = false,
    development = false,
  }: BuildApplicationOptions & { hostEntry?: boolean; filename?: string } = {},
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
  const babelPluginRequire = createRequire(require.resolve("@solidjs/babel-plugin"));
  return Object.fromEntries([
    ["solid-js", solid],
    ["@solidjs/signals", signals],
    ...[
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
    ...(universal === undefined ? [] : [["@solidjs/universal", universal] as const]),
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
    stylexDiagnostics = false,
    development = false,
    extraCapabilities,
    capabilitiesFile,
  }: BuildApplicationOptions = {},
): Promise<PublishedApplication> {
  if (!/^[a-z][a-z0-9-]*$/.test(app)) throw new Error(`Invalid application name: ${app}`);
  root = path.resolve(root);
  appDirectory = path.resolve(appDirectory);
  typecheckProject(path.join(appDirectory, "tsconfig.json"));
  const result = await bundle(path.join(appDirectory, "src/main.tsx"), {
    root,
    appDirectory,
    stylexDiagnostics,
    development,
  });
  return publishApplication(app, result, {
    root,
    appDirectory,
    development,
    extraCapabilities,
    capabilitiesFile,
  });
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

function capabilityPath(value: string): boolean {
  if (value === "/") return true;
  if (value.includes("\\")) return false;
  const parts = value.split("/");
  const root = parts.shift();
  const template = root === "$XDG_RUNTIME_DIR" || root === "$HOME" || root === "$XDG_DATA_HOME";
  return (
    (root === "" || template) &&
    parts.every(
      part =>
        part !== "" &&
        part !== "." &&
        part !== ".." &&
        (!template ||
          part === "$WAYLAND_DISPLAY" ||
          !part.replaceAll("${WAYLAND_DISPLAY}", "").includes("$")),
    )
  );
}

function fileDirectoryPath(value: string): boolean {
  if (value === "/") return true;
  if (value.includes("\\")) return false;
  const parts = value.split("/");
  const root = parts.shift();
  const template = root === "$HOME" || root === "$XDG_DATA_HOME" || root === "$XDG_DOCUMENTS_DIR";
  return (
    (root === "" || template) &&
    parts.every(part => part !== "" && part !== "." && part !== ".." && !part.includes("$"))
  );
}

function isPrivateIpv4(host: string): boolean {
  const parts = host.split(".");
  if (parts.length !== 4 || !parts.every(p => /^\d{1,3}$/.test(p))) return false;
  const nums = parts.map(Number);
  if (nums.some(n => n > 255)) return false;
  const [a, b] = nums as [number, number, number, number];
  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    a >= 224
  );
}

function isPrivateHost(hostname: string): boolean {
  const bare =
    hostname.startsWith("[") && hostname.endsWith("]")
      ? hostname.slice(1, -1).toLowerCase()
      : hostname.toLowerCase();
  if (bare === "localhost" || bare.endsWith(".localhost")) return true;
  if (isPrivateIpv4(bare)) return true;
  if (bare.includes(":")) {
    // Reject loopback, unspecified, ULA (fc00::/7), link-local (fe80::/10), multicast (ff00::/8),
    // and IPv4-mapped IPv6 addresses unless allowPrivateNetworks is explicitly opted in.
    if (bare === "::1" || bare === "::") return true;
    const first = bare.split(":")[0] ?? "";
    const head = Number.parseInt(first || "0", 16);
    if (!Number.isNaN(head)) {
      if ((head & 0xfe00) === 0xfc00) return true;
      if ((head & 0xffc0) === 0xfe80) return true;
      if ((head & 0xff00) === 0xff00) return true;
    }
    if (bare.startsWith("::ffff:")) {
      const mapped = bare.slice("::ffff:".length);
      if (isPrivateIpv4(mapped)) return true;
      // Also handle hex form e.g. ::ffff:7f00:1 produced by WHATWG URL parser
      const hexParts = mapped.split(":");
      if (hexParts.length === 2) {
        const hi = Number.parseInt(hexParts[0]!, 16);
        const lo = Number.parseInt(hexParts[1]!, 16);
        if (!Number.isNaN(hi) && !Number.isNaN(lo)) {
          const ipv4 = `${(hi >> 8) & 255}.${hi & 255}.${(lo >> 8) & 255}.${lo & 255}`;
          if (isPrivateIpv4(ipv4)) return true;
        }
      }
    }
  }
  return false;
}

function parseNetworkOrigin(
  entry: unknown,
  field: string,
): {
  canonical: string;
  insecure: boolean;
  host: string;
} {
  if (!capabilityString(entry) || entry !== entry.trim() || entry.includes("*")) {
    throw new Error(`${field} contains invalid origin ${JSON.stringify(entry)}`);
  }
  let parsed: URL;
  try {
    parsed = new URL(entry);
  } catch {
    throw new Error(`${field} contains invalid origin ${JSON.stringify(entry)}`);
  }
  const scheme = parsed.protocol.slice(0, -1).toLowerCase();
  if (scheme !== "https" && scheme !== "http" && scheme !== "wss" && scheme !== "ws") {
    throw new Error(`${field} origin scheme must be https, http, wss, or ws: ${entry}`);
  }
  if (parsed.username || parsed.password) {
    throw new Error(`${field} origin must not include userinfo: ${entry}`);
  }
  if (parsed.search || parsed.hash) {
    throw new Error(`${field} origin must not include query or fragment: ${entry}`);
  }
  const afterScheme = entry.slice(entry.indexOf("://") + 3);
  if (afterScheme.includes("/") || afterScheme.includes("?") || afterScheme.includes("#")) {
    throw new Error(`${field} origin must not include a path, query, or fragment: ${entry}`);
  }
  const host = parsed.hostname.toLowerCase();
  if (!host) {
    throw new Error(`${field} origin must include a host: ${entry}`);
  }
  const defaultPort = scheme === "https" || scheme === "wss" ? 443 : 80;
  const port = parsed.port ? Number(parsed.port) : defaultPort;
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(`${field} origin has invalid port: ${entry}`);
  }
  const canonical = port === defaultPort ? `${scheme}://${host}` : `${scheme}://${host}:${port}`;
  return { canonical, insecure: scheme === "http" || scheme === "ws", host };
}

function parseCapabilities(input: unknown, source: string): Capabilities {
  if (input !== undefined && (input === null || typeof input !== "object" || Array.isArray(input)))
    throw new Error(`${source} must be an object`);

  const capabilities = Object.create(null) as Capabilities;
  for (const [key, value] of Object.entries(input ?? {}) as [string, unknown][]) {
    const field = `${source}.${key}`;
    switch (key) {
      case "shellSurfaces":
      case "sessionLock":
      case "controlChannel":
        if (typeof value !== "boolean") throw new Error(`${field} must be a boolean`);
        if (value) capabilities[key] = true;
        break;
      case "clipboard": {
        if (!value || typeof value !== "object" || Array.isArray(value))
          throw new Error(`${field} must be an object`);
        for (const [subKey, subVal] of Object.entries(value as Record<string, unknown>)) {
          if (subKey !== "read") throw new Error(`Unknown capability: ${field}.${subKey}`);
          if (typeof subVal !== "boolean") throw new Error(`${field}.read must be a boolean`);
          if (subVal) capabilities.clipboard = { read: true };
        }
        break;
      }
      case "sqlite": {
        if (!value || typeof value !== "object" || Array.isArray(value))
          throw new Error(`${field} must be an object`);
        for (const [subKey, subVal] of Object.entries(value as Record<string, unknown>)) {
          if (subKey !== "databases") throw new Error(`Unknown capability: ${field}.${subKey}`);
          if (!Array.isArray(subVal)) throw new Error(`${field}.databases must be an array`);
          const names = new Set<string>();
          for (const item of subVal as unknown[]) {
            if (typeof item !== "string" || !/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(item))
              throw new Error(`${field}.databases contains invalid entries`);
            if (names.has(item)) throw new Error(`${field}.databases contains duplicates`);
            names.add(item);
          }
          if (names.size) capabilities.sqlite = { databases: [...names] };
        }
        break;
      }
      case "files": {
        if (!value || typeof value !== "object" || Array.isArray(value))
          throw new Error(`${field} must be an object`);
        for (const [subKey, subVal] of Object.entries(value as Record<string, unknown>)) {
          if (subKey !== "directories") throw new Error(`Unknown capability: ${field}.${subKey}`);
          if (!Array.isArray(subVal)) throw new Error(`${field}.directories must be an array`);
          const seen = new Set<string>();
          const dirs: { path: string; mode: "read" | "readwrite" }[] = [];
          for (const item of subVal as unknown[]) {
            if (!item || typeof item !== "object" || Array.isArray(item))
              throw new Error(`${field}.directories entries must be objects`);
            const record = item as Record<string, unknown>;
            const keys = Object.keys(record);
            if (!keys.every(k => k === "path" || k === "mode"))
              throw new Error(`${field}.directories entry contains unknown keys`);
            if (
              !capabilityString(record.path) ||
              !fileDirectoryPath(record.path) ||
              (record.mode !== "read" && record.mode !== "readwrite")
            )
              throw new Error(`${field}.directories contains invalid entries`);
            if (seen.has(record.path))
              throw new Error(`${field}.directories contains duplicate path ${record.path}`);
            seen.add(record.path);
            dirs.push({ path: record.path, mode: record.mode });
          }
          if (dirs.length) capabilities.files = { directories: dirs };
        }
        break;
      }
      case "network": {
        if (!value || typeof value !== "object" || Array.isArray(value))
          throw new Error(`${field} must be an object`);
        const network: {
          origins?: string[];
          allowInsecureHttp?: boolean;
          allowPrivateNetworks?: boolean;
        } = {};
        for (const [subKey, subVal] of Object.entries(value as Record<string, unknown>)) {
          switch (subKey) {
            case "origins": {
              if (!Array.isArray(subVal)) throw new Error(`${field}.origins must be an array`);
              const origins = new Set<string>();
              for (const item of subVal as unknown[]) {
                const parsed = parseNetworkOrigin(item, `${field}.origins`);
                if (origins.has(parsed.canonical))
                  throw new Error(`${field}.origins contains duplicate origin ${parsed.canonical}`);
                origins.add(parsed.canonical);
              }
              if (origins.size) network.origins = [...origins];
              break;
            }
            case "allowInsecureHttp":
            case "allowPrivateNetworks": {
              if (typeof subVal !== "boolean")
                throw new Error(`${field}.${subKey} must be a boolean`);
              if (subVal) network[subKey] = true;
              break;
            }
            default:
              throw new Error(`Unknown capability: ${field}.${subKey}`);
          }
        }
        if (Object.keys(network).length) capabilities.network = network;
        break;
      }
      case "imagePrefixes":
      case "unixSockets":
      case "commands":
      case "namedSockets": {
        if (!Array.isArray(value)) throw new Error(`${field} must be an array`);
        const entries = new Set<string>();
        for (const item of value as unknown[]) {
          if (
            !capabilityString(item) ||
            (key === "namedSockets" && !/^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(item)) ||
            ((key === "imagePrefixes" || key === "unixSockets") && !capabilityPath(item))
          )
            throw new Error(`${field} contains invalid entries`);
          if (entries.has(item)) throw new Error(`${field} contains duplicates`);
          entries.add(item);
        }
        if (entries.size) capabilities[key] = [...entries];
        break;
      }
      default:
        throw new Error(`Unknown capability: ${field}`);
    }
  }
  return capabilities;
}

function capabilityOverlay(input: unknown, source: string): Capabilities {
  if (
    input !== null &&
    typeof input === "object" &&
    "sessionLock" in input &&
    input.sessionLock === true
  )
    throw new Error(`${source}.sessionLock: true may only be declared in base package.json`);
  return parseCapabilities(input, source);
}

async function mergeCapabilities(
  input: unknown,
  {
    extraCapabilities,
    capabilitiesFile,
  }: Pick<BuildApplicationOptions, "extraCapabilities" | "capabilitiesFile">,
): Promise<Capabilities | undefined> {
  const base = parseCapabilities(input, "package.json zgui.capabilities");
  const sources = [base, capabilityOverlay(extraCapabilities, "extraCapabilities")];
  if (capabilitiesFile !== undefined) {
    const filename = path.resolve(capabilitiesFile);
    const source = `capabilitiesFile (${filename})`;
    let overlay: unknown;
    try {
      overlay = JSON.parse(await readFile(filename, "utf8"));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      throw new Error(`${source}: cannot read JSON: ${message}`, { cause: error });
    }
    sources.push(capabilityOverlay(overlay, source));
  }
  const merged: Capabilities = {};
  for (const key of ["shellSurfaces", "controlChannel"] as const)
    if (sources.some(source => source[key])) merged[key] = true;
  if (base.sessionLock) merged.sessionLock = true;
  for (const key of ["imagePrefixes", "unixSockets", "namedSockets", "commands"] as const) {
    const union = new Set(sources.flatMap(source => source[key] ?? []));
    if (union.size) merged[key] = [...union].sort();
  }
  if (sources.some(source => source.clipboard?.read)) merged.clipboard = { read: true };
  const sqliteDatabases = new Set(sources.flatMap(source => source.sqlite?.databases ?? []));
  if (sqliteDatabases.size) merged.sqlite = { databases: [...sqliteDatabases].sort() };
  const fileDirs = new Map<string, "read" | "readwrite">();
  for (const grant of sources.flatMap(source => source.files?.directories ?? [])) {
    const existing = fileDirs.get(grant.path);
    if (!existing || grant.mode === "readwrite") fileDirs.set(grant.path, grant.mode);
  }
  if (fileDirs.size) {
    merged.files = {
      directories: [...fileDirs.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([dirPath, mode]) => ({ path: dirPath, mode })),
    };
  }
  const allowInsecureHttp = sources.some(source => source.network?.allowInsecureHttp);
  const allowPrivateNetworks = sources.some(source => source.network?.allowPrivateNetworks);
  const networkOrigins = new Set(sources.flatMap(source => source.network?.origins ?? []));
  for (const origin of networkOrigins) {
    const parsed = parseNetworkOrigin(origin, "capabilities.network.origins");
    if (parsed.insecure && !allowInsecureHttp) {
      throw new Error(
        `capabilities.network.origins contains insecure origin ${origin} without allowInsecureHttp: true`,
      );
    }
    if (isPrivateHost(parsed.host) && !allowPrivateNetworks) {
      throw new Error(
        `capabilities.network.origins contains private/loopback origin ${origin} without allowPrivateNetworks: true`,
      );
    }
  }
  if (networkOrigins.size || allowInsecureHttp || allowPrivateNetworks) {
    merged.network = {
      ...(networkOrigins.size ? { origins: [...networkOrigins].sort() } : {}),
      ...(allowInsecureHttp ? { allowInsecureHttp: true } : {}),
      ...(allowPrivateNetworks ? { allowPrivateNetworks: true } : {}),
    };
  }
  return Object.keys(merged).length ? merged : undefined;
}

const WINDOW_KEYS = new Set(["title", "width", "height", "minWidth", "minHeight", "resizable"]);

function parseWindowDeclaration(input: unknown): WindowDeclaration | undefined {
  if (input === undefined) return undefined;
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error("package.json zgui.window must be an object");
  }
  const record = input as Record<string, unknown>;
  for (const key of Object.keys(record)) {
    if (!WINDOW_KEYS.has(key)) throw new Error(`Unknown package.json zgui.window key: ${key}`);
  }
  const result: WindowDeclaration = {};
  if (record.title !== undefined) {
    if (!capabilityString(record.title) || record.title !== record.title.trim()) {
      throw new Error(
        "package.json zgui.window.title must be a non-empty, trimmed string without control characters",
      );
    }
    result.title = record.title;
  }
  for (const key of ["width", "height", "minWidth", "minHeight"] as const) {
    const value = record[key];
    if (value !== undefined) {
      if (typeof value !== "number" || !Number.isInteger(value) || value < 1 || value > 16384) {
        throw new Error(`package.json zgui.window.${key} must be an integer between 1 and 16384`);
      }
      result[key] = value;
    }
  }
  if (record.resizable !== undefined) {
    if (typeof record.resizable !== "boolean") {
      throw new Error("package.json zgui.window.resizable must be a boolean");
    }
    result.resizable = record.resizable;
  }
  return Object.keys(result).length ? result : undefined;
}

const WORKER_NAME_RE = /^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/;
const MAX_WORKERS = 32;

function parseWorkerDeclarations(
  input: unknown,
  appDirectory: string,
): { name: string; entry: string }[] {
  if (input === undefined) return [];
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error(
      "package.json zgui.workers must be an object mapping worker names to relative entry paths",
    );
  }
  const entries = Object.entries(input as Record<string, unknown>);
  if (entries.length > MAX_WORKERS) {
    throw new Error(`package.json zgui.workers exceeds maximum of ${MAX_WORKERS} workers`);
  }
  const resolvedAppDir = path.resolve(appDirectory);
  const parsed: { name: string; entry: string }[] = [];
  for (const [name, relPath] of entries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
    if (!WORKER_NAME_RE.test(name)) {
      throw new Error(`Invalid compute worker name: ${name}`);
    }
    if (typeof relPath !== "string" || !relPath.trim()) {
      throw new Error(`package.json zgui.workers.${name} must be a non-empty relative path`);
    }
    resourcePath(relPath);
    const fullPath = path.resolve(resolvedAppDir, relPath);
    if (!fullPath.startsWith(`${resolvedAppDir}${path.sep}`)) {
      throw new Error(
        `package.json zgui.workers.${name} escapes application directory: ${relPath}`,
      );
    }
    parsed.push({ name, entry: fullPath });
  }
  return parsed;
}

export async function publishApplication(
  app: string,
  result: CompilationResult,
  {
    root,
    appDirectory,
    signal,
    development = false,
    extraCapabilities,
    capabilitiesFile,
  }: BuildApplicationOptions & { root: string; appDirectory: string; signal?: AbortSignal },
): Promise<PublishedApplication> {
  signal?.throwIfAborted();
  const files = new Map(result.files);
  const stylesheet = files.get("app.css");
  if (!stylesheet || !files.has("style-origins.json") || !result.styleSystem)
    throw new Error("Missing StyleX compilation output");
  validateStylesheet(stylesheet.toString(), "app.css", { allowDynamicProperties: true });
  const componentsCss = files.get("components.css");
  if (componentsCss !== undefined) {
    validateStylesheet(componentsCss.toString(), "components.css");
  }
  const css = await optionalFile(path.join(appDirectory, "src/app.css"));
  if (css !== undefined) {
    validateStylesheet(css.toString(), "base.css");
    files.set("base.css", css);
  }
  const assets = await collectAssets(path.join(appDirectory, "assets"));
  for (const item of assets) files.set(...item);
  const pkg = await optionalFile(path.join(appDirectory, "package.json"));
  const packageJson:
    | {
        zgui?: {
          id?: unknown;
          fonts?: unknown;
          capabilities?: unknown;
          window?: unknown;
          workers?: unknown;
        };
      }
    | undefined = pkg && JSON.parse(pkg.toString("utf8"));
  const id = packageJson?.zgui?.id === undefined ? `dev.example.${app}` : packageJson.zgui.id;
  if (!capabilityString(id) || id !== id.trim())
    throw new Error(
      "package.json zgui.id must be a non-empty, trimmed string without control characters",
    );
  const windowConfig = parseWindowDeclaration(packageJson?.zgui?.window);
  const capabilities = await mergeCapabilities(packageJson?.zgui?.capabilities, {
    extraCapabilities,
    capabilitiesFile,
  });
  const workerEntries = parseWorkerDeclarations(packageJson?.zgui?.workers, appDirectory);
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
  const chunks = result.graph.chunks
    .map(chunk => chunk.fileName)
    .filter(fileName => fileName !== "app.mjs")
    .sort()
    .map(chunkPath => ({ path: chunkPath, sourceMap: `${chunkPath}.map` }));
  const identity: ManifestIdentity = {
    schemaVersion: 2,
    id,
    hostApiVersion: 1,
    entry: "app.mjs",
    sourceMap: "app.mjs.map",
    ...(chunks.length > 0 ? { chunks } : {}),
    ...(workers.length > 0 ? { workers } : {}),
    styles: [
      ...(componentsCss !== undefined ? ["components.css"] : []),
      ...(css !== undefined ? ["base.css"] : []),
      "app.css",
    ],
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
