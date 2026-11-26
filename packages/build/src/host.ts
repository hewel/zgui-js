import type {
  NativeHostAbi,
  PrepareNativeHostOptions,
  ResolvedNativeHost,
  ResolveNativeHostOptions,
  RunNativeHostOptions,
} from "./index.ts";
import { spawn, type ChildProcess, type SpawnOptions } from "node:child_process";
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
import { constants } from "node:os";
import path from "node:path";
import { createInterface, type Interface } from "node:readline";
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

// A separate POSIX process group also owns cargo's compiler/linker children and native workers.
function ownedProcess(
  executable: string,
  args: readonly string[],
  {
    root,
    signal,
    onLine,
    capture,
  }: {
    root: string;
    signal?: AbortSignal;
    onLine?: (line: string) => void;
    capture?: { stdout: string[]; stderr: string[] };
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
        stdio: [
          capture ? "ignore" : "inherit",
          onLine || capture ? "pipe" : "inherit",
          capture ? "pipe" : "inherit",
        ],
      });
    } catch (error) {
      reject(hostFailure(`Cannot start ${executable}: ${errorMessage(error)}`));
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
      if (spawnError) reject(hostFailure(`Cannot run ${executable}: ${errorMessage(spawnError)}`));
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
    if (capture) {
      child.stdout?.setEncoding("utf8").on("data", (data: string) => capture.stdout.push(data));
      child.stderr?.setEncoding("utf8").on("data", (data: string) => capture.stderr.push(data));
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
  { root = workspaceRoot, signal }: PrepareNativeHostOptions = {},
  processOptions?: ProcessOptions,
) {
  root = path.resolve(root);
  if (root === packagedSdkRoot)
    throw hostFailure(
      "The installed SDK has no Cargo workspace; pass root pointing to a zgui-js checkout",
    );
  let executable: string | undefined;
  const exitCode = await ownedProcess(
    "cargo",
    ["build", "-p", "zgui-host", "--locked", "--message-format=json-render-diagnostics"],
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
          message.target?.name === "zgui-host" &&
          message.target.kind?.includes("bin") &&
          typeof message.executable === "string"
        ) {
          executable = path.resolve(root, message.executable);
        }
      },
    },
    processOptions,
  );
  if (exitCode !== 0) throw hostFailure(`zgui-host build exited with code ${exitCode}`, exitCode);
  if (!executable) throw hostFailure("Cargo did not report a zgui-host executable");
  return executable;
}

/** Inherit native stdio; return its numeric status (signals use 128 + signal number). */
export async function runNativeHost(
  executable: string,
  args: readonly string[],
  { root = workspaceRoot, signal }: RunNativeHostOptions = {},
  processOptions?: ProcessOptions,
) {
  return ownedProcess(executable, args, { root: path.resolve(root), signal }, processOptions);
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
  cwd?: string,
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
    cwd,
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

async function resolveCargoExecutable(
  cargoRoot: string,
  allowCargoBuild: boolean,
  explicit: boolean,
  signal?: AbortSignal,
): Promise<string> {
  if (explicit && !existsSync(cargoRoot)) {
    throw hostFailure(`Explicit --host-root directory not found: ${cargoRoot}`);
  }
  if (allowCargoBuild) return prepareNativeHost({ root: cargoRoot, signal });
  for (const rel of ["target/debug/zgui-host", "target/release/zgui-host"]) {
    const built = path.join(cargoRoot, rel);
    if (existsSync(built)) return built;
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
  const explicitRoot = hostRoot?.trim();
  const explicitRunRoot = explicitRoot ? path.resolve(resolvedCwd, explicitRoot) : undefined;
  const finish = (host: Omit<ResolvedNativeHost, "abi" | "runRoot">): ResolvedNativeHost => {
    signal?.throwIfAborted();
    const runRoot = explicitRunRoot ?? host.cargoRoot ?? resolvedCwd;
    const abi = verifyAbi ? probeNativeHost(host.executable, undefined, runRoot) : undefined;
    return { ...host, runRoot, ...(abi ? { abi } : {}) };
  };

  // 1. Explicit --host-executable or ZGUI_JS_HOST_EXECUTABLE environment variable.
  const explicitRaw = hostExecutable?.trim() || undefined;
  const envRaw = env.ZGUI_JS_HOST_EXECUTABLE?.trim() || undefined;
  if (explicitRaw !== undefined || envRaw !== undefined) {
    const source = explicitRaw !== undefined ? "explicit" : "env";
    const candidate = path.resolve(resolvedCwd, (explicitRaw ?? envRaw)!);
    return finish({ executable: candidate, source });
  }

  // 2. Explicit --host-root overrides any installed platform package so external checkouts can test local Rust builds deterministically.
  if (explicitRunRoot !== undefined) {
    const executable = await resolveCargoExecutable(explicitRunRoot, allowCargoBuild, true, signal);
    return finish({ executable, source: "cargo", cargoRoot: explicitRunRoot });
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
        return finish({ executable: binaryPath, source: "platform-package", packageName });
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
    const executable = await resolveCargoExecutable(autoCargoRoot, allowCargoBuild, false, signal);
    return finish({ executable, source: "cargo", cargoRoot: autoCargoRoot });
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
  host: ResolvedNativeHost,
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
  const capture = { stdout: [] as string[], stderr: [] as string[] };
  const code = await ownedProcess(
    host.executable,
    [
      "compile-bytecode",
      "--input",
      path.resolve(input),
      "--output",
      path.resolve(output),
      "--module-name",
      moduleName,
      ...(sourceDir ? ["--source-dir", path.resolve(sourceDir)] : []),
    ],
    { root: host.runRoot, signal, capture },
  );
  signal?.throwIfAborted();
  if (code !== 0) {
    const detail = `${capture.stderr.join("")}${capture.stdout.join("")}`.trim();
    throw hostFailure(
      `zgui-host compile-bytecode failed for ${moduleName} (exit ${code})${detail ? `: ${detail}` : ""}`,
    );
  }
  return readFile(output);
}
