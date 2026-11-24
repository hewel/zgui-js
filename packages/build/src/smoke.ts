import { realpathSync } from "node:fs";
import { mkdir, mkdtemp } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { bundle, dependencyVersions, workspaceRoot as root } from "./build.ts";
import { sha256 } from "./identity.ts";
import { packagedSdkRoot } from "./sdk.ts";

export function resolveSmokeTsconfigPath(sdkRoot: string = root): string {
  const resolved = path.resolve(sdkRoot);
  if (resolved === packagedSdkRoot) {
    return path.join(packagedSdkRoot, "smoke/tsconfig.smoke.json");
  }
  return path.join(resolved, "packages/build/tsconfig.smoke.json");
}

export async function buildSmoke({
  outDir,
}: {
  outDir?: string;
} = {}): Promise<{ outDir: string; bundlePath: string }> {
  const installed = path.resolve(root) === packagedSdkRoot;
  const fixturePath = installed
    ? path.join(packagedSdkRoot, "smoke/solid-smoke.ts")
    : path.join(root, "tests/fixtures/solid-smoke.ts");
  const appDirectory = installed ? path.resolve(packagedSdkRoot, "..") : root;
  const outfile = "solid-smoke.mjs";
  const result = await bundle(fixturePath, {
    root,
    appDirectory,
    hostEntry: false,
    filename: outfile,
  });
  const realFixture = realpathSync(fixturePath);
  const inputs = result.graph.modules.map(filename => path.relative(root, filename));
  const resolvedModules = result.graph.modules.map(filename => {
    try {
      return realpathSync(filename);
    } catch {
      return path.resolve(filename);
    }
  });
  // The smoke fixture must contain only itself and the production Solid/Signals runtime modules.
  if (
    !resolvedModules.includes(realFixture) ||
    !resolvedModules.every(
      modulePath =>
        modulePath === realFixture ||
        modulePath.includes("/solid-js/dist/") ||
        modulePath.includes("/@solidjs/signals/dist/prod/"),
    ) ||
    !result.solidRuntime
  ) {
    throw new Error(`Unexpected smoke module graph: ${JSON.stringify(inputs)}`);
  }
  const output = result.graph.chunks[0];
  if (!output || output.exports.join(",") !== "runSmoke") {
    throw new Error("Smoke must be one self-contained ESM module exporting runSmoke");
  }
  const packageJson = (await Bun.file(path.join(root, "package.json")).json()) as {
    packageManager: string;
  };
  const metadata = {
    bun: Bun.version,
    packageManager: packageJson.packageManager,
    packages: dependencyVersions({ root, appDirectory, modules: result.graph.modules }),
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
  const targetDir = outDir
    ? path.resolve(outDir)
    : installed
      ? await mkdtemp(path.join(os.tmpdir(), "zgui-smoke-"))
      : path.join(root, "target");
  await mkdir(targetDir, { recursive: true });
  for (const [filename, bytes] of result.files) {
    await Bun.write(path.join(targetDir, filename), bytes);
  }
  await Bun.write(
    path.join(targetDir, "solid-smoke.meta.json"),
    JSON.stringify(result.graph, null, 2) + "\n",
  );
  await Bun.write(
    path.join(targetDir, "solid-smoke.build.json"),
    JSON.stringify(metadata, null, 2) + "\n",
  );
  const displayTarget = installed ? path.join(targetDir, outfile) : `target/${outfile}`;
  console.log(`Built ${displayTarget}; one reactive Solid runtime; no external imports`);
  console.log(JSON.stringify(metadata, null, 2));
  return { outDir: targetDir, bundlePath: path.join(targetDir, outfile) };
}
