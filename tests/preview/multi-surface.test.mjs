import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cp, mkdir, readFile } from "node:fs/promises";
import path from "node:path";
import { after, before, test } from "node:test";
import { buildFixtureApp, createTempWorkspace, defaultProfile, emptyResources, getNode, nativeHostBinary, pixelAt, readFrame, readPng, readTree, runScenario, workspaceRoot, writeJson } from "./harness.mjs";

let temp;
let profile;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-multi-surface-");
  const base = JSON.parse(await readFile(defaultProfile, "utf8"));
  const fonts = path.resolve(path.dirname(defaultProfile), path.dirname(base.fonts.manifest));
  await mkdir(path.join(temp.dir, "profiles"));
  profile = path.join(temp.dir, "profiles/shell.json");
  await writeJson(profile, { ...base, pureShell: true, fonts: { ...base.fonts, manifest: path.relative(path.dirname(profile), path.join(fonts, path.basename(base.fonts.manifest))) } });
  bundle = await buildFixtureApp(path.join(temp.dir, "shell"), "multi-surface", new URL("./fixtures/multi-surface.tsx", import.meta.url), { capabilities: { shellSurfaces: true } });
});
after(async () => { await temp?.cleanup(); });
const left = { id: "left", name: "Left", x: 0, y: 0, width: 240, height: 200, scale: 1, refreshRateMilliHz: 60000 };
const right = { ...left, id: "right", name: "Right", x: 240, width: 300, scale: 2 };
const monitors = (id, ...monitors) => ({ id, op: "monitors.set", monitors });
const capture = (id, surface, captureId = id) => ({ id, op: "capture", surface, captureId });

function run(name, steps, options = {}) {
  return runScenario({ projectDir: temp.dir, bundle, profile, name, steps,
    requiredCapabilities: ["pixels", "virtual-time", "surface.multiple", "selectors.test-id", "input.pointer", "tree.inspect"],
    ...options });
}

