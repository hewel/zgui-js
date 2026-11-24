#!/usr/bin/env bun
import { buildSdkPackages, stageHostPackage, verifyHostPackage } from "@hewel/zgui-build/internal";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { STAGED_LICENSE_FILES, cleanLicenseFiles, stageLicenseFiles } from "./pack-package.ts";

const repoRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));

const SEMVER_RE =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

const PUBLIC_PACKAGES = [
  { name: "@hewel/zgui", dir: "packages/ui" },
  { name: "@hewel/zgui-services", dir: "packages/services" },
  { name: "@hewel/zgui-build", dir: "packages/build" },
  { name: "@hewel/zgui-host-linux-x64", dir: "packages/host-linux-x64" },
  { name: "@hewel/zgui-cli", dir: "packages/cli" },
  { name: "create-zgui-js", dir: "packages/create-zgui-js" },
] as const;

const ALL_WORKSPACE_PACKAGES = [
  ...PUBLIC_PACKAGES,
  { name: "@hewel/zgui-host-linux-arm64", dir: "packages/host-linux-arm64" },
] as const;

const WORKSPACE_CRATES = ["zgui-host", "js-runtime", "ui-bridge", "host-services"] as const;

interface PackageJsonShape {
  name?: string;
  version?: string;
  description?: string;
  license?: string;
  private?: boolean;
  publishConfig?: { access?: string };
  repository?: { type?: string; url?: string; directory?: string };
  homepage?: string;
  bugs?: { url?: string };
  keywords?: string[];
  engines?: { bun?: string };
  files?: string[];
  bin?: Record<string, string>;
  os?: string[];
  cpu?: string[];
  libc?: string[];
  exports?: Record<string, unknown>;
  imports?: Record<string, unknown>;
  scripts?: Record<string, string>;
  dependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
}

export interface ReleaseManifestPackage {
  readonly name: string;
  readonly version: string;
  readonly directory: string;
  readonly tarball: string;
  readonly bytes: number;
  readonly sha256: string;
  readonly integrity: string;
}

export interface ReleaseManifest {
  readonly schemaVersion: 1;
  readonly version: string;
  readonly createdAt: string;
  readonly packages: readonly ReleaseManifestPackage[];
}

const usage = `Usage:
  bun tools/release.ts version --set <SEMVER>
  bun tools/release.ts check [--tag <vSEMVER>]
  bun tools/release.ts pack --out <DIR> --host-binary <PATH> [--glibc-baseline <STR>] [--skip-host-probe] [--keep-staged-host]
  bun tools/release.ts publish --manifest <PATH> [--tag <DIST_TAG>] [--registry <URL>] [--provenance | --no-provenance] [--dry-run]`;

function readJson<T>(filePath: string): T {
  return JSON.parse(readFileSync(filePath, "utf8")) as T;
}

function readCargoWorkspaceVersion(cargoTomlPath: string): string {
  const content = readFileSync(cargoTomlPath, "utf8");
  const section = content.match(/\[workspace\.package\][\s\S]*?(?=\n\[|$)/);
  const versionMatch = section?.[0].match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!versionMatch?.[1]) {
    throw new Error(`Could not locate [workspace.package].version in ${cargoTomlPath}`);
  }
  return versionMatch[1];
}

function readCargoLockWorkspaceVersions(cargoLockPath: string): Map<string, string> {
  const content = readFileSync(cargoLockPath, "utf8");
  const versions = new Map<string, string>();
  const blocks = content.split(/\[\[package\]\]\s*/);
  for (const block of blocks) {
    const nameMatch = block.match(/^name\s*=\s*"([^"]+)"/m);
    const versionMatch = block.match(/^version\s*=\s*"([^"]+)"/m);
    const hasSource = /^source\s*=/m.test(block);
    if (!hasSource && nameMatch?.[1] && versionMatch?.[1]) {
      if ((WORKSPACE_CRATES as readonly string[]).includes(nameMatch[1])) {
        versions.set(nameMatch[1], versionMatch[1]);
      }
    }
  }
  return versions;
}

