#!/usr/bin/env bun
import {
  buildSdkPackages,
  ensureSdkPackagesBuilt,
  type SdkPackageName,
} from "@hewel/zgui-build/internal";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));

const usage = `Usage: bun tools/build-packages.ts [ui] [services] [--if-stale] [--quiet]`;

const args = process.argv.slice(2);
if (args.includes("-h") || args.includes("--help")) {
  console.log(usage);
  process.exit(0);
}

const packages: SdkPackageName[] = [];
let silent = false;
let ifStale = false;

for (const arg of args) {
  if (arg === "--quiet" || arg === "-q") {
    silent = true;
  } else if (arg === "--if-stale") {
    ifStale = true;
  } else if (arg === "ui" || arg === "services") {
    packages.push(arg);
  } else {
    console.error(`Unknown argument: ${arg}\n${usage}`);
    process.exit(1);
  }
}

try {
  const targetPackages: SdkPackageName[] = packages.length > 0 ? packages : ["ui", "services"];
  if (ifStale) {
    await ensureSdkPackagesBuilt(repoRoot, { silent, packages: targetPackages });
  } else {
    await buildSdkPackages({
      root: repoRoot,
      packages: targetPackages,
      silent,
    });
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
