import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import {
  For, LayerSurface, Show, createContext, createSignal, focus, getTextContent,
  mountStylesheet, onCleanup, precedes, renderShell, useContext, useMonitors,
} from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
const Theme = createContext("missing");
const [vertical, setVertical] = createSignal(false);
const [output, setOutput] = createSignal("DP-1");
const [count, setCount] = createSignal(0);
const [visible, setVisible] = createSignal(true);
const [linked, setLinked] = createSignal(true);
const [inputNode, setInputNode] = createSignal<NativeNode>();
let retainInputRef = false;
let disposed = 0;
export { inputNode, setVertical, setOutput, setVisible, setLinked };
export const retainInputNode = () => { retainInputRef = true; };
export const disposals = () => disposed;
export const read = (node: NativeNode) => getTextContent(node);
export const ordered = (first: NativeNode, second: NativeNode) => precedes(first, second);

function Panel(props: { name: string; ref?: (node: NativeNode) => void }) {
  const theme = useContext(Theme);
  onCleanup(() => { disposed++; });
  return <control testId={props.name} ref={props.ref} onClick={event => {
    focus(event.currentTarget);
    mountStylesheet("clicked", "control { opacity: .8; }");
    event.preventDefault();
    setCount(value => value + 1);
  }}>{theme}:{count()}</control>;
}

function Shell() {
  const monitors = useMonitors();
  return <>
    <LayerSurface name="panel" output={output()} layer={vertical() ? "overlay" : "top"}
      anchor={vertical() ? ["left", "top", "bottom"] : ["top", "left", "right"]}
      width={vertical() ? 28 : 0} height={vertical() ? 0 : 28}
      margin={vertical() ? { left: 4 } : { top: 2 }} exclusiveZone={28}
      keyboard={vertical() ? "on-demand" : "none"}
      inputRegion={linked() ? (inputNode() ? [inputNode()!] : []) : [{ x: 2, y: 4, width: 12, height: 10 }]}>
      <Show when={visible()}>
        <box><Panel name="counter" ref={node => {
          setInputNode(node);
          onCleanup(() => {
            if (!retainInputRef) setInputNode(current => current === node ? undefined : current);
          });
        }} /></box>
      </Show>
    </LayerSurface>
    <For each={monitors()} keyed={monitor => monitor.id}>{monitor =>
      <LayerSurface name={'overlay-' + monitor().id} output={monitor().id} inputRegion="empty">
        <Panel name={'counter:' + monitor().id} />
        <text testId={'geometry:' + monitor().id}>{monitor().name}:{monitor().scale}:{monitor().refreshRateMilliHz}</text>
      </LayerSurface>
    }</For>
  </>;
}

