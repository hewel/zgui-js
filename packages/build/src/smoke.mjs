import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { bundle, dependencyVersions, workspaceRoot as root } from "./build.mjs";

const rootRequire = createRequire(path.join(root, "package.json"));
export async function buildSmoke() {
  const fixture = "tests/fixtures/solid-smoke.ts";
  const outfile = "solid-smoke.mjs";
  const result = await bundle(path.join(root, fixture), { root, appDirectory: root, hostEntry: false, filename: outfile });
  const inputs = result.graph.modules.map(filename => path.relative(root, filename));
  // The smoke fixture must contain only itself and the production Solid/Signals runtime modules.
  const isAllowedModule = input =>
    input === fixture ||
    input.includes("/solid-js/dist/") ||
    input.includes("/@solidjs/signals/dist/prod/");
  if (!inputs.includes(fixture) || !inputs.every(isAllowedModule) || !result.solidRuntime) {
    throw new Error(`Unexpected smoke module graph: ${JSON.stringify(inputs)}`);
  }
  const output = result.graph.chunks[0];
  if (output.exports.join(",") !== "runSmoke") {
    throw new Error("Smoke must be one self-contained ESM module exporting runSmoke");
  }
  const metadata = {
    node: process.version, packageManager: rootRequire("./package.json").packageManager,
    packages: dependencyVersions(), conditions: ["browser", "production"], target: "es2022",
    solidRuntime: result.solidRuntime, inputs, exports: output.exports, imports: output.imports,
  };
  await mkdir(path.join(root, "target"), { recursive: true });
  for (const [filename, bytes] of result.files) await writeFile(path.join(root, "target", filename), bytes);
  await writeFile(path.join(root, "target/solid-smoke.meta.json"), JSON.stringify(result.graph, null, 2) + "\n");
  await writeFile(path.join(root, "target/solid-smoke.build.json"), JSON.stringify(metadata, null, 2) + "\n");
  console.log(`Built target/${outfile}; one reactive Solid runtime; no external imports`);
  console.log(JSON.stringify(metadata, null, 2));
}
