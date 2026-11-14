import { spawnSync } from "node:child_process";
import { mkdir, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const workspaceRoot = fileURLToPath(new URL("../../../", import.meta.url));
const nativeHost = path.join(workspaceRoot, "target/debug/native-host");
const maxSummaryBytes = 8 * 1024;
const exitCodes = new Set([0, 2, 3, 4, 5, 6, 7, 8, 9, 130]);
const usage = "Usage: node packages/build/src/preview-check.mjs --out NEW_ROOT [--baseline DIR] [--policy FILE]";

function failure(code, exitCode, message) {
  return Object.assign(new Error(message), { code, exitCode });
}

function options(args) {
  const values = new Map();
  for (let index = 0; index < args.length; index += 2) {
    const flag = args[index], value = args[index + 1];
    if (!["--out", "--baseline", "--policy"].includes(flag) || values.has(flag)
      || !value || value.startsWith("--") || value.includes("\0")) {
      throw failure("E_SCHEMA", 2, usage);
    }
    values.set(flag, value);
  }
  if (!values.has("--out")) throw failure("E_SCHEMA", 2, usage);
  return {
    out: path.resolve(values.get("--out")),
    baseline: path.resolve(values.get("--baseline") ?? path.join(workspaceRoot, "preview/baselines/counter")),
    policy: path.resolve(values.get("--policy") ?? path.join(workspaceRoot, "preview/comparison/strict.json")),
  };
}

function execute(command, args) {
  // Native stdout is a bounded protocol, never streamed alongside the gate result.
  // Native stderr remains available live, including graphics setup diagnostics.
  const child = spawnSync(nativeHost, ["preview", command, ...args], {
    cwd: workspaceRoot, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"],
    maxBuffer: maxSummaryBytes,
  });
  if (child.error) {
    throw failure("E_WORKER_CRASH", 9, `Cannot complete native preview ${command}: ${child.error.message}`);
  }
  if (child.signal) {
    throw failure(child.signal === "SIGINT" ? "E_CANCELED" : "E_WORKER_CRASH",
      child.signal === "SIGINT" ? 130 : 9, `Native preview ${command} terminated by ${child.signal}`);
  }
  let result;
  try {
    result = JSON.parse(child.stdout);
  } catch {
    throw failure("E_HOST_INVARIANT", 9, `Native preview ${command} did not return one JSON result`);
  }
  if (Buffer.byteLength(child.stdout) > maxSummaryBytes || result?.schemaVersion !== 1
    || result.command !== command || !exitCodes.has(child.status) || result.exitCode !== child.status
    || result.status !== (child.status === 0 ? "passed" : "failed")
    || (child.status !== 0 && typeof result.firstError?.code !== "string")) {
    throw failure("E_HOST_INVARIANT", 9, `Native preview ${command} returned an inconsistent result`);
  }
  return { result, stdout: child.stdout };
}

function firstError(error) {
  if (!error) return null;
  return {
    code: String(error.code).slice(0, 64),
    message: String(error.message ?? error.code).slice(0, 256),
    ...(typeof error.stage === "string" ? { stage: error.stage.slice(0, 64) } : {}),
  };
}

const result = {
  schemaVersion: 1, command: "preview-check", status: "failed", exitCode: 9,
  stage: "setup", run: null, comparison: null, firstError: null,
};
let root;
try {
  const selected = options(process.argv.slice(2));
  try {
    await mkdir(path.dirname(selected.out), { recursive: true });
    // A new root is an exclusive claim. Never reuse even an empty directory or symlink.
    await mkdir(selected.out);
  } catch (error) {
    throw failure("E_OUTPUT_IO", 8, `Cannot claim new preview output root: ${error.message}`);
  }
  root = selected.out;
  result.stage = "run";
  const run = execute("run", [
    "--bundle", path.join(workspaceRoot, "apps/examples/counter/dist"),
    "--profile", path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario", path.join(workspaceRoot, "preview/scenarios/counter-virtual.json"),
    "--out", path.join(root, "actual"),
  ]);
  result.run = { status: run.result.status, exitCode: run.result.exitCode, result: "actual/result.json", stdout: "run.stdout.json" };
  result.exitCode = run.result.exitCode;
  result.firstError = firstError(run.result.firstError);
  await writeFile(path.join(root, "run.stdout.json"), run.stdout, { flag: "wx" });
  if (run.result.exitCode === 0) {
    result.stage = "compare";
    const comparison = execute("compare", [
      "--baseline", selected.baseline, "--actual", path.join(root, "actual"),
      "--policy", selected.policy, "--out", path.join(root, "comparison"),
    ]);
    result.comparison = { status: comparison.result.status, exitCode: comparison.result.exitCode,
      result: "comparison/result.json", stdout: "comparison.stdout.json" };
    result.exitCode = comparison.result.exitCode;
    result.firstError = firstError(comparison.result.firstError);
    await writeFile(path.join(root, "comparison.stdout.json"), comparison.stdout, { flag: "wx" });
  }
  result.status = result.exitCode === 0 ? "passed" : "failed";
} catch (error) {
  if (result.firstError) result.cause = result.firstError;
  result.status = "failed";
  result.exitCode = error.exitCode ?? 8;
  result.firstError = firstError(error.code && error.exitCode ? error : failure("E_OUTPUT_IO", 8, error.message));
}

function serialize() {
  const bytes = `${JSON.stringify(result)}\n`;
  if (Buffer.byteLength(bytes) > maxSummaryBytes) throw new Error("Preview gate summary exceeds 8 KiB");
  return bytes;
}

if (root) {
  try {
    await writeFile(path.join(root, ".result.tmp"), serialize(), { flag: "wx" });
    await rename(path.join(root, ".result.tmp"), path.join(root, "result.json"));
  } catch (error) {
    const cause = result.firstError;
    result.status = "failed";
    result.exitCode = 8;
    result.firstError = firstError(failure("E_OUTPUT_IO", 8, `Cannot publish preview gate result: ${error.message}`));
    if (cause) result.cause = cause;
  }
}
process.stdout.write(serialize());
process.exitCode = result.exitCode;
