import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import path from "node:path";
import {
  buildExampleApp,
  buildFixtureApp,
  createTempWorkspace,
  readFrame,
  readTree,
  runPreview as harnessRunPreview,
  workspaceRoot,
  writeScenario,
} from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  bundle = await buildExampleApp("composition");
  temp = await createTempWorkspace("preview-composition-test-");
});
after(async () => {
  await temp?.cleanup();
});

const byTestId = (tree, id) => tree.nodes.find(node => node.testId === id);
const childrenOf = (tree, token, excluded) =>
  tree.nodes.filter(
    node => node.parent === token && node.ownership === "application" && !excluded.has(node.testId),
  );

async function minimalApp(directory, name, source, assets = {}) {
  const appBundle = await buildFixtureApp(path.join(directory, name), name, source, {
    rawSource: true,
    assets,
  });
  const scenario = path.join(directory, `${name}.scenario.json`);
  await writeScenario(
    scenario,
    name,
    [{ id: "initial", op: "capture", captureId: "initial", requiredFrameTimeUs: 0 }],
    ["pixels", "virtual-time", "assets.native-readiness"],
  );
  return { bundle: appBundle, scenario };
}

test("composition scenario passes and avatar ownership survives source removal", async () => {
  await harnessRunPreview({
    bundle,
    scenario: path.join(workspaceRoot, "preview/scenarios/composition-virtual.json"),
    out: temp.dir,
    assertEmptyResources: false,
  });

  const tokens = { avatar: [], fallback: [], badge: [] };
  const images = [];
  for (const capture of ["initial", "fallback", "final"]) {
    const frame = await readFrame(temp.dir, capture);
    assert.equal(frame.resources.requiredAssetsReady, true, `${capture} readiness`);
    const broken = frame.resources.resources.filter(
      resource => resource.source === "assets/broken.png",
    );
    assert.equal(broken.length, 1, `${capture} tracks the failed fixture`);
    assert.equal(broken[0].state, "failed", `${capture} records the decode failure`);
    assert.equal(broken[0].visible, false, `${capture} failed image is not paint-visible`);

    const tree = await readTree(temp.dir, capture);
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

  const initial = await readTree(temp.dir, "initial");
  const failedRoot = byTestId(initial, "avatar-failed");
  const failedFallback = byTestId(initial, "avatar-failed-fallback");
  assert.ok(failedRoot && failedFallback, "failed avatar parts exist");
  assert.equal(failedFallback.state.visible, true, "failed avatar shows its fallback");
  const failedImages = childrenOf(initial, failedRoot.token, new Set(["avatar-failed-fallback"]));
  assert.equal(failedImages.length, 1, "failed avatar keeps one mounted image");
  assert.equal(failedImages[0].state.visible, false, "broken image is not paint-visible");
});

test("a visible decode failure still fails the capture", async t => {
  const neg = await createTempWorkspace("preview-composition-neg-visible-");
  t.after(() => neg.cleanup());
  const { bundle: negBundle, scenario } = await minimalApp(
    neg.dir,
    "composition-neg-visible",
    `
    import { Image, type NativeNode } from "@hewel/zgui";
    import { render } from "@hewel/zgui/renderer";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return render(() => <Image src="assets/bad.png" style={{ width: "64px", height: "16px" }} />, root);
    }
  `,
    { "bad.png": Buffer.from("not a raster image") },
  );
  const { result } = await harnessRunPreview({
    bundle: negBundle,
    scenario,
    out: path.join(neg.dir, "actual"),
    allowFailure: true,
  });
  assert.equal(result.exitCode, 6);
  assert.equal(result.firstError.code, "E_ASSET_FAILED");
  assert.equal(result.captureCount, 0);
});

test("an undeclared source fails the capture even when hidden", async t => {
  const neg = await createTempWorkspace("preview-composition-neg-undeclared-");
  t.after(() => neg.cleanup());
  const { bundle: negBundle, scenario } = await minimalApp(
    neg.dir,
    "composition-neg-undeclared",
    `
    import { Image, type NativeNode } from "@hewel/zgui";
    import { render } from "@hewel/zgui/renderer";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return render(() => <Image src="assets/missing.png" style={{ display: "none", width: "64px", height: "16px" }} />, root);
    }
  `,
  );
  const { result } = await harnessRunPreview({
    bundle: negBundle,
    scenario,
    out: path.join(neg.dir, "actual"),
    allowFailure: true,
  });
  assert.equal(result.exitCode, 6);
  assert.equal(result.firstError.code, "E_ASSET_FAILED");
  assert.match(result.firstError.message, /declared package asset/);
  assert.equal(result.captureCount, 0);
});
