import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { cp, mkdir, mkdtemp, readFile, rm, stat, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { promisify } from "node:util";
import {
  packageApplication,
  probeNativeHost,
  resolveNativeHost,
} from "../../packages/build/src/index.ts";
import {
  stageHostPackage,
  verifyHostPackage,
  workspaceRoot,
} from "../../packages/build/src/internal.ts";

const execute = promisify(execFile);
const cli = path.join(workspaceRoot, "packages/cli/src/cli.ts");
const createCli = path.join(workspaceRoot, "packages/create-zgui-js/src/cli.ts");
const stageScript = path.join(workspaceRoot, "tools/stage-host-package.ts");

function makeMockHostScript({
  version = "0.1.0",
  hostApiVersion = 1,
  schemaVersion = 2,
  platform = `${process.platform}-${process.arch}`,
  tag = "mock-host",
} = {}) {
  return `#!${process.execPath}
const args = process.argv.slice(2);
if (args[0] === "--version" || args[0] === "-V") {
  if (args.includes("--json")) {
    console.log(JSON.stringify({
      name: "zgui-host",
      version: ${JSON.stringify(version)},
      hostApiVersion: ${JSON.stringify(hostApiVersion)},
      schemaVersion: ${JSON.stringify(schemaVersion)},
      platform: ${JSON.stringify(platform)},
      engine: "QuickJS-NG",
      styleProfile: "lightningcss-v1",
    }));
  } else {
    console.log(${JSON.stringify(`zgui-host ${version} (hostApiVersion=${hostApiVersion} schemaVersion=${schemaVersion} platform=${platform})`)});
  }
  process.exit(0);
}
console.log(JSON.stringify({ tag: ${JSON.stringify(tag)}, cwd: process.cwd(), args }));
`;
}

async function linkWorkspaceSdk(appDir) {
  const scopeDir = path.join(appDir, "node_modules/@hewel");
  await mkdir(scopeDir, { recursive: true });
  await symlink(path.join(workspaceRoot, "packages/ui"), path.join(scopeDir, "zgui"), "dir");
  await symlink(
    path.join(workspaceRoot, "packages/services"),
    path.join(scopeDir, "zgui-services"),
    "dir",
  );
  await symlink(
    path.join(workspaceRoot, "node_modules/solid-js"),
    path.join(appDir, "node_modules/solid-js"),
    "dir",
  );
}

test("workspace CLI, scaffolder, and host package manifests follow ADR-0008 boundaries", async () => {
  const buildPkg = JSON.parse(
    await readFile(path.join(workspaceRoot, "packages/build/package.json"), "utf8"),
  );
  const cliPkg = JSON.parse(
    await readFile(path.join(workspaceRoot, "packages/cli/package.json"), "utf8"),
  );
  const createPkg = JSON.parse(
    await readFile(path.join(workspaceRoot, "packages/create-zgui-js/package.json"), "utf8"),
  );
  const x64Pkg = JSON.parse(
    await readFile(path.join(workspaceRoot, "packages/host-linux-x64/package.json"), "utf8"),
  );
  const arm64Pkg = JSON.parse(
    await readFile(path.join(workspaceRoot, "packages/host-linux-arm64/package.json"), "utf8"),
  );

  assert.equal(Object.hasOwn(buildPkg, "bin"), false);
  assert.deepEqual(cliPkg.bin, { "zgui-js": "./src/cli.ts" });
  assert.equal(Object.hasOwn(cliPkg.bin, "zgui"), false, "CLI must never squat on `zgui`");
  assert.equal(cliPkg.optionalDependencies["@hewel/zgui-host-linux-x64"], "workspace:*");
  assert.equal(cliPkg.optionalDependencies["@hewel/zgui-host-linux-arm64"], "workspace:*");
  assert.deepEqual(createPkg.bin, { "create-zgui-js": "./src/cli.ts" });
  assert.deepEqual(x64Pkg.os, ["linux"]);
  assert.deepEqual(x64Pkg.cpu, ["x64"]);
  assert.deepEqual(arm64Pkg.os, ["linux"]);
  assert.deepEqual(arm64Pkg.cpu, ["arm64"]);
});

test("deterministic host resolution honors precedence, verifies staged packages, and rejects ABI/integrity mismatches fail-closed", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-host-resolve-"));
  t.after(() => rm(dir, { recursive: true, force: true }));

  const explicitHost = path.join(dir, "explicit-host");
  const envHost = path.join(dir, "env-host");
  const badAbiHost = path.join(dir, "bad-abi-host");
  const pkgSourceHost = path.join(dir, "pkg-source-host");
  await writeFile(explicitHost, makeMockHostScript({ tag: "explicit" }), { mode: 0o755 });
  await writeFile(envHost, makeMockHostScript({ tag: "env" }), { mode: 0o755 });
  await writeFile(badAbiHost, makeMockHostScript({ hostApiVersion: 99, tag: "bad-abi" }), {
    mode: 0o755,
  });
  await writeFile(pkgSourceHost, makeMockHostScript({ tag: "pkg" }), { mode: 0o755 });

  // Stage a mock @hewel/zgui-host-linux-x64 inside an isolated consumer directory.
  const consumerDir = path.join(dir, "consumer");
  const hostPkgDir = path.join(consumerDir, "node_modules/@hewel/zgui-host-linux-x64");
  await mkdir(hostPkgDir, { recursive: true });
  await cp(
    path.join(workspaceRoot, "packages/host-linux-x64/package.json"),
    path.join(hostPkgDir, "package.json"),
  );
  await cp(
    path.join(workspaceRoot, "packages/host-linux-x64/index.js"),
    path.join(hostPkgDir, "index.js"),
  );
  await writeFile(path.join(consumerDir, "package.json"), '{"name":"consumer","type":"module"}\n');

  const staged = await stageHostPackage({
    platform: "linux-x64",
    binary: pkgSourceHost,
    packageDir: hostPkgDir,
  });
  const verified = await verifyHostPackage(hostPkgDir);
  assert.equal(verified.manifest.sha256, staged.manifest.sha256);
  assert.equal(verified.abi?.hostApiVersion, 1);

  // Also verify tools/stage-host-package.ts CLI --verify succeeds.
  const { stdout: verifyOut } = await execute(process.execPath, [
    stageScript,
    "--verify",
    hostPkgDir,
  ]);
  assert.match(verifyOut, /Verified @hewel\/zgui-host-linux-x64/);

  // 1. Explicit --host-executable beats ZGUI_JS_HOST_EXECUTABLE and platform package.
  const r1 = await resolveNativeHost({
    hostExecutable: explicitHost,
    env: { ZGUI_JS_HOST_EXECUTABLE: envHost },
    cwd: consumerDir,
    platform: "linux",
    arch: "x64",
    root: consumerDir,
  });
  assert.equal(r1.source, "explicit");
  assert.equal(r1.executable, explicitHost);
  assert.equal(r1.abi?.hostApiVersion, 1);

  // 2. ZGUI_JS_HOST_EXECUTABLE beats installed platform package.
  const r2 = await resolveNativeHost({
    env: { ZGUI_JS_HOST_EXECUTABLE: envHost },
    cwd: consumerDir,
    platform: "linux",
    arch: "x64",
    root: consumerDir,
  });
  assert.equal(r2.source, "env");
  assert.equal(r2.executable, envHost);

  // 3. Installed staged platform package beats Cargo fallback (even if hostRoot is passed).
  const r3 = await resolveNativeHost({
    env: {},
    cwd: consumerDir,
    hostRoot: workspaceRoot,
    platform: "linux",
    arch: "x64",
    root: consumerDir,
  });
  assert.equal(r3.source, "platform-package");
  assert.equal(r3.packageName, "@hewel/zgui-host-linux-x64");
  assert.equal(r3.executable, staged.binaryPath);

  // 4. Explicit ABI mismatch fails closed.
  assert.throws(() => probeNativeHost(badAbiHost), /ABI mismatch.*hostApiVersion=99/);
  await assert.rejects(
    resolveNativeHost({
      hostExecutable: badAbiHost,
      cwd: consumerDir,
      root: consumerDir,
    }),
    /ABI mismatch/,
  );

  // 5. Staged platform package with tampered binary fails closed on SHA-256 mismatch.
  await writeFile(staged.binaryPath, makeMockHostScript({ hostApiVersion: 99 }), { mode: 0o755 });
  await assert.rejects(
    resolveNativeHost({
      env: {},
      cwd: consumerDir,
      hostRoot: workspaceRoot,
      platform: "linux",
      arch: "x64",
      root: consumerDir,
    }),
    /integrity mismatch/,
  );

  // 6. Staged platform package without manifest but wrong ABI fails closed (never falls back to Cargo).
  await rm(staged.manifestPath);
  await assert.rejects(
    resolveNativeHost({
      env: {},
      cwd: consumerDir,
      hostRoot: workspaceRoot,
      platform: "linux",
      arch: "x64",
      root: consumerDir,
    }),
    /ABI mismatch/,
  );

  // 7. Missing host outside Cargo checkout returns actionable remediation error.
  const emptyDir = path.join(dir, "no-host");
  await mkdir(emptyDir, { recursive: true });
  await assert.rejects(
    resolveNativeHost({
      env: {},
      cwd: emptyDir,
      root: emptyDir,
      platform: "linux",
      arch: "x64",
    }),
    error => {
      assert.match(error.message, /@hewel\/zgui-host-linux-x64/);
      assert.match(error.message, /ZGUI_JS_HOST_EXECUTABLE/);
      assert.match(error.message, /--host-root/);
      return true;
    },
  );
});

