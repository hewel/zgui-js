import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { LayerSurface, PopupSurface, Show, createSignal, onCleanup, renderShell } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
const [panel, setPanel] = createSignal(true);
const [menu, setMenu] = createSignal(true);
const [anchor, setAnchor] = createSignal<NativeNode>();
const [size, setSize] = createSignal(48);
const [anchorChoice, setAnchorChoice] = createSignal(1);
const dismissed: string[] = [];
const cleaned: string[] = [];
export { anchor, setPanel, setMenu, setSize, setAnchorChoice, dismissed, cleaned };
const rect = { x: 4, y: 8, width: 16, height: 10 };
function Content(props: { name: string }) {
  onCleanup(() => cleaned.push(props.name));
  return <text testId={props.name}>{props.name}</text>;
}
function Anchor(props: { name: string; retainRef: boolean }) {
  return <box testId={props.name} ref={node => {
    setAnchor(node);
    if (!props.retainRef) onCleanup(() => setAnchor(current => current === node ? undefined : current));
  }} />;
}
export function mount(withNode = false, retainRef = false) {
  return renderShell(() => <>
    <Show when={menu()}>
      {withNode ? <PopupSurface name="menu" parent="panel" anchorNode={anchor()} width={size()} height={30}
        onDismiss={() => dismissed.push("menu")}><Content name="menu-content" /></PopupSurface>
      : <PopupSurface name="menu" parent="panel" anchorRect={rect} width={size()} height={30}
        onDismiss={() => dismissed.push("menu")}><Content name="menu-content" /></PopupSurface>}
    </Show>
    <Show when={panel()}>
      <LayerSurface name="panel">
        <Show when={anchorChoice() === 1}>
          <Anchor name="anchor" retainRef={retainRef} />
        </Show>
        <Show when={anchorChoice() === 2}>
          <Anchor name="other-anchor" retainRef={retainRef} />
        </Show>
      </LayerSurface>
    </Show>
    <Show when={anchorChoice() === 3}>
      <LayerSurface name="other-panel">
        <Anchor name="foreign-anchor" retainRef={retainRef} />
      </LayerSurface>
    </Show>
    <PopupSurface name="submenu" parent="menu" anchorRect={rect} width={12} height={14}
      offset={{ x: 3, y: -2 }} grab placement={{ anchor: "right", gravity: "right", slide: false, flip: false, resize: true, reactive: true }}
      onDismiss={() => dismissed.push("submenu")}><Content name="submenu-content" /></PopupSurface>
  </>);
}
`;

async function setup(t, options) {
  const mod = await moduleFor(t, fixture);
  const harness = createHost(options);
  const previous = globalThis.__zguiShell;
  globalThis.__zguiShell = harness.shell;
  let dispose;
  const stop = () => {
    if (dispose) harness.runTurn(dispose);
    dispose = undefined;
    harness.flush();
  };
  t.after(() => {
    try { stop(); }
    finally {
      if (previous === undefined) delete globalThis.__zguiShell;
      else globalThis.__zguiShell = previous;
    }
  });
  const mount = (...args) => { dispose = harness.runTurn(() => mod.mount(...args)); };
  return { harness, mod, mount, stop };
}

const rect = (x, y) => ({ x, y, width: 16, height: 10 });

function geometryFor(harness, testId) {
  const node = harness.byTestId(testId);
  assert.ok(node);
  const observer = harness.geometry.find(record => record.active && record.node === node && record.kind === "borderBox");
  assert.ok(observer);
  return observer;
}

test("named popups wait for live parents, mount independently, and update placement in place", async t => {
  const { harness, mod, mount, stop } = await setup(t);
  mount();
  assert.equal(harness.surfaces.size, 0);
  harness.flush();
  assert.deepEqual([...harness.surfaces.keys()], ["panel", "menu", "submenu"]);
  const menu = harness.surfaces.get("menu");
  const content = harness.byTestId("menu-content");
  assert.ok(content);
  assert.ok(harness.byTestId("submenu-content"));
  assert.deepEqual(menu.placement, {
    parent: "panel", anchorRect: rect(4, 8), anchor: "bottom-left", gravity: "bottom-right", offset: { x: 0, y: 0 },
    slide: true, flip: true, resize: false, grab: false, reactive: false, width: 48, height: 30,
  });
  assert.deepEqual(harness.surfaces.get("submenu").placement, {
    parent: "menu", anchorRect: rect(4, 8), anchor: "right", gravity: "right", offset: { x: 3, y: -2 },
    slide: false, flip: false, resize: true, grab: true, reactive: true, width: 12, height: 14,
  });
  harness.runTurn(() => mod.setSize(64));
  harness.flush();
  assert.equal(harness.surfaces.get("menu"), menu);
  assert.equal(menu.placement.width, 64);
  assert.equal(harness.byTestId("menu-content"), content);
  stop();
  assert.equal(harness.surfaces.size, 0);
  assert.deepEqual(mod.cleaned, ["submenu-content", "menu-content"]);
  assert.deepEqual(mod.dismissed, []);
});

test("node anchors follow layout and dynamic refs without reopening after dismissal", async t => {
  const { harness, mod, mount, stop } = await setup(t);
  mount(true);
  harness.flush();
  assert.deepEqual([...harness.surfaces.keys()], ["panel"]);
  const initial = geometryFor(harness, "anchor");
  initial.emit(rect(11, 13));
  harness.flush();
  const menu = harness.surfaces.get("menu");
  assert.deepEqual(menu.placement.anchorRect, rect(11, 13));
  initial.emit(rect(16, 19));
  harness.flush();
  assert.equal(harness.surfaces.get("menu"), menu);
  assert.deepEqual(menu.placement.anchorRect, rect(16, 19));
  harness.runTurn(() => mod.setAnchorChoice(2));
  harness.flush();
  const replacement = geometryFor(harness, "other-anchor");
  assert.equal(initial.active, false);
  assert.equal(harness.surfaces.has("menu"), false, "replacement waits for its own layout");
  replacement.emit(rect(24, 27));
  harness.flush();
  assert.deepEqual(harness.surfaces.get("menu").placement.anchorRect, rect(24, 27));

  harness.popupDone("menu");
  harness.flush();
  assert.deepEqual(mod.dismissed, ["menu"]);
  assert.deepEqual(mod.cleaned, ["submenu-content", "menu-content", "submenu-content", "menu-content"]);
  assert.equal(replacement.active, false);
  harness.runTurn(() => { mod.setAnchorChoice(1); mod.setSize(96); });
  harness.flush();
  assert.deepEqual([...harness.surfaces.keys()], ["panel"]);
  assert.equal(harness.geometry.some(record => record.active), false);

  harness.runTurn(() => mod.setMenu(false));
  harness.runTurn(() => mod.setMenu(true));
  const fresh = geometryFor(harness, "anchor");
  fresh.emit(rect(31, 37));
  harness.flush();
  assert.notEqual(harness.surfaces.get("menu"), menu);
  assert.equal(harness.surfaces.get("menu").placement.width, 96);
  assert.equal(harness.surfaces.has("submenu"), false, "cascaded dismissal requires a fresh descendant declaration");
  stop();
  assert.equal(fresh.active, false);
});

test("detaching a retained node anchor closes and unmounts the popup without dismissing it", async t => {
  const { harness, mod, mount } = await setup(t);
  mount(true, true);
  harness.flush();
  const initial = geometryFor(harness, "anchor");
  const anchor = mod.anchor();
  initial.emit(rect(11, 13));
  harness.flush();
  assert.ok(harness.byTestId("menu-content"));
  assert.ok(harness.byTestId("submenu-content"));

  harness.runTurn(() => mod.setAnchorChoice(0));
  harness.flush();
  assert.equal(mod.anchor(), anchor, "the app still holds the detached ref");
  assert.equal(initial.active, false);
  assert.deepEqual([...harness.surfaces.keys()], ["panel"]);
  assert.equal(harness.byTestId("menu-content"), undefined);
  assert.deepEqual(mod.cleaned, ["submenu-content", "menu-content"]);
  assert.deepEqual(mod.dismissed, []);

  harness.runTurn(() => mod.setSize(96));
  harness.runTurn(() => mod.setPanel(false));
  harness.flush();
  harness.runTurn(() => mod.setPanel(true));
  harness.flush();
  assert.deepEqual([...harness.surfaces.keys()], ["panel"], "a new parent must not reuse the old placement");

  harness.runTurn(() => mod.setAnchorChoice(2));
  harness.flush();
  const replacement = geometryFor(harness, "other-anchor");
  assert.equal(harness.surfaces.has("menu"), false);
  replacement.emit(rect(24, 27));
  harness.flush();
  assert.deepEqual(harness.surfaces.get("menu").placement.anchorRect, rect(24, 27));
});

test("a node in another shell surface cannot anchor a popup", async t => {
  const { harness, mod, mount } = await setup(t);
  const warnings = [];
  t.mock.method(console, "warn", message => { warnings.push(message); });
  mod.setAnchorChoice(3);
  mount(true);
  harness.flush();
  const node = harness.byTestId("foreign-anchor");
  assert.ok(node);
  assert.equal(harness.geometry.some(record => record.active && record.node === node), false);
  assert.deepEqual(warnings, ['PopupSurface "menu": anchorNode must belong to parent surface "panel"']);
  assert.deepEqual([...harness.surfaces.keys()], ["panel", "other-panel"]);

  harness.runTurn(() => mod.setAnchorChoice(1));
  harness.flush();
  geometryFor(harness, "anchor").emit(rect(31, 37));
  harness.flush();
  assert.deepEqual(harness.surfaces.get("menu").placement.anchorRect, rect(31, 37));
});

test("popup requests reject a cycle on open and update without changing the valid chain", () => {
  const { shell, surfaces, flush } = createHost();
  const attach = () => () => {};
  const dismiss = () => assert.fail("rejecting a cycle must not dismiss a popup");
  const placement = parent => ({
    parent, anchorRect: rect(4, 8), anchor: "bottom-left", gravity: "bottom-right", offset: { x: 0, y: 0 },
    slide: true, flip: true, resize: false, grab: false, reactive: false, width: 48, height: 30,
  });
  shell.openPopup("a", placement("b"), attach, dismiss);
  assert.throws(() => shell.openPopup("b", placement("a"), attach, dismiss), { name: "TypeError", message: /cycle/ });
  const b = shell.openPopup("b", placement("panel"), attach, dismiss);
  assert.throws(() => shell.updatePopup(b, placement("a")), { name: "TypeError", message: /cycle/ });
  shell.openLayer("panel", {}, attach);
  flush();
  assert.deepEqual([...surfaces.keys()], ["panel", "b", "a"]);
});

test("closing a parent unmounts descendants without firing compositor dismissal callbacks", async t => {
  const { harness, mod, mount } = await setup(t);
  mount();
  harness.flush();
  harness.runTurn(() => mod.setPanel(false));
  harness.flush();
  assert.equal(harness.surfaces.size, 0);
  assert.deepEqual(mod.cleaned, ["submenu-content", "menu-content"]);
  assert.deepEqual(mod.dismissed, []);
  harness.runTurn(() => mod.setPanel(true));
  harness.flush();
  assert.deepEqual([...harness.surfaces.keys()], ["panel", "menu", "submenu"]);
  assert.deepEqual(mod.dismissed, []);
});

test("shell surface authority is required for a popup request even while its parent is absent", async t => {
  const { harness, mod, mount } = await setup(t, { shellSurfaces: false });
  mod.setPanel(false);
  assert.throws(() => mount(), { code: "E_CAPABILITY" });
  assert.equal(harness.surfaces.size, 0);
});

test("disposing pending popup declarations cancels asynchronous attachment", async t => {
  const { harness, mod, mount, stop } = await setup(t);
  mount();
  stop();
  assert.equal(harness.surfaces.size, 0);
  assert.deepEqual(mod.cleaned, []);
  assert.deepEqual(mod.dismissed, []);
  mount();
  harness.flush();
  assert.deepEqual([...harness.surfaces.keys()], ["panel", "menu", "submenu"]);
});
