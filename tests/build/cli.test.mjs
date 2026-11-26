import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import {
  cp,
  lstat,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
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
import { withPackLock } from "./pack-lock.mjs";

const execute = promisify(execFile);
const cli = path.join(workspaceRoot, "packages/cli/src/cli.ts");
const createCli = path.join(workspaceRoot, "packages/create-zgui-js/src/cli.ts");
const stageScript = path.join(workspaceRoot, "tools/stage-host-package.ts");
const { version: workspaceHostVersion } = JSON.parse(
  readFileSync(path.join(workspaceRoot, "packages/host-linux-x64/package.json"), "utf8"),
);

function makeMockHostScript({
  version = workspaceHostVersion,
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
  assert.deepEqual(cliPkg.optionalDependencies, {
    "@hewel/zgui-host-linux-x64": "workspace:*",
  });
  assert.deepEqual(createPkg.bin, { "create-zgui-js": "./src/cli.ts" });
  assert.deepEqual(x64Pkg.os, ["linux"]);
  assert.deepEqual(x64Pkg.cpu, ["x64"]);
  assert.deepEqual(x64Pkg.libc, ["glibc"]);
  assert.equal(x64Pkg.private, undefined);
  assert.deepEqual(arm64Pkg.os, ["linux"]);
  assert.deepEqual(arm64Pkg.cpu, ["arm64"]);
  assert.equal(arm64Pkg.private, true);
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

  const explicitCargoRoot = path.join(dir, "explicit-cargo-root");
  await mkdir(path.join(explicitCargoRoot, "crates/zgui-host"), { recursive: true });
  await mkdir(path.join(explicitCargoRoot, "target/debug"), { recursive: true });
  await writeFile(path.join(explicitCargoRoot, "Cargo.toml"), "[workspace]\n");
  await writeFile(
    path.join(explicitCargoRoot, "crates/zgui-host/Cargo.toml"),
    '[package]\nname = "zgui-host"\n',
  );
  const explicitCargoHost = path.join(explicitCargoRoot, "target/debug/zgui-host");
  await writeFile(explicitCargoHost, makeMockHostScript({ tag: "cargo-explicit" }), {
    mode: 0o755,
  });

  // 1. Explicit --host-executable beats ZGUI_JS_HOST_EXECUTABLE, explicit hostRoot, and platform package.
  const r1 = await resolveNativeHost({
    hostExecutable: explicitHost,
    hostRoot: explicitCargoRoot,
    env: { ZGUI_JS_HOST_EXECUTABLE: envHost },
    cwd: consumerDir,
    platform: "linux",
    arch: "x64",
    root: consumerDir,
  });
  assert.equal(r1.source, "explicit");
  assert.equal(r1.executable, explicitHost);
  assert.equal(r1.abi?.hostApiVersion, 1);

  // 2. ZGUI_JS_HOST_EXECUTABLE beats explicit hostRoot and installed platform package.
  const r2 = await resolveNativeHost({
    hostRoot: explicitCargoRoot,
    env: { ZGUI_JS_HOST_EXECUTABLE: envHost },
    cwd: consumerDir,
    platform: "linux",
    arch: "x64",
    root: consumerDir,
  });
  assert.equal(r2.source, "env");
  assert.equal(r2.executable, envHost);

  // 3a. Explicit hostRoot beats installed staged platform package.
  const r3a = await resolveNativeHost({
    env: {},
    cwd: consumerDir,
    hostRoot: explicitCargoRoot,
    platform: "linux",
    arch: "x64",
    root: consumerDir,
    allowCargoBuild: false,
  });
  assert.equal(r3a.source, "cargo");
  assert.equal(r3a.cargoRoot, explicitCargoRoot);
  assert.equal(r3a.executable, explicitCargoHost);

  // Invalid explicit hostRoot fails closed rather than falling back to the platform package.
  await assert.rejects(
    resolveNativeHost({
      env: {},
      cwd: consumerDir,
      hostRoot: path.join(dir, "missing-cargo-root"),
      platform: "linux",
      arch: "x64",
      root: consumerDir,
      allowCargoBuild: false,
    }),
    /Explicit --host-root directory not found/,
  );
  const notACheckout = path.join(dir, "not-a-checkout");
  await mkdir(notACheckout, { recursive: true });
  await assert.rejects(
    resolveNativeHost({
      env: {},
      cwd: consumerDir,
      hostRoot: notACheckout,
      platform: "linux",
      arch: "x64",
      root: consumerDir,
      allowCargoBuild: false,
    }),
    /not a zgui-js Cargo checkout/,
  );

  // 3b. Installed staged platform package beats autodetected Cargo fallback.
  const r3b = await resolveNativeHost({
    env: {},
    cwd: consumerDir,
    platform: "linux",
    arch: "x64",
    root: workspaceRoot,
  });
  assert.equal(r3b.source, "platform-package");
  assert.equal(r3b.packageName, "@hewel/zgui-host-linux-x64");
  assert.equal(r3b.executable, staged.binaryPath);

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
      platform: "linux",
      arch: "x64",
      root: workspaceRoot,
    }),
    /integrity mismatch/,
  );

  // 6. Staged platform package without manifest fails closed BEFORE executing the binary (never falls back to Cargo).
  const markerPath = path.join(dir, "binary-executed.marker");
  await writeFile(
    staged.binaryPath,
    `#!${process.execPath}
import { writeFileSync } from "node:fs";
writeFileSync(${JSON.stringify(markerPath)}, "executed");
console.log(JSON.stringify({ name: "zgui-host", version: ${JSON.stringify(workspaceHostVersion)}, hostApiVersion: 1, schemaVersion: 2, platform: "linux-x64" }));
`,
    { mode: 0o755 },
  );
  await rm(staged.manifestPath);
  await assert.rejects(
    resolveNativeHost({
      env: {},
      cwd: consumerDir,
      platform: "linux",
      arch: "x64",
      root: workspaceRoot,
    }),
    /Missing host-manifest\.json/,
  );
  assert.equal(
    existsSync(markerPath),
    false,
    "unverified binary must never be executed when host-manifest.json is missing",
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
      const scaffoldPkg = JSON.parse(await readFile(path.join(appDir, "package.json"), "utf8"));
      assert.equal(scaffoldPkg.devDependencies["eslint-plugin-solid"], "0.20.1");
      assert.equal(scaffoldPkg.scripts.typecheck, "zgui-js typecheck");

      await linkWorkspaceSdk(appDir);
      assert.equal(existsSync(path.join(appDir, "src/App.module.css.d.ts")), false);
      await execute(process.execPath, [cli, "typecheck"], {
        cwd: appDir,
        timeout: 30000,
      });
      assert.equal(existsSync(path.join(appDir, "src/App.module.css.d.ts")), true);

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
  await writeFile(goodHost, makeMockHostScript(), { mode: 0o755 });
  await writeFile(badHost, makeMockHostScript({ hostApiVersion: 7 }), { mode: 0o755 });

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
    assert.deepEqual(pkgManifest.licenses, [
      "licenses/LICENSE",
      "licenses/NOTICE",
      "licenses/THIRD_PARTY_NOTICES.md",
    ]);
    for (const rel of pkgManifest.licenses) {
      assert.equal(existsSync(path.join(outDir, rel)), true, `expected ${rel}`);
    }

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

test("installed @hewel/zgui-cli stages dereferenced prebuilt inspector-bundle and fails closed if missing", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-cli-inspector-pack-"));
  t.after(() => rm(dir, { recursive: true, force: true }));

  const packScript = path.join(workspaceRoot, "packages/cli/pack-resources.ts");
  const stagedDir = path.join(workspaceRoot, "packages/cli/inspector-bundle");
  const installedCli = path.join(dir, "node_modules/@hewel/zgui-cli");
  await mkdir(path.join(installedCli, "src"), { recursive: true });

  await withPackLock(async () => {
    try {
      await execute(process.execPath, [packScript], {
        cwd: path.join(workspaceRoot, "packages/cli"),
      });

      assert.equal(existsSync(path.join(stagedDir, "manifest.json")), true);
      assert.equal(existsSync(path.join(stagedDir, "app.mjs")), true);

      // Ensure zero symlinks exist inside staged inspector-bundle.
      const checkNoSymlinks = async current => {
        for (const entry of await readdir(current, { withFileTypes: true })) {
          const full = path.join(current, entry.name);
          const info = await lstat(full);
          assert.equal(info.isSymbolicLink(), false, `unexpected symlink in bundle: ${full}`);
          if (info.isDirectory()) await checkNoSymlinks(full);
        }
      };
      await checkNoSymlinks(stagedDir);

      await cp(stagedDir, path.join(installedCli, "inspector-bundle"), { recursive: true });
    } finally {
      await execute(process.execPath, [packScript, "--clean"], {
        cwd: path.join(workspaceRoot, "packages/cli"),
      }).catch(() => {});
    }
  });

  const { resolveInspectorSource } = await import("../../packages/cli/src/inspect.ts");
  const resolved = resolveInspectorSource({
    cliDir: path.join(installedCli, "src"),
  });
  assert.deepEqual(resolved, {
    mode: "prebuilt",
    bundleDir: path.join(installedCli, "inspector-bundle"),
  });

  // Removing inspector-bundle from an installed CLI must fail closed and never fall back to cwd.
  await rm(path.join(installedCli, "inspector-bundle"), { recursive: true, force: true });
  await mkdir(path.join(dir, "apps/devtools/inspector"), { recursive: true });
  await writeFile(path.join(dir, "apps/devtools/inspector/package.json"), '{"name":"decoy"}\n');
  assert.throws(
    () => resolveInspectorSource({ cliDir: path.join(installedCli, "src") }),
    /Missing prebuilt Inspector bundle/,
  );
});

