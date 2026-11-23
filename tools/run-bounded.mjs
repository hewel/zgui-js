import { spawn } from "node:child_process";
import { once } from "node:events";
import { constants } from "node:os";
import { setTimeout as delay } from "node:timers/promises";

const DEFAULT_TIMEOUT_MS = 600_000;
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

export async function runBounded(command, args) {
  const label = [command, ...args].join(" ");
  const timeoutMs = Number(process.env.ZGUI_CARGO_TIMEOUT_MS ?? DEFAULT_TIMEOUT_MS);
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 2_147_483_647) {
    console.error(
      `${label}: ZGUI_CARGO_TIMEOUT_MS must be a positive integer no greater than 2147483647`,
    );
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
  const [command, ...args] = process.argv.slice(2);
  if (command === undefined) {
    console.error("Usage: bun tools/run-bounded.mjs <command> [args ...]");
    process.exitCode = 1;
  } else {
    process.exitCode = await runBounded(command, args);
  }
}
