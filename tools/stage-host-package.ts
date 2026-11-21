#!/usr/bin/env bun
import {
  SUPPORTED_HOST_PACKAGES,
  stageHostPackage,
  verifyHostPackage,
} from "@hewel/zgui-build/internal";
import path from "node:path";

const usage = `Usage: bun tools/stage-host-package.ts --platform <linux-x64|linux-arm64> --binary <PATH> [--package-dir DIR] [--skip-probe]
       bun tools/stage-host-package.ts --verify <PACKAGE_DIR> [--skip-probe]`;

const args = process.argv.slice(2);
if (!args.length || args.includes("-h") || args.includes("--help")) {
  console.log(usage);
  process.exit(0);
}

let platform: string | undefined;
let binary: string | undefined;
let packageDir: string | undefined;
let verifyDir: string | undefined;
let skipProbe = false;

while (args.length > 0) {
  const arg = args.shift()!;
  switch (arg) {
    case "--platform":
      platform = args.shift();
      break;
    case "--binary":
      binary = args.shift();
      break;
    case "--package-dir":
      packageDir = args.shift();
      break;
    case "--verify":
      verifyDir = args.shift();
      break;
    case "--skip-probe":
      skipProbe = true;
      break;
    default:
      console.error(`Unknown option: ${arg}\n${usage}`);
      process.exit(1);
  }
}

try {
  if (verifyDir) {
    const verified = await verifyHostPackage(verifyDir, { verifyAbi: !skipProbe });
    console.log(
      `Verified ${verified.manifest.package} (${verified.manifest.platform}, ${verified.manifest.sha256}, ${verified.manifest.bytes} bytes)`,
    );
  } else {
    if (!platform || !binary) {
      console.error(
        `Both --platform and --binary are required.\nSupported platforms: ${Object.keys(SUPPORTED_HOST_PACKAGES).join(", ")}\n${usage}`,
      );
      process.exit(1);
    }
    const staged = await stageHostPackage({
      platform,
      binary: path.resolve(binary),
      packageDir: packageDir ? path.resolve(packageDir) : undefined,
      skipProbe,
    });
    console.log(
      `Staged ${staged.manifest.package} -> ${staged.binaryPath} (${staged.manifest.sha256}, ${staged.manifest.bytes} bytes)`,
    );
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
