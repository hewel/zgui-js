import { spawn } from "node:child_process";
import { constants } from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { workspaceRoot } from "./build.mjs";

function signalCode(signal) {
  return signal ? 128 + (constants.signals[signal] ?? constants.signals.SIGTERM) : 1;
}

function cancellationSignal(signal) {
  return signal?.reason === "SIGINT" ? "SIGINT" : "SIGTERM";
}

function failure(message, exitCode = 1) {
  return Object.assign(new Error(message), { exitCode });
}

// A separate POSIX process group also owns cargo's compiler/linker children and native workers.
function ownedProcess(executable, args, { root, signal, onLine }, { spawnProcess = spawn, killAfter = 1500 } = {}) {
  if (signal?.aborted) return Promise.resolve(signalCode(cancellationSignal(signal)));
  return new Promise((resolve, reject) => {
    let child;
    let timer;
    let spawnError;
    let lines;
    const grouped = process.platform !== "win32";
    const kill = name => {
      if (!child?.pid) return;
      try {
        if (grouped) process.kill(-child.pid, name);
        else child.kill(name);
      } catch (error) {
        if (error.code !== "ESRCH") spawnError ??= error;
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
      reject(failure(`Cannot start ${executable}: ${error.message}`));
      return;
    }
    child.once("error", error => { spawnError = error; });
    child.once("close", (code, terminatingSignal) => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      lines?.close();
      // A leader may exit before a worker. Never leave a member of our owned group behind.
      if (grouped) kill("SIGKILL");
      if (spawnError) reject(failure(`Cannot run ${executable}: ${spawnError.message}`));
      else resolve(signal?.aborted ? signalCode(cancellationSignal(signal)) : code ?? signalCode(terminatingSignal));
    });
    if (onLine) {
      lines = createInterface({ input: child.stdout });
      lines.on("line", onLine);
    }
    signal?.addEventListener("abort", abort, { once: true });
    // Abort can arrive while spawn hooks are running, before the listener was attached.
    if (signal?.aborted) abort();
  });
}

/** Build incrementally; reject with an exitCode on build/spawn/cancellation failure. */
export async function prepareNativeHost({ root = workspaceRoot, signal } = {}, processOptions) {
  root = path.resolve(root);
  let executable;
  const exitCode = await ownedProcess("cargo", ["build", "-p", "native-host", "--locked", "--message-format=json-render-diagnostics"], {
    root, signal,
    onLine(line) {
      let message;
      try { message = JSON.parse(line); }
      catch { process.stderr.write(`${line}\n`); return; }
      if (message?.reason === "compiler-message" && message.message?.rendered) process.stderr.write(message.message.rendered);
      if (message?.reason === "compiler-artifact" && message.target?.name === "native-host"
        && message.target.kind?.includes("bin") && typeof message.executable === "string") {
        executable = path.resolve(root, message.executable);
      }
    },
  }, processOptions);
  if (exitCode !== 0) throw failure(`native-host build exited with code ${exitCode}`, exitCode);
  if (!executable) throw failure("Cargo did not report a native-host executable");
  return executable;
}

/** Inherit native stdio; return its numeric status (signals use 128 + signal number). */
export async function runNativeHost(executable, args, { root = workspaceRoot, signal } = {}, processOptions) {
  return ownedProcess(executable, args, { root: path.resolve(root), signal }, processOptions);
}
