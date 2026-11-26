import type { NativeHostAbi, ResolvedNativeHost, ResolveNativeHostOptions } from "./index.ts";
import { createHash } from "node:crypto";
import {
  accessSync,
  constants as fsConstants,
  existsSync,
  readFileSync,
  realpathSync,
  statSync,
} from "node:fs";
import { chmod, copyFile, mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { prepareNativeHost } from "./native-process.ts";
import { packagedSdkRoot, workspaceRoot } from "./sdk.ts";

export const EXPECTED_HOST_API_VERSION = 1;
export const EXPECTED_SCHEMA_VERSION = 2;

export const SUPPORTED_HOST_PACKAGES: Readonly<Record<string, string>> = Object.freeze({
  "linux-x64": "@hewel/zgui-host-linux-x64",
  "linux-arm64": "@hewel/zgui-host-linux-arm64",
});

export interface StagedHostManifest {
  readonly schemaVersion: 1;
  readonly package: string;
  readonly platform: string;
  readonly version: string;
  readonly hostApiVersion: number;
  readonly bundleSchemaVersion: number;
  readonly sha256: string;
  readonly bytes: number;
  readonly libc?: "glibc" | undefined;
  readonly glibcBaseline?: string | undefined;
}

function hostFailure(message: string, exitCode = 1): Error & { exitCode: number } {
  return Object.assign(new Error(message), { exitCode });
}

export function platformId(
  platform: string = process.platform,
  arch: string = process.arch,
): string {
  return `${platform}-${arch}`;
}

export function platformHostPackageName(
  platform: string = process.platform,
  arch: string = process.arch,
): string | undefined {
  return SUPPORTED_HOST_PACKAGES[platformId(platform, arch)];
}

function hasCargoNativeHost(directory: string): boolean {
  return (
    existsSync(path.join(directory, "Cargo.toml")) &&
    existsSync(path.join(directory, "crates/zgui-host/Cargo.toml"))
  );
}

function findCargoCheckoutUpwards(startDir: string): string | undefined {
  let current = path.resolve(startDir);
  while (true) {
    if (hasCargoNativeHost(current)) return current;
    const parent = path.dirname(current);
    if (parent === current) return undefined;
    current = parent;
  }
}

function parseVersionText(text: string): NativeHostAbi | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  if (trimmed.startsWith("{")) {
    try {
      const parsed = JSON.parse(trimmed) as Record<string, unknown>;
      if (
        parsed.name !== "zgui-host" ||
        typeof parsed.version !== "string" ||
        typeof parsed.hostApiVersion !== "number" ||
        typeof parsed.schemaVersion !== "number" ||
        typeof parsed.platform !== "string"
      ) {
        return null;
      }
      return {
        name: "zgui-host",
        version: parsed.version,
        hostApiVersion: parsed.hostApiVersion,
        schemaVersion: parsed.schemaVersion,
        platform: parsed.platform,
        ...(typeof parsed.engine === "string" ? { engine: parsed.engine } : {}),
        ...(typeof parsed.engineRevision === "string"
          ? { engineRevision: parsed.engineRevision }
          : {}),
        ...(typeof parsed.bytecodeFormat === "string"
          ? { bytecodeFormat: parsed.bytecodeFormat }
          : {}),
        ...(typeof parsed.quickjsAbi === "string" ? { quickjsAbi: parsed.quickjsAbi } : {}),
        ...(typeof parsed.styleProfile === "string" ? { styleProfile: parsed.styleProfile } : {}),
      };
    } catch {
      return null;
    }
  }
  const match = trimmed.match(
    /^zgui-host\s+(\S+)\s+\(hostApiVersion=(\d+)\s+schemaVersion=(\d+)(?:\s+platform=([^\s)]+))?\)$/,
  );
  if (!match) return null;
  return {
    name: "zgui-host",
    version: match[1]!,
    hostApiVersion: Number(match[2]),
    schemaVersion: Number(match[3]),
    platform: match[4] ?? platformId(),
  };
}