test(
  "zgui-js init and create-zgui-js scaffold desktop-app and wayland-shell templates that build cleanly",
  { timeout: 30000 },
  async t => {
    const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-init-"));
    t.after(() => rm(dir, { recursive: true, force: true }));

    const desktopDir = path.join(dir, "my-desktop");
    const { stdout: initOut } = await execute(process.execPath, [cli, "init", desktopDir], {
      cwd: dir,
    });
    assert.match(initOut, /Scaffolded zgui-js desktop-app "my-desktop"/);

    // Refuses non-empty directory without --force.
    await assert.rejects(
      execute(process.execPath, [cli, "init", desktopDir], { cwd: dir }),
      /not empty/,
    );

    const shellDir = path.join(dir, "my-shell");
    const { stdout: createOut } = await execute(
      process.execPath,
      [createCli, shellDir, "--template", "wayland-shell"],
      { cwd: dir },
    );
    assert.match(createOut, /Scaffolded zgui-js wayland-shell "my-shell"/);

    for (const [appDir, expectedCap] of [
      [desktopDir, false],
      [shellDir, true],
    ]) {
      assert.ok(existsSync(path.join(appDir, "tsconfig.json")));
      assert.ok(existsSync(path.join(appDir, ".oxlintrc.json")));
      assert.ok(existsSync(path.join(appDir, "zgui.capabilities.json")));
      assert.ok(existsSync(path.join(appDir, "src/App.module.css")));
      assert.ok(existsSync(path.join(appDir, "src/main.tsx")));

      await linkWorkspaceSdk(appDir);
      const { stdout: buildOut } = await execute(process.execPath, [cli, "build"], {
        cwd: appDir,
        timeout: 30000,
      });
      assert.match(buildOut, /Published .* sha256:/);
      const manifest = JSON.parse(await readFile(path.join(appDir, "dist/manifest.json"), "utf8"));
      assert.equal(manifest.schemaVersion, 2);
      assert.equal(manifest.hostApiVersion, 1);
      if (expectedCap) {
        assert.equal(manifest.capabilities?.shellSurfaces, true);
        assert.equal(manifest.capabilities?.controlChannel, true);
      } else {
        assert.equal(manifest.window?.width, 960);
      }
    }
  },
);