test("zgui-js typecheck syncs multi-root CSS Module types, supports --types-only, and surfaces tsrs/CSS errors", async t => {
  const ws = await mkdtemp(path.join(os.tmpdir(), "zgui-cli-typecheck-"));
  t.after(() => rm(ws, { recursive: true, force: true }));

  const appDir = path.join(ws, "apps/shell");
  const sdkSrc = path.join(ws, "packages/sdk/src");
  const extraDir = path.join(ws, "plugins/custom/src");
  await mkdir(path.join(appDir, "src"), { recursive: true });
  await mkdir(sdkSrc, { recursive: true });
  await mkdir(extraDir, { recursive: true });
  await symlink(path.join(workspaceRoot, "node_modules"), path.join(ws, "node_modules"), "dir");

  await writeFile(
    path.join(appDir, "tsconfig.json"),
    JSON.stringify({
      extends: "@hewel/zgui/tsconfig.native.json",
      compilerOptions: { allowImportingTsExtensions: true },
      include: ["src/**/*.tsx", "../../packages/sdk/src/zgui-*.tsx"],
    }),
  );
  await writeFile(path.join(sdkSrc, "zgui-card.module.css"), `.card { display: flex; }\n`);
  await writeFile(
    path.join(sdkSrc, "zgui-card.tsx"),
    `import styles from "./zgui-card.module.css";
export function Card() {
  return <box class={styles.card}><text>Card</text></box>;
}
`,
  );
  await writeFile(path.join(extraDir, "extra.module.css"), `.badge { padding: 4px; }\n`);
  await writeFile(
    path.join(appDir, "src/main.tsx"),
    `import { renderApp, type NativeNode } from "@hewel/zgui";
import { Card } from "../../../packages/sdk/src/zgui-card.tsx";
export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  return renderApp(() => <Card />, root);
}
`,
  );

  // --types-only generates .d.ts for tsconfig + --include without running tsrs.
  await writeFile(
    path.join(appDir, "src/broken-only-for-tsrs.tsx"),
    `const bad: number = "oops";\n`,
  );
  await execute(
    process.execPath,
    [
      cli,
      "typecheck",
      "--project",
      path.join(appDir, "tsconfig.json"),
      "--include",
      "plugins/*/src",
      "--types-only",
    ],
    { cwd: ws, timeout: 30000 },
  );
  assert.equal(existsSync(path.join(sdkSrc, "zgui-card.module.css.d.ts")), true);
  assert.equal(existsSync(path.join(extraDir, "extra.module.css.d.ts")), true);

  // Full typecheck fails on tsrs error, then passes once fixed.
  await assert.rejects(
    execute(process.execPath, [cli, "typecheck", "--project", path.join(appDir, "tsconfig.json")], {
      cwd: ws,
      timeout: 30000,
    }),
    /Type 'string' is not assignable to type 'number'/,
  );
  await rm(path.join(appDir, "src/broken-only-for-tsrs.tsx"));

  await execute(
    process.execPath,
    [cli, "typecheck", "--project", path.join(appDir, "tsconfig.json")],
    { cwd: ws, timeout: 30000 },
  );

  // Fails on unknown CSS class access in TSX.
  await writeFile(
    path.join(sdkSrc, "zgui-card.tsx"),
    `import styles from "./zgui-card.module.css";
export function Card() {
  return <box class={styles.missingClass}><text>Card</text></box>;
}
`,
  );
  await assert.rejects(
    execute(process.execPath, [cli, "typecheck", "--project", path.join(appDir, "tsconfig.json")], {
      cwd: ws,
      timeout: 30000,
    }),
    /missingClass/,
  );

  // Fails on unsupported CSS Modules feature.
  await writeFile(path.join(sdkSrc, "zgui-card.module.css"), `@import "./other.css";\n`);
  await assert.rejects(
    execute(process.execPath, [cli, "typecheck", "--project", path.join(appDir, "tsconfig.json")], {
      cwd: ws,
      timeout: 30000,
    }),
    /@import/,
  );
});
