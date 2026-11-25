import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import {
  chmod,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  realpath,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { promisify } from "node:util";
import { watchApplication } from "../../packages/build/src/index.ts";
import { workspaceRoot } from "../../packages/build/src/internal.ts";
import { withPackLock } from "./pack-lock.mjs";

const execute = promisify(execFile);
const releaseTool = path.join(workspaceRoot, "tools/release.ts");
const pinnedTsc = path.join(workspaceRoot, "node_modules/.bin/tsc");
const { version: workspaceHostVersion } = JSON.parse(
  readFileSync(path.join(workspaceRoot, "packages/host-linux-x64/package.json"), "utf8"),
);

function makeMockReleaseHost() {
  return `#!${process.execPath}
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
if (args[0] === "--version" || args[0] === "-V") {
  console.log(JSON.stringify({
    name: "zgui-host",
    version: ${JSON.stringify(workspaceHostVersion)},
    hostApiVersion: 1,
    schemaVersion: 2,
    platform: "linux-x64",
    engine: "QuickJS-NG",
    bytecodeFormat: "qjs-bytecode-v1",
    quickjsAbi: "qjs-ng-mock-1",
    styleProfile: "lightningcss-v1",
  }));
  process.exit(0);
}
if (args[0] === "compile-bytecode") {
  const input = args[args.indexOf("--input") + 1];
  const output = args[args.indexOf("--output") + 1];
  const source = readFileSync(input);
  mkdirSync(path.dirname(output), { recursive: true });
  writeFileSync(output, Buffer.concat([Buffer.from("QJSB_MOCK\\0"), source]));
  process.exit(0);
}
console.log(JSON.stringify({ tag: "mock-release-host", cwd: process.cwd(), args }));
`;
}

async function indexLocalBunStore() {
  const bunStore = path.join(workspaceRoot, "node_modules/.bun");
  const entries = await readdir(bunStore, { withFileTypes: true });
  const index = new Map();

  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name === "node_modules") continue;
    const nmDir = path.join(bunStore, entry.name, "node_modules");
    if (!existsSync(nmDir)) continue;
    for (const item of await readdir(nmDir, { withFileTypes: true })) {
      if (item.name.startsWith("@")) {
        const scopeDir = path.join(nmDir, item.name);
        for (const scoped of await readdir(scopeDir, { withFileTypes: true })) {
          const pkgDir = path.join(scopeDir, scoped.name);
          const pkgJsonPath = path.join(pkgDir, "package.json");
          if (!existsSync(pkgJsonPath)) continue;
          const pkgJson = JSON.parse(await readFile(pkgJsonPath, "utf8"));
          if (!pkgJson.name || !pkgJson.version) continue;
          let versions = index.get(pkgJson.name);
          if (!versions) {
            versions = new Map();
            index.set(pkgJson.name, versions);
          }
          if (!versions.has(pkgJson.version)) {
            versions.set(pkgJson.version, { dir: pkgDir, pkgJson });
          }
        }
      } else {
        const pkgDir = path.join(nmDir, item.name);
        const pkgJsonPath = path.join(pkgDir, "package.json");
        if (!existsSync(pkgJsonPath)) continue;
        const pkgJson = JSON.parse(await readFile(pkgJsonPath, "utf8"));
        if (!pkgJson.name || !pkgJson.version) continue;
        let versions = index.get(pkgJson.name);
        if (!versions) {
          versions = new Map();
          index.set(pkgJson.name, versions);
        }
        if (!versions.has(pkgJson.version)) {
          versions.set(pkgJson.version, { dir: pkgDir, pkgJson });
        }
      }
    }
  }
  return index;
}

async function packDirectoryTarball(dir, tmpRoot, slug) {
  const outPath = path.join(tmpRoot, `${slug}.tgz`);
  await execute("tar", ["-czf", outPath, "--transform", "s,^\\.,package,", "."], {
    cwd: dir,
  });
  const bytes = await readFile(outPath);
  const shasum = createHash("sha1").update(bytes).digest("hex");
  const integrity = `sha512-${createHash("sha512").update(bytes).digest("base64")}`;
  return { bytes, shasum, integrity };
}

