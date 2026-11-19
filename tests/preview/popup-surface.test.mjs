import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { after, before, test } from "node:test";
import { buildFixtureApp, createTempWorkspace, emptyResources, getNode, readFrame, readPng, readTree, runScenario } from "./harness.mjs";

let temp;
let bundle;
before(async () => {
  temp = await createTempWorkspace("preview-popup-surface-");
  bundle = await buildFixtureApp(path.join(temp.dir, "allowed"), "popup-surface", new URL("./fixtures/popup-surface.tsx", import.meta.url), {
    capabilities: { shellSurfaces: true },
  });
});
after(async () => { await temp?.cleanup(); });

const left = { id: "left", name: "Left", x: 0, y: 0, width: 320, height: 240, scale: 1, refreshRateMilliHz: 60000 };
const right = { ...left, id: "right", name: "Right", x: 320, width: 640, height: 480, scale: 2 };
const monitors = id => ({ id, op: "monitors.set", monitors: [left, right] });
const capture = (id, surface) => ({ id, op: "capture", captureId: id, surface });
const click = (testId, id = testId) => ({ id, op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const advance = id => ({ id, op: "clock.advance", byMs: 40 });
const dismissals = (id, equals) => ({ id, op: "assert", target: { surface: "main", testId: "dismissals" }, check: { property: "text", equals } });

function run(name, steps, options = {}) {
  return runScenario({ projectDir: temp.dir, bundle, name, steps, requiredCapabilities: [], width: 400, height: 240, scale: 1, ...options });
}

test("bar-anchored and nested popups share a session and retire with their parent", async () => {
  const { out, result } = await run("popup-lifecycle", [
    monitors("outputs"), capture("bar-initial", "bar"), capture("menu-initial", "menu"), capture("submenu-initial", "submenu"),
    dismissals("before-dismiss", "0:0"),
    click("move-anchor"), advance("dispatch-resize"), capture("menu-updated", "menu"),
    { id: "dismiss-menu", op: "popupDone", surface: "menu" },
    advance("present-dismissal"), dismissals("after-dismiss", "1:0"),
    click("move-anchor", "move-dismissed-anchor"), advance("dispatch-move"), monitors("after-anchor-move"), dismissals("no-repeat-dismissal", "1:0"),
    click("remove-menus"), advance("dispatch-remove-menus"), click("declare-menus"), advance("dispatch-declare-menus"),
    capture("menu-redeclared", "menu"), capture("submenu-redeclared", "submenu"),
    click("remove-bar"), advance("dispatch-remove-bar"), monitors("after-bar-removal"), dismissals("cascade-is-not-dismissal", "1:0"),
    click("restore-bar"), advance("dispatch-restore"),
    { id: "replace-session", op: "surface.recreate", surface: "main" },
    capture("submenu-reloaded", "submenu"),
  ], { assertEmptyResources: true });

  const initialDocuments = new Set();
  for (const [surface, text, width, height] of [["bar", "Bar", 640, 56], ["menu", "Bar menu", 240, 192], ["submenu", "Nested menu", 176, 144]]) {
    const artifact = `surfaces/${surface}/${surface}-initial`;
    const tree = await readTree(out, artifact);
    const frame = await readFrame(out, artifact);
    const image = await readPng(path.join(out, `captures/${artifact}.png`));
    assert.equal(tree.surface, surface);
    assert.equal(getNode(tree, "content").text, text);
    assert.equal(frame.generation, 1);
    assert.equal(frame.scale, 2, `${surface} must use the bar's monitor scale`);
    assert.deepEqual([image.width, image.height], [width, height]);
    initialDocuments.add(frame.documentIdentity);
  }
  assert.equal(initialDocuments.size, 3, "each surface owns a separate mount region");
  const updated = await readFrame(out, "surfaces/menu/menu-updated");
  const initial = await readFrame(out, "surfaces/menu/menu-initial");
  const resized = await readPng(path.join(out, "captures/surfaces/menu/menu-updated.png"));
  assert.equal(updated.documentIdentity, initial.documentIdentity, "reposition must preserve the popup mount region");
  assert.deepEqual([resized.width, resized.height], [260, 192], "live popup size tracks its updated logical extent");
  for (const surface of ["menu", "submenu"]) {
    const frame = await readFrame(out, `surfaces/${surface}/${surface}-redeclared`);
    assert.equal(frame.generation, 1);
    assert.ok(!initialDocuments.has(frame.documentIdentity), `${surface} must mount a fresh document`);
  }
  const steps = (await readFile(path.join(out, "steps.jsonl"), "utf8")).trim().split("\n").map(JSON.parse);
  const surfaces = id => steps.find(step => step.id === id).result.surfaces.toSorted();
  assert.deepEqual(surfaces("after-anchor-move"), ["bar", "main"], "dismissed popup declarations must not reopen on placement updates");
  assert.deepEqual(surfaces("after-bar-removal"), ["main"], "closing the bar must detach both popup mount regions");
  assert.equal((await readFrame(out, "surfaces/submenu/submenu-reloaded")).generation, 2);
  assert.equal(result.captureCount, 7);
});

test("a PopupSurface request without shellSurfaces retires the session", async () => {
  const denied = await buildFixtureApp(path.join(temp.dir, "denied"), "denied-popup-surface", `
    import { PopupSurface, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    export const hostApiVersion = 1;
    export function mount(root: NativeNode) {
      return renderApp(() => <PopupSurface name="denied" parent="missing"
        anchorRect={{ x: 0, y: 0, width: 20, height: 10 }} width={80} height={40} />, root);
    }
  `, { rawSource: true });
  const { result } = await run("popup-denied", [capture("denied", "main")], { bundle: denied, allowFailure: true });
  assert.equal(result.firstError?.code, "E_CAPABILITY");
  assert.match(result.firstError?.message, /shellSurfaces/);
  assert.equal(result.captureCount, 0);
  assert.equal(result.teardown.clean, true);
  assert.deepEqual(result.teardown.resources, emptyResources);
});
