import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Portal, Show, createContext, createSignal, renderApp, useContext } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button } from "@hewel/zgui/components";

const Context = createContext("missing");
const [shown, setShown] = createSignal(true);
export function toggle() { setShown(v => !v); }

function Content() {
  const value = useContext(Context);
  const [count, setCount] = createSignal(0);
  return <Button testId="counter" onClick={() => setCount(n => n + 1)}><text>{value}:{count()}</text></Button>;
}

export function mount(root: NativeNode) {
  return renderApp(() => Context.Provider({ value: "kept", get children() {
    return <column><Show when={shown()}><Portal layer="modal"><Content /></Portal></Show></column>;
  } }), root);
}
`;

const fixtureDefault = `
import { Portal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export function mount(root: NativeNode) {
  return renderApp(() => <Portal><text>default</text></Portal>, root);
}
`;

const textOf = current =>
  current.tag === "#text" ? current.props.get("text") ?? "" : current.children.map(textOf).join("");

async function mounted(t, source) {
  const harness = createHost();
  const layers = harness.portals;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, source);
  assert.equal(layers.length, 0, "module import must not allocate native nodes");
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  return { mod, root, layers, harness, finish: () => { const end = dispose; dispose = undefined; end?.(); } };
}

test("portal mounts children on the requested band and keeps context", async t => {
  const { mod, root, layers, harness, finish } = await mounted(t, fixture);
  assert.equal(layers.length, 1);
  assert.equal(layers[0].layer, "modal");
  const counter = harness.byTestId("counter");
  assert.ok(counter, "portal content should be attached");
  assert.equal(textOf(counter), "kept:0");

  harness.dispatch(counter, "Click");
  assert.equal(textOf(counter), "kept:1");

  mod.toggle();
  assert.equal(harness.byTestId("counter"), undefined);
  mod.toggle();
  assert.equal(layers.length, 2, "remount creates a fresh portal");
  const remounted = harness.byTestId("counter");
  assert.ok(remounted);
  assert.equal(textOf(remounted), "kept:0");
  assert.notEqual(remounted, counter);

  finish();
  assert.equal(root.children.length, 0);
});

test("portal defaults to the popover band", async t => {
  const { layers, finish } = await mounted(t, fixtureDefault);
  assert.equal(layers.length, 1);
  assert.equal(layers[0].layer, "popover");
  finish();
});