test("a pure-shell Session captures independent outputs and retains shared state through hotplug", async () => {
  const { out, result } = await run("shell-hotplug", [
    { id: "zero-windows", op: "settle" }, monitors("plug", left, right),
    capture("left-initial", "left", "initial"), capture("right-initial", "right", "initial"),
    { id: "click-left", op: "pointer.click", target: { surface: "left", testId: "counter" }, button: "primary" },
    { id: "dispatch-click", op: "clock.advance", byMs: 20 },
    capture("left-changed", "left"), capture("right-changed", "right"),
    { id: "hidden-hit-area", op: "pointer.click", surface: "left", position: { x: 48, y: 12 }, button: "primary" },
    { id: "right-shared-state", op: "assert", target: { surface: "right", testId: "counter" }, check: { property: "text", equals: "Right:1" } },
    monitors("unplug-left", right), capture("right-alone", "right"),
    { id: "recreate-right", op: "surface.recreate", surface: "right" }, capture("right-recreated", "right"),
    monitors("unplug-all"), { id: "settle-empty", op: "settle" },
    monitors("replug-left", { ...left, width: 260 }), capture("left-replugged", "left"),
  ]);
  assert.equal(result.captureCount, 7);
  const expected = [
    ["left/initial", "left", "Left:0", 240, 1], ["right/initial", "right", "Right:0", 300, 2],
    ["left/left-changed", "left", "Left:1", 240, 1], ["right/right-changed", "right", "Right:1", 300, 2],
    ["right/right-alone", "right", "Right:1", 300, 2], ["right/right-recreated", "right", "Right:1", 300, 2],
    ["left/left-replugged", "left", "Left:1", 260, 1],
  ];
  const documents = new Map();
  for (const [file, surface, text, width, scale] of expected) {
    const tree = await readTree(out, `surfaces/${file}`);
    const frame = await readFrame(out, `surfaces/${file}`);
    const image = await readPng(path.join(out, `captures/surfaces/${file}.png`));
    assert.equal(frame.surface, surface);
    assert.equal(tree.surface, surface);
    assert.equal(frame.generation, 1);
    assert.equal(frame.scale, scale);
    documents.set(file, frame.documentIdentity);
    const node = getNode(tree, "counter");
    assert.equal(node.text, text);
    assert.equal(node.boundsLogical.width, 96);
    assert.equal(node.boundsDevice.width, 96 * scale);
    assert.deepEqual([image.width, image.height], [width, 48 * scale]);
    assert.deepEqual(pixelAt(image, width - 2, 48 * scale - 2), [18, 52, 86, 255]);
  }
  assert.equal(documents.get("right/initial"), documents.get("right/right-alone"), "unplugging left kept the right document");
  assert.notEqual(documents.get("right/right-alone"), documents.get("right/right-recreated"), "named recreate replaced only its own document");
  assert.notEqual(documents.get("left/initial"), documents.get("left/left-replugged"), "replugging left mounted a fresh document");
  const steps = (await readFile(path.join(out, "steps.jsonl"), "utf8")).trim().split("\n").map(JSON.parse);
  const settled = steps.find(step => step.id === "zero-windows").result;
  assert.equal(settled.appReady, true);
  assert.equal(settled.frameCurrent, false);
  assert.equal(steps.find(step => step.id === "hidden-hit-area").result.inputRegionAccepted, false);
  assert.deepEqual(steps.find(step => step.id === "unplug-all").result.surfaces, []);

  const native = (...args) => {
    const child = spawnSync(nativeHostBinary, ["preview", ...args], { cwd: workspaceRoot, encoding: "utf8", timeout: 15000 });
    assert.ifError(child.error);
    assert.equal(child.status, 0, `${child.stdout}\n${child.stderr}`);
    return JSON.parse(child.stdout);
  };
  const inspected = native("inspect", "--run", out, "--capture", "initial", "--surface", "right", "--test-id", "counter", "--fields", "surface,text");
  assert.deepEqual(inspected.nodes[0], { surface: "right", text: "Right:0", truncatedFields: [] });

  const reviewed = path.join(temp.dir, "reviewed");
  const policy = path.join(reviewed, "policy.json");
  const copied = path.join(reviewed, "run");
  await mkdir(reviewed);
  await cp(out, copied, { recursive: true });
  await cp(path.join(workspaceRoot, "preview/comparison/strict.json"), policy);
  const digest = async file => `sha256:${createHash("sha256").update(await readFile(file)).digest("hex")}`;
  await writeJson(path.join(reviewed, "baseline.json"), {
    schemaVersion: 1,
    run: "run",
    artifactsDigest: await digest(path.join(copied, "artifacts.json")),
    policyDigest: await digest(policy),
    review: {
      by: "multi-surface-test", at: "2026-11-18", summary: "Check both named output capture identities.",
      buildId: result.build.buildId,
      captures: expected.map(([file, surface]) => ({ captureId: path.basename(file), surface })),
    },
  });
  const compared = native("compare", "--baseline", reviewed, "--actual", out, "--policy", policy, "--out", path.join(temp.dir, "comparison"));
  assert.equal(compared.captures.compared, result.captureCount);
});

test("a caught unauthorized LayerSurface open still retires the Session", async () => {
  const deniedBundle = await buildFixtureApp(path.join(temp.dir, "denied"), "denied-shell", `
    import { LayerSurface, renderApp, renderShell } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => {
        try { renderShell(() => <LayerSurface name="denied" width={40} height={40} />); } catch {}
        return <text>caught</text>;
      }, root);
    }
  `, { rawSource: true });
  const { result } = await run("shell-denied", [capture("denied", "main")], {
    bundle: deniedBundle, profile: defaultProfile, allowFailure: true,
  });
  assert.equal(result.firstError?.code, "E_CAPABILITY");
  assert.match(result.firstError?.message, /shellSurfaces/);
  assert.equal(result.captureCount, 0);
});

