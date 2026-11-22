import { cp, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const cliDir = fileURLToPath(new URL(".", import.meta.url));
const repoRoot = path.resolve(cliDir, "../..");
const packagedInspectorDir = path.join(cliDir, "inspector");
const sourceInspectorDir = path.join(repoRoot, "apps/devtools/inspector");

if (process.argv[2] === "--clean") {
  await rm(packagedInspectorDir, { recursive: true, force: true });
} else {
  await rm(packagedInspectorDir, { recursive: true, force: true });
  await cp(sourceInspectorDir, packagedInspectorDir, {
    recursive: true,
    filter(source) {
      const base = path.basename(source);
      return (
        base !== "dist" &&
        base !== "node_modules" &&
        !base.startsWith(".zgui-") &&
        !base.startsWith(".dist-")
      );
    },
  });
}