export function probeNativeHost(
  executable: string,
  {
    expectedHostApiVersion = EXPECTED_HOST_API_VERSION,
    expectedSchemaVersion = EXPECTED_SCHEMA_VERSION,
  }: {
    expectedHostApiVersion?: number;
    expectedSchemaVersion?: number;
  } = {},
): NativeHostAbi {
  const resolved = path.resolve(executable);
  if (!existsSync(resolved)) {
    throw hostFailure(`zgui-host executable not found: ${resolved}`);
  }
  try {
    accessSync(resolved, fsConstants.X_OK);
  } catch {
    throw hostFailure(`zgui-host executable is not executable: ${resolved}`);
  }
  const child = Bun.spawnSync([resolved, "--version", "--json"], {
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
    timeout: 5000,
  });
  if (!child.success) {
    const detail = `${child.stderr.toString()}${child.stdout.toString()}`.trim();
    throw hostFailure(
      `zgui-host ABI handshake failed (${resolved} --version --json exited with ${child.signalCode ?? child.exitCode})${detail ? `: ${detail}` : ""}`,
    );
  }
  const stdout = child.stdout.toString("utf8").trim();
  const abi = parseVersionText(stdout);
  if (!abi) {
    throw hostFailure(
      `zgui-host ABI handshake returned unrecognized version metadata from ${resolved}: ${stdout || "(empty output)"}`,
    );
  }
  if (
    abi.hostApiVersion !== expectedHostApiVersion ||
    abi.schemaVersion !== expectedSchemaVersion
  ) {
    throw hostFailure(
      `zgui-host ABI mismatch at ${resolved}: host reports hostApiVersion=${abi.hostApiVersion} schemaVersion=${abi.schemaVersion}, expected hostApiVersion=${expectedHostApiVersion} schemaVersion=${expectedSchemaVersion}`,
    );
  }
  return abi;
}

function sha256File(filename: string): string {
  return `sha256:${createHash("sha256").update(readFileSync(filename)).digest("hex")}`;
}

function tryResolvePlatformPackage(
  packageName: string,
  searchFrom: readonly string[],
): string | undefined {
  for (const base of searchFrom) {
    try {
      const anchor = existsSync(path.join(base, "package.json"))
        ? path.join(base, "package.json")
        : base;
      const req = createRequire(anchor);
      return path.dirname(req.resolve(`${packageName}/package.json`));
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code === "MODULE_NOT_FOUND" || code === "ERR_PACKAGE_PATH_NOT_EXPORTED") continue;
      throw error;
    }
  }
  return undefined;
}

