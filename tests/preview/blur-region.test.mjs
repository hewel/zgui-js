import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { after, before, test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  defaultProfile,
  getNode,
  pixelAt,
  readFrame,
  readPng,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

const monitor = {
  id: "DP-1",
  name: "DP-1",
  x: 0,
  y: 0,
  width: 640,
  height: 400,
  scale: 1,
  refreshRateMilliHz: 60000,
};

const capture = (id, surface) => ({ id, op: "capture", captureId: id, surface });
const click = (surface, testId, id = `${surface}-${testId}`) => ({
  id,
  op: "pointer.click",
  target: { surface, testId },
  button: "primary",
});
const advance = id => ({ id, op: "clock.advance", byMs: 40 });

let temp;
let profile;
let bundle;

before(async () => {
  temp = await createTempWorkspace("preview-blur-region-");
  const base = JSON.parse(await readFile(defaultProfile, "utf8"));
  const manifest = path.resolve(path.dirname(defaultProfile), base.fonts.manifest);
  profile = path.join(temp.dir, "transparent-blur.json");
  await writeJson(profile, {
    ...base,
    pureShell: false,
    fonts: { ...base.fonts, manifest: path.relative(temp.dir, manifest) },
    surface: {
      ...base.surface,
      width: 320,
      height: 220,
      scale: 1,
      opaque: false,
      background: "#00000000",
    },
  });

  bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "blur-region",
    `
    import {
      Button,
      LayerSurface,
      PopupSurface,
      SessionLockSurface,
      Show,
      WindowSurface,
      createSignal,
      renderApp,
      requestSessionLock,
    } from "@hewel/zgui";
    import type { BlurRegion, NativeNode, SessionLockRequest } from "@hewel/zgui";

    export const hostApiVersion = 1;

    export function mount(root: NativeNode) {
      return renderApp(() => {
        // WindowSurface state
        const [winMode, setWinMode] = createSignal<"default" | "rects" | "node" | "full" | "clear">("default");
        const [winMoved, setWinMoved] = createSignal(false);
        const [winNodeVisible, setWinNodeVisible] = createSignal(true);
        const [winNodeMounted, setWinNodeMounted] = createSignal(true);
        const [winBlurNode, setWinBlurNode] = createSignal<NativeNode | null>(null);

        // LayerSurface state
        const [layerMode, setLayerMode] = createSignal<"default" | "rects" | "node" | "full" | "clear">("default");
        const [layerMoved, setLayerMoved] = createSignal(false);
        const [layerBlurNode, setLayerBlurNode] = createSignal<NativeNode | null>(null);
        const [layerHitNode, setLayerHitNode] = createSignal<NativeNode | null>(null);
        const [layerClicks, setLayerClicks] = createSignal(0);

        // PopupSurface state
        const [popupOpen, setPopupOpen] = createSignal(true);
        const [popupDismissedCount, setPopupDismissedCount] = createSignal(0);
        const [popupMode, setPopupMode] = createSignal<"default" | "rects" | "node" | "full" | "clear">("default");
        const [popupBlurNode, setPopupBlurNode] = createSignal<NativeNode | null>(null);
        const [popupHitNode, setPopupHitNode] = createSignal<NativeNode | null>(null);
        const [popupClicks, setPopupClicks] = createSignal(0);

        // SessionLock state
        const [lockReq, setLockReq] = createSignal<SessionLockRequest>();

        const resolveWinBlur = (): BlurRegion | undefined => {
          switch (winMode()) {
            case "default":
              return undefined;
            case "rects":
              return [
                { x: 10, y: 12, width: 80, height: 30 },
                { x: 120, y: 16, width: 60, height: 24 },
              ];
            case "node": {
              const node = winBlurNode();
              return node ? [node] : [];
            }
            case "full":
              return "full";
            case "clear":
              return undefined;
          }
        };

        const resolveLayerBlur = (): BlurRegion | undefined => {
          switch (layerMode()) {
            case "default":
              return undefined;
            case "rects":
              return [
                { x: 4, y: 6, width: 90, height: 28 },
                { x: 140, y: 8, width: 70, height: 20 },
              ];
            case "node": {
              const node = layerBlurNode();
              return node ? [node] : [];
            }
            case "full":
              return "full";
            case "clear":
              return undefined;
          }
        };

        const resolvePopupBlur = (): BlurRegion | undefined => {
          switch (popupMode()) {
            case "default":
              return undefined;
            case "rects":
              return [{ x: 8, y: 8, width: 100, height: 40 }];
            case "node": {
              const node = popupBlurNode();
              return node ? [node] : [];
            }
            case "full":
              return "full";
            case "clear":
              return undefined;
          }
        };

        return (
          <column style={{ width: "320px", height: "220px", backgroundColor: "transparent" }}>
            <row>
              <Button testId="win-rects" onClick={() => setWinMode("rects")}>WR</Button>
              <Button testId="win-node" onClick={() => setWinMode("node")}>WN</Button>
              <Button testId="win-move" onClick={() => setWinMoved(true)}>WM</Button>
              <Button testId="win-hide" onClick={() => setWinNodeVisible(false)}>WH</Button>
              <Button testId="win-show" onClick={() => setWinNodeVisible(true)}>WS</Button>
              <Button testId="win-detach" onClick={() => setWinNodeMounted(false)}>WD</Button>
              <Button testId="win-full" onClick={() => setWinMode("full")}>WF</Button>
              <Button testId="win-clear" onClick={() => setWinMode("clear")}>WC</Button>
            </row>
            <row>
              <Button testId="layer-rects" onClick={() => setLayerMode("rects")}>LR</Button>
              <Button testId="layer-node" onClick={() => setLayerMode("node")}>LN</Button>
              <Button testId="layer-move" onClick={() => setLayerMoved(true)}>LM</Button>
              <Button testId="layer-full" onClick={() => setLayerMode("full")}>LF</Button>
              <Button testId="layer-clear" onClick={() => setLayerMode("clear")}>LC</Button>
            </row>
            <row>
              <Button testId="pop-rects" onClick={() => setPopupMode("rects")}>PR</Button>
              <Button testId="pop-node" onClick={() => setPopupMode("node")}>PN</Button>
              <Button testId="pop-full" onClick={() => setPopupMode("full")}>PF</Button>
              <Button testId="pop-clear" onClick={() => setPopupMode("clear")}>PC</Button>
              <Button testId="pop-reopen" onClick={() => setPopupOpen(true)}>PO</Button>
              <Button testId="lock-start" onClick={() => setLockReq(requestSessionLock())}>LK</Button>
            </row>
            <text testId="status">
              {\`layerClicks=\${layerClicks()} popupClicks=\${popupClicks()} dismissed=\${popupDismissedCount()} lock=\${lockReq()?.state() ?? "unlocked"}\`}
            </text>

            <WindowSurface
              name="glass-win"
              title="Glass Window"
              width={260}
              height={160}
              transparent
              blurRegion={resolveWinBlur()}
            >
              <box style={{ width: "260px", height: "160px", backgroundColor: "transparent" }}>
                <Show when={winNodeMounted()}>
                  <box
                    ref={setWinBlurNode}
                    testId="win-card"
                    style={{
                      display: winNodeVisible() ? "flex" : "none",
                      position: "fixed",
                      left: winMoved() ? "64px" : "16px",
                      top: winMoved() ? "42px" : "20px",
                      width: "96px",
                      height: "44px",
                      backgroundColor: "rgba(30, 60, 120, 0.45)",
                    }}
                  >
                    <text>Card</text>
                  </box>
                </Show>
              </box>
            </WindowSurface>

            <LayerSurface
              name="glass-bar"
              output="DP-1"
              layer="top"
              anchor={["top", "left", "right"]}
              height={60}
              inputRegion={layerHitNode() ? [layerHitNode()!] : "full"}
              blurRegion={resolveLayerBlur()}
            >
              <box style={{ width: "640px", height: "60px", backgroundColor: "transparent" }}>
                <box
                  ref={setLayerBlurNode}
                  testId="layer-pill"
                  style={{
                    position: "fixed",
                    left: layerMoved() ? "180px" : "24px",
                    top: "10px",
                    width: "120px",
                    height: "36px",
                    backgroundColor: "rgba(20, 40, 80, 0.5)",
                  }}
                >
                  <Button
                    ref={setLayerHitNode}
                    testId="layer-hit"
                    onClick={() => setLayerClicks(n => n + 1)}
                    style={{ width: "80px", height: "28px" }}
                  >
                    Hit
                  </Button>
                </box>
                <Button
                  testId="layer-gap"
                  onClick={() => setLayerClicks(n => n + 100)}
                  style={{
                    position: "fixed",
                    left: "400px",
                    top: "10px",
                    width: "80px",
                    height: "28px",
                  }}
                >
                  Gap
                </Button>
              </box>
            </LayerSurface>

            <Show when={popupOpen()}>
              <PopupSurface
                name="glass-pop"
                parent="glass-bar"
                anchorRect={{ x: 24, y: 10, width: 80, height: 28 }}
                width={200}
                height={100}
                inputRegion={popupHitNode() ? [popupHitNode()!] : "full"}
                blurRegion={resolvePopupBlur()}
                onDismiss={() => {
                  setPopupDismissedCount(n => n + 1);
                  setPopupOpen(false);
                }}
              >
                <box style={{ width: "200px", height: "100px", backgroundColor: "transparent" }}>
                  <box
                    ref={setPopupBlurNode}
                    testId="popup-card"
                    style={{
                      position: "fixed",
                      left: "12px",
                      top: "14px",
                      width: "110px",
                      height: "48px",
                      backgroundColor: "rgba(50, 80, 140, 0.5)",
                    }}
                  >
                    <Button
                      ref={setPopupHitNode}
                      testId="popup-hit"
                      onClick={() => setPopupClicks(n => n + 1)}
                      style={{ width: "64px", height: "28px" }}
                    >
                      PopHit
                    </Button>
                  </box>
                </box>
              </PopupSurface>
            </Show>

            <Show when={lockReq()}>
              <SessionLockSurface name="lock-dp1" output="DP-1">
                <column style={{ width: "100%", height: "100%", backgroundColor: "#142030" }}>
                  <text testId="lock-label">Locked</text>
                </column>
              </SessionLockSurface>
            </Show>
          </column>
        );
      }, root);
    }
  `,
    { rawSource: true, capabilities: { shellSurfaces: true, sessionLock: true } },
  );
});

