import { execFileSync, spawn } from "node:child_process";
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

// Dev compilation must leave the event loop free for file changes and shutdown.
export function typecheckProjectAsync(configPath, { signal } = {}) {
  signal?.throwIfAborted();
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [compilerCli, "--project", path.resolve(configPath), "--noEmit", "--pretty", "false"], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "", stderr = "", failure, killTimer;
    child.stdout.setEncoding("utf8").on("data", text => { stdout += text; });
    child.stderr.setEncoding("utf8").on("data", text => { stderr += text; });
    const abort = () => {
      child.kill("SIGTERM");
      killTimer = setTimeout(() => child.kill("SIGKILL"), 1000);
      killTimer.unref();
    };
    signal?.addEventListener("abort", abort, { once: true });
    child.on("error", error => { failure = error; });
    child.on("close", (code, exitSignal) => {
      clearTimeout(killTimer);
      signal?.removeEventListener("abort", abort);
      if (signal?.aborted) reject(signal.reason);
      else if (failure || code !== 0) reject(new Error(`${stdout}${stderr}`.trim() || failure?.message || `TypeScript exited with ${exitSignal ?? code}`, { cause: failure }));
      else resolve();
    });
    if (signal?.aborted) abort();
  });
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
