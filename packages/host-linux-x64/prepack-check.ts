import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { chmod, copyFile, mkdtemp, rm, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const pkgDir = path.dirname(fileURLToPath(import.meta.url));
const pkgJson = JSON.parse(readFileSync(path.join(pkgDir, "package.json"), "utf8")) as {
  name: string;
  version: string;
};

const requiredFiles = [
  "bin/zgui-host",
  "host-manifest.json",
  "index.js",
  "index.d.ts",
  "README.md",
  "LICENSE",
  "NOTICE",
  "THIRD_PARTY_NOTICES.md",
];

for (const rel of requiredFiles) {
  const full = path.join(pkgDir, rel);
  if (!existsSync(full)) {
    throw new Error(
      `Refusing to pack ${pkgJson.name}: missing required file ${rel}. Stage a compiled binary first via tools/stage-host-package.ts.`,
    );
  }
}

const manifest = JSON.parse(readFileSync(path.join(pkgDir, "host-manifest.json"), "utf8")) as {
  schemaVersion?: unknown;
  package?: unknown;
  platform?: unknown;
  version?: unknown;
  hostApiVersion?: unknown;
  bundleSchemaVersion?: unknown;
  sha256?: unknown;
  bytes?: unknown;
  libc?: unknown;
};

const binaryPath = path.join(pkgDir, "bin/zgui-host");
const info = await stat(binaryPath);
const digest = `sha256:${createHash("sha256").update(readFileSync(binaryPath)).digest("hex")}`;

if (
  manifest.schemaVersion !== 1 ||
  manifest.package !== pkgJson.name ||
  manifest.platform !== "linux-x64" ||
  manifest.version !== pkgJson.version ||
  manifest.hostApiVersion !== 1 ||
  manifest.bundleSchemaVersion !== 2 ||
  manifest.bytes !== info.size ||
  manifest.sha256 !== digest
) {
  throw new Error(
    `Refusing to pack ${pkgJson.name}: host-manifest.json does not match staged bin/zgui-host or package version ${pkgJson.version}.`,
  );
}

const tempDir = await mkdtemp(path.join(os.tmpdir(), "zgui-host-prepack-"));
try {
  const copied = path.join(tempDir, "zgui-host");
  await copyFile(binaryPath, copied);
  await chmod(copied, 0o755);
  const probe = Bun.spawnSync([copied, "--version", "--json"], {
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
    timeout: 5000,
  });
  if (!probe.success) {
    const detail = `${probe.stderr.toString()}${probe.stdout.toString()}`.trim();
    throw new Error(
      `Refusing to pack ${pkgJson.name}: copied binary failed --version --json probe (${probe.signalCode ?? probe.exitCode})${detail ? `: ${detail}` : ""}`,
    );
  }
  const reported = JSON.parse(probe.stdout.toString("utf8").trim()) as {
    name?: unknown;
    version?: unknown;
    hostApiVersion?: unknown;
    schemaVersion?: unknown;
    platform?: unknown;
  };
  if (
    reported.name !== "zgui-host" ||
    reported.version !== pkgJson.version ||
    reported.hostApiVersion !== 1 ||
    reported.schemaVersion !== 2 ||
    reported.platform !== "linux-x64"
  ) {
    throw new Error(
      `Refusing to pack ${pkgJson.name}: copied binary ABI/version mismatch: ${JSON.stringify(reported)}`,
    );
  }
} finally {
  await rm(tempDir, { recursive: true, force: true });
}