after(async () => {
  await temp?.cleanup();
});

test("blurRegion end-to-end across transparent WindowSurface, LayerSurface, PopupSurface, recreation, dismissal, and lock fallback", async () => {
  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    profile,
    name: "blur-region-lifecycle",
    requiredCapabilities: ["pixels", "capture.transparent", "surface.multiple", "session-lock"],
    assertEmptyResources: true,
    steps: [
      { id: "outputs", op: "monitors.set", monitors: [monitor] },

      // 1. Defaults: omitted blurRegion === "none" on all three surfaces
      capture("win-default", "glass-win"),
      capture("layer-default", "glass-bar"),
      capture("pop-default", "glass-pop"),

      // 2. Fixed rect union on window, layer, popup
      click("main", "win-rects"),
      click("main", "layer-rects"),
      click("main", "pop-rects"),
      advance("after-rects"),
      capture("win-rects", "glass-win"),
      capture("layer-rects", "glass-bar"),
      capture("pop-rects", "glass-pop"),

      // 3. Node-tracked blurRegion + movement + hide + show + detach
      click("main", "win-node"),
      click("main", "layer-node"),
      click("main", "pop-node"),
      advance("after-node"),
      capture("win-node", "glass-win"),
      capture("layer-node", "glass-bar"),
      capture("pop-node", "glass-pop"),

      click("main", "win-move"),
      click("main", "layer-move"),
      advance("after-move"),
      capture("win-moved", "glass-win"),
      capture("layer-moved", "glass-bar"),

      click("main", "win-hide"),
      advance("after-hide"),
      capture("win-hidden", "glass-win"),

      click("main", "win-show"),
      advance("after-show"),
      capture("win-shown", "glass-win"),

      click("main", "win-detach"),
      advance("after-detach"),
      capture("win-detached", "glass-win"),

      // 4. Full blur + cleared prop + recreated output (stale observer isolation)
      click("main", "win-full"),
      click("main", "layer-full"),
      click("main", "pop-full"),
      advance("after-full"),
      capture("win-full", "glass-win"),
      capture("layer-full", "glass-bar"),
      capture("pop-full", "glass-pop"),

      // Click inside inputRegion hit button on layer & popup while blurRegion="full"
      click("glass-bar", "layer-hit"),
      click("glass-pop", "popup-hit"),
      advance("after-hits"),

      // Recreate layer output while node-observed blur is active (verifies fresh surface & stale observer isolation)
      click("main", "layer-node", "layer-node-before-recreate"),
      advance("before-recreate"),
      { id: "recreate-bar", op: "surface.recreate", surface: "glass-bar" },
      advance("after-recreate"),
      capture("layer-recreated", "glass-bar"),

      // Clear props back to undefined -> "none"
      click("main", "win-clear"),
      click("main", "layer-clear"),
      advance("after-clear"),
      capture("win-cleared", "glass-win"),
      capture("layer-cleared", "glass-bar"),

      // 5. Dismiss popup and verify clean teardown & reopen
      { id: "dismiss-pop", op: "popupDone", surface: "glass-pop" },
      advance("after-dismiss"),
      click("main", "pop-reopen"),
      advance("after-reopen"),
      capture("pop-reopened", "glass-pop"),

      // 6. SessionLockSurface remains opaque and blurRegion === "none"
      click("main", "lock-start"),
      advance("after-lock"),
      capture("lock-surface", "lock-dp1"),
      capture("main-final", "main"),
    ],
  });

  assert.equal(result.teardown?.clean, true, "teardown must be completely clean");

  // Defaults + transparent capture contract
  for (const [cap, surface] of [
    ["win-default", "glass-win"],
    ["layer-default", "glass-bar"],
    ["pop-default", "glass-pop"],
  ]) {
    const frame = await readFrame(out, `surfaces/${surface}/${cap}`);
    assert.equal(frame.blurRegion, "none", `${surface} defaults to blurRegion "none"`);
    assert.equal(frame.renderer.compositorTransparencyValidated, false);
    assert.equal(frame.renderer.alphaMode, "PreMultiplied");
  }

  // Fixed rect unions
  assert.deepEqual((await readFrame(out, "surfaces/glass-win/win-rects")).blurRegion, [
    { x: 10, y: 12, width: 80, height: 30 },
    { x: 120, y: 16, width: 60, height: 24 },
  ]);
  assert.deepEqual((await readFrame(out, "surfaces/glass-bar/layer-rects")).blurRegion, [
    { x: 4, y: 6, width: 90, height: 28 },
    { x: 140, y: 8, width: 70, height: 20 },
  ]);
  assert.deepEqual((await readFrame(out, "surfaces/glass-pop/pop-rects")).blurRegion, [
    { x: 8, y: 8, width: 100, height: 40 },
  ]);

  // Node tracking + movement + hide + show + detach
  assert.deepEqual((await readFrame(out, "surfaces/glass-win/win-node")).blurRegion, [
    { x: 16, y: 20, width: 96, height: 44 },
  ]);
  assert.deepEqual((await readFrame(out, "surfaces/glass-win/win-moved")).blurRegion, [
    { x: 64, y: 42, width: 96, height: 44 },
  ]);
  assert.equal((await readFrame(out, "surfaces/glass-win/win-hidden")).blurRegion, "none");
  assert.deepEqual((await readFrame(out, "surfaces/glass-win/win-shown")).blurRegion, [
    { x: 64, y: 42, width: 96, height: 44 },
  ]);
  assert.equal((await readFrame(out, "surfaces/glass-win/win-detached")).blurRegion, "none");

  assert.deepEqual((await readFrame(out, "surfaces/glass-bar/layer-node")).blurRegion, [
    { x: 24, y: 10, width: 120, height: 36 },
  ]);
  assert.deepEqual((await readFrame(out, "surfaces/glass-bar/layer-moved")).blurRegion, [
    { x: 180, y: 10, width: 120, height: 36 },
  ]);
  assert.deepEqual((await readFrame(out, "surfaces/glass-pop/pop-node")).blurRegion, [
    { x: 12, y: 14, width: 110, height: 48 },
  ]);

  // Full blur
  assert.equal((await readFrame(out, "surfaces/glass-win/win-full")).blurRegion, "full");
  assert.equal((await readFrame(out, "surfaces/glass-bar/layer-full")).blurRegion, "full");
  assert.equal((await readFrame(out, "surfaces/glass-pop/pop-full")).blurRegion, "full");

  // Recreated layer gets a new document identity and preserves tracked node blur
  const beforeRecreate = await readFrame(out, "surfaces/glass-bar/layer-moved");
  const afterRecreate = await readFrame(out, "surfaces/glass-bar/layer-recreated");
  assert.notEqual(
    afterRecreate.documentIdentity,
    beforeRecreate.documentIdentity,
    "surface.recreate must mount a fresh document",
  );
  assert.deepEqual(afterRecreate.blurRegion, [{ x: 180, y: 10, width: 120, height: 36 }]);

  // Cleared props normalize to "none"
  assert.equal((await readFrame(out, "surfaces/glass-win/win-cleared")).blurRegion, "none");
  assert.equal((await readFrame(out, "surfaces/glass-bar/layer-cleared")).blurRegion, "none");

  // Reopened popup resumes its blurRegion cleanly after dismissal
  assert.equal((await readFrame(out, "surfaces/glass-pop/pop-reopened")).blurRegion, "full");

  // Lock surface stays opaque (alpha 255) and blurRegion === "none"
  const lockFrame = await readFrame(out, "surfaces/lock-dp1/lock-surface");
  assert.equal(lockFrame.blurRegion, "none");
  const lockPng = await readPng(path.join(out, "captures/surfaces/lock-dp1/lock-surface.png"));
  assert.equal(pixelAt(lockPng, 10, 10)[3], 255, "lock surface must remain opaque");

  // Verify hit button clicks and popup dismissal succeeded
  const mainTree = await readTree(out, "main-final");
  assert.equal(
    getNode(mainTree, "status").text,
    "layerClicks=1 popupClicks=1 dismissed=1 lock=locked",
  );
});