test("zgui-js doctor --json reports structured checks and fails closed on ABI mismatch", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-doctor-"));
  t.after(() => rm(dir, { recursive: true, force: true }));

  const goodHost = path.join(dir, "good-host");
  const badHost = path.join(dir, "bad-host");
  await writeFile(goodHost, makeMockHostScript({ version: "0.1.0", hostApiVersion: 1 }), {
    mode: 0o755,
  });
  await writeFile(badHost, makeMockHostScript({ version: "0.1.0", hostApiVersion: 7 }), {
    mode: 0o755,
  });

  const { stdout } = await execute(
    process.execPath,
    [cli, "doctor", "--json", "--host-executable", goodHost],
    { cwd: workspaceRoot },
  );
  const report = JSON.parse(stdout);
  assert.equal(report.schemaVersion, 1);
  assert.equal(report.ok, true);
  const byId = new Map(report.checks.map(item => [item.id, item]));
  for (const id of ["bun", "zgui-host", "wayland", "vulkan", "xdg-portal", "tsrs", "oxlint"]) {
    assert.ok(byId.has(id), `expected doctor check ${id}`);
  }
  assert.equal(byId.get("bun").status, "ok");
  assert.equal(byId.get("zgui-host").status, "ok");
  assert.match(byId.get("zgui-host").detail, /hostApiVersion=1 schemaVersion=2/);
  assert.equal(byId.get("tsrs").status, "ok");

  await assert.rejects(
    execute(process.execPath, [cli, "doctor", "--json", "--host-executable", badHost], {
      cwd: workspaceRoot,
    }),
    error => {
      assert.equal(error.code, 1);
      const failedReport = JSON.parse(error.stdout);
      assert.equal(failedReport.ok, false);
      const hostCheck = failedReport.checks.find(item => item.id === "zgui-host");
      assert.equal(hostCheck?.status, "fail");
      assert.match(hostCheck?.detail ?? "", /ABI mismatch/);
      return true;
    },
  );
});

