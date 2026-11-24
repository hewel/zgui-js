#!/usr/bin/env bun
import { copyFile, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));

export const STAGED_LICENSE_FILES = ["LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md"] as const;

export async function stageLicenseFiles(packageDir: string, root = repoRoot): Promise<void> {
  const targetDir = path.resolve(packageDir);
  for (const filename of STAGED_LICENSE_FILES) {
    await copyFile(path.join(root, filename), path.join(targetDir, filename));
  }
}

export async function cleanLicenseFiles(packageDir: string): Promise<void> {
  const targetDir = path.resolve(packageDir);
  for (const filename of STAGED_LICENSE_FILES) {
    await rm(path.join(targetDir, filename), { force: true });
  }
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  let clean = false;
  let targetDir: string | undefined;
  for (const arg of args) {
    if (arg === "--clean") {
      clean = true;
    } else if (!arg.startsWith("-")) {
      targetDir = arg;
    } else {
      console.error(`Unknown option: ${arg}`);
      process.exit(1);
    }
  }
  const dir = path.resolve(targetDir ?? process.cwd());
  if (clean) {
    await cleanLicenseFiles(dir);
  } else {
    await stageLicenseFiles(dir);
  }
}