export function mount() {
  return renderShell(() => <Theme value="shared"><Shell /></Theme>);
}
`;

const monitor = (id, name, x) => ({ id, name, x, y: 0, width: 1920, height: 1080, scale: 2, refreshRateMilliHz: 60000 });

async function mounted(t, attach = true) {
  const harness = createHost({ monitors: [monitor("DP-1", "Desk", 0)] });
  const mod = await moduleFor(t, fixture);
  const previous = globalThis.__zguiShell;
  globalThis.__zguiShell = harness.shell;
  let dispose;
  t.after(() => {
    try { dispose?.(); harness.flush(); }
    finally {
      if (previous === undefined) delete globalThis.__zguiShell;
      else globalThis.__zguiShell = previous;
    }
  });
  dispose = harness.runTurn(mod.mount);
  assert.equal(harness.byTestId("counter"), undefined, "shell opens attach asynchronously");
  if (attach) harness.flush();
  return { harness, mod, stop: () => { dispose(); dispose = undefined; harness.flush(); } };
}

const textOf = (harness, testId) => {
  const node = harness.byTestId(testId);
  assert.ok(node, `missing ${testId}`);
  return node.children.map(child => child.props.get("text")).join("");
};

test("pure-shell mount shares state and routes events across output hotplug", async t => {
  const { harness, mod, stop } = await mounted(t);
  const panel = harness.byTestId("counter");
  const panelId = harness.surfaces.get("panel").id;
  const first = harness.byTestId("counter:DP-1");
  assert.equal(textOf(harness, "geometry:DP-1"), "Desk:2:60000");
  assert.equal(textOf(harness, "counter"), "shared:0");
  const event = harness.dispatch(first, "click");
  assert.equal(event.defaultPrevented, true);
  assert.equal(harness.focused.current, first);
  assert.equal(harness.surfaces.get("overlay-DP-1").region.stylesheets.has("clicked"), true);
  assert.equal(harness.surfaces.get("panel").region.stylesheets.has("clicked"), false);
  assert.equal(textOf(harness, "counter"), "shared:1");
  assert.equal(textOf(harness, "counter:DP-1"), "shared:1");
  assert.throws(() => harness.runTurn(() => mod.ordered(panel, first)), /foreign mount region/);

  harness.emitMonitors([{ ...monitor("DP-1", "Desk", 0), scale: 1.5 }, monitor("HDMI-1", "Side", 1920)]);
  harness.flush();
  assert.equal(harness.byTestId("counter:DP-1"), first, "geometry changes preserve the keyed surface");
  assert.equal(harness.byTestId("counter"), panel);
  assert.equal(textOf(harness, "geometry:DP-1"), "Desk:1.5:60000");
  assert.equal(textOf(harness, "counter:HDMI-1"), "shared:1");

  harness.emitMonitors([monitor("HDMI-1", "Side", 1920)]);
  harness.flush();
  assert.equal(harness.byTestId("counter"), undefined);
  assert.equal(harness.byTestId("counter:DP-1"), undefined);
  assert.equal(textOf(harness, "counter:HDMI-1"), "shared:1");
  assert.throws(() => harness.runTurn(() => mod.read(first)), /stale detached/);
  assert.throws(() => harness.runTurn(() => mod.read(panel)), /stale detached/);
  assert.equal(mod.disposals(), 2);

  harness.emitMonitors([monitor("DP-1", "Desk", 0), monitor("HDMI-1", "Side", 1920)]);
  harness.flush();
  assert.equal(harness.surfaces.get("panel").id, panelId, "the retained request reattaches");
  assert.notEqual(harness.byTestId("counter"), panel);
  assert.equal(textOf(harness, "counter"), "shared:1");

  stop();
  assert.equal(harness.surfaces.size, 0);
  assert.equal(harness.monitorSubscribers.size, 0);
  assert.equal(mod.disposals(), 5);
  assert.equal(harness.geometry.some(record => record.active), false);
});

test("live placement preserves the mount region and updates layout-driven input rectangles", async t => {
  const { harness, mod } = await mounted(t);
  const surface = harness.surfaces.get("panel");
  const panel = harness.byTestId("counter");
  const observer = harness.geometry.find(record => record.node === panel && record.kind === "borderBox");
  assert.ok(observer, "input nodes are observed in the surface's mount region");
  assert.deepEqual(surface.inputRegion, []);
  const rect = { x: 10, y: 4, width: 24, height: 12 };
  observer.emit(rect);
  harness.flush();
  assert.deepEqual(surface.inputRegion, [rect]);

  mod.setVertical(true);
  harness.flush();
  assert.equal(harness.surfaces.get("panel"), surface);
  assert.equal(harness.byTestId("counter"), panel);
  assert.deepEqual(surface.placement, {
    output: "DP-1", layer: "overlay", anchor: ["left", "top", "bottom"], width: 28, height: 0,
    margin: { top: 0, right: 0, bottom: 0, left: 4 }, exclusiveZone: 28, keyboard: "on-demand",
  });
  assert.ok(harness.shellLog.some(entry => entry.op === "update" && entry.id === surface.id));

  mod.setVisible(false);
  harness.flush();
  assert.equal(observer.active, false);
  assert.deepEqual(surface.inputRegion, []);
  mod.setLinked(false);
  harness.flush();
  assert.deepEqual(surface.inputRegion, [{ x: 2, y: 4, width: 12, height: 10 }]);

  mod.setVisible(true);
  harness.flush();
  const oldNode = harness.byTestId("counter");
  harness.emitMonitors([monitor("DP-1", "Desk", 0), monitor("HDMI-1", "Side", 1920)]);
  mod.setOutput("HDMI-1");
  harness.flush();
  const replacement = harness.surfaces.get("panel");
  assert.notEqual(replacement, surface);
  assert.equal(replacement.placement.output, "HDMI-1");
  assert.notEqual(harness.byTestId("counter"), oldNode);
  assert.throws(() => harness.runTurn(() => mod.read(oldNode)), /stale detached/);
});

test("a retained input node stops intercepting when its branch detaches", async t => {
  const { harness, mod } = await mounted(t);
  const surface = harness.surfaces.get("panel");
  const panel = harness.byTestId("counter");
  const observer = harness.geometry.find(record => record.node === panel && record.kind === "borderBox");
  assert.ok(observer);
  const rect = { x: 10, y: 4, width: 24, height: 12 };
  observer.emit(rect);
  assert.deepEqual(surface.inputRegion, [rect]);

  harness.runTurn(() => {
    mod.retainInputNode();
    mod.setVisible(false);
  });
  assert.equal(mod.inputNode(), panel, "the input-region ref was not cleared by the app");
  assert.equal(harness.byTestId("counter"), undefined);
  assert.deepEqual(surface.inputRegion, [], "no stale hit rectangle at the end of the detach turn");
});

test("disposing before an asynchronous attach cancels the pending surface", async t => {
  const { harness, mod, stop } = await mounted(t, false);
  stop();
  assert.equal(harness.surfaces.size, 0);
  assert.equal(harness.monitorSubscribers.size, 0);
  assert.equal(mod.disposals(), 0);
  const opened = harness.shellLog.filter(entry => entry.op === "open").map(entry => entry.id);
  const closed = harness.shellLog.filter(entry => entry.op === "close").map(entry => entry.id);
  assert.equal(opened.length, 2);
  assert.deepEqual(new Set(closed), new Set(opened));
});
