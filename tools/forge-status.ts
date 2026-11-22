#!/usr/bin/env bun
/**
 * Read-only local vs remote forge checkout comparison.
 * Never adds remotes, never runs `git fetch`, and never mutates either checkout.
 */
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = fileURLToPath(new URL("../", import.meta.url));
const defaultHost = process.env.FORGE_HOST ?? "forge";
const defaultRemoteDir = process.env.FORGE_DIR ?? "/home/hewel/Codes/zgui-js";

function run(command: string, args: string[], cwd = repoRoot) {
  const proc = spawnSync(command, args, { cwd, encoding: "utf8", timeout: 10000 });
  return {
    ok: !proc.error && proc.status === 0,
    stdout: (proc.stdout ?? "").trim(),
    stderr: (proc.stderr ?? "").trim(),
  };
}

function sha256File(filePath: string): string | null {
  if (!existsSync(filePath)) return null;
  return createHash("sha256").update(readFileSync(filePath)).digest("hex").slice(0, 12);
}

export function collectLocalSnapshot(root = repoRoot) {
  const head = run("git", ["rev-parse", "--short", "HEAD"], root);
  const branch = run("git", ["rev-parse", "--abbrev-ref", "HEAD"], root);
  const status = run("git", ["status", "--porcelain"], root);
  const dirtyPaths = status.ok && status.stdout ? status.stdout.split("\n") : [];
  const nativeHostPath = path.join(root, "target/debug/native-host");
  return {
    root,
    branch: branch.ok ? branch.stdout : null,
    head: head.ok ? head.stdout : null,
    dirtyCount: dirtyPaths.length,
    dirtyPaths,
    bunLockSha12: sha256File(path.join(root, "bun.lock")),
    cargoLockSha12: sha256File(path.join(root, "Cargo.lock")),
    debugHostPresent: existsSync(nativeHostPath),
  };
}

export function collectRemoteSnapshot(host = defaultHost, remoteDir = defaultRemoteDir) {
  const probe = `
    set -eu
    cd ${JSON.stringify(remoteDir)}
    echo "BRANCH=$(git rev-parse --abbrev-ref HEAD)"
    echo "HEAD=$(git rev-parse --short HEAD)"
    echo "BUN_LOCK=$(sha256sum bun.lock 2>/dev/null | cut -c1-12 || true)"
    echo "CARGO_LOCK=$(sha256sum Cargo.lock 2>/dev/null | cut -c1-12 || true)"
    echo "DEBUG_HOST=$(test -x target/debug/native-host && echo 1 || echo 0)"
    echo "DIRTY_START"
    git status --porcelain
  `;
  const res = run("ssh", ["-o", "BatchMode=yes", "-o", "ConnectTimeout=5", host, probe]);
  if (!res.ok) {
    return { reachable: false, host, remoteDir, error: res.stderr || "ssh probe failed" };
  }
  const lines = res.stdout.split("\n");
  const fields = new Map<string, string>();
  const dirtyPaths: string[] = [];
  let inDirty = false;
  for (const line of lines) {
    if (line === "DIRTY_START") {
      inDirty = true;
      continue;
    }
    if (inDirty) {
      if (line.trim()) dirtyPaths.push(line);
      continue;
    }
    const idx = line.indexOf("=");
    if (idx > 0) fields.set(line.slice(0, idx), line.slice(idx + 1));
  }
  return {
    reachable: true,
    host,
    remoteDir,
    branch: fields.get("BRANCH") ?? null,
    head: fields.get("HEAD") ?? null,
    dirtyCount: dirtyPaths.length,
    dirtyPaths,
    bunLockSha12: fields.get("BUN_LOCK") || null,
    cargoLockSha12: fields.get("CARGO_LOCK") || null,
    debugHostPresent: fields.get("DEBUG_HOST") === "1",
  };
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const asJson = args.includes("--json");
  const localOnly = args.includes("--local-only");
  const local = collectLocalSnapshot();
  const remote = localOnly ? null : collectRemoteSnapshot();
  const report = {
    schemaVersion: 1,
    readOnly: true,
    local,
    remote,
    inSync: Boolean(
      remote &&
      remote.reachable &&
      local.head === remote.head &&
      local.bunLockSha12 === remote.bunLockSha12 &&
      local.cargoLockSha12 === remote.cargoLockSha12,
    ),
  };
  if (asJson) {
    console.log(JSON.stringify(report, null, 2));
  } else {
    console.log(
      `Local : ${local.branch}@${local.head} dirty=${local.dirtyCount} bun.lock=${local.bunLockSha12} Cargo.lock=${local.cargoLockSha12} host=${local.debugHostPresent ? "present" : "missing"}`,
    );
    if (remote) {
      if (!remote.reachable) {
        console.log(`Remote: ${remote.host}:${remote.remoteDir} unreachable (${remote.error})`);
      } else {
        console.log(
          `Remote: ${remote.branch}@${remote.head} dirty=${remote.dirtyCount} bun.lock=${remote.bunLockSha12} Cargo.lock=${remote.cargoLockSha12} host=${remote.debugHostPresent ? "present" : "missing"}`,
        );
      }
    }
  }
}