async function startOfflineRegistry(manifestPath, tmpRoot) {
  const manifestDir = path.dirname(manifestPath);
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  const releasePackages = new Map();

  for (const pkg of manifest.packages) {
    const tarballPath = path.join(manifestDir, pkg.tarball);
    const bytes = await readFile(tarballPath);
    const { stdout: pkgJsonText } = await execute("tar", [
      "-xOf",
      tarballPath,
      "package/package.json",
    ]);
    const pkgJson = JSON.parse(pkgJsonText);
    const shasum = createHash("sha1").update(bytes).digest("hex");
    releasePackages.set(pkg.name, {
      name: pkg.name,
      version: pkg.version,
      tarballName: pkg.tarball,
      bytes,
      shasum,
      integrity: pkg.integrity,
      pkgJson,
    });
  }

  const storeIndex = await indexLocalBunStore();
  const packedStoreCache = new Map();
  let tamperPackageName = null;

  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(req) {
      const url = new URL(req.url);
      if (url.pathname.startsWith("/-/release/")) {
        const filename = decodeURIComponent(url.pathname.slice("/-/release/".length));
        for (const entry of releasePackages.values()) {
          if (entry.tarballName === filename) {
            return new Response(entry.bytes, {
              headers: { "content-type": "application/octet-stream" },
            });
          }
        }
        return new Response("Not Found", { status: 404 });
      }
      if (url.pathname.startsWith("/-/store/")) {
        const key = decodeURIComponent(url.pathname.slice("/-/store/".length));
        const cached = packedStoreCache.get(key);
        if (cached) {
          return new Response(cached.bytes, {
            headers: { "content-type": "application/octet-stream" },
          });
        }
        return new Response("Not Found", { status: 404 });
      }

      const pkgName = decodeURIComponent(url.pathname.slice(1));
      const origin = `http://127.0.0.1:${server.port}`;

      const rel = releasePackages.get(pkgName);
      if (rel) {
        const integrity =
          tamperPackageName === pkgName ? "sha512-TAMPERED_INTEGRITY_HASH==" : rel.integrity;
        return Response.json({
          name: rel.name,
          "dist-tags": { latest: rel.version },
          versions: {
            [rel.version]: {
              ...rel.pkgJson,
              dist: {
                tarball: `${origin}/-/release/${encodeURIComponent(rel.tarballName)}`,
                shasum: rel.shasum,
                integrity,
              },
            },
          },
        });
      }

      const storeVersions = storeIndex.get(pkgName);
      if (storeVersions) {
        const versions = {};
        let latest = "";
        for (const [ver, info] of storeVersions) {
          latest = ver;
          const cacheKey = `${pkgName.replace(/^@/, "").replace(/\//g, "-")}-${ver}`;
          let packed = packedStoreCache.get(cacheKey);
          if (!packed) {
            packed = await packDirectoryTarball(info.dir, tmpRoot, cacheKey);
            packedStoreCache.set(cacheKey, packed);
          }
          versions[ver] = {
            ...info.pkgJson,
            dist: {
              tarball: `${origin}/-/store/${encodeURIComponent(cacheKey)}`,
              shasum: packed.shasum,
              integrity: packed.integrity,
            },
          };
        }
        return Response.json({
          name: pkgName,
          "dist-tags": { latest },
          versions,
        });
      }

      return new Response("Not Found", { status: 404 });
    },
  });

  return {
    manifest,
    url: `http://127.0.0.1:${server.port}`,
    setTampered(name) {
      tamperPackageName = name;
    },
    stop: () => server.stop(true),
  };
}

async function setDirModeRecursive(rootDir, mode) {
  const dirs = [];
  const walk = async current => {
    dirs.push(current);
    for (const entry of await readdir(current, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        await walk(path.join(current, entry.name));
      }
    }
  };
  await walk(rootDir);
  for (const dir of dirs.reverse()) {
    await chmod(dir, mode);
  }
}

