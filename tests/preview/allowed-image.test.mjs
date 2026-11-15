import assert from "node:assert/strict";
import { test } from "node:test";
import { copyFile, cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

const hash = bytes => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
const json = async file => JSON.parse(await readFile(file, "utf8"));
function native(args, env = process.env) {
  const child = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", ...args], {
    cwd: workspaceRoot, env, encoding: "utf8", timeout: 25000, maxBuffer: 1024 * 1024,
  });
  assert.ifError(child.error);
  assert.equal(child.signal, null, child.stderr);
  const result = JSON.parse(child.stdout);
  assert.equal(child.status, result.exitCode);
  assert.equal(result.exitCode, 0, `${child.stdout}\n${child.stderr}`);
  return result;
}

test("a real explicitly allowed pending image remains a valid comparison capture", async t => {
  const directory = await mkdtemp(path.join(workspaceRoot, "target/preview-allowed-image-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const app = path.join(directory, "app");
  await mkdir(path.join(app, "src"), { recursive: true });
  await mkdir(path.join(app, "assets"));
  await copyFile(path.join(workspaceRoot, "vendor/zgui-runtime/tests/fixtures/red2x2.png"), path.join(app, "assets/red.png"));
  await writeFile(path.join(app, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await writeFile(path.join(app, "src/main.tsx"), `
    import { Image, type NativeNode } from "@hewel/zgui";
    import { render } from "@hewel/zgui/renderer";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return render(() => <Image src="assets/red.png" style={{ width: "64px", height: "16px" }} />, root);
    }
  `);
  await buildApplication("allowed-image-regression", { appDirectory: app });
  const scenario = path.join(directory, "scenario.json");
  await writeFile(scenario, JSON.stringify({ schemaVersion: 1, id: "allowed-image", requiredCapabilities: ["pixels", "virtual-time", "assets.native-readiness"],
    steps: [{ id: "pending", op: "capture", captureId: "pending", requiredFrameTimeUs: 0, allowedPendingAssets: ["image-1"] }],
  }));
  const actual = path.join(directory, "actual");
  const result = native(["run", "--bundle", path.join(app, "dist"), "--profile", "preview/profiles/default.json",
    "--fixture", "preview/fixtures/empty.json", "--scenario", scenario, "--out", actual]);
  const frame = await json(path.join(actual, "captures/pending.frame.json"));
  assert.ok(frame.resources.pendingIds.length > 0, "the regression requires an actual pending native image");
  assert.equal(frame.resources.requiredAssetsReady, true, "the native producer certifies the explicit allowance");

  // This isolated fixture exercises comparison semantics; it never blesses the CI baseline.
  const baseline = path.join(directory, "reviewed");
  await mkdir(baseline);
  await cp(actual, path.join(baseline, "run"), { recursive: true });
  const policy = await readFile(path.join(workspaceRoot, "preview/comparison/strict.json"));
  await writeFile(path.join(baseline, "policy.json"), policy);
  await writeFile(path.join(baseline, "baseline.json"), JSON.stringify({
    schemaVersion: 1, run: "run", artifactsDigest: hash(await readFile(path.join(actual, "artifacts.json"))), policyDigest: hash(policy),
    review: { by: "allowed-image-regression-test", at: "isolated-test-fixture", summary: "Real native pending resource explicitly allowed by the capture step.",
      buildId: result.build.buildId, captures: [{ captureId: "pending", surface: "main" }] },
  }));
  const compared = native(["compare", "--baseline", baseline, "--actual", actual, "--policy", path.join(baseline, "policy.json"), "--out", path.join(directory, "comparison")],
    { ...process.env, VK_DRIVER_FILES: "/nonexistent/no-vulkan.json", VK_ICD_FILENAMES: "/nonexistent/no-vulkan.json", WAYLAND_DISPLAY: "no-wayland" });
  assert.equal(compared.captures.compared, 1);
  assert.equal(compared.captures.different, 0);
});
