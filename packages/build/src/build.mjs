import { readFile, readdir, lstat, mkdir, writeFile, rename, symlink, rm } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { rolldown } from "rolldown";
import { compilerOptions, outputOptions, compilationResult } from "./compiler.mjs";
import { typecheckProject } from "./typecheck.mjs";
import { canonicalJson, sha256, moduleIdentity } from "./identity.mjs";
import { validateStylesheet } from "./stylex.mjs";

export const workspaceRoot = fileURLToPath(new URL("../../../", import.meta.url));
const require = createRequire(import.meta.url);
const json = value => JSON.stringify(value, null, 2) + "\n";

export async function optionalFile(filename) {
  try { return await readFile(filename); }
  catch (error) { if (error.code === "ENOENT") return undefined; throw error; }
}

export async function bundle(entry, { root = workspaceRoot, appDirectory = path.dirname(entry), hostEntry = true, filename = "app.mjs", stylexDiagnostics = false } = {}) {
  let result;
  const compiler = await rolldown(compilerOptions(entry, { root, appDirectory, stylexDiagnostics, plugins: [{
    name: "capture-native-graph",
    generateBundle(_options, output) {
      result = compilationResult(Object.values(output), this.getModuleIds(), { entry: hostEntry });
    },
  }] }));
  try {
    await compiler.generate(outputOptions(filename));
    return result;
  } finally { await compiler.close(); }
}

export function dependencyVersions() {
  const presetRequire = createRequire(require.resolve("babel-preset-solid"));
  const compilerRequire = createRequire(presetRequire.resolve("babel-plugin-jsx-dom-expressions"));
  return Object.fromEntries([
    ...["solid-js", "babel-preset-solid", "@babel/core", "@babel/parser", "@babel/traverse", "@babel/preset-typescript", "rolldown", "typescript", "@stylexjs/stylex", "@stylexjs/babel-plugin", "styleq"]
      .map(name => [name, require(`${name}/package.json`).version]),
    ["babel-plugin-jsx-dom-expressions", presetRequire("babel-plugin-jsx-dom-expressions/package.json").version],
    ["@babel/plugin-syntax-jsx", compilerRequire("@babel/plugin-syntax-jsx/package.json").version],
  ]);
}

export async function buildInfo(root, result, { appDirectory = root } = {}) {
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
    conditions: ["browser", "production"], target: "es2022", solidRuntime: identity(result.solidRuntime),
    graph: {
      modules: result.graph.modules.map(identity).sort(),
      chunks: result.graph.chunks.map(chunk => ({ ...chunk, modules: chunk.modules.map(identity).sort() })),
    },
    styleSystem: result.styleSystem,
  };
}

function resourcePath(name) {
  if (!name || name.includes("\\") || name.includes("\0") || name.startsWith("/") || name.split("/").some(part => !part || part === "." || part === "..")) {
    throw new Error(`Non-canonical resource path: ${name}`);
  }
  return name;
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

export async function publishGeneration(appDirectory, files) {
  const generations = path.join(appDirectory, ".zgui-generations");
  await mkdir(generations, { recursive: true });
  const token = randomUUID();
  const staging = path.join(generations, `.building-${token}`);
  const generation = path.join(generations, token);
  const link = path.join(appDirectory, `.dist-${token}`);
  const dist = path.join(appDirectory, "dist");
  let published = false;
  try {
    try {
      const existing = await lstat(dist);
      if (!existing.isSymbolicLink()) throw new Error("dist must be absent or a generation symlink; refusing to overwrite it");
    } catch (error) { if (error.code !== "ENOENT") throw error; }
    await mkdir(staging);
    for (const [name, bytes] of files) {
      resourcePath(name);
      const destination = path.join(staging, name);
      await mkdir(path.dirname(destination), { recursive: true });
      await writeFile(destination, bytes, { flag: "wx" });
    }
    await rename(staging, generation);
    await symlink(path.relative(appDirectory, generation), link, "dir");
    // Linux rename replaces the symlink atomically. Prior generations remain pinned for active readers.
    await rename(link, dist);
    published = true;
    return dist;
  } finally {
    await rm(link, { force: true });
    await rm(staging, { recursive: true, force: true });
    if (!published) await rm(generation, { recursive: true, force: true });
  }
}

export async function buildApplication(app, { root = workspaceRoot, appDirectory = path.join(root, "apps/examples", app), stylexDiagnostics = false } = {}) {
  if (!/^[a-z][a-z0-9-]*$/.test(app)) throw new Error(`Invalid application name: ${app}`);
  typecheckProject(path.join(appDirectory, "tsconfig.json"));
  const result = await bundle(path.join(appDirectory, "src/main.tsx"), { root, appDirectory, stylexDiagnostics });
  return publishApplication(app, result, { root, appDirectory });
}

export async function publishApplication(app, result, { root, appDirectory, signal }) {
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
  files.set("build-info.json", Buffer.from(json(await buildInfo(root, result, { appDirectory }))));
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
