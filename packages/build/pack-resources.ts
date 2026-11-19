import { copyFile, cp, mkdir, mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
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
    "schemas/stylex-profile.json",
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

  // Publish only the inferred declarations for the explicitly typed entry point.
  const types = await mkdtemp(path.join(tmpdir(), "zgui-build-types-"));
  try {
    const compiler = path.join(
      path.dirname(createRequire(import.meta.url).resolve("tsrs/package.json")),
      "bin/tsrs",
    );
    const result = Bun.spawnSync(
      [
        process.execPath,
        compiler,
        "--project",
        path.join(import.meta.dir, "tsconfig.json"),
        "--rootDir",
        workspaceRoot,
        "--noEmit",
        "false",
        "--declaration",
        "--emitDeclarationOnly",
        "--outDir",
        types,
      ],
      { stdout: "inherit", stderr: "inherit" },
    );
    if (!result.success) throw new Error(`Declaration emit failed (${result.exitCode})`);
    await copyFile(
      path.join(types, "packages/build/src/index.d.ts"),
      path.join(packagedSdkRoot, "index.d.ts"),
    );
  } finally {
    await rm(types, { recursive: true, force: true });
  }
}
