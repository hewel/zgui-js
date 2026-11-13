import ts from "typescript";
import { readdir, access } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

export function typecheckProject(configPath) {
  const loaded = ts.readConfigFile(configPath, ts.sys.readFile);
  const parsed = loaded.error ? { errors: [loaded.error] } : ts.parseJsonConfigFileContent(loaded.config, ts.sys, path.dirname(configPath), { noEmit: true }, configPath);
  let diagnostics = parsed.errors;
  if (!diagnostics.length) {
    const program = ts.createProgram({ rootNames: parsed.fileNames, options: parsed.options, projectReferences: parsed.projectReferences });
    diagnostics = ts.getPreEmitDiagnostics(program);
  }
  if (diagnostics.length) {
    throw new Error(ts.formatDiagnosticsWithColorAndContext(diagnostics, {
      getCanonicalFileName: filename => filename,
      getCurrentDirectory: ts.sys.getCurrentDirectory,
      getNewLine: () => "\n",
    }));
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
