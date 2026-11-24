import { buildApplication } from "@hewel/zgui-build";
import { existsSync } from "node:fs";
import { copyFile, cp, realpath, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const cliDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(cliDir, "../..");
const packagedBundleDir = path.join(cliDir, "inspector-bundle");
const legacyInspectorDir = path.join(cliDir, "inspector");
const sourceInspectorDir = path.join(repoRoot, "apps/devtools/inspector");
const LICENSE_FILES = ["LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md"] as const;

if (process.argv[2] === "--clean") {
  await rm(packagedBundleDir, { recursive: true, force: true });
  await rm(legacyInspectorDir, { recursive: true, force: true });
  for (const filename of LICENSE_FILES) {
    await rm(path.join(cliDir, filename), { force: true });
  }
} else {
  await rm(packagedBundleDir, { recursive: true, force: true });
  await rm(legacyInspectorDir, { recursive: true, force: true });
  for (const filename of LICENSE_FILES) {
    await copyFile(path.join(repoRoot, filename), path.join(cliDir, filename));
  }
  const built = await buildApplication("inspector", {
    root: repoRoot,
    appDirectory: sourceInspectorDir,
    development: false,
  });
  const realDist = await realpath(built.dist);
  await cp(realDist, packagedBundleDir, { recursive: true, dereference: true });
  for (const required of ["manifest.json", "app.mjs"]) {
    if (!existsSync(path.join(packagedBundleDir, required))) {
      throw new Error(`Missing prebuilt inspector artifact: ${required}`);
    }
  }
}
