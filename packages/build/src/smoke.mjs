import { readFile, mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformAsync } from "@babel/core";
import typescriptPreset from "@babel/preset-typescript";
import solidPreset from "babel-preset-solid";
import { build } from "esbuild";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const require = createRequire(import.meta.url);
const rootRequire = createRequire(path.join(root, "package.json"));
export async function buildSmoke() {
  const fixture = "tests/fixtures/solid-smoke.ts";
  const outfile = "target/solid-smoke.mjs";
  const result = await build({
    absWorkingDir: root,
    entryPoints: [fixture],
    outfile,
    bundle: true,
    format: "esm",
    platform: "neutral",
    conditions: ["browser", "production"],
    mainFields: ["module", "main"],
    target: "es2022",
    splitting: false,
    sourcemap: "external",
    sourcesContent: true,
    metafile: true,
    write: false,
    define: { "process.env.NODE_ENV": '"production"' },
    plugins: [{
      name: "solid-typescript",
      setup(builder) {
        builder.onLoad({ filter: /\.[cm]?tsx?$/ }, async ({ path: filename }) => {
          const source = await readFile(filename, "utf8");
          const transformed = await transformAsync(source, {
            filename,
            babelrc: false,
            configFile: false,
            sourceMaps: "inline",
            presets: [
              [solidPreset, {
                generate: "universal",
                moduleName: "@zgui/ui/renderer",
                hydratable: false,
              }],
              [typescriptPreset, { onlyRemoveTypeImports: true }],
            ],
          });
          if (!transformed?.code) {
            throw new Error(`Babel emitted no JavaScript for ${filename}`);
          }
          return {
            contents: transformed.code,
            loader: "js",
            resolveDir: path.dirname(filename),
          };
        });
      },
    }],
  });

  const inputs = Object.keys(result.metafile.inputs);
  const runtimes = inputs.filter((input) => input.endsWith("/solid-js/dist/solid.js"));
  // A strict Phase 0 allowlist catches server, DOM, builtin, and duplicate-runtime drift.
  if (runtimes.length !== 1 || inputs.length !== 2 || !inputs.includes(fixture)) {
    throw new Error(`Unexpected smoke module graph: ${JSON.stringify(inputs)}`);
  }
  const output = result.metafile.outputs[outfile];
  if (!output || output.imports.length !== 0 || output.exports.join(",") !== "runSmoke") {
    throw new Error("Smoke must be one self-contained ESM module exporting runSmoke");
  }

  const presetRequire = createRequire(require.resolve("babel-preset-solid"));
  const compilerRequire = createRequire(presetRequire.resolve("babel-plugin-jsx-dom-expressions"));
  const packages = {
    "solid-js": require("solid-js/package.json").version,
    "babel-preset-solid": require("babel-preset-solid/package.json").version,
    "@babel/core": require("@babel/core/package.json").version,
    "@babel/preset-typescript": require("@babel/preset-typescript/package.json").version,
    "babel-plugin-jsx-dom-expressions": presetRequire("babel-plugin-jsx-dom-expressions/package.json").version,
    "@babel/plugin-syntax-jsx": compilerRequire("@babel/plugin-syntax-jsx/package.json").version,
    esbuild: require("esbuild/package.json").version,
    typescript: rootRequire("typescript/package.json").version,
  };
  const metadata = {
    node: process.version,
    packageManager: rootRequire("./package.json").packageManager,
    packages,
    conditions: ["browser", "production"],
    target: "es2022",
    solidRuntime: runtimes[0],
    inputs,
    exports: output.exports,
    imports: output.imports,
  };
  await mkdir(path.join(root, "target"), { recursive: true });
  for (const file of result.outputFiles) {
    await writeFile(file.path, file.contents);
  }
  await writeFile(path.join(root, "target/solid-smoke.meta.json"), JSON.stringify(result.metafile, null, 2) + "\n");
  await writeFile(path.join(root, "target/solid-smoke.build.json"), JSON.stringify(metadata, null, 2) + "\n");
  console.log(`Built ${outfile}; one reactive Solid runtime; no external imports`);
  console.log(JSON.stringify(metadata, null, 2));
}