test(
  "packed 6-package release installs offline via bun --linker isolated and passes full downstream contract",
  { timeout: 30000 },
  async t => {
    const tmpDir = await mkdtemp(path.join(os.tmpdir(), "zgui-npm-e2e-"));
    t.after(() => rm(tmpDir, { recursive: true, force: true }));

    const artifactsDir = path.join(tmpDir, "release-artifacts");
    const storeTarDir = path.join(tmpDir, "store-tarballs");
    const bunCacheDir = path.join(tmpDir, "bun-cache");
    await mkdir(storeTarDir, { recursive: true });
    await mkdir(bunCacheDir, { recursive: true });

    const mockHost = path.join(tmpDir, "mock-zgui-host");
    await writeFile(mockHost, makeMockReleaseHost(), { mode: 0o755 });

    await withPackLock(async () => {
      await execute(
        process.execPath,
        [releaseTool, "pack", "--out", artifactsDir, "--host-binary", mockHost],
        { cwd: workspaceRoot },
      );
    });

    const manifestPath = path.join(artifactsDir, "release-manifest.json");
    const registry = await startOfflineRegistry(manifestPath, storeTarDir);
    t.after(() => registry.stop());

    const cleanPath = (process.env.PATH ?? "")
      .split(path.delimiter)
      .filter(entry => !entry.includes(".cargo"))
      .join(path.delimiter);
    const cleanEnv = {
      ...process.env,
      PATH: cleanPath,
      BUN_INSTALL_CACHE_DIR: bunCacheDir,
      NPM_CONFIG_REGISTRY: registry.url,
    };
    delete cleanEnv.ZGUI_JS_HOST_EXECUTABLE;
    delete cleanEnv.ZGUI_JS_HOST_ROOT;

    // 1. Install create-zgui-js from the local HTTP registry and scaffold an external app.
    const scaffolderDir = path.join(tmpDir, "scaffolder");
    await mkdir(scaffolderDir, { recursive: true });
    await writeFile(
      path.join(scaffolderDir, "bunfig.toml"),
      `[install]\nregistry = ${JSON.stringify(registry.url)}\nlinker = "isolated"\n`,
    );
    await writeFile(
      path.join(scaffolderDir, "package.json"),
      JSON.stringify({
        name: "scaffolder-test",
        private: true,
        dependencies: { "create-zgui-js": registry.manifest.version },
      }),
    );
    await execute(process.execPath, ["install", "--linker", "isolated"], {
      cwd: scaffolderDir,
      env: cleanEnv,
    });

    const appDir = path.join(tmpDir, "consumer-app");
    await execute(
      path.join(scaffolderDir, "node_modules/.bin/create-zgui-js"),
      [appDir, "--name", "consumer-app"],
      { cwd: tmpDir, env: cleanEnv },
    );

    const scaffoldPkg = JSON.parse(await readFile(path.join(appDir, "package.json"), "utf8"));
    assert.equal(Object.hasOwn(scaffoldPkg.dependencies, "@hewel/zgui-host-linux-x64"), false);
    assert.equal(Object.hasOwn(scaffoldPkg.devDependencies, "@hewel/zgui-host-linux-x64"), false);
    const oxlintCfg = JSON.parse(await readFile(path.join(appDir, ".oxlintrc.json"), "utf8"));
    assert.deepEqual(oxlintCfg.extends, ["./node_modules/@hewel/zgui/oxlint/recommended.json"]);

    await writeFile(
      path.join(appDir, "bunfig.toml"),
      `[install]\nregistry = ${JSON.stringify(registry.url)}\nlinker = "isolated"\n`,
    );

    // 2. Real bun install --linker isolated against the local temporary registry.
    await execute(process.execPath, ["install", "--linker", "isolated"], {
      cwd: appDir,
      env: cleanEnv,
    });

    const zguiBin = path.join(appDir, "node_modules/.bin/zgui-js");

    // 3. Verify doctor resolves @hewel/zgui-host-linux-x64 via platform-package with zero Cargo/env.
    const { stdout: doctorOut } = await execute(zguiBin, ["doctor", "--json"], {
      cwd: appDir,
      env: cleanEnv,
    });
    const doctor = JSON.parse(doctorOut);
    assert.equal(doctor.ok, true);
    const hostCheck = doctor.checks.find(c => c.id === "zgui-host");
    assert.equal(hostCheck?.status, "ok");
    assert.match(hostCheck?.detail ?? "", /\[platform-package\]/);

    // Expand src/main.tsx to cover all public SDK entry points under strict typing + hoisting.
    await writeFile(
      path.join(appDir, "src/main.tsx"),
      `import { createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import { Button, ThemeProvider } from "@hewel/zgui/components";
import { CHECK, Icon } from "@hewel/zgui/icons";
import { createOverlayState } from "@hewel/zgui/primitives";
import { createRouter, Outlet, RouterProvider } from "@hewel/zgui/router";
import { getResolvedCapabilities } from "@hewel/zgui-services";
import styles from "./App.module.css";

export const hostApiVersion = 1;

function Home() {
  const [count, setCount] = createSignal(0);
  const overlay = createOverlayState({ defaultOpen: false });
  const caps = getResolvedCapabilities();
  return (
    <box class={styles.shell}>
      <text>{caps.imagePrefixes.length + count() + (overlay.open() ? 1 : 0)}</text>
      <Button onClick={() => setCount(n => n + 1)}>
        <Icon icon={CHECK} label="Increment" />
      </Button>
    </box>
  );
}

function App() {
  const router = createRouter([{ path: "/", component: Home }] as const);
  return (
    <ThemeProvider>
      <RouterProvider router={router}>
        <Outlet />
      </RouterProvider>
    </ThemeProvider>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <App />, root);
}
`,
    );

    // 4. zgui-js build + static components.css hoisting.
    await execute(zguiBin, ["build"], { cwd: appDir, env: cleanEnv });
    const buildManifest = JSON.parse(
      await readFile(path.join(appDir, "dist/manifest.json"), "utf8"),
    );
    assert.equal(buildManifest.styles.includes("components.css"), true);
    const componentsCss = await readFile(path.join(appDir, "dist/components.css"), "utf8");
    assert.match(componentsCss, /@layer zgui\.components/);
    assert.match(componentsCss, /\.zs-/);

    // 5. zgui-js package --bytecode with ZERO explicit host flags/env/Cargo + licenses/.
    const pkgOut = path.join(tmpDir, "packaged-app");
    const { stdout: packOut } = await execute(
      zguiBin,
      ["package", "--bytecode", "--desktop-entry", "--out", pkgOut],
      { cwd: appDir, env: cleanEnv },
    );
    assert.match(packOut, /host=platform-package/);
    const bcManifest = JSON.parse(
      await readFile(path.join(pkgOut, "bundle/manifest.json"), "utf8"),
    );
    assert.equal(bcManifest.entry, "app.qjsb");
    assert.equal(bcManifest.bytecodeFormat, "qjs-bytecode-v1");
    assert.equal(existsSync(path.join(pkgOut, "bundle/app.qjsb")), true);
    assert.equal(existsSync(path.join(pkgOut, "licenses/LICENSE")), true);
    assert.equal(existsSync(path.join(pkgOut, "licenses/NOTICE")), true);
    assert.equal(existsSync(path.join(pkgOut, "licenses/THIRD_PARTY_NOTICES.md")), true);

    // 6. Typecheck with both tsrs and pinned official tsc (zero DOM, zero paths, exactOptionalPropertyTypes, skipLibCheck false & true).
    await execute(path.join(appDir, "node_modules/.bin/tsrs"), ["--noEmit"], {
      cwd: appDir,
      env: cleanEnv,
    });
    for (const skipLibCheck of [false, true]) {
      const cfgPath = path.join(appDir, `tsconfig.tsc-${String(skipLibCheck)}.json`);
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
      await execute(pinnedTsc, ["--project", cfgPath, "--noEmit"], {
        cwd: appDir,
        env: cleanEnv,
      });
    }

    // 7. oxlint passes on valid code and rejects forbidden raw setTimeout via @hewel/zgui/oxlint/recommended.json.
    await execute(path.join(appDir, "node_modules/.bin/oxlint"), ["--deny-warnings"], {
      cwd: appDir,
      env: cleanEnv,
    });
    const badFile = path.join(appDir, "src/bad-lint.ts");
    await writeFile(badFile, "export function bad() { setTimeout(() => {}, 10); }\n");
    await assert.rejects(
      execute(path.join(appDir, "node_modules/.bin/oxlint"), ["--deny-warnings"], {
        cwd: appDir,
        env: cleanEnv,
      }),
      error => {
        assert.match(`${error.stdout ?? ""}${error.stderr ?? ""}`, /setTimeout/);
        return true;
      },
    );
    await rm(badFile);

    // 8. Lock installed CLI & build packages read-only (0o555) and verify smoke + prebuilt inspect --ui.
    const realCliDir = await realpath(path.join(appDir, "node_modules/@hewel/zgui-cli"));
    const realBuildDir = await realpath(path.join(realCliDir, "../zgui-build"));
    await setDirModeRecursive(realCliDir, 0o555);
    await setDirModeRecursive(realBuildDir, 0o555);
    try {
      const { stdout: smokeOut } = await execute(zguiBin, ["smoke"], {
        cwd: appDir,
        env: cleanEnv,
      });
      assert.match(smokeOut, /one reactive Solid runtime/);

      const { stdout: inspectOut } = await execute(
        zguiBin,
        ["inspect", "--ui", "--socket", "/tmp/zgui-e2e.sock"],
        { cwd: appDir, env: cleanEnv },
      );
      const inspectPayload = JSON.parse(inspectOut);
      assert.equal(inspectPayload.tag, "mock-release-host");
      const bundleIdx = inspectPayload.args.indexOf("--bundle");
      assert.ok(bundleIdx >= 0);
      assert.match(inspectPayload.args[bundleIdx + 1], /inspector-bundle$/);
    } finally {
      await setDirModeRecursive(realCliDir, 0o755);
      await setDirModeRecursive(realBuildDir, 0o755);
    }

    // 9. Verify tools/release.ts publish dist.integrity skip, mismatch fail-closed, and prerelease dist-tag guard.
    const { stdout: skipOut } = await execute(
      process.execPath,
      [releaseTool, "publish", "--manifest", manifestPath, "--registry", registry.url, "--dry-run"],
      { cwd: workspaceRoot },
    );
    const { version } = registry.manifest;
    assert.ok(skipOut.includes(`SKIP @hewel/zgui@${version} (`), skipOut);
    assert.ok(skipOut.includes(`SKIP create-zgui-js@${version} (`), skipOut);

    registry.setTampered("@hewel/zgui-cli");
    await assert.rejects(
      execute(
        process.execPath,
        [
          releaseTool,
          "publish",
          "--manifest",
          manifestPath,
          "--registry",
          registry.url,
          "--dry-run",
        ],
        { cwd: workspaceRoot },
      ),
      new RegExp(
        RegExp.escape(`FAIL-CLOSED dist.integrity mismatch for @hewel/zgui-cli@${version}:`),
      ),
    );
    registry.setTampered(null);

    const prereleaseManifestPath = path.join(tmpDir, "prerelease-manifest.json");
    const preManifest = JSON.parse(await readFile(manifestPath, "utf8"));
    preManifest.version = "0.2.0-rc.1";
    preManifest.packages = preManifest.packages.map(p => ({ ...p, version: "0.2.0-rc.1" }));
    await writeFile(prereleaseManifestPath, JSON.stringify(preManifest, null, 2));
    await assert.rejects(
      execute(
        process.execPath,
        [
          releaseTool,
          "publish",
          "--manifest",
          prereleaseManifestPath,
          "--tag",
          "latest",
          "--dry-run",
        ],
        { cwd: workspaceRoot },
      ),
      /Refusing to publish prerelease version 0\.2\.0-rc\.1 under dist-tag "latest"/,
    );
  },
);

