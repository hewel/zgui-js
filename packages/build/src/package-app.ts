import type { PackageApplicationOptions, PackagedApplication } from "./index.ts";
import { chmod, copyFile, cp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { buildApplication } from "./build.ts";
import { EXPECTED_HOST_API_VERSION, resolveNativeHost } from "./host.ts";
import { workspaceRoot } from "./sdk.ts";

function sanitizeDesktopField(value: string): string {
  return value.replace(/[\r\n]+/g, " ").trim();
}

function buildLauncherScript(app: string, { pureShell = false }: { pureShell?: boolean }): string {
  const extraFlags = pureShell ? " --pure-shell" : "";
  return `#!/usr/bin/env sh
set -eu
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$DIR/bin/native-host" --bundle "$DIR/bundle"${extraFlags} "$@"
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
    signal,
  });
  signal?.throwIfAborted();

  const bundleSource = await realpath(dist);
  const stagingDir = `${outputDir}.staging-${process.pid}-${Date.now()}`;
  const binDir = path.join(stagingDir, "bin");
  const bundleDir = path.join(stagingDir, "bundle");
  const executablePath = path.join(binDir, "native-host");
  const launcherPath = path.join(stagingDir, app);
  const desktopFileName = `${app}.desktop`;

  try {
    await mkdir(binDir, { recursive: true });
    await copyFile(host.executable, executablePath);
    await chmod(executablePath, 0o755);
    await cp(bundleSource, bundleDir, { recursive: true, dereference: true });
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
      executable: "bin/native-host",
      bundle: "bundle",
      launcher: app,
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
    executable: path.join(outputDir, "bin/native-host"),
    bundle: path.join(outputDir, "bundle"),
    ...(desktopEntry ? { desktopEntry: path.join(outputDir, desktopFileName) } : {}),
    host,
    manifest,
  };
}
