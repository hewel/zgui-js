import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import { pngRgba } from "./png.mjs";

export { workspaceRoot };
export const defaultProfile = path.join(workspaceRoot, "preview/profiles/default.json");
export const functionalProfile = path.join(workspaceRoot, "preview/profiles/functional.json");
export const emptyFixture = path.join(workspaceRoot, "preview/fixtures/empty.json");
export const nativeHostBinary = path.join(workspaceRoot, "target/debug/zgui-host");
const failedWorkspaces = new Set();
export const emptyResources = {
  nodes: 0,
  wrappers: 0,
  callbacks: 0,
  focusTraps: 0,
  scrollLocks: 0,
  timers: 0,
  frameCallbacks: 0,
  requests: 0,
  shellSurfaces: 0,
  fonts: 0,
  engineAlive: false,
};

export async function createTempWorkspace(prefix) {
  await mkdir(path.join(workspaceRoot, "target"), { recursive: true });
  const dir = await mkdtemp(path.join(workspaceRoot, "target", prefix));
  return {
    dir,
    async cleanup() {
      const failed = [...failedWorkspaces].some(
        outDir => outDir === dir || outDir.startsWith(`${dir}${path.sep}`),
      );
      if (process.env.ZGUI_KEEP_PREVIEW_OUTPUT === "1" || failed) {
        console.log(`retained preview output: ${dir}`);
      } else {
        await rm(dir, { recursive: true, force: true });
      }
    },
  };
}

export function validateScenarioSpec(spec) {
  assert.equal(spec?.schemaVersion, 1, "scenario.schemaVersion must be 1");
  assert.equal(typeof spec.id, "string", "scenario.id must be a string");
  assert.ok(Array.isArray(spec.steps), "scenario.steps must be an array");
  for (let i = 0; i < spec.steps.length; i++) {
    const step = spec.steps[i];
    assert.ok(step && typeof step === "object", `scenario.steps[${i}] must be an object`);
    assert.equal(typeof step.op, "string", `scenario.steps[${i}].op must be a string`);
    if (step.op === "capture") {
      assert.ok(
        !("label" in step),
        `scenario.steps[${i}]: use "captureId" (not "label") on capture steps`,
      );
      assert.equal(
        typeof step.captureId,
        "string",
        `scenario.steps[${i}]: capture step requires string captureId`,
      );
    }
    if (step.op === "clock.virtual") {
      assert.fail(`scenario.steps[${i}]: use "clock.advance" (not "clock.virtual")`);
    }
  }
}

export function validateFixtureSpec(spec) {
  assert.equal(spec?.schemaVersion, 1, "fixture.schemaVersion must be 1");
  const services = spec.services;
  if (!services || typeof services !== "object") return;
  for (const kind of ["sockets", "commands"]) {
    const scripts = services[kind];
    if (!Array.isArray(scripts)) continue;
    for (let i = 0; i < scripts.length; i++) {
      assert.ok(
        Array.isArray(scripts[i]?.events),
        `fixture.services.${kind}[${i}] must declare an "events" array (use [] if none)`,
      );
    }
  }
}

export async function writeJson(filePath, value) {
  await writeFile(filePath, JSON.stringify(value));
}

export async function writeScenario(filePath, id, steps, requiredCapabilities) {
  const spec = {
    schemaVersion: 1,
    id,
    requiredCapabilities,
    steps,
  };
  validateScenarioSpec(spec);
  await writeJson(filePath, spec);
}