test("zgui-js inspect forwards subcommand arguments to the resolved zgui-host", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-inspect-"));
  t.after(() => rm(dir, { recursive: true, force: true }));

  const host = path.join(dir, "inspect-host");
  await writeFile(host, makeMockHostScript({ tag: "inspect-proxy" }), { mode: 0o755 });

  const { stdout } = await execute(
    process.execPath,
    [
      cli,
      "inspect",
      "--host-executable",
      host,
      "--socket",
      "/tmp/zgui-test.sock",
      "--op",
      "snapshot",
      "--surface",
      "main",
    ],
    { cwd: dir },
  );
  const parsed = JSON.parse(stdout);
  assert.equal(parsed.tag, "inspect-proxy");
  assert.deepEqual(parsed.args, [
    "inspect",
    "--socket",
    "/tmp/zgui-test.sock",
    "--op",
    "snapshot",
    "--surface",
    "main",
  ]);
});

test("zgui-js package resolves explicit host and output paths from the caller's cwd", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-package-paths-"));
  t.after(() => rm(dir, { recursive: true, force: true }));

  const appDir = path.join(dir, "packaged-demo");
  await execute(process.execPath, [cli, "init", "packaged-demo"], { cwd: dir });
  await linkWorkspaceSdk(appDir);

  const host = makeMockHostScript({ tag: "caller-host" });
  await writeFile(path.join(dir, "relative-host"), host, { mode: 0o755 });
  await execute(
    process.execPath,
    [
      cli,
      "package",
      "--app",
      "packaged-demo",
      "--host-executable",
      "./relative-host",
      "--out",
      "./release",
    ],
    { cwd: dir },
  );

  assert.equal(await readFile(path.join(dir, "release/bin/zgui-host"), "utf8"), host);
  assert.equal(existsSync(path.join(appDir, "release")), false);
});

