import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.mjs";

let out;
before(async () => {
  await buildApplication("composition", {
    appDirectory: path.join(workspaceRoot, "apps/examples/composition"),
  });
  out = await mkdtemp(path.join(workspaceRoot, "target/preview-composition-test-"));
});
after(async () => { if (out) await rm(out, { recursive: true, force: true }); });

const byTestId = (tree, id) => tree.nodes.find(node => node.testId === id);
const childrenOf = (tree, token, excluded) =>
  tree.nodes.filter(node => node.parent === token && node.ownership === "application" && !excluded.has(node.testId));
const json = async file => JSON.parse(await readFile(file, "utf8"));

function runPreview(bundle, scenario, directory) {
  const child = spawnSync(path.join(workspaceRoot, "target/debug/native-host"), ["preview", "run",
    "--bundle", bundle,
    "--profile", path.join(workspaceRoot, "preview/profiles/default.json"),
    "--fixture", path.join(workspaceRoot, "preview/fixtures/empty.json"),
    "--scenario", scenario,
    "--out", directory,
  ], { cwd: workspaceRoot, encoding: "utf8", timeout: 60000 });
  assert.ifError(child.error);
  assert.equal(child.signal, null, child.stderr);
  const result = JSON.parse(child.stdout);
  assert.equal(child.status, result.exitCode);
  return result;
}

async function minimalApp(directory, name, source, assets = {}) {
  const app = path.join(directory, name);
  await mkdir(path.join(app, "src"), { recursive: true });
  await mkdir(path.join(app, "assets"), { recursive: true });
  for (const [file, bytes] of Object.entries(assets)) {
    await writeFile(path.join(app, "assets", file), bytes);
  }
  await writeFile(path.join(app, "tsconfig.json"), JSON.stringify({
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"), include: ["src/**/*.tsx"],
  }));
  await writeFile(path.join(app, "src/main.tsx"), source);
  await buildApplication(name, { appDirectory: app });
  const scenario = path.join(directory, `${name}.scenario.json`);
  await writeFile(scenario, JSON.stringify({ schemaVersion: 1, id: name,
    requiredCapabilities: ["pixels", "virtual-time", "assets.native-readiness"],
    steps: [{ id: "initial", op: "capture", captureId: "initial", requiredFrameTimeUs: 0 }],
  }));
  return { bundle: path.join(app, "dist"), scenario };
}

test("composition scenario passes and avatar ownership survives source removal", async () => {
  const result = runPreview(
    path.join(workspaceRoot, "apps/examples/composition/dist"),
    path.join(workspaceRoot, "preview/scenarios/composition-virtual.json"), out);
  assert.equal(result.exitCode, 0, JSON.stringify(result.firstError));
  assert.equal(result.status, "passed");
  assert.equal(result.assertions.failed, 0);
  assert.equal(result.teardown.clean, true);

  const tokens = { avatar: [], fallback: [], badge: [] };
  const images = [];
  for (const capture of ["initial", "fallback", "final"]) {
    const frame = await json(path.join(out, `captures/${capture}.frame.json`));
    assert.equal(frame.resources.requiredAssetsReady, true, `${capture} readiness`);
    const broken = frame.resources.resources.filter(resource => resource.source === "assets/broken.png");
    assert.equal(broken.length, 1, `${capture} tracks the failed fixture`);
    assert.equal(broken[0].state, "failed", `${capture} records the decode failure`);
    assert.equal(broken[0].visible, false, `${capture} failed image is not paint-visible`);

    const tree = await json(path.join(out, `captures/${capture}.tree.json`));
    const avatar = byTestId(tree, "avatar");
    const fallback = byTestId(tree, "avatar-fallback");
    const badge = byTestId(tree, "avatar-badge");
    assert.ok(avatar && fallback && badge, `${capture} avatar parts`);
    tokens.avatar.push(avatar.token);
    tokens.fallback.push(fallback.token);
    tokens.badge.push(badge.token);
    images.push(childrenOf(tree, avatar.token, new Set(["avatar-fallback", "avatar-badge"])));
  }
  for (const key of ["avatar", "fallback", "badge"]) {
    assert.equal(tokens[key][1], tokens[key][0], `${key} stable through removal`);
    assert.equal(tokens[key][2], tokens[key][0], `${key} stable through restore`);
  }
  assert.equal(images[0].length, 1, "initial owned image present");
  assert.equal(images[1].length, 0, "hidden capture drops the owned image");
  assert.equal(images[2].length, 1, "restored owned image present");
  assert.notEqual(images[2][0].token, images[0][0].token, "restored image is a fresh node");

  const initial = await json(path.join(out, "captures/initial.tree.json"));
  const failedRoot = byTestId(initial, "avatar-failed");
  const failedFallback = byTestId(initial, "avatar-failed-fallback");
  assert.ok(failedRoot && failedFallback, "failed avatar parts exist");
  assert.equal(failedFallback.state.visible, true, "failed avatar shows its fallback");
  const failedImages = childrenOf(initial, failedRoot.token, new Set(["avatar-failed-fallback"]));
  assert.equal(failedImages.length, 1, "failed avatar keeps one mounted image");
  assert.equal(failedImages[0].state.visible, false, "broken image is not paint-visible");
});

test("a visible decode failure still fails the capture", async t => {
  const directory = await mkdtemp(path.join(workspaceRoot, "target/preview-composition-neg-visible-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const { bundle, scenario } = await minimalApp(directory, "composition-neg-visible", `
    import { Image, type NativeNode } from "@hewel/zgui";
    import { render } from "@hewel/zgui/renderer";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return render(() => <Image src="assets/bad.png" style={{ width: "64px", height: "16px" }} />, root);
    }
  `, { "bad.png": Buffer.from("not a raster image") });
  const result = runPreview(bundle, scenario, path.join(directory, "actual"));
  assert.equal(result.exitCode, 6);
  assert.equal(result.firstError.code, "E_ASSET_FAILED");
  assert.equal(result.captureCount, 0);
});

test("an undeclared source fails the capture even when hidden", async t => {
  const directory = await mkdtemp(path.join(workspaceRoot, "target/preview-composition-neg-undeclared-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const { bundle, scenario } = await minimalApp(directory, "composition-neg-undeclared", `
    import { Image, type NativeNode } from "@hewel/zgui";
    import { render } from "@hewel/zgui/renderer";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return render(() => <Image src="assets/missing.png" style={{ display: "none", width: "64px", height: "16px" }} />, root);
    }
  `);
  const result = runPreview(bundle, scenario, path.join(directory, "actual"));
  assert.equal(result.exitCode, 6);
  assert.equal(result.firstError.code, "E_ASSET_FAILED");
  assert.match(result.firstError.message, /declared package asset/);
  assert.equal(result.captureCount, 0);
});