export async function buildFixtureApp(
  appDir,
  name,
  source,
  {
    rawSource = false,
    assets = {},
    files = {},
    fonts = [],
    capabilities,
    development = false,
  } = {},
) {
  await mkdir(path.join(appDir, "src"), { recursive: true });
  const content = rawSource
    ? source
    : await readFile(source instanceof URL ? fileURLToPath(source) : source, "utf8");
  await writeFile(path.join(appDir, "src/main.tsx"), content);
  for (const [relPath, fileContent] of Object.entries(files)) {
    const dest = path.join(appDir, relPath);
    await mkdir(path.dirname(dest), { recursive: true });
    await writeFile(dest, fileContent);
  }
  const assetEntries = Object.entries(assets);
  if (fonts.length > 0 || capabilities !== undefined) {
    await writeJson(path.join(appDir, "package.json"), {
      name,
      private: true,
      zgui: { fonts, capabilities },
    });
  }
  if (assetEntries.length > 0) {
    await mkdir(path.join(appDir, "assets"), { recursive: true });
    for (const [file, bytes] of assetEntries) {
      await writeFile(path.join(appDir, "assets", file), bytes);
    }
  }
  await writeJson(path.join(appDir, "tsconfig.json"), {
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
    include: ["src/**/*.ts", "src/**/*.tsx"],
  });
  await buildApplication(name, { appDirectory: appDir, development });
  return path.join(appDir, "dist");
}

export async function buildExampleApp(name, options = {}) {
  const appDirectory = path.join(workspaceRoot, "apps/examples", name);
  await buildApplication(name, { appDirectory, ...options });
  return path.join(appDirectory, "dist");
}

export async function runPreview({
  bundle,
  scenario,
  out,
  profile = defaultProfile,
  fixture = emptyFixture,
  width,
  height,
  scale,
  theme,
  timeoutMs,
  maxJsStackBytes,
  stderrPath,
  expectedAssertions,
  allowFailure = false,
  assertEmptyResources = true,
  forbidJsDiagnostics = false,
  env,
}) {
  const args = [
    "preview",
    "run",
    "--bundle",
    bundle,
    "--profile",
    profile,
    "--fixture",
    fixture,
    "--scenario",
    scenario,
    "--out",
    out,
  ];
  if (width !== undefined) args.push("--width", String(width));
  if (height !== undefined) args.push("--height", String(height));
  if (scale !== undefined) args.push("--scale", String(scale));
  if (theme !== undefined) args.push("--theme", theme);
  if (timeoutMs !== undefined) args.push("--timeout-ms", String(timeoutMs));
  if (maxJsStackBytes !== undefined) args.push("--max-js-stack-bytes", String(maxJsStackBytes));

  if (!allowFailure) {
    try {
      validateScenarioSpec(JSON.parse(await readFile(scenario, "utf8")));
      validateFixtureSpec(JSON.parse(await readFile(fixture, "utf8")));
    } catch (error) {
      throw new Error(`Preview input preflight failed before zgui-host launch: ${error.message}`);
    }
  }
  const proc = spawnSync(nativeHostBinary, args, {
    cwd: workspaceRoot,
    encoding: "utf8",
    env: env ? { ...process.env, ...env } : undefined,
    timeout: Math.max(60000, timeoutMs ?? 0),
  });
  if (proc.stderr && stderrPath) await writeFile(stderrPath, proc.stderr);
  let result;
  try {
    assert.ifError(proc.error);
    assert.equal(proc.signal, null, proc.stderr);
    result = JSON.parse(proc.stdout);
    assert.equal(proc.status, result.exitCode);
    if (!allowFailure) {
      const summary = JSON.stringify({
        out,
        code: result.firstError?.code ?? null,
        stage: result.firstError?.stage ?? null,
        message: result.firstError?.message ?? null,
        details: result.firstError?.details ?? null,
        teardownClean: result.teardown?.clean ?? null,
      });
      assert.equal(result.exitCode, 0, `${summary}\n${proc.stdout}`);
      assert.equal(result.status, "passed", `${summary}\n${proc.stdout}`);
      assert.equal(result.assertions.failed, 0, `${summary}\n${proc.stdout}`);
      if (expectedAssertions !== undefined) {
        assert.equal(result.assertions.passed, expectedAssertions, summary);
      }
      assert.equal(result.teardown.clean, true, `${summary}\n${proc.stdout}`);
      if (assertEmptyResources) {
        assert.deepEqual(result.teardown.resources, emptyResources, summary);
      }
      if (forbidJsDiagnostics) {
        const bad = (proc.stderr ?? "")
          .split("\n")
          .filter(
            line =>
              line.includes("js[error]") ||
              line.includes("js[warn]") ||
              line.includes("REACTIVE_WRITE_IN_OWNED_SCOPE") ||
              line.includes("STRICT_READ_UNTRACKED") ||
              line.includes("REACTIVITY_HALTED") ||
              line.includes("FLUSH_IN_EFFECT_CALLBACK"),
          );
        assert.deepEqual(
          bad,
          [],
          `${summary}\nunexpected JS diagnostics in stderr:\n${bad.join("\n")}`,
        );
      }
    }
  } catch (error) {
    if (!allowFailure) failedWorkspaces.add(out);
    throw error;
  }
  return { result, out, stderr: proc.stderr ?? "" };
}

