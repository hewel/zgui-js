import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { after, before, test } from "node:test";
import {
  buildFixtureApp, createTempWorkspace, defaultProfile, nativeHostBinary, pixelAt,
  readFrame, readPng, runScenario, workspaceRoot, writeJson,
} from "./harness.mjs";

const requiredCapabilities = ["pixels", "capture.transparent"];
const capture = (surface = "main") => ({ id: surface, op: "capture", captureId: surface, surface });
const readJson = async file => JSON.parse(await readFile(file, "utf8"));
const digest = async file => `sha256:${createHash("sha256").update(await readFile(file)).digest("hex")}`;
let temp;
let base;
let profile;
let main;

before(async () => {
  temp = await createTempWorkspace("preview-transparent-");
  base = await readJson(defaultProfile);
  profile = await writeProfile("transparent");
});
after(async () => { await temp?.cleanup(); });

async function writeProfile(name, { pureShell = false, opaque = false, background = "#00000000" } = {}) {
  const file = path.join(temp.dir, `${name}.json`);
  const manifest = path.resolve(path.dirname(defaultProfile), base.fonts.manifest);
  await writeJson(file, {
    ...base, pureShell,
    fonts: { ...base.fonts, manifest: path.relative(temp.dir, manifest) },
    surface: { ...base.surface, width: 120, height: 80, scale: 1, opaque, background },
  });
  return file;
}

function native(args, env = process.env) {
  const proc = spawnSync(nativeHostBinary, ["preview", ...args], { cwd: workspaceRoot, env, encoding: "utf8", timeout: 30000 });
  assert.ifError(proc.error);
  assert.equal(proc.signal, null, proc.stderr);
  const result = JSON.parse(proc.stdout);
  assert.equal(proc.status, result.exitCode, proc.stderr);
  return result;
}

function assertTransparentRenderer(renderer) {
  assert.equal(renderer.alphaMode, "PreMultiplied");
  assert.equal(renderer.compositorTransparencyValidated, false);
}

function assertRgba(actual, expected, tolerance = 0) {
  assert.ok(actual.every((channel, index) => Math.abs(channel - expected[index]) <= tolerance),
    `expected ${expected} ±${tolerance}, got ${actual}`);
}

test("doctor validates raw and PNG alpha on clear and translucent backgrounds", async () => {
  const tinted = await writeProfile("tinted", { background: "#11223380" });
  for (const [name, file, rawRoot, pngRoot, rawSwatch, pngSwatch, tolerance] of [
    ["clear", profile, [0, 0, 0, 0], [0, 0, 0, 0], [128, 0, 0, 128], [255, 0, 0, 128], 0],
    ["tinted", tinted, [9, 17, 26, 128], [17, 34, 51, 128], [132, 9, 13, 192], [175, 11, 17, 192], 1],
  ]) {
    const out = path.join(temp.dir, `doctor-${name}`);
    const result = native(["doctor", "--profile", file, "--out", out]);
    assert.equal(result.status, "passed", JSON.stringify(result));
    assert.equal(result.capabilities["capture.transparent"], true);
    assertTransparentRenderer(result.graphics);
    const frame = await readJson(path.join(out, "calibration.frame.json"));
    const image = await readPng(path.join(out, "calibration.png"));
    for (const checks of [frame.checks, frame.oddWidthChecks]) {
      assertRgba(checks.transparent.root.premultipliedRgba, rawRoot, tolerance);
      assertRgba(checks.transparent.halfRedOverRoot.premultipliedRgba, rawSwatch, 1);
    }
    assertRgba(pixelAt(image, 45, 70), pngRoot, tolerance * 2);
    assertRgba(pixelAt(image, 75, 70), pngSwatch, 2);
  }
});

async function mainBundle() {
  main ??= await buildFixtureApp(path.join(temp.dir, "main"), "transparent-main", `
    import { renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => <box>
        <box style="display: block; position: fixed; left: 20px; top: 10px; width: 30px; height: 30px; background: rgba(204, 102, 51, 0.5)" />
        <box style="display: block; position: fixed; left: 60px; top: 10px; width: 30px; height: 30px; background: #123456" />
      </box>, root);
    }
  `, { rawSource: true });
  return main;
}

async function runMain(name, input = profile, out) {
  const bundle = await mainBundle();
  return runScenario({ projectDir: temp.dir, bundle, name, id: "alpha", steps: [capture()],
    requiredCapabilities, profile: input, out });
}

