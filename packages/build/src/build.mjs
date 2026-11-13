import { readFile, readdir, lstat, mkdir, writeFile, rename, symlink, rm } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformAsync } from "@babel/core";
import typescriptPreset from "@babel/preset-typescript";
import solidPreset from "babel-preset-solid";
import { build } from "esbuild";
import { validateModule, validateImport, validateGraph } from "./validate.mjs";
import { typecheckProject } from "./typecheck.mjs";

export const workspaceRoot = fileURLToPath(new URL("../../../", import.meta.url));
const require = createRequire(import.meta.url);
const json = value => JSON.stringify(value, null, 2) + "\n";
const sha256 = bytes => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;

export async function optionalFile(filename) {
  try { return await readFile(filename); }
  catch (error) { if (error.code === "ENOENT") return undefined; throw error; }
}

export function compilerPlugin() {
  return {
    name: "solid-universal-typescript",
    setup(builder) {
      builder.onResolve({ filter: /.*/ }, args => { validateImport(args.path); });
      builder.onLoad({ filter: /\.[cm]?[jt]sx?$/ }, async ({ path: filename }) => {
        const source = await readFile(filename, "utf8");
        const transformed = await transformAsync(source, {
          filename, babelrc: false, configFile: false, sourceMaps: "inline",
          presets: [
            [solidPreset, { generate: "universal", moduleName: "@zgui/ui/renderer", hydratable: false }],
            [typescriptPreset, { onlyRemoveTypeImports: true, allExtensions: true, isTSX: /\.tsx$/.test(filename) }],
          ],
        });
        if (!transformed?.code) throw new Error(`Babel emitted no JavaScript for ${filename}`);
        validateModule(transformed.code, { filename });
        return { contents: transformed.code, loader: "js", resolveDir: path.dirname(filename) };
      });
    },
  };
}

export async function bundle(entry, { root = workspaceRoot } = {}) {
  const result = await build({
    absWorkingDir: root, entryPoints: [entry], outfile: path.join(root, "app.mjs"),
    bundle: true, format: "esm", platform: "neutral", conditions: ["browser", "production"],
    mainFields: ["module", "main"], target: "es2022", splitting: false,
    sourcemap: "linked", sourcesContent: true, metafile: true, write: false,
    define: { "process.env.NODE_ENV": '"production"' }, plugins: [compilerPlugin()], logLevel: "silent",
  });
  const solidRuntime = validateGraph(result.metafile);
  const files = new Map(result.outputFiles.map(file => [path.basename(file.path), Buffer.from(file.contents)]));
  validateModule(files.get("app.mjs").toString(), { bundled: true, entry: true });
  return { files, metafile: result.metafile, solidRuntime };
}

export function dependencyVersions() {
  const presetRequire = createRequire(require.resolve("babel-preset-solid"));
  const compilerRequire = createRequire(presetRequire.resolve("babel-plugin-jsx-dom-expressions"));
  return Object.fromEntries([
    ...["solid-js", "babel-preset-solid", "@babel/core", "@babel/parser", "@babel/traverse", "@babel/preset-typescript", "esbuild", "typescript"]
      .map(name => [name, require(`${name}/package.json`).version]),
    ["babel-plugin-jsx-dom-expressions", presetRequire("babel-plugin-jsx-dom-expressions/package.json").version],
    ["@babel/plugin-syntax-jsx", compilerRequire("@babel/plugin-syntax-jsx/package.json").version],
  ]);
}

export async function buildInfo(root, result) {
  const cargoLock = await readFile(path.join(root, "Cargo.lock"), "utf8");
  const revision = cargoLock.match(/name = "rquickjs-sys"\nversion = "([^"]+)"\nsource = "[^"\n]+#([a-f0-9]+)"/);
  // This gitlink was verified against the pinned rquickjs checkout, not inferred from its crate version.
  const engines = { "73fcd6062ee615aeb2a4675cd32be67a56e93973": "6d46d07d04041b40f4f49eaa7fdebe44c314c699" };
  if (!revision || !engines[revision[2]]) throw new Error("Unknown resolved rquickjs engine; audit its QuickJS-NG gitlink before packaging");
  const packageJson = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
  return {
    schemaVersion: 1, hostApiVersion: 1, node: process.version, packageManager: packageJson.packageManager,
    packages: dependencyVersions(), lockfile: { path: "pnpm-lock.yaml", digest: sha256(await readFile(path.join(root, "pnpm-lock.yaml"))) },
    cargoLock: { path: "Cargo.lock", digest: sha256(cargoLock) },
    engine: { name: "QuickJS-NG", revision: engines[revision[2]], rquickjsVersion: revision[1], rquickjsRevision: revision[2] },
    conditions: ["browser", "production"], target: "es2022", solidRuntime: result.solidRuntime,
    inputs: Object.keys(result.metafile.inputs),
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

// Contract v1: resources only, not manifest/build-info. Host uses the same byte framing.
export function contentDigest(files, manifest) {
  const names = [manifest.entry, manifest.sourceMap, ...manifest.styles, ...manifest.assets].sort();
  if (new Set(names).size !== names.length) throw new Error("Duplicate package resource");
  const hash = createHash("sha256");
  for (const name of names) {
    resourcePath(name);
    const bytes = files.get(name);
    if (!bytes) throw new Error(`Missing package resource: ${name}`);
    hash.update(name, "utf8").update("\0").update(bytes).update("\0");
  }
  return `sha256:${hash.digest("hex")}`;
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

export async function buildApplication(app, { root = workspaceRoot, appDirectory = path.join(root, "apps/examples", app) } = {}) {
  if (!/^[a-z][a-z0-9-]*$/.test(app)) throw new Error(`Invalid application name: ${app}`);
  typecheckProject(path.join(appDirectory, "tsconfig.json"));
  const result = await bundle(path.join(appDirectory, "src/main.tsx"), { root });
  const files = result.files;
  const css = await optionalFile(path.join(appDirectory, "src/app.css"));
  if (css !== undefined) files.set("app.css", css);
  const assets = await collectAssets(path.join(appDirectory, "assets"));
  for (const item of assets) files.set(...item);
  const manifest = {
    schemaVersion: 1, id: `dev.example.${app}`, hostApiVersion: 1, entry: "app.mjs", sourceMap: "app.mjs.map",
    styles: css === undefined ? [] : ["app.css"], assets: [...assets.keys()].sort(),
  };
  manifest.buildId = contentDigest(files, manifest);
  files.set("build-info.json", Buffer.from(json(await buildInfo(root, result))));
  files.set("manifest.json", Buffer.from(json(manifest)));
  const dist = await publishGeneration(appDirectory, files);
  return { dist, manifest };
}
