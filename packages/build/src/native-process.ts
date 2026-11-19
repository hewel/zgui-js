import { spawn, type ChildProcess, type SpawnOptions } from "node:child_process";
import { constants } from "node:os";
import path from "node:path";
import { createInterface, type Interface } from "node:readline";
import { workspaceRoot } from "./build.ts";

interface NativeHostOptions {
  root?: string;
  signal?: AbortSignal;
}

interface ProcessOptions {
  spawnProcess?: (
    executable: string,
    args: readonly string[],
    options: SpawnOptions,
  ) => ChildProcess;
  killAfter?: number;
}

interface CargoMessage {
  reason?: string;
  message?: { rendered?: string };
  target?: { name?: string; kind?: string[] };
  executable?: string;
}

function errorMessage(error: unknown): string {
  return error !== null && typeof error === "object" && "message" in error
    ? String(error.message)
    : String(error);
}

function signalCode(signal: NodeJS.Signals | null) {
  return signal ? 128 + (constants.signals[signal] ?? constants.signals.SIGTERM) : 1;
}

function cancellationSignal(signal?: AbortSignal) {
  return signal?.reason === "SIGINT" ? "SIGINT" : "SIGTERM";
}

function failure(message: string, exitCode = 1) {
  return Object.assign(new Error(message), { exitCode });
}

// A separate POSIX process group also owns cargo's compiler/linker children and native workers.
function ownedProcess(
  executable: string,
  args: readonly string[],
  {
    root,
    signal,
    onLine,
  }: {
    root: string;
    signal?: AbortSignal;
    onLine?: (line: string) => void;
  },
  { spawnProcess = spawn, killAfter = 1500 }: ProcessOptions = {},
): Promise<number> {
  if (signal?.aborted) return Promise.resolve(signalCode(cancellationSignal(signal)));
  const { promise, resolve, reject } = Promise.withResolvers<number>();
  try {
    let child: ChildProcess;
    let timer: NodeJS.Timeout | undefined;
    let spawnError: unknown;
    let lines: Interface | undefined;
    const grouped = process.platform !== "win32";
    const kill = (name: NodeJS.Signals) => {
      if (!child?.pid) return;
      try {
        if (grouped) process.kill(-child.pid, name);
        else child.kill(name);
      } catch (error) {
        if (
          !(
            error !== null &&
            typeof error === "object" &&
            "code" in error &&
            error.code === "ESRCH"
          )
        )
          spawnError ??= error;
      }
    };
    const abort = () => {
      kill(cancellationSignal(signal));
      timer ??= setTimeout(() => kill("SIGKILL"), killAfter);
    };
    try {
      child = spawnProcess(executable, args, {
        cwd: root,
        detached: grouped,
        stdio: ["inherit", onLine ? "pipe" : "inherit", "inherit"],
      });
    } catch (error) {
      reject(failure(`Cannot start ${executable}: ${errorMessage(error)}`));
      return promise;
    }
    child.once("error", error => {
      spawnError = error;
    });
    child.once("close", (code, terminatingSignal) => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      lines?.close();
      // A leader may exit before a worker. Never leave a member of our owned group behind.
      if (grouped) kill("SIGKILL");
      if (spawnError) reject(failure(`Cannot run ${executable}: ${errorMessage(spawnError)}`));
      else
        resolve(
          signal?.aborted
            ? signalCode(cancellationSignal(signal))
            : (code ?? signalCode(terminatingSignal)),
        );
    });
    if (onLine) {
      lines = createInterface({ input: child.stdout! });
      lines.on("line", onLine);
    }
    signal?.addEventListener("abort", abort, { once: true });
    // Abort can arrive while spawn hooks are running, before the listener was attached.
    if (signal?.aborted) abort();
  } catch (error) {
    reject(error);
  }
  return promise;
}

/** Build incrementally; reject with an exitCode on build/spawn/cancellation failure. */
export async function prepareNativeHost(
  { root = workspaceRoot, signal }: NativeHostOptions = {},
  processOptions?: ProcessOptions,
) {
  root = path.resolve(root);
  let executable: string | undefined;
  const exitCode = await ownedProcess(
    "cargo",
    ["build", "-p", "native-host", "--locked", "--message-format=json-render-diagnostics"],
    {
      root,
      signal,
      onLine(line) {
        let message: CargoMessage | null;
        try {
          message = JSON.parse(line) as CargoMessage | null;
        } catch {
          process.stderr.write(`${line}\n`);
          return;
        }
        if (message?.reason === "compiler-message" && message.message?.rendered)
          process.stderr.write(message.message.rendered);
        if (
          message?.reason === "compiler-artifact" &&
          message.target?.name === "native-host" &&
          message.target.kind?.includes("bin") &&
          typeof message.executable === "string"
        ) {
          executable = path.resolve(root, message.executable);
        }
      },
    },
    processOptions,
  );
  if (exitCode !== 0) throw failure(`native-host build exited with code ${exitCode}`, exitCode);
  if (!executable) throw failure("Cargo did not report a native-host executable");
  return executable;
}

/** Inherit native stdio; return its numeric status (signals use 128 + signal number). */
export async function runNativeHost(
  executable: string,
  args: readonly string[],
  { root = workspaceRoot, signal }: NativeHostOptions = {},
  processOptions?: ProcessOptions,
) {
  return ownedProcess(executable, args, { root: path.resolve(root), signal }, processOptions);
}
