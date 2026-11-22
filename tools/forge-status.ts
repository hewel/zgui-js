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

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

function run(command: string, args: string[], cwd = repoRoot) {
  const proc = spawnSync(command, args, { cwd, encoding: "utf8", timeout: 10000 });
  return {
    ok: !proc.error && proc.status === 0,
    stdout: (proc.stdout ?? "").trimEnd(),
    stderr: (proc.stderr ?? "").trim(),
  };
}

function sha256File(filePath: string): string | null {
  if (!existsSync(filePath)) return null;
  return createHash("sha256").update(readFileSync(filePath)).digest("hex").slice(0, 12);
}

interface CheckoutSnapshot {
  branch: string | null;
  head: string | null;
  dirtyCount: number;
  dirtyPaths: string[];
  bunLockSha12: string | null;
  cargoLockSha12: string | null;
  debugHostPresent: boolean;
}

export function collectLocalSnapshot(root = repoRoot) {
  const head = run("git", ["rev-parse", "--short", "HEAD"], root);
  const branch = run("git", ["rev-parse", "--abbrev-ref", "HEAD"], root);
  const status = run("git", ["status", "--porcelain"], root);
  const dirtyPaths = status.ok && status.stdout ? status.stdout.split("\n") : [];
  const nativeHostPath = path.join(root, "target/debug/zgui-host");
  return {
    root,
    branch: branch.ok ? branch.stdout.trim() : null,
    head: head.ok ? head.stdout.trim() : null,
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
    cd ${shellQuote(remoteDir)}
    echo "BRANCH=$(git rev-parse --abbrev-ref HEAD)"
    echo "HEAD=$(git rev-parse --short HEAD)"
    echo "BUN_LOCK=$(sha256sum bun.lock 2>/dev/null | cut -c1-12 || true)"
    echo "CARGO_LOCK=$(sha256sum Cargo.lock 2>/dev/null | cut -c1-12 || true)"
    echo "DEBUG_HOST=$(test -x target/debug/zgui-host && echo 1 || echo 0)"
    echo "DIRTY_START"
    git status --porcelain
  `;
  const res = run("ssh", ["-o", "BatchMode=yes", "-o", "ConnectTimeout=5", host, probe]);
  if (!res.ok) {
    return { reachable: false as const, host, remoteDir, error: res.stderr || "ssh probe failed" };
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
      if (line.length > 0) dirtyPaths.push(line);
      continue;
    }
    const idx = line.indexOf("=");
    if (idx > 0) fields.set(line.slice(0, idx), line.slice(idx + 1));
  }
  return {
    reachable: true as const,
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

function formatSnapshot(label: string, snap: CheckoutSnapshot): string {
  const summary = `${label}: ${snap.branch}@${snap.head} dirty=${snap.dirtyCount} bun.lock=${snap.bunLockSha12} Cargo.lock=${snap.cargoLockSha12} host=${snap.debugHostPresent ? "present" : "missing"}`;
  if (snap.dirtyPaths.length === 0) return summary;
  return `${summary}\n${snap.dirtyPaths.map(entry => `  ${entry}`).join("\n")}`;
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
      local.dirtyCount === 0 &&
      remote.dirtyCount === 0 &&
      local.bunLockSha12 === remote.bunLockSha12 &&
      local.cargoLockSha12 === remote.cargoLockSha12,
    ),
  };
  if (asJson) {
    console.log(JSON.stringify(report, null, 2));
  } else {
    console.log(formatSnapshot("Local ", local));
    if (remote) {
      if (!remote.reachable) {
        console.log(`Remote: ${remote.host}:${remote.remoteDir} unreachable (${remote.error})`);
      } else {
        console.log(formatSnapshot("Remote", remote));
      }
    }
  }
}