function detectGlibcBaseline(): string | undefined {
  if (process.platform !== "linux") return undefined;
  const child = Bun.spawnSync(["ldd", "--version"], {
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
    timeout: 3000,
  });
  const text = `${child.stdout.toString("utf8")}\n${child.stderr.toString("utf8")}`;
  const match = text.match(/(?:GNU libc|GLIBC)[^0-9]*([0-9]+\.[0-9]+)/i);
  return match ? `glibc-${match[1]}` : undefined;
}

function computeDigests(bytes: Uint8Array): { sha256: string; integrity: string } {
  const sha256 = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
  const integrity = `sha512-${createHash("sha512").update(bytes).digest("base64")}`;
  return { sha256, integrity };
}

function runCommandSync(cmd: string[], cwd: string): { stdout: string; stderr: string } {
  const child = Bun.spawnSync(cmd, {
    cwd,
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
  });
  const stdout = child.stdout.toString("utf8");
  const stderr = child.stderr.toString("utf8");
  if (!child.success) {
    const detail = `${stderr}${stdout}`.trim();
    throw new Error(
      `Command failed (${cmd.join(" ")}) with exit ${child.signalCode ?? child.exitCode}${detail ? `:\n${detail}` : ""}`,
    );
  }
  return { stdout, stderr };
}

