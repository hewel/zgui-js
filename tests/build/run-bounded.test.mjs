import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const runner = fileURLToPath(new URL("../../tools/run-bounded.mjs", import.meta.url));

function isRunning(pid) {
  const statPath = `/proc/${pid}/stat`;
  try {
    const stat = readFileSync(statPath, "utf8");
    return !["Z", "X"].includes(stat[stat.lastIndexOf(")") + 2]);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    return false;
  }
}

function killIfRunning(pid) {
  if (!isRunning(pid)) return;
  try {
    process.kill(pid, "SIGKILL");
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
}

test(
  "Cargo timeout kills a TERM-resistant grandchild after its leader exits",
  { skip: !existsSync("/proc/self/stat") },
  async () => {
    const fixture = `
    trap 'echo fixture-received-TERM; exit 0' TERM
    printf 'child=%s\\n' "$$"
    sh -c 'trap "" TERM; printf "grandchild=%s\\n" "$$"; exec sleep 60' &
    wait
  `;
    const started = performance.now();
    const result = spawnSync(
      process.execPath,
      [runner, "sh", "-c", fixture, "stalled-cargo-fixture"],
      {
        env: { ...process.env, ZGUI_CARGO_TIMEOUT_MS: "1000" },
        encoding: "utf8",
        timeout: 8_000,
        killSignal: "SIGKILL",
      },
    );
    const output = `${result.stdout}\n${result.stderr}`;
    const child = Number(result.stdout?.match(/^child=(\d+)$/m)?.[1]);
    const grandchild = Number(result.stdout?.match(/^grandchild=(\d+)$/m)?.[1]);
    const pids = [child, grandchild].filter(Number.isSafeInteger);
    try {
      assert.ifError(result.error);
      assert.equal(result.status, 124, output);
      assert.ok(performance.now() - started < 5_000, output);
      assert.ok(child > 0 && grandchild > 0 && child !== grandchild, output);
      assert.match(output, /^fixture-received-TERM$/m);
      assert.match(
        result.stderr,
        /stalled-cargo-fixture.*timed out after \d+ ms.*killed process group \d+/s,
      );

      // SIGKILL is asynchronous; an unreaped zombie is no longer running.
      const deadline = performance.now() + 1_000;
      while (pids.some(isRunning) && performance.now() < deadline) await delay(10);
      assert.deepEqual(
        pids.filter(isRunning),
        [],
        `surviving fixture processes: ${pids.join(", ")}`,
      );
    } finally {
      pids.forEach(killIfRunning);
    }
  },
);
