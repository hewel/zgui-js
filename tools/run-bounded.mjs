import { spawn } from "node:child_process";
import { once } from "node:events";
import { constants } from "node:os";
import { setTimeout as delay } from "node:timers/promises";

const DEFAULT_BUILD_TIMEOUT_MS = 1_200_000;
const DEFAULT_TEST_TIMEOUT_MS = 300_000;
const TERMINATION_GRACE_MS = 500;
const TIMEOUT_STATUS = 124;

function signalGroup(pid, signal) {
  try {
    process.kill(-pid, signal);
    return true;
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
    return false;
  }
}

function resolveTimeoutMs(phase) {
  if (process.env.ZGUI_CARGO_TIMEOUT_MS !== undefined) {
    return {
      envName: "ZGUI_CARGO_TIMEOUT_MS",
      timeoutMs: Number(process.env.ZGUI_CARGO_TIMEOUT_MS),
    };
  }
  if (phase === "build") {
    return {
      envName: "ZGUI_CARGO_BUILD_TIMEOUT_MS",
      timeoutMs: Number(process.env.ZGUI_CARGO_BUILD_TIMEOUT_MS ?? DEFAULT_BUILD_TIMEOUT_MS),
    };
  }
  return { envName: "ZGUI_CARGO_TIMEOUT_MS", timeoutMs: DEFAULT_TEST_TIMEOUT_MS };
}

export async function runBounded(command, args, { phase = "test" } = {}) {
  const label = [command, ...args].join(" ");
  const { envName, timeoutMs } = resolveTimeoutMs(phase);
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 2_147_483_647) {
    console.error(`${label}: ${envName} must be a positive integer no greater than 2147483647`);
    return 1;
  }

  const started = performance.now();
  const child = spawn(command, args, { detached: true, stdio: "inherit" });
  const pid = child.pid;
  const stopped = Promise.withResolvers();
  let stopping = false;

  async function terminate(signal, status) {
    const elapsed = Math.round(performance.now() - started);
    const signaled = pid !== undefined && signalGroup(pid, signal);
    if (signaled) {
      await delay(TERMINATION_GRACE_MS);
      // The leader can exit before its descendants. Always escalate against the group.
      signalGroup(pid, "SIGKILL");
    }
    if (status === TIMEOUT_STATUS) {
      const outcome = signaled
        ? `killed process group ${pid}`
        : `process group ${pid} already exited`;
      console.error(`${label}: timed out after ${elapsed} ms (limit ${timeoutMs} ms); ${outcome}`);
    }
    return status;
  }

  function stop(signal, status) {
    if (stopping) return;
    stopping = true;
    stopped.resolve(terminate(signal, status));
  }

  const onInterrupt = () => stop("SIGINT", 130);
  const onTerminate = () => stop("SIGTERM", 143);
  const timeout = setTimeout(() => stop("SIGTERM", TIMEOUT_STATUS), timeoutMs);
  process.on("SIGINT", onInterrupt);
  process.on("SIGTERM", onTerminate);
  try {
    const exited = once(child, "exit").then(([code, signal]) => {
      if (stopping) return stopped.promise;
      return code ?? 128 + constants.signals[signal];
    });
    return await Promise.race([exited, stopped.promise]);
  } catch (error) {
    console.error(`${label}: ${error.message}`);
    return 1;
  } finally {
    clearTimeout(timeout);
    process.off("SIGINT", onInterrupt);
    process.off("SIGTERM", onTerminate);
    child.unref();
  }
}

if (import.meta.main) {
  let phase = "test";
  let rest = process.argv.slice(2);
  if (rest[0] === "--build" || rest[0] === "--test") {
    phase = rest[0].slice(2);
    rest = rest.slice(1);
  }
  const [command, ...args] = rest;
  if (command === undefined) {
    console.error("Usage: bun tools/run-bounded.mjs [--build | --test] <command> [args ...]");
    process.exitCode = 1;
  } else {
    process.exitCode = await runBounded(command, args, { phase });
  }
}
