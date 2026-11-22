import type { NativeHostAbi, ResolvedNativeHost, ResolveNativeHostOptions } from "./index.ts";
import { createHash } from "node:crypto";
import { accessSync, constants as fsConstants, existsSync, readFileSync } from "node:fs";
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

  // 2. Installed platform package (@hewel/zgui-host-<os>-<arch>) with ABI handshake.
  const packageName = platformHostPackageName(platform, arch);
  if (packageName) {
    const entryScript =
      typeof process.argv[1] === "string" && existsSync(process.argv[1])
        ? path.dirname(path.resolve(process.argv[1]))
        : undefined;
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
      if (existsSync(binaryPath)) {
        const manifestPath = path.join(pkgDir, "host-manifest.json");
        if (existsSync(manifestPath)) {
          const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as StagedHostManifest;
          const actualDigest = sha256File(binaryPath);
          if (manifest.sha256 !== actualDigest) {
            throw hostFailure(
              `Prebuilt host integrity mismatch in ${packageName}: expected ${manifest.sha256}, got ${actualDigest}`,
            );
          }
          if (
            manifest.hostApiVersion !== EXPECTED_HOST_API_VERSION ||
            manifest.bundleSchemaVersion !== EXPECTED_SCHEMA_VERSION
          ) {
            throw hostFailure(
              `Prebuilt host manifest ABI mismatch in ${packageName}: hostApiVersion=${manifest.hostApiVersion} bundleSchemaVersion=${manifest.bundleSchemaVersion}, expected hostApiVersion=${EXPECTED_HOST_API_VERSION} bundleSchemaVersion=${EXPECTED_SCHEMA_VERSION}`,
            );
          }
        }
        const abi = verifyAbi ? probeNativeHost(binaryPath) : undefined;
        return {
          executable: binaryPath,
          source: "platform-package",
          packageName,
          ...(abi ? { abi } : {}),
        };
      }
    }
  }

  // 3. Local Cargo workspace build (--host-root or detected checkout containing crates/zgui-host).
  const resolvedRoot = path.resolve(root);
  const cargoRoot =
    hostRoot !== undefined
      ? path.resolve(resolvedCwd, hostRoot)
      : resolvedRoot !== packagedSdkRoot && hasCargoNativeHost(resolvedRoot)
        ? resolvedRoot
        : findCargoCheckoutUpwards(resolvedCwd);

  if (cargoRoot !== undefined) {
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

  // 4. Actionable error when no prebuilt host or Cargo checkout is available.
  const currentPlatform = platformId(platform, arch);
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
}: {
  platform: string;
  binary: string;
  packageDir?: string;
  skipProbe?: boolean;
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
  };
  const manifest = JSON.parse(
    await readFile(path.join(dir, "host-manifest.json"), "utf8"),
  ) as StagedHostManifest;
  if (manifest.schemaVersion !== 1 || manifest.package !== pkgJson.name) {
    throw hostFailure(`Invalid host-manifest.json in ${dir}`);
  }
  const binaryPath = path.join(dir, "bin", "zgui-host");
  if (!existsSync(binaryPath)) {
    throw hostFailure(`Missing staged binary: ${binaryPath}`);
  }
  const actualDigest = sha256File(binaryPath);
  if (actualDigest !== manifest.sha256) {
    throw hostFailure(
      `Staged host digest mismatch in ${dir}: expected ${manifest.sha256}, got ${actualDigest}`,
    );
  }
  const info = await stat(binaryPath);
  if (info.size !== manifest.bytes) {
    throw hostFailure(
      `Staged host size mismatch in ${dir}: expected ${manifest.bytes}, got ${info.size}`,
    );
  }
  const abi = verifyAbi ? probeNativeHost(binaryPath) : undefined;
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
