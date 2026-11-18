import { readFileSync } from "node:fs";
import { readFile, readdir, lstat } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { compileBundle } from "./compiler.mjs";
import { typecheckProject } from "./typecheck.mjs";
import { canonicalJson, sha256, moduleIdentity } from "./identity.mjs";
import { publishGeneration, resourcePath } from "./publication.mjs";
import { validateStylesheet } from "./stylex.mjs";

export { publishGeneration };
export const workspaceRoot = fileURLToPath(new URL("../../../", import.meta.url));
const require = createRequire(import.meta.url);
const json = value => JSON.stringify(value, null, 2) + "\n";

function packageVersion(req, id) {
  let dir = path.dirname(req.resolve(id));
  while (true) {
    try {
      const pkg = JSON.parse(readFileSync(path.join(dir, "package.json"), "utf8"));
      if (pkg.name === id) return pkg.version;
    } catch {}
    const parent = path.dirname(dir);
    if (parent === dir) throw new Error(`Cannot locate package.json for ${id}`);
    dir = parent;
  }
}

export async function optionalFile(filename) {
  try { return await readFile(filename); }
  catch (error) { if (error.code === "ENOENT") return undefined; throw error; }
}

export async function bundle(entry, { root = workspaceRoot, appDirectory = path.dirname(entry), hostEntry = true, filename = "app.mjs", stylexDiagnostics = false, development = false } = {}) {
  return compileBundle(entry, { root, appDirectory, hostEntry, filename, stylexDiagnostics, development });
}

export function dependencyVersions() {
  const solidRoot = path.dirname(require.resolve("solid-js/package.json"));
  const signalsPkgPath = path.join(path.dirname(solidRoot), "@solidjs/signals/package.json");
  const babelPluginRequire = createRequire(require.resolve("@solidjs/babel-plugin"));
  const uiRequire = createRequire(path.join(workspaceRoot, "packages/ui/package.json"));
  return Object.fromEntries([
    ...["solid-js", "@solidjs/babel-plugin", "@babel/core", "@babel/parser", "@babel/traverse", "@babel/preset-typescript", "rolldown", "tsrs", "@stylexjs/stylex", "@stylexjs/babel-plugin", "styleq"]
      .map(name => [name, require(`${name}/package.json`).version]),
    ["@solidjs/universal", packageVersion(uiRequire, "@solidjs/universal")],
    ["@solidjs/signals", JSON.parse(readFileSync(signalsPkgPath, "utf8")).version],
    ["@babel/plugin-syntax-jsx", babelPluginRequire("@babel/plugin-syntax-jsx/package.json").version],
  ]);
}

export async function buildInfo(root, result, { appDirectory = root, development = false } = {}) {
  const cargoLock = await readFile(path.join(root, "Cargo.lock"), "utf8");
  const revision = cargoLock.match(/name = "rquickjs-sys"\nversion = "([^"]+)"\nsource = "[^"\n]+#([a-f0-9]+)"/);
  // This gitlink was verified against the pinned rquickjs checkout, not inferred from its crate version.
  const engines = { "73fcd6062ee615aeb2a4675cd32be67a56e93973": "6d46d07d04041b40f4f49eaa7fdebe44c314c699" };
  if (!revision || !engines[revision[2]]) throw new Error("Unknown resolved rquickjs engine; audit its QuickJS-NG gitlink before packaging");
  const packageJson = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
  const identity = filename => moduleIdentity(filename, { root, appDirectory });
  return {
    schemaVersion: 2, hostApiVersion: 1, node: process.version, packageManager: packageJson.packageManager,
    packages: dependencyVersions(), lockfile: { path: "pnpm-lock.yaml", digest: sha256(await readFile(path.join(root, "pnpm-lock.yaml"))) },
    cargoLock: { path: "Cargo.lock", digest: sha256(cargoLock) },
    engine: { name: "QuickJS-NG", revision: engines[revision[2]], rquickjsVersion: revision[1], rquickjsRevision: revision[2] },
    conditions: development ? ["development", "browser"] : ["browser", "production"], target: "es2022", solidRuntime: identity(result.solidRuntime),
    graph: {
      modules: result.graph.modules.map(identity).sort(),
      chunks: result.graph.chunks.map(chunk => ({ ...chunk, modules: chunk.modules.map(identity).sort() })),
    },
    styleSystem: result.styleSystem,
  };
}