test("a native fault with two live shell regions releases every bridge and surface", async () => {
  // At 5x scale a stretched width of 16384 device pixels rounds to 3277 logical
  // pixels, exceeding the native preview's device extent limit on reconfigure.
  const { result } = await run("shell-fault", [
    monitors("plug-both", left, right), capture("left-before-fault", "left"), capture("right-before-fault", "right"),
    monitors("oversize-right", left, { ...right, width: 16384, scale: 5 }),
  ], { allowFailure: true });
  assert.equal(result.captureCount, 2, "both mount regions must present before the fault");
  assert.equal(result.firstError?.step, "oversize-right", "the fault must come from reconfiguring a live shell surface");
  assert.match(result.firstError?.message, /shell surface exceeds its physical extent budget/);
  assert.equal(result.teardown.clean, true, "the faulted session must retire cleanly");
  assert.deepEqual(result.teardown.resources, emptyResources, "no bridge resources or native shell surfaces may survive");
});

const themeCapabilities = { shellSurfaces: true, controlChannel: true };
const themeSource = `
  import { onControlMessage } from "@hewel/zgui-services";
  export const hostApiVersion = 1;
  const fill = "position: fixed; left: 0; top: 0; width: 100%; height: 100%; background-color: var(--color-primary);";
  const theme = (color: string) => ":root { --color-primary: " + color + "; }";
`;
const themeCommand = command => ({
  id: command, op: "service.emit", channel: "control.message",
  value: { args: [command], reply: { text: "" } },
});

async function runTheme(name, source, steps) {
  const bundle = await buildFixtureApp(path.join(temp.dir, name), name, themeSource + source, {
    rawSource: true, capabilities: themeCapabilities,
  });
  const fixture = path.join(temp.dir, `${name}-fixture.json`);
  await writeJson(fixture, { schemaVersion: 1, id: name, services: { control: {} }, events: [] });
  return run(name, steps, {
    bundle, fixture, requiredCapabilities: ["pixels", "surface.multiple", "services.fixtures"],
  });
}

async function checkTheme(out, surface, id, color) {
  const artifact = `surfaces/${surface}/${id}`;
  const image = await readPng(path.join(out, `captures/${artifact}.png`));
  assert.deepEqual(pixelAt(image, 12, 12), color, artifact);
  return readFrame(out, artifact);
}

test("a pure CSS custom property replacement presents new pixels on the first settle", async () => {
  const { out } = await runTheme("region-theme", `
    import { getOwner, LayerSurface, mountStylesheet, renderShell, runWithOwner } from "@hewel/zgui";
    function SurfaceContent() {
      mountStylesheet("theme", theme("#ff0000"));
      const owner = getOwner();
      onControlMessage(() => { runWithOwner(owner, () => mountStylesheet("theme", theme("#7dd3fc"))); });
      return <box style={fill} />;
    }
    export function mount() {
      return renderShell(() => <LayerSurface name="left" output="left" anchor={["top", "left", "right"]} height={32}>
        <SurfaceContent />
      </LayerSurface>);
    }
  `, [
    monitors("plug-left", left),
    { id: "settle-red", op: "settle" },
    capture("red", "left"),
    themeCommand("replace-theme"),
    { id: "settle-blue", op: "settle" },
    capture("blue", "left"),
  ]);
  const red = await checkTheme(out, "left", "red", [255, 0, 0, 255]);
  const blue = await checkTheme(out, "left", "blue", [125, 211, 252, 255]);
  const steps = (await readFile(path.join(out, "steps.jsonl"), "utf8")).trim().split("\n").map(JSON.parse);
  const settled = steps.find(step => step.id === "settle-blue").result;
  assert.equal(settled.frameCurrent, true);
  assert.equal(blue.readiness.timeUs, red.readiness.timeUs);
  assert.equal(blue.pixelPresentation.documentRevision, red.pixelPresentation.documentRevision);
  assert.ok(blue.pixelPresentation.styleRevision > red.pixelPresentation.styleRevision);
  assert.equal(blue.pixelPresentation.styleRevision, settled.requiredStyleRevision);
});

