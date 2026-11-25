#!/usr/bin/env bun
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import type { ReleaseManifest } from "./release.ts";

const UPSTREAM_REGISTRY = "https://registry.npmjs.org";

interface ParsedArgs {
  manifestPath: string;
  skipPreview: boolean;
  tscBin?: string;
}

function parseArgs(argv: string[]): ParsedArgs {
  let manifestPath: string | undefined;
  let skipPreview = false;
  let tscBin: string | undefined;
  const rest = [...argv];
  while (rest.length > 0) {
    const arg = rest.shift()!;
    if (arg === "--manifest") {
      manifestPath = rest.shift();
    } else if (arg === "--skip-preview") {
      skipPreview = true;
    } else if (arg === "--tsc") {
      tscBin = rest.shift();
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }
  if (!manifestPath) {
    throw new Error(
      "Usage: bun tools/verify-release-artifacts.ts --manifest <PATH> [--skip-preview] [--tsc <PATH>]",
    );
  }
  return { manifestPath: path.resolve(manifestPath), skipPreview, tscBin };
}

async function runCommand(
  cmd: string[],
  {
    cwd,
    env,
    timeout = 120_000,
  }: { cwd: string; env?: Record<string, string | undefined>; timeout?: number },
): Promise<{ stdout: string; stderr: string }> {
  // Spawn asynchronously so the in-process Bun.serve local release registry keeps serving requests.
  const child = Bun.spawn(cmd, {
    cwd,
    env,
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
  });
  let timedOut = false;
  let forceKillTimer: ReturnType<typeof setTimeout> | undefined;
  const timer = setTimeout(() => {
    timedOut = true;
    child.kill("SIGTERM");
    forceKillTimer = setTimeout(() => {
      child.kill("SIGKILL");
    }, 5_000);
    forceKillTimer.unref?.();
  }, timeout);
  try {
    const [stdout, stderr, exitCode] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ]);
    if (timedOut || exitCode !== 0) {
      const reason = timedOut
        ? `timed out after ${timeout}ms`
        : `exit ${child.signalCode ?? exitCode}`;
      throw new Error(
        `Command failed (${cmd.join(" ")}) in ${cwd} (${reason}):\nSTDOUT:\n${stdout}\nSTDERR:\n${stderr}`,
      );
    }
    return { stdout, stderr };
  } finally {
    clearTimeout(timer);
    if (forceKillTimer !== undefined) {
      clearTimeout(forceKillTimer);
    }
  }
}

async function readPackedPackageJson(tarballPath: string): Promise<Record<string, unknown>> {
  const { stdout } = await runCommand(["tar", "-xOf", tarballPath, "package/package.json"], {
    cwd: path.dirname(tarballPath),
  });
  return JSON.parse(stdout) as Record<string, unknown>;
}

async function startLocalReleaseRegistry(manifestPath: string) {
  const manifestDir = path.dirname(manifestPath);
  const manifest = JSON.parse(await readFile(manifestPath, "utf8")) as ReleaseManifest;
  const localPackages = new Map<
    string,
    {
      name: string;
      version: string;
      tarballName: string;
      tarballBytes: Buffer;
      integrity: string;
      shasum: string;
      pkgJson: Record<string, unknown>;
    }
  >();

  for (const pkg of manifest.packages) {
    const tarballPath = path.join(manifestDir, pkg.tarball);
    const tarballBytes = Buffer.from(await readFile(tarballPath));
    const shasum = new Bun.CryptoHasher("sha1").update(tarballBytes).digest("hex");
    const pkgJson = await readPackedPackageJson(tarballPath);
    localPackages.set(pkg.name, {
      name: pkg.name,
      version: pkg.version,
      tarballName: pkg.tarball,
      tarballBytes,
      integrity: pkg.integrity,
      shasum,
      pkgJson,
    });
  }

  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(req) {
      const url = new URL(req.url);
      if (url.pathname.startsWith("/-/tarballs/")) {
        const filename = decodeURIComponent(url.pathname.slice("/-/tarballs/".length));
        for (const entry of localPackages.values()) {
          if (entry.tarballName === filename) {
            return new Response(entry.tarballBytes, {
              headers: { "content-type": "application/octet-stream" },
            });
          }
        }
        return new Response("Not Found", { status: 404 });
      }

      const pkgName = decodeURIComponent(url.pathname.slice(1));
      const local = localPackages.get(pkgName);
      if (local) {
        const origin = `http://127.0.0.1:${server.port}`;
        const versionObj = {
          ...local.pkgJson,
          dist: {
            tarball: `${origin}/-/tarballs/${encodeURIComponent(local.tarballName)}`,
            shasum: local.shasum,
            integrity: local.integrity,
          },
        };
        return Response.json({
          name: local.name,
          "dist-tags": { latest: local.version },
          versions: { [local.version]: versionObj },
        });
      }

      const upstreamUrl = `${UPSTREAM_REGISTRY}${url.pathname}${url.search}`;
      const upstream = await fetch(upstreamUrl, {
        method: req.method,
        headers: { Accept: req.headers.get("accept") ?? "application/json" },
      });
      return new Response(upstream.body, {
        status: upstream.status,
        headers: {
          "content-type": upstream.headers.get("content-type") ?? "application/json",
        },
      });
    },
  });

  return {
    manifest,
    url: `http://127.0.0.1:${server.port}`,
    stop: () => server.stop(true),
  };
}

