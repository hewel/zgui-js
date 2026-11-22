import { mkdir, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const workspaceRoot = fileURLToPath(new URL("../../../", import.meta.url));
const nativeHost = path.join(workspaceRoot, "target/debug/native-host");
const maxSummaryBytes = 8 * 1024;
const exitCodes = new Set([0, 2, 3, 4, 5, 6, 7, 8, 9, 130]);
const usage =
  "Usage: bun packages/build/src/preview-check.ts --out NEW_ROOT [--baseline DIR] [--policy FILE]";

export interface PreviewError {
  code: string;
  message: string;
  stage?: string;
  details?: Record<string, unknown>;
}

export type PreviewStage = "setup" | "run" | "compare";

interface PreviewStageResult {
  status: "passed" | "failed";
  exitCode: number;
  result: string;
  stdout: string;
}

export interface PreviewCheckResult {
  schemaVersion: 1;
  command: "preview-check";
  status: "passed" | "failed";
  exitCode: number;
  stage: PreviewStage;
  run: PreviewStageResult | null;
  comparison: PreviewStageResult | null;
  firstError: PreviewError | null;
  cause?: PreviewError;
}

function properties(value: unknown): Record<string, unknown> {
  return value !== null && (typeof value === "object" || typeof value === "function")
    ? (value as Record<string, unknown>)
    : {};
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function failure(code: string, exitCode: number, message: string) {
  return Object.assign(new Error(message), { code, exitCode });
}

function options(args: string[]) {
  const values = new Map<string, string>();
  for (let index = 0; index < args.length; index += 2) {
    const flag = args[index],
      value = args[index + 1];
    if (
      !["--out", "--baseline", "--policy"].includes(flag) ||
      values.has(flag) ||
      !value ||
      value.startsWith("--") ||
      value.includes("\0")
    ) {
      throw failure("E_SCHEMA", 2, usage);
    }
    values.set(flag, value);
  }
  const out = values.get("--out");
  if (out === undefined) throw failure("E_SCHEMA", 2, usage);
  return {
    out: path.resolve(out),
    baseline: path.resolve(
      values.get("--baseline") ?? path.join(workspaceRoot, "preview/baselines/counter"),
    ),
    policy: path.resolve(
      values.get("--policy") ?? path.join(workspaceRoot, "preview/comparison/strict.json"),
    ),
  };
}

function execute(
  command: "run" | "compare",
  args: string[],
): {
  result: Pick<PreviewStageResult, "status" | "exitCode"> & { firstError: unknown };
  stdout: string;
} {
  // Native stdout is a bounded protocol, never streamed alongside the gate result.
  // Native stderr remains available live, including graphics setup diagnostics.
  let child: Bun.SyncSubprocess<"pipe", "inherit">;
  try {
    child = Bun.spawnSync([nativeHost, "preview", command, ...args], {
      cwd: workspaceRoot,
      stdin: "ignore",
      stdout: "pipe",
      stderr: "inherit",
    });
  } catch (error) {
    throw failure(
      "E_WORKER_CRASH",
      9,
      `Cannot complete native preview ${command}: ${message(error)}`,
    );
  }
  if (child.signalCode) {
    throw failure(
      child.signalCode === "SIGINT" ? "E_CANCELED" : "E_WORKER_CRASH",
      child.signalCode === "SIGINT" ? 130 : 9,
      `Native preview ${command} terminated by ${child.signalCode}`,
    );
  }
  if (child.stdout.byteLength > maxSummaryBytes) {
    throw failure(
      "E_HOST_INVARIANT",
      9,
      `Native preview ${command} stdout exceeded ${maxSummaryBytes} bytes`,
    );
  }
  const stdout = child.stdout.toString("utf8");
  let parsed: unknown;
  try {
    parsed = JSON.parse(stdout);
  } catch {
    throw failure(
      "E_HOST_INVARIANT",
      9,
      `Native preview ${command} did not return one JSON result`,
    );
  }
  const result = properties(parsed);
  const status = child.exitCode === 0 ? "passed" : "failed";
  if (
    result.schemaVersion !== 1 ||
    result.command !== command ||
    !exitCodes.has(child.exitCode) ||
    result.exitCode !== child.exitCode ||
    result.status !== status ||
    (child.exitCode !== 0 && typeof properties(result.firstError).code !== "string")
  ) {
    throw failure(
      "E_HOST_INVARIANT",
      9,
      `Native preview ${command} returned an inconsistent result`,
    );
  }
  return {
    result: { status, exitCode: child.exitCode, firstError: result.firstError },
    stdout,
  };
}

function firstError(error: unknown): PreviewError | null {
  if (!error) return null;
  const data = properties(error);
  const details =
    data.details && typeof data.details === "object" && !Array.isArray(data.details)
      ? (data.details as Record<string, unknown>)
      : undefined;
  return {
    code: String(data.code).slice(0, 64),
    message: String(data.message ?? data.code).slice(0, 512),
    ...(typeof data.stage === "string" ? { stage: data.stage.slice(0, 64) } : {}),
    ...(details ? { details } : {}),
  };
}

const result: PreviewCheckResult = {
  schemaVersion: 1,
  command: "preview-check",
  status: "failed",
  exitCode: 9,
  stage: "setup",
  run: null,
  comparison: null,
  firstError: null,
};
let root: string | undefined;
try {
  const selected = options(process.argv.slice(2));
  try {
    await mkdir(path.dirname(selected.out), { recursive: true });
    // A new root is an exclusive claim. Never reuse even an empty directory or symlink.
    await mkdir(selected.out);
  } catch (error) {
    throw failure("E_OUTPUT_IO", 8, `Cannot claim new preview output root: ${message(error)}`);
  }
  root = selected.out;
  result.stage = "run";
  const run = execute("run", [
    "--bundle",
    path.join(workspaceRoot, "apps/examples/counter/dist"),
    "--profile",
    path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture",
    path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario",
    path.join(workspaceRoot, "preview/scenarios/counter-virtual.json"),
    "--out",
    path.join(root, "actual"),
  ]);
  result.run = {
    status: run.result.status,
    exitCode: run.result.exitCode,
    result: "actual/result.json",
    stdout: "run.stdout.json",
  };
  result.exitCode = run.result.exitCode;
  result.firstError = firstError(run.result.firstError);
  await writeFile(path.join(root, "run.stdout.json"), run.stdout, { flag: "wx" });
  if (run.result.exitCode === 0) {
    result.stage = "compare";
    const comparison = execute("compare", [
      "--baseline",
      selected.baseline,
      "--actual",
      path.join(root, "actual"),
      "--policy",
      selected.policy,
      "--out",
      path.join(root, "comparison"),
    ]);
    result.comparison = {
      status: comparison.result.status,
      exitCode: comparison.result.exitCode,
      result: "comparison/result.json",
      stdout: "comparison.stdout.json",
    };
    result.exitCode = comparison.result.exitCode;
    result.firstError = firstError(comparison.result.firstError);
    await writeFile(path.join(root, "comparison.stdout.json"), comparison.stdout, { flag: "wx" });
  }
  result.status = result.exitCode === 0 ? "passed" : "failed";
} catch (error) {
  if (result.firstError) result.cause = result.firstError;
  result.status = "failed";
  const data = properties(error);
  result.exitCode = typeof data.exitCode === "number" ? data.exitCode : 8;
  result.firstError = firstError(
    data.code && data.exitCode ? error : failure("E_OUTPUT_IO", 8, message(error)),
  );
}

function serialize() {
  const bytes = `${JSON.stringify(result)}\n`;
  if (Buffer.byteLength(bytes) > maxSummaryBytes)
    throw new Error("Preview gate summary exceeds 8 KiB");
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
    result.firstError = firstError(
      failure("E_OUTPUT_IO", 8, `Cannot publish preview gate result: ${message(error)}`),
    );
    if (cause) result.cause = cause;
  }
}
process.stdout.write(serialize());
process.exitCode = result.exitCode;