test("an ownerless shell theme effect updates live layers and a later popup inherits it", async () => {
  const { out } = await runTheme("shell-theme", `
    import { createEffect, createSignal, getOwner, LayerSurface, mountShellStylesheet,
      onCleanup, PopupSurface, renderShell, Show } from "@hewel/zgui";
    export function mount() {
      return renderShell(() => {
        const [color, setColor] = createSignal("#ff0000");
        const [open, setOpen] = createSignal(false);
        let dispose: (() => void) | undefined;
        createEffect(color, value => {
          if (getOwner() !== null) throw new Error("expected an ownerless apply callback");
          dispose = mountShellStylesheet("theme", theme(value));
        });
        onCleanup(() => dispose?.());
        onControlMessage(([command]) => {
          if (command === "replace-theme") setColor("#7dd3fc");
          if (command === "open-popup") setOpen(true);
        });
        return <>
          <LayerSurface name="left" output="left" anchor={["top", "left", "right"]} height={32}><box style={fill} /></LayerSurface>
          <LayerSurface name="right" output="right" anchor={["top", "left", "right"]} height={32}><box style={fill} /></LayerSurface>
          <Show when={open()}>
            <PopupSurface name="menu" parent="left" width={64} height={48}
              anchorRect={{ x: 8, y: 0, width: 32, height: 32 }}><box style={fill} /></PopupSurface>
          </Show>
        </>;
      });
    }
  `, [
    monitors("plug-both", left, right),
    capture("left-red", "left"), capture("right-red", "right"),
    themeCommand("replace-theme"),
    { id: "settle-theme", op: "settle" },
    capture("left-blue", "left"), capture("right-blue", "right"),
    themeCommand("open-popup"),
    capture("menu-blue", "menu"),
  ]);
  for (const surface of ["left", "right"]) {
    const red = await checkTheme(out, surface, `${surface}-red`, [255, 0, 0, 255]);
    const blue = await checkTheme(out, surface, `${surface}-blue`, [125, 211, 252, 255]);
    assert.ok(blue.pixelPresentation.styleRevision > red.pixelPresentation.styleRevision);
  }
  await checkTheme(out, "menu", "menu-blue", [125, 211, 252, 255]);
});

test("a shell effect releases its targeted sheet after the output retires", async () => {
  const { out } = await runTheme("targeted-hotplug", `
    import { createEffect, createSignal, LayerSurface, mountStylesheet, renderShell } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    export function mount() {
      return renderShell(() => {
        const [target, setTarget] = createSignal<NativeNode>();
        const [enabled, setEnabled] = createSignal(true);
        createEffect(() => enabled() ? target() : undefined,
          node => node ? mountStylesheet("theme", theme("#ff0000"), node) : undefined);
        onControlMessage(([command]) => { setEnabled(command === "enable"); });
        return <>
          <LayerSurface name="left" output="left" anchor={["top", "left", "right"]} height={32}>
            <box ref={node => { setTarget(node); }} style={fill + "background-color: var(--color-primary, #0000ff);"} />
          </LayerSurface>
          <LayerSurface name="right" output="right"><box /></LayerSurface>
        </>;
      });
    }
  `, [
    monitors("plug", left, right),
    capture("installed", "left"),
    monitors("unplug-left", right),
    themeCommand("disable"),
    monitors("replug-left", left, right),
    capture("retired", "left"),
  ]);
  await checkTheme(out, "left", "installed", [255, 0, 0, 255]);
  await checkTheme(out, "left", "retired", [0, 0, 255, 255]);
});
