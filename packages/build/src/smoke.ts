import { mkdir } from "node:fs/promises";
import path from "node:path";
import { bundle, dependencyVersions, workspaceRoot as root } from "./build.ts";
import { sha256 } from "./identity.ts";

export async function buildSmoke(): Promise<void> {
  const fixture = "tests/fixtures/solid-smoke.ts";
  const outfile = "solid-smoke.mjs";
  const result = await bundle(path.join(root, fixture), {
    root,
    appDirectory: root,
    hostEntry: false,
    filename: outfile,
  });
  const inputs = result.graph.modules.map(filename => path.relative(root, filename));
  // The smoke fixture must contain only itself and the production Solid/Signals runtime modules.
  if (
    !inputs.includes(fixture) ||
    !inputs.every(
      input =>
        input === fixture ||
        input.includes("/solid-js/dist/") ||
        input.includes("/@solidjs/signals/dist/prod/"),
    ) ||
    !result.solidRuntime
  ) {
    throw new Error(`Unexpected smoke module graph: ${JSON.stringify(inputs)}`);
  }
  const output = result.graph.chunks[0];
  if (output.exports.join(",") !== "runSmoke") {
    throw new Error("Smoke must be one self-contained ESM module exporting runSmoke");
  }
  const packageJson = (await Bun.file(path.join(root, "package.json")).json()) as {
    packageManager: string;
  };
  const metadata = {
    bun: Bun.version,
    packageManager: packageJson.packageManager,
    packages: dependencyVersions({ modules: result.graph.modules }),
    conditions: ["browser", "production"],
    target: "es2022",
    lockfile: {
      path: "bun.lock",
      digest: sha256(await Bun.file(path.join(root, "bun.lock")).bytes()),
    },
    solidRuntime: result.solidRuntime,
    inputs,
    exports: output.exports,
    imports: output.imports,
  };
  await mkdir(path.join(root, "target"), { recursive: true });
  for (const [filename, bytes] of result.files)
    await Bun.write(path.join(root, "target", filename), bytes);
  await Bun.write(
    path.join(root, "target/solid-smoke.meta.json"),
    JSON.stringify(result.graph, null, 2) + "\n",
  );
  await Bun.write(
    path.join(root, "target/solid-smoke.build.json"),
    JSON.stringify(metadata, null, 2) + "\n",
  );
  console.log(`Built target/${outfile}; one reactive Solid runtime; no external imports`);
  console.log(JSON.stringify(metadata, null, 2));
}
