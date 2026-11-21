import { copyFile, cp, mkdir, rm } from "node:fs/promises";
import path from "node:path";
import { packagedSdkRoot, workspaceRoot } from "./src/sdk.ts";

if (process.argv[2] === "--clean") {
  await rm(packagedSdkRoot, { recursive: true, force: true });
} else {
  if (workspaceRoot === packagedSdkRoot) throw new Error("Pack from a zgui-js checkout");
  for (const filename of [
    "LICENSE",
    "Cargo.lock",
    "bun.lock",
    "package.json",
    "preview/profiles/default.json",
    "preview/fixtures/empty.json",
    "preview/scenarios/counter-capture.json",
  ]) {
    const target = path.join(packagedSdkRoot, filename);
    await mkdir(path.dirname(target), { recursive: true });
    await copyFile(path.join(workspaceRoot, filename), target);
  }
  await cp(path.join(workspaceRoot, "preview/fonts"), path.join(packagedSdkRoot, "preview/fonts"), {
    recursive: true,
  });
}