async function cmdVersion(newVersion: string): Promise<void> {
  if (!SEMVER_RE.test(newVersion)) {
    throw new Error(`Invalid SemVer version: ${newVersion}`);
  }

  const rootPkgPath = path.join(repoRoot, "package.json");
  const rootPkg = readJson<PackageJsonShape>(rootPkgPath);
  rootPkg.version = newVersion;
  await writeFile(rootPkgPath, `${JSON.stringify(rootPkg, null, 2)}\n`, "utf8");

  for (const pkg of ALL_WORKSPACE_PACKAGES) {
    const pkgPath = path.join(repoRoot, pkg.dir, "package.json");
    const manifest = readJson<PackageJsonShape>(pkgPath);
    manifest.version = newVersion;
    await writeFile(pkgPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  }

  const cargoTomlPath = path.join(repoRoot, "Cargo.toml");
  const cargoToml = await readFile(cargoTomlPath, "utf8");
  const updatedCargoToml = cargoToml.replace(
    /(\[workspace\.package\][\s\S]*?^\s*version\s*=\s*")([^"]+)(")/m,
    `$1${newVersion}$3`,
  );
  if (updatedCargoToml === cargoToml && readCargoWorkspaceVersion(cargoTomlPath) !== newVersion) {
    throw new Error("Failed to update [workspace.package].version in Cargo.toml");
  }
  await writeFile(cargoTomlPath, updatedCargoToml, "utf8");

  // Synchronize workspace crate versions in Cargo.lock so offline release checks stay consistent.
  const cargoLockPath = path.join(repoRoot, "Cargo.lock");
  if (existsSync(cargoLockPath)) {
    let cargoLock = await readFile(cargoLockPath, "utf8");
    for (const crateName of WORKSPACE_CRATES) {
      const pattern = new RegExp(
        `(\\[\\[package\\]\\]\\nname = "${crateName}"\\nversion = ")([^"]+)(")`,
        "m",
      );
      cargoLock = cargoLock.replace(pattern, `$1${newVersion}$3`);
    }
    await writeFile(cargoLockPath, cargoLock, "utf8");
  }

  console.log(
    `Synchronized root package.json, ${ALL_WORKSPACE_PACKAGES.length} workspace packages, Cargo.toml, and Cargo.lock to ${newVersion}`,
  );
}

function cmdCheck({ tag }: { tag?: string } = {}): { version: string } {
  const errors: string[] = [];
  const rootPkg = readJson<PackageJsonShape>(path.join(repoRoot, "package.json"));
  const expectedVersion = rootPkg.version;
  if (!expectedVersion || !SEMVER_RE.test(expectedVersion)) {
    errors.push(`Root package.json has invalid version: ${String(expectedVersion)}`);
  }

  if (tag !== undefined) {
    const expectedTag = `v${expectedVersion}`;
    if (tag !== expectedTag) {
      errors.push(`Release tag mismatch: expected ${expectedTag}, got ${tag}`);
    }
  }

  for (const notice of STAGED_LICENSE_FILES) {
    if (!existsSync(path.join(repoRoot, notice))) {
      errors.push(`Missing root license/notice file: ${notice}`);
    }
  }

  const cargoVersion = readCargoWorkspaceVersion(path.join(repoRoot, "Cargo.toml"));
  if (cargoVersion !== expectedVersion) {
    errors.push(
      `Cargo.toml [workspace.package].version (${cargoVersion}) !== root package.json version (${expectedVersion})`,
    );
  }

  const cargoLockPath = path.join(repoRoot, "Cargo.lock");
  if (!existsSync(cargoLockPath)) {
    errors.push("Missing Cargo.lock");
  } else {
    const lockVersions = readCargoLockWorkspaceVersions(cargoLockPath);
    for (const crateName of WORKSPACE_CRATES) {
      const lockVersion = lockVersions.get(crateName);
      if (lockVersion !== expectedVersion) {
        errors.push(
          `Cargo.lock crate ${crateName} version (${String(lockVersion)}) !== ${expectedVersion}`,
        );
      }
    }
  }

  for (const pkg of ALL_WORKSPACE_PACKAGES) {
    const pkgDir = path.join(repoRoot, pkg.dir);
    const manifest = readJson<PackageJsonShape>(path.join(pkgDir, "package.json"));
    if (manifest.name !== pkg.name) {
      errors.push(
        `${pkg.dir}/package.json name mismatch: expected ${pkg.name}, got ${manifest.name}`,
      );
    }
    if (manifest.version !== expectedVersion) {
      errors.push(
        `${pkg.dir}/package.json version (${String(manifest.version)}) !== ${expectedVersion}`,
      );
    }
  }

  for (const pkg of PUBLIC_PACKAGES) {
    const pkgDir = path.join(repoRoot, pkg.dir);
    const manifest = readJson<PackageJsonShape>(path.join(pkgDir, "package.json"));
    if (manifest.private === true) {
      errors.push(`Public package ${pkg.name} must not set private: true`);
    }
    if (manifest.license !== "Apache-2.0") {
      errors.push(`${pkg.name} must declare license: "Apache-2.0"`);
    }
    if (manifest.publishConfig?.access !== "public") {
      errors.push(`${pkg.name} must declare publishConfig.access: "public"`);
    }
    if (!manifest.description?.trim()) {
      errors.push(`${pkg.name} is missing description`);
    }
    if (!manifest.repository?.url || manifest.repository.directory !== pkg.dir) {
      errors.push(`${pkg.name} repository metadata must point to ${pkg.dir}`);
    }
    if (!manifest.homepage?.trim() || !manifest.bugs?.url?.trim()) {
      errors.push(`${pkg.name} must declare homepage and bugs.url`);
    }
    if (!Array.isArray(manifest.keywords) || manifest.keywords.length === 0) {
      errors.push(`${pkg.name} must declare keywords`);
    }
    if (!manifest.engines?.bun) {
      errors.push(`${pkg.name} must declare engines.bun`);
    }
    const files = manifest.files ?? [];
    for (const required of ["README.md", ...STAGED_LICENSE_FILES]) {
      if (!files.includes(required)) {
        errors.push(`${pkg.name} files[] must include ${required}`);
      }
    }
    if (!existsSync(path.join(pkgDir, "README.md"))) {
      errors.push(`${pkg.name} is missing ${pkg.dir}/README.md`);
    }
  }

  for (const sdkDir of ["packages/ui", "packages/services"] as const) {
    const manifest = readJson<PackageJsonShape>(path.join(repoRoot, sdkDir, "package.json"));
    const serializedExports = JSON.stringify(manifest.exports ?? {});
    if (serializedExports.includes("./src/")) {
      errors.push(`${sdkDir}/package.json exports must point to ./dist/*, not ./src/*`);
    }
    const solidImport = manifest.imports?.["#solid"] as { types?: string } | undefined;
    if (!solidImport?.types?.startsWith("./dist/")) {
      errors.push(`${sdkDir}/package.json imports["#solid"].types must point to ./dist/*`);
    }
    if (manifest.scripts?.postpack?.includes("dist")) {
      errors.push(`${sdkDir}/package.json postpack must never delete dist/`);
    }
  }

  const x64Pkg = readJson<PackageJsonShape>(
    path.join(repoRoot, "packages/host-linux-x64/package.json"),
  );
  if (
    JSON.stringify(x64Pkg.os) !== '["linux"]' ||
    JSON.stringify(x64Pkg.cpu) !== '["x64"]' ||
    JSON.stringify(x64Pkg.libc) !== '["glibc"]'
  ) {
    errors.push(
      `@hewel/zgui-host-linux-x64 must declare os=["linux"], cpu=["x64"], libc=["glibc"]`,
    );
  }
  if (!x64Pkg.scripts?.prepack) {
    errors.push("@hewel/zgui-host-linux-x64 must define a fail-closed prepack guard");
  }

  const arm64Pkg = readJson<PackageJsonShape>(
    path.join(repoRoot, "packages/host-linux-arm64/package.json"),
  );
  if (arm64Pkg.private !== true) {
    errors.push("@hewel/zgui-host-linux-arm64 must remain private: true until arm64 binaries ship");
  }

  const cliPkg = readJson<PackageJsonShape>(path.join(repoRoot, "packages/cli/package.json"));
  const optKeys = Object.keys(cliPkg.optionalDependencies ?? {}).sort();
  if (JSON.stringify(optKeys) !== '["@hewel/zgui-host-linux-x64"]') {
    errors.push(
      `@hewel/zgui-cli optionalDependencies must contain only @hewel/zgui-host-linux-x64, got ${JSON.stringify(optKeys)}`,
    );
  }
  if (!cliPkg.files?.includes("inspector-bundle") || cliPkg.files?.includes("inspector")) {
    errors.push("@hewel/zgui-cli files[] must include inspector-bundle (not source inspector)");
  }
  if (JSON.stringify(cliPkg.bin) !== '{"zgui-js":"./src/cli.ts"}') {
    errors.push('@hewel/zgui-cli bin must declare only {"zgui-js":"./src/cli.ts"}');
  }

  const initSource = readFileSync(path.join(repoRoot, "packages/cli/src/init.ts"), "utf8");
  if (/["']@hewel\/zgui(?:-services|-cli)?["']\s*:\s*["']\^0\.1\.0["']/.test(initSource)) {
    errors.push("packages/cli/src/init.ts must derive @hewel/* dependency versions dynamically");
  }
  if (!initSource.includes("./node_modules/@hewel/zgui/oxlint/recommended.json")) {
    errors.push(
      "packages/cli/src/init.ts must extend ./node_modules/@hewel/zgui/oxlint/recommended.json",
    );
  }

  if (errors.length > 0) {
    throw new Error(`Release check failed (${errors.length} issue(s)):\n- ${errors.join("\n- ")}`);
  }

  console.log(
    `Release check passed for v${expectedVersion} (${PUBLIC_PACKAGES.length} public packages, Cargo workspace, lockfiles, metadata)`,
  );
  return { version: expectedVersion! };
}

function auditTarball(tarballPath: string, expectedName: string, expectedVersion: string): void {
  const pkgJsonText = runCommandSync(
    ["tar", "-xOf", tarballPath, "package/package.json"],
    repoRoot,
  ).stdout;
  const packedPkg = JSON.parse(pkgJsonText) as PackageJsonShape;
  if (packedPkg.name !== expectedName || packedPkg.version !== expectedVersion) {
    throw new Error(
      `Packed manifest mismatch in ${tarballPath}: expected ${expectedName}@${expectedVersion}, got ${packedPkg.name}@${packedPkg.version}`,
    );
  }
  for (const section of ["dependencies", "optionalDependencies", "peerDependencies"] as const) {
    for (const [dep, spec] of Object.entries(packedPkg[section] ?? {})) {
      if (spec.startsWith("workspace:")) {
        throw new Error(
          `Tarball ${tarballPath} retained unresolved workspace protocol in ${section}.${dep}: ${spec}`,
        );
      }
    }
  }

  const listing = runCommandSync(["tar", "-tzf", tarballPath], repoRoot)
    .stdout.split("\n")
    .map(line => line.trim())
    .filter(Boolean);
  const fileSet = new Set(listing);

  const requiredCommon = [
    "package/package.json",
    "package/README.md",
    ...STAGED_LICENSE_FILES.map(f => `package/${f}`),
  ];
  const requiredByPackage: Record<string, readonly string[]> = {
    "@hewel/zgui": [
      "package/dist/index.js",
      "package/dist/index.d.ts",
      "package/dist/solid-native.d.ts",
      "package/dist/events.d.ts",
      "package/dist/styles.d.ts",
      "package/dist/components/generated/ButtonStyle.js",
      "package/oxlint/recommended.json",
      "package/oxlint/plugin.mjs",
      "package/tsconfig.native.json",
      "package/upstream/LICENSE",
      "package/upstream/NOTICE",
      "package/upstream/ORIGIN",
    ],
    "@hewel/zgui-services": [
      "package/dist/index.js",
      "package/dist/index.d.ts",
      "package/dist/solid.d.ts",
    ],
    "@hewel/zgui-build": [
      "package/src/index.ts",
      "package/src/public-api.d.ts",
      "package/sdk/LICENSE",
      "package/sdk/NOTICE",
      "package/sdk/THIRD_PARTY_NOTICES.md",
      "package/sdk/smoke/solid-smoke.ts",
      "package/sdk/smoke/tsconfig.smoke.json",
      "package/sdk/preview/profiles/default.json",
    ],
    "@hewel/zgui-host-linux-x64": [
      "package/bin/zgui-host",
      "package/host-manifest.json",
      "package/index.js",
      "package/index.d.ts",
    ],
    "@hewel/zgui-cli": [
      "package/src/cli.ts",
      "package/inspector-bundle/manifest.json",
      "package/inspector-bundle/app.mjs",
    ],
    "create-zgui-js": ["package/src/cli.ts"],
  };

  for (const required of [...requiredCommon, ...(requiredByPackage[expectedName] ?? [])]) {
    if (!fileSet.has(required)) {
      throw new Error(`Tarball ${tarballPath} is missing required file: ${required}`);
    }
  }

  if (expectedName === "@hewel/zgui" || expectedName === "@hewel/zgui-services") {
    const leakedSrc = listing.filter(entry => entry.startsWith("package/src/"));
    if (leakedSrc.length > 0) {
      throw new Error(`Tarball ${tarballPath} should not pack source files: ${leakedSrc[0]}`);
    }
  }

  if (expectedName === "@hewel/zgui-cli") {
    const verbose = runCommandSync(["tar", "-tvzf", tarballPath], repoRoot).stdout.split("\n");
    for (const line of verbose) {
      if (line.includes("package/inspector-bundle") && line.startsWith("l")) {
        throw new Error(
          `Tarball ${tarballPath} contains a symlink inside inspector-bundle: ${line}`,
        );
      }
    }
  }
}

async function cmdPack({
  outDir,
  hostBinary,
  glibcBaseline,
  skipHostProbe = false,
  keepStagedHost = false,
}: {
  outDir: string;
  hostBinary: string;
  glibcBaseline?: string;
  skipHostProbe?: boolean;
  keepStagedHost?: boolean;
}): Promise<ReleaseManifest> {
  const { version } = cmdCheck();
  const resolvedOut = path.resolve(outDir);
  await mkdir(resolvedOut, { recursive: true });

  await buildSdkPackages({ root: repoRoot });

  const hostPkgDir = path.join(repoRoot, "packages/host-linux-x64");
  const resolvedBaseline = glibcBaseline ?? detectGlibcBaseline();
  await stageHostPackage({
    platform: "linux-x64",
    binary: path.resolve(hostBinary),
    packageDir: hostPkgDir,
    skipProbe: skipHostProbe,
    glibcBaseline: resolvedBaseline,
  });
  await stageLicenseFiles(hostPkgDir, repoRoot);
  await verifyHostPackage(hostPkgDir, { verifyAbi: !skipHostProbe });

  const packedEntries: ReleaseManifestPackage[] = [];
  try {
    for (const pkg of PUBLIC_PACKAGES) {
      const pkgDir = path.join(repoRoot, pkg.dir);
      const slug = pkg.name.replace(/^@/, "").replace(/\//g, "-");
      const tarballName = `${slug}-${version}.tgz`;
      const tarballPath = path.join(resolvedOut, tarballName);
      await rm(tarballPath, { force: true });

      runCommandSync([process.execPath, "pm", "pack", "--filename", tarballPath], pkgDir);
      auditTarball(tarballPath, pkg.name, version);

      const bytes = await readFile(tarballPath);
      const info = await stat(tarballPath);
      const digests = computeDigests(bytes);
      packedEntries.push({
        name: pkg.name,
        version,
        directory: pkg.dir,
        tarball: tarballName,
        bytes: info.size,
        sha256: digests.sha256,
        integrity: digests.integrity,
      });
      console.log(`Packed & audited ${pkg.name}@${version} -> ${tarballName} (${info.size} bytes)`);
    }
  } finally {
    if (!keepStagedHost) {
      await rm(path.join(hostPkgDir, "bin"), { recursive: true, force: true });
      await rm(path.join(hostPkgDir, "host-manifest.json"), { force: true });
      await cleanLicenseFiles(hostPkgDir);
    }
  }

  const manifest: ReleaseManifest = {
    schemaVersion: 1,
    version,
    createdAt: new Date().toISOString(),
    packages: packedEntries,
  };
  const manifestPath = path.join(resolvedOut, "release-manifest.json");
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  console.log(`Wrote ${manifestPath} (${packedEntries.length} packages)`);
  return manifest;
}

function resolveDistTag(version: string, explicitTag?: string): string {
  const match = version.match(SEMVER_RE);
  if (!match) throw new Error(`Invalid manifest version: ${version}`);
  const prerelease = match[4];
  if (prerelease) {
    if (explicitTag === "latest") {
      throw new Error(
        `Refusing to publish prerelease version ${version} under dist-tag "latest". Use a prerelease tag such as "rc", "beta", or "next".`,
      );
    }
    if (explicitTag) return explicitTag;
    const channel = prerelease.split(".")[0];
    return channel && /^[A-Za-z][0-9A-Za-z-]*$/.test(channel) ? channel : "next";
  }
  return explicitTag ?? "latest";
}

type RegistryLookup =
  | { status: "missing" }
  | { status: "published"; integrity: string | undefined }
  | { status: "unreachable"; detail: string };

async function lookupRegistryIntegrity(
  registry: string,
  name: string,
  version: string,
): Promise<RegistryLookup> {
  const base = registry.replace(/\/+$/, "");
  const encoded = name.replace("/", "%2F");
  const url = `${base}/${encoded}`;
  try {
    const response = await fetch(url, {
      headers: { Accept: "application/vnd.npm.install-v1+json, application/json" },
    });
    if (response.status === 404) {
      return { status: "missing" };
    }
    if (!response.ok) {
      return {
        status: "unreachable",
        detail: `Registry returned HTTP ${response.status} for ${url}`,
      };
    }
    const payload = (await response.json()) as {
      versions?: Record<string, { dist?: { integrity?: string } }>;
    };
    const versionEntry = payload.versions?.[version];
    if (!versionEntry) {
      return { status: "missing" };
    }
    return { status: "published", integrity: versionEntry.dist?.integrity };
  } catch (error) {
    return {
      status: "unreachable",
      detail: error instanceof Error ? error.message : String(error),
    };
  }
}

async function cmdPublish({
  manifestPath,
  explicitTag,
  registry = "https://registry.npmjs.org",
  explicitRegistry = false,
  provenance = true,
  dryRun = false,
}: {
  manifestPath: string;
  explicitTag?: string;
  registry?: string;
  explicitRegistry?: boolean;
  provenance?: boolean;
  dryRun?: boolean;
}): Promise<void> {
  const resolvedManifest = path.resolve(manifestPath);
  const manifestDir = path.dirname(resolvedManifest);
  const manifest = readJson<ReleaseManifest>(resolvedManifest);

  if (manifest.schemaVersion !== 1 || !Array.isArray(manifest.packages)) {
    throw new Error(`Invalid release-manifest.json at ${resolvedManifest}`);
  }
  const expectedOrder = PUBLIC_PACKAGES.map(p => p.name);
  const actualOrder = manifest.packages.map(p => p.name);
  if (JSON.stringify(actualOrder) !== JSON.stringify(expectedOrder)) {
    throw new Error(
      `Release manifest package order mismatch: expected ${JSON.stringify(expectedOrder)}, got ${JSON.stringify(actualOrder)}`,
    );
  }

  const distTag = resolveDistTag(manifest.version, explicitTag);

  // Verify every local tarball matches manifest size, sha256, and SRI sha512 before publishing anything.
  for (const pkg of manifest.packages) {
    if (pkg.version !== manifest.version) {
      throw new Error(`Package version mismatch in manifest for ${pkg.name}: ${pkg.version}`);
    }
    const tarballPath = path.join(manifestDir, pkg.tarball);
    if (!existsSync(tarballPath)) {
      throw new Error(`Missing tarball listed in manifest: ${tarballPath}`);
    }
    const bytes = await readFile(tarballPath);
    const digests = computeDigests(bytes);
    if (
      bytes.length !== pkg.bytes ||
      digests.sha256 !== pkg.sha256 ||
      digests.integrity !== pkg.integrity
    ) {
      throw new Error(
        `Local tarball integrity mismatch for ${pkg.name}@${pkg.version}: ${tarballPath} does not match release-manifest.json`,
      );
    }
  }

  for (const pkg of manifest.packages) {
    const tarballPath = path.join(manifestDir, pkg.tarball);
    const lookup = await lookupRegistryIntegrity(registry, pkg.name, pkg.version);
    if (lookup.status === "published") {
      if (lookup.integrity === pkg.integrity) {
        console.log(
          `SKIP ${pkg.name}@${pkg.version} (already published with matching dist.integrity ${pkg.integrity})`,
        );
        continue;
      }
      throw new Error(
        `FAIL-CLOSED dist.integrity mismatch for ${pkg.name}@${pkg.version}: registry has ${String(lookup.integrity)}, local artifact has ${pkg.integrity}`,
      );
    }
    if (lookup.status === "unreachable") {
      if (!dryRun || explicitRegistry) {
        throw new Error(
          `Registry integrity check failed for ${pkg.name}@${pkg.version}: ${lookup.detail}`,
        );
      }
      console.log(
        `WARN [dry-run] Could not reach ${registry} for ${pkg.name}@${pkg.version} (${lookup.detail}); proceeding with local dry-run validation`,
      );
    }

    const publishCmd = [
      "npm",
      "publish",
      tarballPath,
      "--access",
      "public",
      "--tag",
      distTag,
      "--registry",
      registry,
      ...(provenance ? ["--provenance"] : []),
      ...(dryRun ? ["--dry-run"] : []),
    ];

    if (dryRun) {
      console.log(
        `DRY-RUN publish ${pkg.name}@${pkg.version} (tag=${distTag}, integrity=${pkg.integrity})`,
      );
      console.log(`  $ ${publishCmd.join(" ")}`);
      continue;
    }

    console.log(`Publishing ${pkg.name}@${pkg.version} (tag=${distTag})...`);
    runCommandSync(publishCmd, manifestDir);
    console.log(`Published ${pkg.name}@${pkg.version}`);
  }
}

const args = process.argv.slice(2);
const subcommand = args.shift();

if (!subcommand || subcommand === "-h" || subcommand === "--help") {
  console.log(usage);
  process.exit(0);
}

try {
  if (subcommand === "version") {
    let setVersion: string | undefined;
    while (args.length > 0) {
      const arg = args.shift()!;
      if (arg === "--set") {
        setVersion = args.shift();
      } else {
        throw new Error(`Unknown option for version: ${arg}\n${usage}`);
      }
    }
    if (!setVersion) throw new Error(`version requires --set <SEMVER>\n${usage}`);
    await cmdVersion(setVersion);
  } else if (subcommand === "check") {
    let tag: string | undefined;
    while (args.length > 0) {
      const arg = args.shift()!;
      if (arg === "--tag") {
        tag = args.shift();
        if (!tag) throw new Error("--tag requires a value");
      } else {
        throw new Error(`Unknown option for check: ${arg}\n${usage}`);
      }
    }
    cmdCheck({ tag });
  } else if (subcommand === "pack") {
    let outDir: string | undefined;
    let hostBinary: string | undefined;
    let glibcBaseline: string | undefined;
    let skipHostProbe = false;
    let keepStagedHost = false;
    while (args.length > 0) {
      const arg = args.shift()!;
      switch (arg) {
        case "--out":
          outDir = args.shift();
          break;
        case "--host-binary":
          hostBinary = args.shift();
          break;
        case "--glibc-baseline":
          glibcBaseline = args.shift();
          break;
        case "--skip-host-probe":
          skipHostProbe = true;
          break;
        case "--keep-staged-host":
          keepStagedHost = true;
          break;
        default:
          throw new Error(`Unknown option for pack: ${arg}\n${usage}`);
      }
    }
    if (!outDir || !hostBinary) {
      throw new Error(`pack requires --out <DIR> and --host-binary <PATH>\n${usage}`);
    }
    await cmdPack({ outDir, hostBinary, glibcBaseline, skipHostProbe, keepStagedHost });
  } else if (subcommand === "publish") {
    let manifestPath: string | undefined;
    let explicitTag: string | undefined;
    let registry = "https://registry.npmjs.org";
    let explicitRegistry = false;
    let provenance = true;
    let dryRun = false;
    while (args.length > 0) {
      const arg = args.shift()!;
      switch (arg) {
        case "--manifest":
          manifestPath = args.shift();
          break;
        case "--tag":
          explicitTag = args.shift();
          break;
        case "--registry":
          registry = args.shift() ?? "";
          explicitRegistry = true;
          break;
        case "--provenance":
          provenance = true;
          break;
        case "--no-provenance":
          provenance = false;
          break;
        case "--dry-run":
          dryRun = true;
          break;
        default:
          throw new Error(`Unknown option for publish: ${arg}\n${usage}`);
      }
    }
    if (!manifestPath) {
      throw new Error(`publish requires --manifest <PATH>\n${usage}`);
    }
    await cmdPublish({
      manifestPath,
      explicitTag,
      registry,
      explicitRegistry,
      provenance,
      dryRun,
    });
  } else {
    throw new Error(`Unknown subcommand: ${subcommand}\n${usage}`);
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