export async function runScenario({
  projectDir,
  bundle,
  name,
  id,
  requiredCapabilities,
  steps,
  out = path.join(projectDir, `run-${name}`),
  stderrPath = path.join(projectDir, `${name}.stderr`),
  expectedAssertions = steps.filter(step => step.op === "assert" || step.op === "assert.service")
    .length,
  ...rest
}) {
  const scenario = path.join(projectDir, `${name}.json`);
  await writeScenario(scenario, id ?? name, steps, requiredCapabilities);
  return runPreview({ bundle, scenario, out, stderrPath, expectedAssertions, ...rest });
}

export async function readTree(out, name) {
  return JSON.parse(await readFile(path.join(out, `captures/${name}.tree.json`), "utf8"));
}

export async function readFrame(out, name) {
  return JSON.parse(await readFile(path.join(out, `captures/${name}.frame.json`), "utf8"));
}

export async function readMetrics(out) {
  return JSON.parse(await readFile(path.join(out, "metrics.json"), "utf8"));
}

export function getNode(tree, testId) {
  const node = tree.nodes.find(entry => entry.testId === testId);
  assert.ok(node, `missing ${testId}`);
  return node;
}

export const readPng = pngRgba;

export function pixelAt(image, x, y) {
  const offset = (y * image.width + x) * 4;
  return [
    image.data[offset],
    image.data[offset + 1],
    image.data[offset + 2],
    image.data[offset + 3],
  ];
}

export function regionStats(image, rect) {
  const left = Math.max(0, Math.floor(rect.x));
  const top = Math.max(0, Math.floor(rect.y));
  const right = Math.min(image.width, Math.ceil(rect.x + rect.width));
  const bottom = Math.min(image.height, Math.ceil(rect.y + rect.height));
  let pixels = 0;
  let minR = 255;
  let minG = 255;
  let minB = 255;
  let maxR = 0;
  let maxG = 0;
  let maxB = 0;
  let sumR = 0;
  let sumG = 0;
  let sumB = 0;
  const colors = new Set();
  for (let y = top; y < bottom; y++) {
    for (let x = left; x < right; x++) {
      const [r, g, b, a] = pixelAt(image, x, y);
      pixels += 1;
      if (r < minR) minR = r;
      if (g < minG) minG = g;
      if (b < minB) minB = b;
      if (r > maxR) maxR = r;
      if (g > maxG) maxG = g;
      if (b > maxB) maxB = b;
      sumR += r;
      sumG += g;
      sumB += b;
      colors.add(`${r},${g},${b},${a}`);
    }
  }
  return {
    pixels,
    minR,
    minG,
    minB,
    maxR,
    maxG,
    maxB,
    meanR: pixels ? sumR / pixels : 0,
    meanG: pixels ? sumG / pixels : 0,
    meanB: pixels ? sumB / pixels : 0,
    colors,
  };
}

export function uniqueColors(image, rect) {
  return regionStats(image, rect).colors;
}