test("unavailable graphics disables pixel and transparent-capture capabilities", async () => {
  const scenario = path.join(temp.dir, "no-graphics.json");
  await writeJson(scenario, { schemaVersion: 1, id: "no-graphics", requiredCapabilities, steps: [capture()] });
  const env = { ...process.env, ZGUI_BACKENDS: "none" };
  const out = path.join(temp.dir, "no-graphics");
  const run = native(["run", "--bundle", await mainBundle(), "--profile", profile,
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"), "--scenario", scenario, "--out", out], env);
  const doctor = native(["doctor", "--profile", profile], env);
  for (const result of [run, doctor]) {
    assert.equal(result.exitCode, 7, JSON.stringify(result));
    assert.equal(result.firstError.code, "E_GRAPHICS_UNAVAILABLE");
  }
  const environment = await readJson(path.join(out, "environment.json"));
  for (const { capabilities } of [environment, doctor]) {
    assert.equal(capabilities?.pixels, false);
    assert.equal(capabilities?.["capture.transparent"], false);
  }
});

test("main capture writes clear, straight-alpha and opaque pixels", async () => {
  const { out } = await runMain("main");
  assertTransparentRenderer((await readFrame(out, "main")).renderer);
  const image = await readPng(path.join(out, "captures/main.png"));
  assertRgba(pixelAt(image, 5, 5), [0, 0, 0, 0]);
  assertRgba(pixelAt(image, 30, 20), [204, 102, 51, 128], 2);
  assertRgba(pixelAt(image, 70, 20), [18, 52, 86, 255]);
});

test("pure-shell layer and popup capture transparent surround and retire cleanly", async () => {
  const shellProfile = await writeProfile("shell-profile", { pureShell: true });
  const bundle = await buildFixtureApp(path.join(temp.dir, "shell"), "transparent-shell", `
    import { LayerSurface, PopupSurface, renderShell } from "@hewel/zgui";
    export const hostApiVersion = 1;
    function Card() {
      return <box style="display: block; position: fixed; left: 12px; top: 12px; width: 28px; height: 24px; background: #123456" />;
    }
    export function mount() {
      return renderShell(() => <>
        <LayerSurface name="bar" output="left" width={80} height={48}><Card /></LayerSurface>
        <PopupSurface name="menu" parent="bar" anchorRect={{ x: 12, y: 12, width: 28, height: 24 }}
          width={64} height={48}><Card /></PopupSurface>
      </>);
    }
  `, { rawSource: true, capabilities: { shellSurfaces: true } });
  const { out, result } = await runScenario({ projectDir: temp.dir, bundle, profile: shellProfile, name: "shell", requiredCapabilities, steps: [
    { id: "output", op: "monitors.set", monitors: [{ id: "left", name: "Left", x: 0, y: 0, width: 320, height: 240, scale: 1, refreshRateMilliHz: 60000 }] },
    capture("bar"), capture("menu"),
  ] });
  assert.equal(result.captureCount, 2);
  for (const name of ["bar", "menu"]) {
    const artifact = `surfaces/${name}/${name}`;
    assertTransparentRenderer((await readFrame(out, artifact)).renderer);
    const image = await readPng(path.join(out, `captures/${artifact}.png`));
    assertRgba(pixelAt(image, 4, 4), [0, 0, 0, 0]);
    assertRgba(pixelAt(image, image.width - 5, image.height - 5), [0, 0, 0, 0]);
    assertRgba(pixelAt(image, 20, 20), [18, 52, 86, 255]);
  }
});

test("lock surfaces stay opaque when author CSS makes the root transparent", async () => {
  const app = path.join(temp.dir, "lock");
  await mkdir(path.join(app, "src"), { recursive: true });
  await writeFile(path.join(app, "src/app.css"), ":root { background-color: transparent; }\n");
  const bundle = await buildFixtureApp(app, "transparent-lock", `
    import { Button, SessionLockSurface, renderApp, requestSessionLock } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => <column>
        <Button testId="lock" onClick={() => requestSessionLock()}>Lock</Button>
        <SessionLockSurface name="lock-left" output="left">
          <box style="display: block; position: fixed; left: 10px; top: 10px; width: 30px; height: 30px; background: #123456" />
        </SessionLockSurface>
      </column>, root);
    }
  `, { rawSource: true, capabilities: { sessionLock: true } });
  const { out } = await runScenario({ projectDir: temp.dir, bundle, profile, name: "lock", requiredCapabilities: [
    ...requiredCapabilities, "session-lock",
  ], steps: [
    { id: "output", op: "monitors.set", monitors: [{ id: "left", name: "Left", x: 0, y: 0, width: 120, height: 80, scale: 1, refreshRateMilliHz: 60000 }] },
    capture(),
    { id: "request", op: "pointer.click", target: { surface: "main", testId: "lock" }, button: "primary" },
    { id: "dispatch", op: "clock.advance", byMs: 40 },
    capture("lock-left"),
  ] });
  assertTransparentRenderer((await readFrame(out, "main")).renderer);
  const artifact = "surfaces/lock-left/lock-left";
  assert.equal((await readFrame(out, artifact)).renderer.alphaMode, "Opaque");
  const png = await readPng(path.join(out, `captures/${artifact}.png`));
  assertRgba(pixelAt(png, 119, 79), [0, 0, 0, 255]);
  assertRgba(pixelAt(png, 20, 20), [18, 52, 86, 255]);
});

test("compare rejects opaque and transparent captures of the same scenario", async () => {
  const opaque = await writeProfile("opaque", { opaque: true, background: "#000000ff" });
  const reviewed = path.join(temp.dir, "reviewed");
  await mkdir(reviewed);
  const baseline = await runMain("baseline", opaque, path.join(reviewed, "run"));
  const actual = await runMain("actual");
  const policy = path.join(reviewed, "policy.json");
  await copyFile(path.join(workspaceRoot, "preview/comparison/strict.json"), policy);
  await writeJson(path.join(reviewed, "baseline.json"), {
    schemaVersion: 1, run: "run", artifactsDigest: await digest(path.join(baseline.out, "artifacts.json")),
    policyDigest: await digest(policy),
    review: {
      by: "transparent-capture-test", at: "2026-11-18", summary: "Review an opaque capture for alpha compatibility.",
      buildId: baseline.result.build.buildId, captures: [{ captureId: "main", surface: "main" }],
    },
  });
  const out = path.join(temp.dir, "compare");
  const result = native(["compare", "--baseline", reviewed, "--actual", actual.out, "--policy", policy, "--out", out]);
  assert.equal(result.firstError?.code, "E_BASELINE_INCOMPATIBLE", JSON.stringify(result));
  assert.equal(result.captures.compared, 0);
  const report = await readJson(path.join(out, "report.json"));
  assert.ok(report.compatibilityIssues.some(issue => issue.field === "renderer.alphaMode"));
});
