import { copyFile, cp, mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { packagedSdkRoot, workspaceRoot } from "./src/sdk.ts";

const packageDir = path.dirname(fileURLToPath(import.meta.url));
const LICENSE_FILES = ["LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md"] as const;

if (process.argv[2] === "--clean") {
  await rm(packagedSdkRoot, { recursive: true, force: true });
  for (const filename of LICENSE_FILES) {
    await rm(path.join(packageDir, filename), { force: true });
  }
} else {
  if (workspaceRoot === packagedSdkRoot) throw new Error("Pack from a zgui-js checkout");
  await rm(packagedSdkRoot, { recursive: true, force: true });
  for (const filename of LICENSE_FILES) {
    await copyFile(path.join(workspaceRoot, filename), path.join(packageDir, filename));
  }
  for (const filename of [
    ...LICENSE_FILES,
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

  const smokeDir = path.join(packagedSdkRoot, "smoke");
  await mkdir(smokeDir, { recursive: true });
  await copyFile(
    path.join(workspaceRoot, "tests/fixtures/solid-smoke.ts"),
    path.join(smokeDir, "solid-smoke.ts"),
  );
  const smokeTsconfig = {
    compilerOptions: {
      target: "ES2022",
      lib: ["ES2022", "DOM"],
      module: "ESNext",
      moduleResolution: "Bundler",
      customConditions: ["browser", "production"],
      strict: true,
      noEmit: true,
      types: [],
    },
    include: ["./solid-smoke.ts"],
  };
  await writeFile(
    path.join(smokeDir, "tsconfig.smoke.json"),
    `${JSON.stringify(smokeTsconfig, null, 2)}\n`,
    "utf8",
  );
}
