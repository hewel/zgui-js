import type { PackageApplicationOptions, PackagedApplication } from "./index.ts";
import { existsSync } from "node:fs";
import { chmod, copyFile, cp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { buildApplication } from "./build.ts";
import { EXPECTED_HOST_API_VERSION, resolveNativeHost } from "./host.ts";
import { packagedSdkRoot, workspaceRoot } from "./sdk.ts";

const DISTRIBUTED_LICENSE_FILES = ["LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md"] as const;

function resolveLicenseFile(filename: string, root: string): string {
  const candidates = [
    path.join(root, filename),
    path.join(packagedSdkRoot, filename),
    path.resolve(packagedSdkRoot, "..", filename),
  ];
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
  }
  throw new Error(`Missing required license notice file for packaging: ${filename}`);
}

function sanitizeDesktopField(value: string): string {
  return value.replace(/[\r\n]+/g, " ").trim();
}

function buildLauncherScript(app: string, { pureShell = false }: { pureShell?: boolean }): string {
  const extraFlags = pureShell ? " --pure-shell" : "";
  return `#!/usr/bin/env sh
set -eu
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$DIR/bin/zgui-host" --bundle "$DIR/bundle"${extraFlags} "$@"
`;
}

function buildDesktopEntry({
  name,
  id,
  execPath,
}: {
  name: string;
  id: string;
  execPath: string;
}): string {
  return `[Desktop Entry]
Type=Application
Version=1.0
Name=${sanitizeDesktopField(name)}
Exec=${sanitizeDesktopField(execPath)}
Terminal=false
Categories=Utility;
StartupWMClass=${sanitizeDesktopField(id)}
`;
}

export async function packageApplication(
  app: string,
  {
    root = workspaceRoot,
    appDirectory = path.join(root, "apps/examples", app),
    out = path.join(appDirectory, "dist-package"),
    bytecode = false,
    hostRoot,
    hostExecutable,
    hostSearchFrom,
    extraCapabilities,
    capabilitiesFile,
    desktopEntry = false,
    pureShell = false,
    signal,
  }: PackageApplicationOptions = {},
): Promise<PackagedApplication> {
  signal?.throwIfAborted();
  const resolvedAppDir = path.resolve(appDirectory);
  const outputDir = path.resolve(out);
  const { dist, manifest } = await buildApplication(app, {
    root,
    appDirectory: resolvedAppDir,
    development: false,
    bytecode,
    hostRoot,
    hostExecutable,
    hostSearchFrom,
    extraCapabilities,
    capabilitiesFile,
  });
  signal?.throwIfAborted();

  if (pureShell && !manifest.capabilities?.shellSurfaces && !manifest.capabilities?.sessionLock) {
    throw new Error(
      "--pure-shell requires capabilities.shellSurfaces or capabilities.sessionLock in manifest",
    );
  }

  const host = await resolveNativeHost({
    hostExecutable,
    hostRoot,
    root,
    cwd: resolvedAppDir,
    searchFrom: hostSearchFrom,
    signal,
  });
  signal?.throwIfAborted();

  const bundleSource = await realpath(dist);
  const stagingDir = `${outputDir}.staging-${process.pid}-${Date.now()}`;
  const binDir = path.join(stagingDir, "bin");
  const bundleDir = path.join(stagingDir, "bundle");
  const executablePath = path.join(binDir, "zgui-host");
  const launcherPath = path.join(stagingDir, app);
  const desktopFileName = `${app}.desktop`;

  try {
    await mkdir(binDir, { recursive: true });
    await copyFile(host.executable, executablePath);
    await chmod(executablePath, 0o755);
    await cp(bundleSource, bundleDir, { recursive: true, dereference: true });
    const licensesDir = path.join(stagingDir, "licenses");
    await mkdir(licensesDir, { recursive: true });
    const licensePaths: string[] = [];
    for (const filename of DISTRIBUTED_LICENSE_FILES) {
      const source = resolveLicenseFile(filename, path.resolve(root));
      await copyFile(source, path.join(licensesDir, filename));
      licensePaths.push(`licenses/${filename}`);
    }
    await writeFile(launcherPath, buildLauncherScript(app, { pureShell }), "utf8");
    await chmod(launcherPath, 0o755);

    if (desktopEntry) {
      const displayName = manifest.window?.title ?? app;
      await writeFile(
        path.join(stagingDir, desktopFileName),
        buildDesktopEntry({
          name: displayName,
          id: manifest.id,
          execPath: path.join(outputDir, app),
        }),
        "utf8",
      );
    }

    const packageManifest = {
      schemaVersion: 1 as const,
      app,
      id: manifest.id,
      buildId: manifest.buildId,
      hostApiVersion: host.abi?.hostApiVersion ?? EXPECTED_HOST_API_VERSION,
      hostVersion: host.abi?.version ?? "local",
      hostSource: host.source,
      executable: "bin/zgui-host",
      bundle: "bundle",
      launcher: app,
      licenses: licensePaths,
      ...(desktopEntry ? { desktopEntry: desktopFileName } : {}),
    };
    await writeFile(
      path.join(stagingDir, "package-manifest.json"),
      `${JSON.stringify(packageManifest, null, 2)}\n`,
      "utf8",
    );

    await rm(outputDir, { recursive: true, force: true });
    await mkdir(path.dirname(outputDir), { recursive: true });
    await cp(stagingDir, outputDir, { recursive: true });
  } finally {
    await rm(stagingDir, { recursive: true, force: true });
  }

  return {
    out: outputDir,
    launcher: path.join(outputDir, app),
    executable: path.join(outputDir, "bin/zgui-host"),
    bundle: path.join(outputDir, "bundle"),
    ...(desktopEntry ? { desktopEntry: path.join(outputDir, desktopFileName) } : {}),
    host,
    manifest,
  };
}
