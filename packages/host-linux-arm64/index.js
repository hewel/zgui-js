import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageDir = path.dirname(fileURLToPath(import.meta.url));

export const platform = "linux-arm64";
export const binaryPath = path.join(packageDir, "bin", "zgui-host");
export const manifestPath = path.join(packageDir, "host-manifest.json");

export function resolveHostBinary() {
  return existsSync(binaryPath) ? binaryPath : null;
}

export function readHostManifest() {
  if (!existsSync(manifestPath)) return null;
  return JSON.parse(readFileSync(manifestPath, "utf8"));
}
