import { execFileSync } from "node:child_process";
import { readdir, access } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const compilerCli = path.join(path.dirname(createRequire(import.meta.url).resolve("typescript/package.json")), "bin/tsc");

export function typecheckProject(configPath) {
  try {
    execFileSync(process.execPath, [compilerCli, "--project", path.resolve(configPath), "--noEmit", "--pretty", "false"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const diagnostics = `${error.stdout ?? ""}${error.stderr ?? ""}`.trim();
    throw new Error(diagnostics || error.message, { cause: error });
  }
}

export async function typecheckWorkspace(root = fileURLToPath(new URL("../../../", import.meta.url))) {
  const configs = [
    path.join(root, "packages/ui/tsconfig.native.json"),
    path.join(root, "packages/services/tsconfig.json"),
    path.join(root, "tests/types/tsconfig.json"),
    path.join(root, "tests/types/tsconfig.services.json"),
  ];
  const examples = path.join(root, "apps/examples");
  let entries = [];
  try { entries = await readdir(examples, { withFileTypes: true }); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    if (!entry.isDirectory()) continue;
    const config = path.join(examples, entry.name, "tsconfig.json");
    try { await access(config); configs.push(config); }
    catch (error) { if (error.code !== "ENOENT") throw error; }
  }
  for (const config of configs) {
    typecheckProject(config);
    console.log(`Typechecked ${path.relative(root, config)}`);
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { await typecheckWorkspace(); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