async function collectAssets(directory, prefix = "assets") {
  let entries;
  try {
    if (!(await lstat(directory)).isDirectory()) throw new Error(`Assets must be regular files/directories, not links: ${prefix}`);
    entries = await readdir(directory, { withFileTypes: true });
  }
  catch (error) { if (error.code === "ENOENT") return new Map(); throw error; }
  const files = new Map();
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
export function contentDigest(files, manifest) {
  const { buildId: _buildId, ...identity } = manifest;
  const names = Object.keys(manifest.artifactDigests).sort();
  const packaged = [...files.keys()].filter(name => name !== "manifest.json").sort();
  if (JSON.stringify(names) !== JSON.stringify(packaged)) throw new Error("Artifact digest inventory does not match packaged files");
  for (const name of names) {
    resourcePath(name);
    const bytes = files.get(name);
    if (!bytes) throw new Error(`Missing package resource: ${name}`);
    if (sha256(bytes) !== manifest.artifactDigests[name]) throw new Error(`Artifact digest mismatch: ${name}`);
  }
  return sha256(canonicalJson(identity));
}

export async function buildApplication(app, { root = workspaceRoot, appDirectory = path.join(root, "apps/examples", app), stylexDiagnostics = false, development = false } = {}) {
  if (!/^[a-z][a-z0-9-]*$/.test(app)) throw new Error(`Invalid application name: ${app}`);
  typecheckProject(path.join(appDirectory, "tsconfig.json"));
  const result = await bundle(path.join(appDirectory, "src/main.tsx"), { root, appDirectory, stylexDiagnostics, development });
  return publishApplication(app, result, { root, appDirectory, development });
}

export async function publishApplication(app, result, { root, appDirectory, signal, development = false }) {
  signal?.throwIfAborted();
  const files = new Map(result.files);
  if (!files.has("app.css") || !files.has("style-origins.json") || !result.styleSystem) throw new Error("Missing StyleX compilation output");
  validateStylesheet(files.get("app.css").toString(), "app.css", { allowDynamicProperties: true });
  const css = await optionalFile(path.join(appDirectory, "src/app.css"));
  if (css !== undefined) {
    validateStylesheet(css.toString(), "base.css");
    files.set("base.css", css);
  }
  const assets = await collectAssets(path.join(appDirectory, "assets"));
  for (const item of assets) files.set(...item);
  const pkg = await optionalFile(path.join(appDirectory, "package.json"));
  const fonts = pkg === undefined ? [] : (JSON.parse(pkg.toString("utf8")).zgui?.fonts ?? []);
  if (!Array.isArray(fonts)) throw new Error("package.json zgui.fonts must be an array of {family,path}");
  const families = new Set();
  for (const font of fonts) {
    if (!font || typeof font.family !== "string" || !font.family.trim() || font.family !== font.family.trim() ||
        typeof font.path !== "string" || Object.keys(font).some(key => !["family", "path"].includes(key))) {
      throw new Error("Invalid application font mapping; expected {family,path}");
    }
    resourcePath(font.path);
    if (!assets.has(font.path)) throw new Error(`Font path must be a declared asset: ${font.path}`);
    const family = font.family.toLowerCase();
    if (families.has(family)) throw new Error(`Duplicate application font family: ${font.family}`);
    families.add(family);
  }
  files.set("build-info.json", Buffer.from(json(await buildInfo(root, result, { appDirectory, development }))));
  const manifest = {
    schemaVersion: 2, id: `dev.example.${app}`, hostApiVersion: 1, entry: "app.mjs", sourceMap: "app.mjs.map",
    styles: css === undefined ? ["app.css"] : ["base.css", "app.css"], assets: [...assets.keys()].sort(),
    fonts, styleSystem: result.styleSystem,
    artifactDigests: Object.fromEntries([...files].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([name, bytes]) => [name, sha256(bytes)])),
  };
  manifest.buildId = contentDigest(files, manifest);
  signal?.throwIfAborted();
  files.set("manifest.json", Buffer.from(json(manifest)));
  signal?.throwIfAborted();
  const dist = await publishGeneration(appDirectory, files);
  return { dist, manifest };
}