function parseStagedManifest(manifestPath: string, label: string): StagedHostManifest {
  if (!existsSync(manifestPath)) {
    throw hostFailure(
      `Missing host-manifest.json in ${label} (refusing to execute unverified binary)`,
    );
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(manifestPath, "utf8"));
  } catch (error) {
    throw hostFailure(
      `Malformed host-manifest.json in ${label}: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw hostFailure(`Invalid host-manifest.json in ${label}: expected JSON object`);
  }
  return parsed as StagedHostManifest;
}

function validateStagedManifest({
  manifest,
  binaryPath,
  expectedPackage,
  expectedPlatform,
  expectedVersion,
}: {
  manifest: StagedHostManifest;
  binaryPath: string;
  expectedPackage: string;
  expectedPlatform: string;
  expectedVersion?: string;
}): void {
  if (manifest.schemaVersion !== 1) {
    throw hostFailure(
      `Unsupported host-manifest.json schemaVersion in ${expectedPackage}: ${String(manifest.schemaVersion)}`,
    );
  }
  if (manifest.package !== expectedPackage) {
    throw hostFailure(
      `Prebuilt host manifest package mismatch: expected ${expectedPackage}, got ${String(manifest.package)}`,
    );
  }
  if (manifest.platform !== expectedPlatform) {
    throw hostFailure(
      `Prebuilt host manifest platform mismatch in ${expectedPackage}: expected ${expectedPlatform}, got ${String(manifest.platform)}`,
    );
  }
  if (typeof manifest.version !== "string" || !manifest.version.trim()) {
    throw hostFailure(`Prebuilt host manifest missing version in ${expectedPackage}`);
  }
  if (expectedVersion !== undefined && manifest.version !== expectedVersion) {
    throw hostFailure(
      `Prebuilt host manifest version mismatch in ${expectedPackage}: expected ${expectedVersion}, got ${manifest.version}`,
    );
  }
  if (
    manifest.hostApiVersion !== EXPECTED_HOST_API_VERSION ||
    manifest.bundleSchemaVersion !== EXPECTED_SCHEMA_VERSION
  ) {
    throw hostFailure(
      `Prebuilt host manifest ABI mismatch in ${expectedPackage}: hostApiVersion=${String(manifest.hostApiVersion)} bundleSchemaVersion=${String(manifest.bundleSchemaVersion)}, expected hostApiVersion=${EXPECTED_HOST_API_VERSION} bundleSchemaVersion=${EXPECTED_SCHEMA_VERSION}`,
    );
  }
  const actualBytes = statSync(binaryPath).size;
  if (typeof manifest.bytes !== "number" || manifest.bytes !== actualBytes) {
    throw hostFailure(
      `Prebuilt host integrity mismatch (size) in ${expectedPackage}: expected ${String(manifest.bytes)} bytes, got ${actualBytes}`,
    );
  }
  const actualDigest = sha256File(binaryPath);
  if (typeof manifest.sha256 !== "string" || manifest.sha256 !== actualDigest) {
    throw hostFailure(
      `Prebuilt host integrity mismatch in ${expectedPackage}: expected ${String(manifest.sha256)}, got ${actualDigest}`,
    );
  }
}

function resolveEntryScriptDir(): string | undefined {
  const arg = process.argv[1];
  if (typeof arg !== "string" || !arg || !existsSync(arg)) return undefined;
  try {
    return path.dirname(realpathSync(arg));
  } catch {
    return path.dirname(path.resolve(arg));
  }
}

async function resolveCargoHost(
  cargoRoot: string,
  {
    allowCargoBuild,
    verifyAbi,
    signal,
    explicit,
  }: {
    allowCargoBuild: boolean;
    verifyAbi: boolean;
    signal?: AbortSignal;
    explicit: boolean;
  },
): Promise<ResolvedNativeHost> {
  if (explicit && !existsSync(cargoRoot)) {
    throw hostFailure(`Explicit --host-root directory not found: ${cargoRoot}`);
  }
  if (!allowCargoBuild) {
    for (const rel of ["target/debug/zgui-host", "target/release/zgui-host"]) {
      const built = path.join(cargoRoot, rel);
      if (existsSync(built)) {
        const abi = verifyAbi ? probeNativeHost(built) : undefined;
        return {
          executable: built,
          source: "cargo",
          cargoRoot,
          ...(abi ? { abi } : {}),
        };
      }
    }
    if (explicit && !hasCargoNativeHost(cargoRoot)) {
      throw hostFailure(
        `Explicit --host-root is not a zgui-js Cargo checkout (missing crates/zgui-host/Cargo.toml): ${cargoRoot}`,
      );
    }
    throw hostFailure(
      `Local Cargo checkout detected at ${cargoRoot}, but zgui-host has not been built yet. Run \`cargo build -p zgui-host\` or invoke \`zgui-js dev\`.`,
    );
  }
  const executable = await prepareNativeHost({ root: cargoRoot, signal });
  return {
    executable,
    source: "cargo",
    cargoRoot,
  };
}

