import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { renderApp, getWindowRect, getTextContent, precedes } from "@hewel/zgui";
export let first, second;
export function mount(root, configure) {
  return renderApp(() => {
    first = <box><text>{"  é𐐷"}</text></box>;
    second = <box />;
    configure?.(first, second);
    globalThis.__seen.rect = getWindowRect(first);
    globalThis.__seen.missing = getWindowRect(second);
    globalThis.__seen.text = getTextContent(first);
    return <column>{first}{second}</column>;
  }, root);
}
export function checkOrder() {
  return {
    ordered: precedes(first, second),
    reverse: precedes(second, first),
    self: precedes(first, first),
  };
}
`;

test("node queries resolve the host lazily and forward native snapshots", async t => {
  const harness = createHost();
  const rect = { x: 10, y: 5, width: 20, height: 8 };
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  assert.equal(harness.queries.rect.length, 0);
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; delete globalThis.__seen; } });
  globalThis.__zgui = harness.host;
  globalThis.__seen = {};
  const root = harness.makeNode("root");
  dispose = mod.mount(root, firstNode => {
    harness.rects.set(firstNode, rect);
  });

  const column = root.children[0];
  const [first, second] = column.children;
  const order = mod.checkOrder();
  assert.equal(globalThis.__seen.rect, rect, "the host answer is passed through, not rebuilt");
  assert.equal(globalThis.__seen.missing, null);
  assert.equal(globalThis.__seen.text, "  é𐐷", "the text snapshot is passed through exactly");
  assert.equal(order.ordered, true);
  assert.equal(order.reverse, false);
  assert.equal(order.self, false);
  assert.deepEqual(harness.queries.rect, [first, second]);
  assert.deepEqual(harness.queries.text, [first]);
  assert.deepEqual(harness.queries.precedes, [[first, second], [second, first], [first, first]]);
});

const textFixture = `
import { For, createSignal, getTextContent, renderApp } from "@hewel/zgui";
const [items, setItems] = createSignal(["a", "b"]);
const [label, setLabel] = createSignal("漢字");
let list;
export { setItems, setLabel };
export const read = () => getTextContent(list);
export function mount(root) {
  return renderApp(() => <box ref={node => { list = node; }}><For each={items()}>{id => <text>{id}{label()}</text>}</For></box>, root);
}
`;

test("getTextContent reads current subtree text after reactive updates and reorders", async t => {
  const harness = createHost();
  const mod = await moduleFor(t, textFixture);
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  assert.equal(mod.read(), "a漢字b漢字");
  mod.setLabel("é𐐷");
  harness.flush();
  assert.equal(mod.read(), "aé𐐷bé𐐷");
  mod.setItems(["b", "a"]);
  harness.flush();
  assert.equal(mod.read(), "bé𐐷aé𐐷");
});

const stylesheetFixture = `
import { mountStylesheet, renderApp } from "@hewel/zgui";
export function mount(root) {
  return renderApp(() => {
    mountStylesheet("sample-sheet", ".sample { color: red; }");
    return <box />;
  }, root);
}
`;

test("mountStylesheet installs on mount and removes the stylesheet on disposal", async t => {
  const harness = createHost();
  const mod = await moduleFor(t, stylesheetFixture);
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  assert.equal(harness.stylesheets.get("sample-sheet"), ".sample { color: red; }");
  dispose();
  dispose = undefined;
  assert.equal(harness.stylesheets.has("sample-sheet"), false);
});