test(
  "zgui-js package produces a self-contained runnable Linux app directory with zero Cargo invocation",
  { timeout: 30000 },
  async t => {
    const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-package-"));
    t.after(() => rm(dir, { recursive: true, force: true }));

    const appDir = path.join(dir, "packaged-demo");
    await execute(process.execPath, [cli, "init", appDir], { cwd: dir });
    await linkWorkspaceSdk(appDir);

    const mockHost = path.join(dir, "mock-zgui-host");
    await writeFile(mockHost, makeMockHostScript({ tag: "packaged-host" }), { mode: 0o755 });

    const outDir = path.join(dir, "release-out");
    // Empty PATH proves zero Cargo invocation when a prebuilt host is resolved.
    const packaged = await packageApplication("packaged-demo", {
      root: workspaceRoot,
      appDirectory: appDir,
      out: outDir,
      hostExecutable: mockHost,
      desktopEntry: true,
    });

    assert.equal(packaged.out, outDir);
    assert.equal(packaged.host.source, "explicit");
    assert.ok(existsSync(packaged.executable));
    assert.ok(existsSync(packaged.launcher));
    assert.ok(existsSync(path.join(packaged.bundle, "manifest.json")));
    assert.ok(existsSync(path.join(packaged.bundle, "app.mjs")));
    assert.ok(packaged.desktopEntry && existsSync(packaged.desktopEntry));

    const launcherMode = (await stat(packaged.launcher)).mode & 0o111;
    assert.notEqual(launcherMode, 0, "launcher must be executable");

    const desktopText = await readFile(packaged.desktopEntry, "utf8");
    assert.match(desktopText, /\[Desktop Entry\]/);
    assert.match(desktopText, /StartupWMClass=dev\.example\.packaged-demo/);

    const pkgManifest = JSON.parse(
      await readFile(path.join(outDir, "package-manifest.json"), "utf8"),
    );
    assert.equal(pkgManifest.schemaVersion, 1);
    assert.equal(pkgManifest.hostApiVersion, 1);
    assert.equal(pkgManifest.buildId, packaged.manifest.buildId);

    // Execute the generated POSIX launcher wrapper and verify it boots bin/zgui-host --bundle bundle/.
    const { stdout: runOut } = await execute(packaged.launcher, ["--exit-after-ms", "1"], {
      cwd: dir,
    });
    const boot = JSON.parse(runOut);
    assert.equal(boot.tag, "packaged-host");
    assert.deepEqual(boot.args, ["--bundle", path.join(outDir, "bundle"), "--exit-after-ms", "1"]);

    // If the real zgui-host binary is already built in target/debug, also verify real binary ABI + packaging.
    const realHost = path.join(workspaceRoot, "target/debug/zgui-host");
    if (existsSync(realHost)) {
      const cliOutDir = path.join(dir, "real-release");
      const { stdout: cliPackOut } = await execute(
        process.execPath,
        [
          cli,
          "package",
          appDir,
          "--out",
          cliOutDir,
          "--host-executable",
          realHost,
          "--desktop-entry",
        ],
        {
          cwd: dir,
          env: { ...process.env, PATH: path.dirname(process.execPath) },
        },
      );
      assert.match(cliPackOut, /Packaged .* host=explicit/);
      const abi = probeNativeHost(path.join(cliOutDir, "bin/zgui-host"));
      assert.equal(abi.hostApiVersion, 1);
      assert.equal(abi.schemaVersion, 2);
      const { stdout: helpOut } = await execute(path.join(cliOutDir, "packaged-demo"), ["--help"], {
        cwd: dir,
      });
      assert.match(helpOut, /Usage: zgui-host/);
      assert.equal(abi.bytecodeFormat, "qjs-bytecode-v1");
      assert.match(abi.quickjsAbi ?? "", /^qjs-ng-/);

      const bcOutDir = path.join(dir, "bytecode-release");
      const { stdout: bcPackOut } = await execute(
        process.execPath,
        [cli, "package", appDir, "--out", bcOutDir, "--bytecode", "--host-executable", realHost],
        {
          cwd: dir,
          env: { ...process.env, PATH: path.dirname(process.execPath) },
        },
      );
      assert.match(bcPackOut, /Packaged .* host=explicit/);
      const bcManifest = JSON.parse(
        await readFile(path.join(bcOutDir, "bundle/manifest.json"), "utf8"),
      );
      assert.equal(bcManifest.entry, "app.qjsb");
      assert.equal(bcManifest.bytecodeFormat, "qjs-bytecode-v1");
      assert.equal(bcManifest.quickjsAbi, abi.quickjsAbi);
      assert.equal(existsSync(path.join(bcOutDir, "bundle/app.qjsb")), true);
      assert.equal(existsSync(path.join(bcOutDir, "bundle/app.mjs")), false);
    }
  },
);

test("installed @hewel/zgui-cli stages apps/devtools/inspector so inspect --ui works outside a repo checkout", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-cli-inspector-pack-"));
  t.after(() => rm(dir, { recursive: true, force: true }));

  const packScript = path.join(workspaceRoot, "packages/cli/pack-resources.ts");
  const stagedDir = path.join(workspaceRoot, "packages/cli/inspector");
  await execute(process.execPath, [packScript], { cwd: path.join(workspaceRoot, "packages/cli") });
  t.after(async () => {
    await execute(process.execPath, [packScript, "--clean"], {
      cwd: path.join(workspaceRoot, "packages/cli"),
    }).catch(() => {});
  });

  assert.equal(existsSync(path.join(stagedDir, "package.json")), true);
  assert.equal(existsSync(path.join(stagedDir, "src/main.tsx")), true);
  assert.equal(existsSync(path.join(stagedDir, "dist")), false);

  // Copy only the published CLI files (`src` + `inspector`) into an isolated external directory.
  const installedCli = path.join(dir, "node_modules/@hewel/zgui-cli");
  await mkdir(path.join(installedCli, "src"), { recursive: true });
  await cp(stagedDir, path.join(installedCli, "inspector"), { recursive: true });

  const { findInspectorAppDir } = await import("../../packages/cli/src/inspect.ts");
  const resolved = findInspectorAppDir({
    cliDir: path.join(installedCli, "src"),
    cwd: dir,
  });
  assert.equal(resolved, path.join(installedCli, "inspector"));
});