export async function resolveNativeHost({
  hostExecutable,
  hostRoot,
  root = workspaceRoot,
  cwd = process.cwd(),
  env = process.env,
  platform = process.platform,
  arch = process.arch,
  searchFrom = [],
  verifyAbi = true,
  allowCargoBuild = true,
  signal,
}: ResolveNativeHostOptions = {}): Promise<ResolvedNativeHost> {
  signal?.throwIfAborted();
  const resolvedCwd = path.resolve(cwd);

  // 1. Explicit --host-executable or ZGUI_JS_HOST_EXECUTABLE environment variable.
  const explicitRaw = hostExecutable?.trim() || undefined;
  const envRaw = env.ZGUI_JS_HOST_EXECUTABLE?.trim() || undefined;
  if (explicitRaw !== undefined || envRaw !== undefined) {
    const source = explicitRaw !== undefined ? "explicit" : "env";
    const candidate = path.resolve(resolvedCwd, (explicitRaw ?? envRaw)!);
    const abi = verifyAbi ? probeNativeHost(candidate) : undefined;
    return {
      executable: candidate,
      source,
      ...(abi ? { abi } : {}),
    };
  }

  // 2. Explicit --host-root overrides any installed platform package so external checkouts can test local Rust builds deterministically.
  const explicitHostRoot = hostRoot?.trim() || undefined;
  if (explicitHostRoot !== undefined) {
    return resolveCargoHost(path.resolve(resolvedCwd, explicitHostRoot), {
      allowCargoBuild,
      verifyAbi,
      signal,
      explicit: true,
    });
  }

  // 3. Installed platform package (@hewel/zgui-host-<os>-<arch>) with mandatory manifest + ABI checks.
  const currentPlatform = platformId(platform, arch);
  const packageName = platformHostPackageName(platform, arch);
  if (packageName) {
    const entryScript = resolveEntryScriptDir();
    const searchBases = [
      ...searchFrom,
      resolvedCwd,
      ...(entryScript ? [entryScript] : []),
      path.resolve(root),
      import.meta.url,
    ];
    const pkgDir = tryResolvePlatformPackage(packageName, searchBases);
    if (pkgDir) {
      const binaryPath = path.join(pkgDir, "bin", "zgui-host");
      const manifestPath = path.join(pkgDir, "host-manifest.json");
      if (existsSync(binaryPath)) {
        const manifest = parseStagedManifest(manifestPath, packageName);
        validateStagedManifest({
          manifest,
          binaryPath,
          expectedPackage: packageName,
          expectedPlatform: currentPlatform,
        });
        const abi = verifyAbi ? probeNativeHost(binaryPath) : undefined;
        return {
          executable: binaryPath,
          source: "platform-package",
          packageName,
          ...(abi ? { abi } : {}),
        };
      }
      if (existsSync(manifestPath)) {
        throw hostFailure(
          `Incomplete prebuilt host package ${packageName}: found host-manifest.json without bin/zgui-host`,
        );
      }
    }
  }

  // 4. Autodetected local Cargo workspace checkout containing crates/zgui-host.
  const resolvedRoot = path.resolve(root);
  const autoCargoRoot =
    resolvedRoot !== packagedSdkRoot && hasCargoNativeHost(resolvedRoot)
      ? resolvedRoot
      : findCargoCheckoutUpwards(resolvedCwd);

  if (autoCargoRoot !== undefined) {
    return resolveCargoHost(autoCargoRoot, {
      allowCargoBuild,
      verifyAbi,
      signal,
      explicit: false,
    });
  }

  // 5. Actionable error when no prebuilt host or Cargo checkout is available.
  const installHint = packageName
    ? `Install the prebuilt host package (${packageName})`
    : `No prebuilt host package is published for ${currentPlatform}`;
  throw hostFailure(
    `Cannot locate a zgui-js zgui-host binary for ${currentPlatform}.\n` +
      `  1. ${installHint} (included in @hewel/zgui-cli optionalDependencies on supported platforms), or\n` +
      `  2. Pass --host-executable <PATH> or set ZGUI_JS_HOST_EXECUTABLE=<PATH>, or\n` +
      `  3. Pass --host-root <DIR> pointing to a zgui-js Cargo checkout containing crates/zgui-host.`,
  );
}

export async function stageHostPackage({
  platform,
  binary,
  packageDir,
  skipProbe = false,
  glibcBaseline,
}: {
  platform: string;
  binary: string;
  packageDir?: string;
  skipProbe?: boolean;
  glibcBaseline?: string;
}): Promise<{
  packageDir: string;
  binaryPath: string;
  manifestPath: string;
  manifest: StagedHostManifest;
}> {
  const packageName = SUPPORTED_HOST_PACKAGES[platform];
  if (!packageName) {
    throw hostFailure(
      `Unsupported host platform "${platform}". Expected one of: ${Object.keys(SUPPORTED_HOST_PACKAGES).join(", ")}`,
    );
  }
  const sourceBinary = path.resolve(binary);
  if (!existsSync(sourceBinary)) {
    throw hostFailure(`Source zgui-host binary not found: ${sourceBinary}`);
  }
  const targetDir = path.resolve(
    packageDir ?? path.join(workspaceRoot, `packages/host-${platform}`),
  );
  const pkgJsonPath = path.join(targetDir, "package.json");
  const pkgJson = JSON.parse(await readFile(pkgJsonPath, "utf8")) as {
    name?: string;
    version?: string;
  };
  if (pkgJson.name !== packageName || typeof pkgJson.version !== "string") {
    throw hostFailure(`Invalid platform host package manifest at ${pkgJsonPath}`);
  }

  let version = pkgJson.version;
  let hostApiVersion = EXPECTED_HOST_API_VERSION;
  let bundleSchemaVersion = EXPECTED_SCHEMA_VERSION;
  if (!skipProbe) {
    const abi = probeNativeHost(sourceBinary);
    if (abi.platform !== platform) {
      throw hostFailure(
        `Platform mismatch staging ${packageName}: binary reports platform=${abi.platform}, expected ${platform}`,
      );
    }
    if (abi.version !== pkgJson.version) {
      throw hostFailure(
        `Version mismatch staging ${packageName}: binary reports version=${abi.version}, expected package.json version=${pkgJson.version}`,
      );
    }
    version = abi.version;
    hostApiVersion = abi.hostApiVersion;
    bundleSchemaVersion = abi.schemaVersion;
  }

  const binDir = path.join(targetDir, "bin");
  const destBinary = path.join(binDir, "zgui-host");
  await mkdir(binDir, { recursive: true });
  await copyFile(sourceBinary, destBinary);
  await chmod(destBinary, 0o755);

  const info = await stat(destBinary);
  const manifest: StagedHostManifest = {
    schemaVersion: 1,
    package: packageName,
    platform,
    version,
    hostApiVersion,
    bundleSchemaVersion,
    sha256: sha256File(destBinary),
    bytes: info.size,
    ...(platform.startsWith("linux-") ? { libc: "glibc" as const } : {}),
    ...(glibcBaseline ? { glibcBaseline } : {}),
  };
  const manifestPath = path.join(targetDir, "host-manifest.json");
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  return { packageDir: targetDir, binaryPath: destBinary, manifestPath, manifest };
}

