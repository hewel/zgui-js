#!/usr/bin/env bun
import { spawn, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function parseArgs(argv) {
  const opts = {
    binary: path.join(root, "target/release/native-host"),
    apps: ["baseline", "counter", "motion-lab", "sysmon", "workspace", "gallery"],
    width: 1240,
    height: 780,
    build: false,
    json: false,
    live: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--binary" && argv[i + 1]) opts.binary = path.resolve(argv[++i]);
    else if (arg === "--apps" && argv[i + 1]) opts.apps = argv[++i].split(",").filter(Boolean);
    else if (arg === "--width" && argv[i + 1]) opts.width = Number(argv[++i]);
    else if (arg === "--height" && argv[i + 1]) opts.height = Number(argv[++i]);
    else if (arg === "--build") opts.build = true;
    else if (arg === "--json") opts.json = true;
    else if (arg === "--live") opts.live = true;
    else if (arg === "-h" || arg === "--help") {
      console.log(
        "Usage: bun tools/measure-memory.mjs [--binary <path>] [--apps baseline,counter,motion-lab,sysmon,workspace,gallery] [--width 1240] [--height 780] [--build] [--live] [--json]",
      );
      process.exit(0);
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }
  return opts;
}

function readBinaryInfo(binary) {
  if (!existsSync(binary)) {
    throw new Error(
      `native-host binary not found at ${binary}. Run: nix-shell --run "cargo build --release -p native-host --offline"`,
    );
  }
  const bytes = statSync(binary).size;
  let sections = null;
  const sizeProc = spawnSync("size", [binary], { encoding: "utf8" });
  if (sizeProc.status === 0) {
    const lines = sizeProc.stdout.trim().split("\n");
    const row = lines[1]?.trim().split(/\s+/);
    if (row && row.length >= 4) {
      sections = {
        textBytes: Number(row[0]),
        dataBytes: Number(row[1]),
        bssBytes: Number(row[2]),
        totalBytes: Number(row[3]),
      };
    }
  }
  return { binary, bytes, sections };
}

function bundleSizes(distDir) {
  const readSize = name => {
    const p = path.join(distDir, name);
    return existsSync(p) ? statSync(p).size : 0;
  };
  return {
    appMjsBytes: readSize("app.mjs"),
    sourceMapBytes: readSize("app.mjs.map"),
    componentsCssBytes: readSize("components.css"),
    appCssBytes: readSize("app.css"),
    baseCssBytes: readSize("base.css"),
  };
}

function writeSysmonFixture(filePath) {
  const fixture = {
    schemaVersion: 1,
    id: "sysmon-memory-fixture",
    services: {
      control: {},
      commands: [
        {
          program: "/usr/bin/uname",
          args: ["-srm"],
          mode: "buffered",
          events: [
            { kind: "output", atMs: 1, status: 0, stdout: "Linux 6.12.0 x86_64\n", stderr: "" },
          ],
        },
        {
          program: "/usr/bin/free",
          args: ["-m"],
          mode: "buffered",
          events: [
            {
              kind: "output",
              atMs: 1,
              status: 0,
              stdout:
                "               total        used        free      shared  buff/cache   available\n" +
                "Mem:           32768       10240       15360         512        7168       22016\n" +
                "Swap:           8192           0        8192\n",
              stderr: "",
            },
          ],
        },
        {
          program: "/usr/bin/ps",
          args: ["-eo", "pid,pcpu,pmem,rss,stat,comm", "--sort=-pcpu"],
          mode: "buffered",
          events: [
            {
              kind: "output",
              atMs: 1,
              status: 0,
              stdout:
                "  PID %CPU %MEM   RSS STAT COMMAND\n" +
                "  100  2.4  0.8 42000 Sl   native-host\n" +
                "    1  0.1  0.1 14200 Ss   systemd\n",
              stderr: "",
            },
          ],
        },
        {
          program: "/usr/bin/df",
          args: ["-h", "--output=target,pcent,used,size", "-x", "tmpfs", "-x", "devtmpfs"],
          mode: "buffered",
          events: [
            {
              kind: "output",
              atMs: 1,
              status: 0,
              stdout: "Mounted on Use% Used Size\n/ 38% 180G 476G\n",
              stderr: "",
            },
          ],
        },
        {
          program: "/usr/bin/vmstat",
          args: ["1"],
          mode: "streaming",
          events: [
            {
              kind: "stdout",
              atMs: 2,
              text: " 1  0      0 8000000 400000 3000000 0    0     2     4  100  200  8  3 89  0  0  0",
            },
            {
              kind: "stdout",
              atMs: 12,
              text: " 1  0      0 7980000 400000 3010000 0    0     1     2  110  220 12  4 84  0  0  0",
            },
          ],
        },
      ],
    },
    events: [],
  };
  writeFileSync(filePath, JSON.stringify(fixture, null, 2));
}

async function prepareBaselineBundle(tmpDir) {
  const appDir = path.join(tmpDir, "baseline-app");
  mkdirSync(path.join(appDir, "src"), { recursive: true });
  writeFileSync(
    path.join(appDir, "src/main.tsx"),
    `import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => <box><text>baseline</text></box>, root);
}
`,
  );
  writeFileSync(
    path.join(appDir, "tsconfig.json"),
    JSON.stringify({
      extends: path.join(root, "packages/ui/tsconfig.native.json"),
      include: ["src/**/*.tsx"],
    }),
  );
  const { buildApplication } = await import(path.join(root, "packages/build/src/build.ts"));
  const built = await buildApplication("baseline", { appDirectory: appDir });
  return built.dist;
}

function readSmaps(pid) {
  try {
    const text = readFileSync(`/proc/${pid}/smaps_rollup`, "utf8");
    const out = {};
    for (const line of text.split("\n")) {
      const match = line.match(/^([A-Za-z_]+):\s+(\d+)\s+kB/);
      if (match) out[match[1]] = Number(match[2]) * 1024;
    }
    return out;
  } catch {
    return null;
  }
}

async function measureLive(binary, bundle, width, height) {
  const args = ["--width", String(width), "--height", String(height), "--exit-after-ms", "2000"];
  if (bundle) args.push("--bundle", bundle);
  const child = spawn(binary, args, { stdio: ["ignore", "ignore", "pipe"] });
  let steady = null;
  const sampleTimer = setTimeout(() => {
    steady = readSmaps(child.pid);
  }, 1500);
  const code = await new Promise(resolve => child.on("close", resolve));
  clearTimeout(sampleTimer);
  return {
    exitCode: code,
    rssBytes: steady?.Rss ?? null,
    pssBytes: steady?.Pss ?? null,
    pssAnonBytes: steady?.Pss_Anon ?? null,
    pssFileBytes: steady?.Pss_File ?? null,
    privateDirtyBytes: steady?.Private_Dirty ?? null,
    jsHeapBytes: null,
    jsHeapLiveBytes: null,
    jsHeapPostDisposeBytes: null,
  };
}

function measurePreview(binary, bundle, fixturePath, scenarioPath, outDir, width, height) {
  const proc = spawnSync(
    binary,
    [
      "preview",
      "run",
      "--bundle",
      bundle,
      "--profile",
      path.join(root, "preview/profiles/default.json"),
      "--fixture",
      fixturePath,
      "--scenario",
      scenarioPath,
      "--out",
      outDir,
      "--width",
      String(width),
      "--height",
      String(height),
    ],
    { cwd: root, encoding: "utf8" },
  );
  const metricsPath = path.join(outDir, "metrics.json");
  const metrics = existsSync(metricsPath) ? JSON.parse(readFileSync(metricsPath, "utf8")) : {};
  return {
    exitCode: proc.status ?? 1,
    rssBytes: metrics.rssBytes ?? null,
    pssBytes: metrics.pssBytes ?? null,
    pssAnonBytes: metrics.pssAnonBytes ?? null,
    pssFileBytes: metrics.pssFileBytes ?? null,
    privateDirtyBytes: metrics.privateDirtyBytes ?? null,
    jsHeapBytes: metrics.jsHeapBytes ?? null,
    jsHeapLiveBytes: metrics.jsHeapLiveBytes ?? null,
    jsHeapPostDisposeBytes: metrics.jsHeapPostDisposeBytes ?? null,
  };
}

function fmtMiB(bytes) {
  return bytes === null || bytes === undefined
    ? "n/a"
    : `${(bytes / (1024 * 1024)).toFixed(2)} MiB`;
}

function fmtKiB(bytes) {
  return bytes === null || bytes === undefined ? "n/a" : `${(bytes / 1024).toFixed(1)} KiB`;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const binaryInfo = readBinaryInfo(opts.binary);
  mkdirSync(path.join(root, "target"), { recursive: true });
  const tmpDir = mkdtempSync(path.join(root, "target", "zgui-measure-mem-"));
  try {
    const scenarioPath = path.join(tmpDir, "steady.json");
    writeFileSync(
      scenarioPath,
      JSON.stringify({
        schemaVersion: 1,
        id: "memory-benchmark",
        requiredCapabilities: [],
        steps: [
          { id: "settle", op: "clock.advance", byMs: 32 },
          { id: "capture", op: "capture", captureId: "initial" },
        ],
      }),
    );
    const sysmonScenarioPath = path.join(tmpDir, "sysmon.json");
    writeFileSync(
      sysmonScenarioPath,
      JSON.stringify({
        schemaVersion: 1,
        id: "sysmon-memory-benchmark",
        requiredCapabilities: [],
        steps: [
          { id: "settle", op: "clock.advance", byMs: 32 },
          { id: "capture", op: "capture", captureId: "initial" },
        ],
      }),
    );
    const sysmonFixturePath = path.join(tmpDir, "sysmon-fixture.json");
    writeSysmonFixture(sysmonFixturePath);

    const rows = [];
    for (const app of opts.apps) {
      let distDir = null;
      if (app === "baseline") {
        distDir = opts.live ? null : await prepareBaselineBundle(tmpDir);
      } else {
        const appDir = path.join(root, "apps/examples", app);
        distDir = path.join(appDir, "dist");
        if (opts.build || !existsSync(path.join(distDir, "manifest.json"))) {
          const build = spawnSync("bun", ["run", "build", "--app", app], {
            cwd: root,
            encoding: "utf8",
          });
          if (build.status !== 0) {
            throw new Error(`Failed to build ${app}: ${build.stderr || build.stdout}`);
          }
        }
      }
      const sizes = distDir ? bundleSizes(distDir) : null;
      const metrics = opts.live
        ? await measureLive(opts.binary, distDir, opts.width, opts.height)
        : measurePreview(
            opts.binary,
            distDir,
            app === "sysmon" ? sysmonFixturePath : path.join(root, "preview/fixtures/empty.json"),
            app === "sysmon" ? sysmonScenarioPath : scenarioPath,
            path.join(tmpDir, `out-${app}`),
            opts.width,
            opts.height,
          );
      rows.push({ app, sizes, metrics });
    }

    const report = {
      mode: opts.live ? "live" : "preview",
      viewport: { width: opts.width, height: opts.height },
      binary: binaryInfo,
      rows,
    };

    if (opts.json) {
      console.log(JSON.stringify(report, null, 2));
      return;
    }

    console.log(`Binary: ${binaryInfo.binary}`);
    console.log(
      `  File size: ${fmtMiB(binaryInfo.bytes)} (${binaryInfo.bytes} B)` +
        (binaryInfo.sections
          ? ` | .text=${fmtMiB(binaryInfo.sections.textBytes)} .data=${fmtMiB(binaryInfo.sections.dataBytes)}`
          : ""),
    );
    console.log("");
    console.log(
      "| App | Exit | app.mjs | comp.css | Rss | Pss | Pss_Anon | Pss_File | PrivDirty | JS Live | JS Peak | JS Post |",
    );
    console.log("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for (const { app, sizes, metrics } of rows) {
      console.log(
        `| ${app} | ${metrics.exitCode} | ${sizes ? fmtKiB(sizes.appMjsBytes) : "—"} | ${sizes ? fmtKiB(sizes.componentsCssBytes) : "—"} | ${fmtMiB(metrics.rssBytes)} | ${fmtMiB(metrics.pssBytes)} | ${fmtMiB(metrics.pssAnonBytes)} | ${fmtMiB(metrics.pssFileBytes)} | ${fmtMiB(metrics.privateDirtyBytes)} | ${fmtKiB(metrics.jsHeapLiveBytes)} | ${fmtKiB(metrics.jsHeapBytes)} | ${fmtKiB(metrics.jsHeapPostDisposeBytes)} |`,
      );
    }
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }
}

await main();
