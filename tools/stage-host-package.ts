#!/usr/bin/env bun
import {
  SUPPORTED_HOST_PACKAGES,
  stageHostPackage,
  verifyHostPackage,
} from "@hewel/zgui-build/internal";
import { existsSync } from "node:fs";
import path from "node:path";
import { STAGED_LICENSE_FILES, stageLicenseFiles } from "./pack-package.ts";

const usage = `Usage: bun tools/stage-host-package.ts --platform <linux-x64|linux-arm64> --binary <PATH> [--package-dir DIR] [--glibc-baseline STR] [--skip-probe]
       bun tools/stage-host-package.ts --verify <PACKAGE_DIR> [--require-licenses] [--skip-probe]`;

function detectGlibcBaseline(): string | undefined {
  if (process.platform !== "linux") return undefined;
  const child = Bun.spawnSync(["ldd", "--version"], {
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
    timeout: 3000,
  });
  const text = `${child.stdout.toString("utf8")}\n${child.stderr.toString("utf8")}`;
  const match = text.match(/(?:GNU libc|GLIBC)[^0-9]*([0-9]+\.[0-9]+)/i);
  return match ? `glibc-${match[1]}` : undefined;
}

const args = process.argv.slice(2);
if (!args.length || args.includes("-h") || args.includes("--help")) {
  console.log(usage);
  process.exit(0);
}

let platform: string | undefined;
let binary: string | undefined;
let packageDir: string | undefined;
let verifyDir: string | undefined;
let glibcBaseline: string | undefined;
let skipProbe = false;
let requireLicenses = false;

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
    case "--glibc-baseline":
      glibcBaseline = args.shift();
      break;
    case "--skip-probe":
      skipProbe = true;
      break;
    case "--require-licenses":
      requireLicenses = true;
      break;
    default:
      console.error(`Unknown option: ${arg}\n${usage}`);
      process.exit(1);
  }
}

try {
  if (verifyDir) {
    const resolvedVerifyDir = path.resolve(verifyDir);
    const verified = await verifyHostPackage(resolvedVerifyDir, { verifyAbi: !skipProbe });
    if (requireLicenses) {
      for (const file of STAGED_LICENSE_FILES) {
        if (!existsSync(path.join(resolvedVerifyDir, file))) {
          throw new Error(
            `Missing required staged license file: ${path.join(resolvedVerifyDir, file)}`,
          );
        }
      }
    }
    const glibcInfo = verified.manifest.glibcBaseline ? `, ${verified.manifest.glibcBaseline}` : "";
    console.log(
      `Verified ${verified.manifest.package} (${verified.manifest.platform}${glibcInfo}, ${verified.manifest.sha256}, ${verified.manifest.bytes} bytes)`,
    );
  } else {
    if (!platform || !binary) {
      console.error(
        `Both --platform and --binary are required.\nSupported platforms: ${Object.keys(SUPPORTED_HOST_PACKAGES).join(", ")}\n${usage}`,
      );
      process.exit(1);
    }
    const resolvedBaseline =
      glibcBaseline ?? (platform.startsWith("linux-") ? detectGlibcBaseline() : undefined);
    const staged = await stageHostPackage({
      platform,
      binary: path.resolve(binary),
      packageDir: packageDir ? path.resolve(packageDir) : undefined,
      skipProbe,
      glibcBaseline: resolvedBaseline,
    });
    await stageLicenseFiles(staged.packageDir);
    const glibcInfo = staged.manifest.glibcBaseline ? `, ${staged.manifest.glibcBaseline}` : "";
    console.log(
      `Staged ${staged.manifest.package} -> ${staged.binaryPath} (${staged.manifest.sha256}, ${staged.manifest.bytes} bytes${glibcInfo})`,
    );
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