async function chmodTree(rootDir: string, dirMode: number, fileMode?: number): Promise<void> {
  const entries = await Bun.$`find ${rootDir} -type d`.text();
  const dirs = entries
    .split("\n")
    .map(s => s.trim())
    .filter(Boolean)
    .reverse();
  if (fileMode !== undefined) {
    const files = (await Bun.$`find ${rootDir} -type f`.text())
      .split("\n")
      .map(s => s.trim())
      .filter(Boolean);
    for (const file of files) {
      await chmod(file, fileMode).catch(() => {});
    }
  }
  for (const dir of dirs) {
    await chmod(dir, dirMode).catch(() => {});
  }
}

const { manifestPath, skipPreview, tscBin } = parseArgs(process.argv.slice(2));
const registry = await startLocalReleaseRegistry(manifestPath);
const workDir = await mkdtemp(path.join(os.tmpdir(), "zgui-release-verify-"));

const cleanPath = (process.env.PATH ?? "")
  .split(path.delimiter)
  .filter(entry => !entry.includes(".cargo"))
  .join(path.delimiter);

const cleanEnv: Record<string, string | undefined> = {
  ...process.env,
  PATH: cleanPath,
  BUN_INSTALL_CACHE_DIR: path.join(workDir, ".bun-cache"),
  ZGUI_JS_HOST_EXECUTABLE: undefined,
  ZGUI_JS_HOST_ROOT: undefined,
  NPM_CONFIG_REGISTRY: registry.url,
};