test(
  "cold-start checkout builds missing SDK dist automatically and watchApplication rebuilds on SDK src edits",
  { timeout: 30000 },
  async t => {
    const checkout = await mkdtemp(path.join(os.tmpdir(), "zgui-sdk-watch-"));
    t.after(() => rm(checkout, { recursive: true, force: true }));

    // Minimal checkout mirror with Cargo.lock/bun.lock/package.json and fresh packages/{ui,services} without dist/.
    for (const file of ["Cargo.lock", "bun.lock", "package.json"]) {
      await writeFile(path.join(checkout, file), await readFile(path.join(workspaceRoot, file)));
    }

    const uiDir = path.join(checkout, "packages/ui");
    const servicesDir = path.join(checkout, "packages/services");
    const appDir = path.join(checkout, "apps/examples/mini");
    await mkdir(path.join(uiDir, "src/components/generated"), { recursive: true });
    await mkdir(path.join(servicesDir, "src"), { recursive: true });
    await mkdir(path.join(appDir, "src"), { recursive: true });
    await mkdir(path.join(checkout, "node_modules/@hewel"), { recursive: true });

    await writeFile(
      path.join(uiDir, "package.json"),
      JSON.stringify({
        name: "@hewel/zgui",
        version: "0.1.0",
        type: "module",
        exports: {
          ".": { types: "./dist/index.d.ts", default: "./dist/index.js" },
          "./renderer": {
            types: "./dist/renderer/index.d.ts",
            default: "./dist/renderer/index.js",
          },
          "./jsx-runtime": { types: "./dist/jsx-runtime.d.ts", default: "./dist/jsx-runtime.js" },
          "./tsconfig.native.json": "./tsconfig.native.json",
        },
        imports: { "#solid": { types: "./dist/solid-native.d.ts", default: "solid-js" } },
      }),
    );
    await writeFile(
      path.join(uiDir, "tsconfig.native.json"),
      await readFile(path.join(workspaceRoot, "packages/ui/tsconfig.native.json")),
    );
    await writeFile(
      path.join(uiDir, "tsconfig.json"),
      await readFile(path.join(workspaceRoot, "packages/ui/tsconfig.json")),
    );
    await writeFile(
      path.join(uiDir, "tsconfig.build.json"),
      await readFile(path.join(workspaceRoot, "packages/ui/tsconfig.build.json")),
    );
    for (const dts of ["solid-native.d.ts", "events.d.ts", "styles.d.ts"]) {
      await writeFile(path.join(uiDir, "src", dts), "export {};\n");
    }
    await writeFile(
      path.join(uiDir, "src/components/generated/MiniStyle.ts"),
      'export const sheet = { name: "MiniStyle", className: "zs-mini", css: ".zs-mini { color: red; }" };\n',
    );
    await writeFile(path.join(uiDir, "src/index.ts"), 'export const sdkMarker = "sdk-v1";\n');

    await writeFile(
      path.join(servicesDir, "package.json"),
      JSON.stringify({
        name: "@hewel/zgui-services",
        version: "0.1.0",
        type: "module",
        exports: { ".": { types: "./dist/index.d.ts", default: "./dist/index.js" } },
        imports: { "#solid": { types: "./dist/solid.d.ts", default: "solid-js" } },
      }),
    );
    await writeFile(
      path.join(servicesDir, "tsconfig.json"),
      await readFile(path.join(workspaceRoot, "packages/services/tsconfig.json")),
    );
    await writeFile(
      path.join(servicesDir, "tsconfig.build.json"),
      await readFile(path.join(workspaceRoot, "packages/services/tsconfig.build.json")),
    );
    await writeFile(path.join(servicesDir, "src/solid.d.ts"), "export {};\n");
    await writeFile(path.join(servicesDir, "src/index.ts"), 'export const svcMarker = "svc-v1";\n');

    await symlink(uiDir, path.join(checkout, "node_modules/@hewel/zgui"), "dir");
    await symlink(servicesDir, path.join(checkout, "node_modules/@hewel/zgui-services"), "dir");
    for (const dep of ["solid-js", "@solidjs"]) {
      const src = path.join(workspaceRoot, "node_modules", dep);
      if (existsSync(src)) {
        await symlink(src, path.join(checkout, "node_modules", dep), "dir");
      }
    }

    await writeFile(
      path.join(appDir, "package.json"),
      JSON.stringify({ name: "mini", type: "module" }),
    );
    await writeFile(
      path.join(appDir, "tsconfig.json"),
      JSON.stringify({
        compilerOptions: {
          target: "ES2022",
          module: "ESNext",
          moduleResolution: "Bundler",
          lib: ["ES2022"],
          types: [],
          strict: true,
          noEmit: true,
        },
        include: ["src"],
      }),
    );
    await writeFile(
      path.join(appDir, "src/main.tsx"),
      `import { createSignal } from "solid-js";
import { sdkMarker } from "@hewel/zgui";
import { svcMarker } from "@hewel/zgui-services";
export const hostApiVersion = 1;
export function mount() {
  const [v] = createSignal(sdkMarker + ":" + svcMarker);
  return () => { void v(); };
}
`,
    );

    assert.equal(existsSync(path.join(uiDir, "dist")), false);
    assert.equal(existsSync(path.join(servicesDir, "dist")), false);

    let next = Promise.withResolvers();
    const watcher = watchApplication("mini", {
      root: checkout,
      appDirectory: appDir,
      onSuccess: result => next.resolve(result),
      onError: error => next.reject(error),
    });
    t.after(() => watcher.close());

    try {
      const first = await next.promise;
      assert.equal(existsSync(path.join(uiDir, "dist/index.js")), true);
      assert.equal(existsSync(path.join(servicesDir, "dist/index.js")), true);
      const firstCode = await readFile(path.join(first.dist, "app.mjs"), "utf8");
      assert.match(firstCode, /sdk-v1/);

      next = Promise.withResolvers();
      await writeFile(path.join(uiDir, "src/index.ts"), 'export const sdkMarker = "sdk-v2";\n');
      const second = await next.promise;
      assert.notEqual(second.manifest.buildId, first.manifest.buildId);
      const secondCode = await readFile(path.join(second.dist, "app.mjs"), "utf8");
      assert.match(secondCode, /sdk-v2/);
    } finally {
      await watcher.close();
    }
  },
);