test('input-pass-through gap remains non-clickable under blurRegion="full"', async () => {
  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    profile,
    name: "blur-full-input-passthrough",
    requiredCapabilities: ["pixels", "capture.transparent", "surface.multiple"],
    allowFailure: true,
    assertEmptyResources: true,
    steps: [
      { id: "outputs", op: "monitors.set", monitors: [monitor] },
      click("main", "layer-full"),
      advance("after-full"),
      // layer-gap sits at x=400, outside inputRegion ([layer-hit] at x=24..104) while blurRegion="full"!
      click("glass-bar", "layer-gap"),
    ],
  });
  assert.equal(
    result.status,
    "failed",
    'clicking in input-pass-through gap must be blocked even under blurRegion="full"',
  );
  assert.equal(result.teardown?.clean, true, "failed hit-test run must still tear down cleanly");
});

test("getBackgroundBlurSupported defaults false, honors profile backgroundBlur, and reacts live to setBackgroundBlurSupported", async () => {
  const base = JSON.parse(await readFile(defaultProfile, "utf8"));
  const manifest = path.resolve(path.dirname(defaultProfile), base.fonts.manifest);
  const pureShellDefaultProfile = path.join(temp.dir, "pure-shell-blur-default.json");
  const pureShellEnabledProfile = path.join(temp.dir, "pure-shell-blur-enabled.json");
  const basePure = {
    ...base,
    pureShell: true,
    fonts: { ...base.fonts, manifest: path.relative(temp.dir, manifest) },
    surface: {
      ...base.surface,
      width: 320,
      height: 120,
      scale: 1,
      opaque: false,
      background: "#00000000",
    },
  };
  await writeJson(pureShellDefaultProfile, basePure);
  await writeJson(pureShellEnabledProfile, { ...basePure, backgroundBlur: true });

  const shellBundle = await buildFixtureApp(
    path.join(temp.dir, "app-blur-cap"),
    "blur-capability",
    `
    import {
      LayerSurface,
      createMemo,
      getBackgroundBlurSupported,
      mountShellStylesheet,
      renderShell,
    } from "@hewel/zgui";

    export const hostApiVersion = 1;

    export function mount() {
      return renderShell(() => {
        mountShellStylesheet("reset", ":root { padding: 0; background-color: transparent; }");
        const mode = createMemo(() => (getBackgroundBlurSupported() ? "glass" : "solid"));
        return (
          <LayerSurface
            name="status-bar"
            output="DP-1"
            layer="top"
            anchor={["top", "left"]}
            width={240}
            height={60}
            blurRegion={getBackgroundBlurSupported() ? "full" : "none"}
          >
            <box
              testId="panel"
              style={{
                width: "240px",
                height: "60px",
                backgroundColor: getBackgroundBlurSupported() ? "#38bdf880" : "#1e293b",
              }}
            >
              <text testId="blur-mode">{mode()}</text>
            </box>
          </LayerSurface>
        );
      });
    }
    `,
    { rawSource: true, capabilities: { shellSurfaces: true } },
  );

  // 1. Default profile (backgroundBlur omitted -> false): live false -> true -> false.
  {
    const { out, result } = await runScenario({
      projectDir: temp.dir,
      bundle: shellBundle,
      profile: pureShellDefaultProfile,
      name: "blur-capability-toggle",
      requiredCapabilities: ["pixels", "capture.transparent", "surface.multiple"],
      assertEmptyResources: true,
      steps: [
        { id: "outputs", op: "monitors.set", monitors: [monitor] },
        capture("initial-solid", "status-bar"),
        { id: "enable-blur", op: "setBackgroundBlurSupported", supported: true },
        capture("live-glass", "status-bar"),
        { id: "disable-blur", op: "setBackgroundBlurSupported", supported: false },
        capture("fallback-solid", "status-bar"),
      ],
    });
    assert.equal(result.teardown?.clean, true, "default toggle run must tear down cleanly");

    const solidTree = await readTree(out, "surfaces/status-bar/initial-solid");
    assert.equal(getNode(solidTree, "blur-mode").text, "solid");
    const solidFrame = await readFrame(out, "surfaces/status-bar/initial-solid");
    assert.equal(solidFrame.blurRegion, "none");
    assert.equal(solidFrame.renderer.compositorTransparencyValidated, false);
    const solidPng = await readPng(
      path.join(out, "captures/surfaces/status-bar/initial-solid.png"),
    );
    assert.equal(pixelAt(solidPng, 200, 30)[3], 255, "solid fallback must render opaque alpha");

    const glassTree = await readTree(out, "surfaces/status-bar/live-glass");
    assert.equal(getNode(glassTree, "blur-mode").text, "glass");
    const glassFrame = await readFrame(out, "surfaces/status-bar/live-glass");
    assert.equal(glassFrame.blurRegion, "full");
    assert.equal(glassFrame.renderer.compositorTransparencyValidated, false);
    const glassPng = await readPng(path.join(out, "captures/surfaces/status-bar/live-glass.png"));
    assert.ok(
      pixelAt(glassPng, 200, 30)[3] < 200 && pixelAt(glassPng, 200, 30)[3] > 60,
      "glass mode must render translucent panel pixels",
    );

    const fallbackTree = await readTree(out, "surfaces/status-bar/fallback-solid");
    assert.equal(getNode(fallbackTree, "blur-mode").text, "solid");
    const fallbackFrame = await readFrame(out, "surfaces/status-bar/fallback-solid");
    assert.equal(fallbackFrame.blurRegion, "none");
    assert.equal(fallbackFrame.renderer.compositorTransparencyValidated, false);
  }

  // 2. Profile with backgroundBlur: true starts directly in glass mode and falls back on toggle false.
  {
    const { out, result } = await runScenario({
      projectDir: temp.dir,
      bundle: shellBundle,
      profile: pureShellEnabledProfile,
      name: "blur-capability-profile-enabled",
      requiredCapabilities: ["pixels", "capture.transparent", "surface.multiple"],
      assertEmptyResources: true,
      steps: [
        { id: "outputs", op: "monitors.set", monitors: [monitor] },
        capture("initial-glass", "status-bar"),
        { id: "withdraw-blur", op: "setBackgroundBlurSupported", supported: false },
        capture("withdrawn-solid", "status-bar"),
      ],
    });
    assert.equal(result.teardown?.clean, true, "profile-enabled run must tear down cleanly");

    const initTree = await readTree(out, "surfaces/status-bar/initial-glass");
    assert.equal(getNode(initTree, "blur-mode").text, "glass");
    const initFrame = await readFrame(out, "surfaces/status-bar/initial-glass");
    assert.equal(initFrame.blurRegion, "full");
    assert.equal(initFrame.renderer.compositorTransparencyValidated, false);

    const withdrawnTree = await readTree(out, "surfaces/status-bar/withdrawn-solid");
    assert.equal(getNode(withdrawnTree, "blur-mode").text, "solid");
    const withdrawnFrame = await readFrame(out, "surfaces/status-bar/withdrawn-solid");
    assert.equal(withdrawnFrame.blurRegion, "none");
  }
});