export async function verifyHostPackage(
  packageDir: string,
  { verifyAbi = true }: { verifyAbi?: boolean } = {},
): Promise<{ manifest: StagedHostManifest; binaryPath: string; abi?: NativeHostAbi }> {
  const dir = path.resolve(packageDir);
  const pkgJson = JSON.parse(await readFile(path.join(dir, "package.json"), "utf8")) as {
    name?: string;
    version?: string;
  };
  if (typeof pkgJson.name !== "string" || typeof pkgJson.version !== "string") {
    throw hostFailure(`Invalid package.json in ${dir}`);
  }
  const binaryPath = path.join(dir, "bin", "zgui-host");
  if (!existsSync(binaryPath)) {
    throw hostFailure(`Missing staged binary: ${binaryPath}`);
  }
  const manifest = parseStagedManifest(path.join(dir, "host-manifest.json"), pkgJson.name);
  const expectedPlatform = Object.entries(SUPPORTED_HOST_PACKAGES).find(
    ([, name]) => name === pkgJson.name,
  )?.[0];
  validateStagedManifest({
    manifest,
    binaryPath,
    expectedPackage: pkgJson.name,
    expectedPlatform: expectedPlatform ?? manifest.platform,
    expectedVersion: pkgJson.version,
  });
  const abi = verifyAbi ? probeNativeHost(binaryPath) : undefined;
  if (abi && abi.version !== pkgJson.version) {
    throw hostFailure(
      `Staged host version mismatch in ${dir}: binary reports ${abi.version}, expected ${pkgJson.version}`,
    );
  }
  if (abi && expectedPlatform && abi.platform !== expectedPlatform) {
    throw hostFailure(
      `Staged host platform mismatch in ${dir}: binary reports ${abi.platform}, expected ${expectedPlatform}`,
    );
  }
  return { manifest, binaryPath, ...(abi ? { abi } : {}) };
}

export async function compileModuleBytecode(
  hostExecutable: string,
  {
    input,
    output,
    moduleName,
    sourceDir,
    signal,
  }: {
    input: string;
    output: string;
    moduleName: string;
    sourceDir?: string;
    signal?: AbortSignal;
  },
): Promise<Buffer> {
  signal?.throwIfAborted();
  const child = Bun.spawn(
    [
      path.resolve(hostExecutable),
      "compile-bytecode",
      "--input",
      path.resolve(input),
      "--output",
      path.resolve(output),
      "--module-name",
      moduleName,
      ...(sourceDir ? ["--source-dir", path.resolve(sourceDir)] : []),
    ],
    {
      stdin: "ignore",
      stdout: "pipe",
      stderr: "pipe",
    },
  );
  const onAbort = () => child.kill("SIGTERM");
  signal?.addEventListener("abort", onAbort, { once: true });
  try {
    const [code, stdout, stderr] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ]);
    if (code !== 0 || child.signalCode) {
      const detail = `${stderr}${stdout}`.trim();
      throw hostFailure(
        `zgui-host compile-bytecode failed for ${moduleName} (exit ${child.signalCode ?? code})${detail ? `: ${detail}` : ""}`,
      );
    }
    return Buffer.from(await readFile(output));
  } finally {
    signal?.removeEventListener("abort", onAbort);
    signal?.throwIfAborted();
  }
}