try {
  // 1. Install create-zgui-js from the local release registry and scaffold an external app.
  const scaffolderDir = path.join(workDir, "scaffolder");
  await mkdir(scaffolderDir, { recursive: true });
  await writeFile(
    path.join(scaffolderDir, "bunfig.toml"),
    `[install]\nregistry = ${JSON.stringify(registry.url)}\nlinker = "isolated"\n`,
  );
  await writeFile(
    path.join(scaffolderDir, "package.json"),
    JSON.stringify({
      name: "scaffolder-runner",
      private: true,
      dependencies: { "create-zgui-js": registry.manifest.version },
    }),
  );
  await runCommand([process.execPath, "install", "--linker", "isolated"], {
    cwd: scaffolderDir,
    env: cleanEnv,
  });

  const appDir = path.join(workDir, "demo-app");
  await runCommand(
    [path.join(scaffolderDir, "node_modules/.bin/create-zgui-js"), appDir, "--name", "demo-app"],
    { cwd: workDir, env: cleanEnv },
  );

  // Verify scaffolded package.json has NO direct host dependency and matches release version.
  const appPkg = JSON.parse(await readFile(path.join(appDir, "package.json"), "utf8")) as {
    dependencies: Record<string, string>;
    devDependencies: Record<string, string>;
  };
  if (Object.hasOwn(appPkg.dependencies, "@hewel/zgui-host-linux-x64")) {
    throw new Error(
      "Scaffolded app must not list @hewel/zgui-host-linux-x64 as a direct dependency",
    );
  }
  appPkg.devDependencies["typescript"] = "7.0.2";
  await writeFile(path.join(appDir, "package.json"), `${JSON.stringify(appPkg, null, 2)}\n`);
  await writeFile(
    path.join(appDir, "bunfig.toml"),
    `[install]\nregistry = ${JSON.stringify(registry.url)}\nlinker = "isolated"\n`,
  );

  // 2. Real bun install --linker isolated against the local release registry.
  await runCommand([process.execPath, "install", "--linker", "isolated"], {
    cwd: appDir,
    env: cleanEnv,
  });

  const zguiBin = path.join(appDir, "node_modules/.bin/zgui-js");
  const resolvedTsc = tscBin ?? path.join(appDir, "node_modules/.bin/tsc");

  // 3. Doctor must resolve @hewel/zgui-host-linux-x64 via platform-package with zero Cargo/env.
  const doctorOut = (
    await runCommand([zguiBin, "doctor", "--json"], { cwd: appDir, env: cleanEnv })
  ).stdout;
  const doctor = JSON.parse(doctorOut) as {
    ok: boolean;
    checks: { id: string; status: string; detail: string }[];
  };
  const hostCheck = doctor.checks.find(c => c.id === "zgui-host");
  if (
    !doctor.ok ||
    hostCheck?.status !== "ok" ||
    !hostCheck.detail.includes("[platform-package]")
  ) {
    throw new Error(`Doctor failed to resolve platform-package host:\n${doctorOut}`);
  }
  console.log("PASS doctor:", hostCheck.detail);

  // 4. Build and verify static components.css hoisting.
  await runCommand([zguiBin, "build"], { cwd: appDir, env: cleanEnv });
  const buildManifest = JSON.parse(
    await readFile(path.join(appDir, "dist/manifest.json"), "utf8"),
  ) as { styles: string[] };
  if (!buildManifest.styles.includes("components.css")) {
    throw new Error(
      `Expected static components.css hoisting, got: ${JSON.stringify(buildManifest.styles)}`,
    );
  }
  console.log("PASS build + components.css static hoisting");

  // 5. Package with --bytecode (zero explicit host flags) and verify licenses/ redistribution.
  const pkgOut = path.join(workDir, "packaged-demo");
  await runCommand([zguiBin, "package", "--bytecode", "--desktop-entry", "--out", pkgOut], {
    cwd: appDir,
    env: cleanEnv,
  });
  for (const rel of [
    "bin/zgui-host",
    "bundle/app.qjsb",
    "bundle/manifest.json",
    "demo-app",
    "demo-app.desktop",
    "package-manifest.json",
    "licenses/LICENSE",
    "licenses/NOTICE",
    "licenses/THIRD_PARTY_NOTICES.md",
  ]) {
    if (!existsSync(path.join(pkgOut, rel))) {
      throw new Error(`Missing packaged artifact: ${rel}`);
    }
  }
  console.log("PASS package --bytecode + licenses/ redistribution");

  // 6. Headless Vulkan preview capture (unless explicitly skipped).
  if (!skipPreview) {
    const previewOut = path.join(workDir, "preview-out");
    await runCommand([zguiBin, "preview", "--out", previewOut], { cwd: appDir, env: cleanEnv });
    const resultJson = JSON.parse(await readFile(path.join(previewOut, "result.json"), "utf8")) as {
      status?: string;
    };
    if (resultJson.status !== "ok") {
      throw new Error(`Headless preview returned non-ok status: ${JSON.stringify(resultJson)}`);
    }
    console.log("PASS headless preview capture");
  }

  // 7. Lint (oxlint), tsrs, and pinned official tsc (skipLibCheck false & true, exactOptionalPropertyTypes).
  await runCommand([path.join(appDir, "node_modules/.bin/oxlint"), "--deny-warnings"], {
    cwd: appDir,
    env: cleanEnv,
  });
  await runCommand([path.join(appDir, "node_modules/.bin/tsrs"), "--noEmit"], {
    cwd: appDir,
    env: cleanEnv,
  });
  for (const skipLibCheck of [false, true]) {
    const cfgPath = path.join(appDir, `tsconfig.verify-${String(skipLibCheck)}.json`);
    await writeFile(
      cfgPath,
      JSON.stringify({
        extends: "@hewel/zgui/tsconfig.native.json",
        compilerOptions: {
          skipLibCheck,
          exactOptionalPropertyTypes: true,
          lib: ["ES2022"],
          types: [],
        },
        include: ["src"],
      }),
    );
    await runCommand([resolvedTsc, "--project", cfgPath, "--noEmit"], {
      cwd: appDir,
      env: cleanEnv,
    });
  }
  console.log(
    "PASS oxlint + tsrs + official tsc (skipLibCheck=false/true, exactOptionalPropertyTypes=true)",
  );

  // 8. Lock node_modules read-only (0o555 dirs) and verify smoke + prebuilt inspect --ui.
  const nodeModulesDir = path.join(appDir, "node_modules");
  await chmodTree(nodeModulesDir, 0o555);
  try {
    await runCommand([zguiBin, "smoke"], { cwd: appDir, env: cleanEnv });
    console.log("PASS read-only zgui-js smoke");

    // Verify prebuilt inspect --ui executes cleanly against a dummy mock host without writing to node_modules.
    const probeHost = path.join(workDir, "probe-inspect-host");
    await writeFile(
      probeHost,
      `#!${process.execPath}
const args = process.argv.slice(2);
if (args.includes("--version")) {
  console.log(JSON.stringify({ name: "zgui-host", version: ${JSON.stringify(registry.manifest.version)}, hostApiVersion: 1, schemaVersion: 2, platform: "linux-x64" }));
  process.exit(0);
}
console.log(JSON.stringify({ args }));
`,
      { mode: 0o755 },
    );
    const inspectOut = (
      await runCommand(
        [zguiBin, "inspect", "--ui", "--host-executable", probeHost, "--socket", "/tmp/zgui.sock"],
        { cwd: appDir, env: cleanEnv },
      )
    ).stdout;
    const inspectParsed = JSON.parse(inspectOut) as { args: string[] };
    const bundleIdx = inspectParsed.args.indexOf("--bundle");
    const bundleDir = bundleIdx >= 0 ? inspectParsed.args[bundleIdx + 1] : undefined;
    if (
      !bundleDir ||
      !bundleDir.includes("inspector-bundle") ||
      !existsSync(path.join(bundleDir, "manifest.json"))
    ) {
      throw new Error(`Expected prebuilt inspector-bundle in read-only mode, got: ${inspectOut}`);
    }
    console.log("PASS read-only zgui-js inspect --ui");
  } finally {
    await chmodTree(nodeModulesDir, 0o755);
  }
} finally {
  registry.stop();
  await rm(workDir, { recursive: true, force: true });
}
