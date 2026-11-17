import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { ThemeProvider, CardHeader, Item } from "@hewel/zgui/components";
const [order, setOrder] = createSignal(["a", "b"]);
export const reorder = () => setOrder(["b", "a"]);
function Counter(props: { id: string }) {
  const [count, setCount] = createSignal(0);
  return <control testId={props.id} onClick={() => setCount(n => n + 1)}>{count()}</control>;
}
export function mount(root: NativeNode, kind: "theme" | "card" | "item") {
  const Parent = kind === "theme" ? ThemeProvider : kind === "card" ? CardHeader : Item;
  return renderApp(() => <Parent><For each={order()}>{id => <Counter id={id} />}</For></Parent>, root);
}
`;

function cleanup(t, harness, dispose) {
  t.after(() => {
    const previous = globalThis.__zgui;
    globalThis.__zgui = harness.host;
    try { dispose(); }
    finally {
      if (previous && previous !== harness.host) globalThis.__zgui = previous;
      else delete globalThis.__zgui;
    }
  });
}

const textOf = node => {
  if (node.tag === "#text") return node.props.get("text") ?? "";
  return node.children.map(textOf).join("");
};

const click = () => ({
  type: "click",
  preventDefault() {}, stopPropagation() {}, stopImmediatePropagation() {},
});

for (const kind of ["theme", "card", "item"]) {
  test(`${kind} provider children keep node and state identity across keyed reorder`, async t => {
    const harness = createHost();
    globalThis.__zgui = harness.host;
    const mod = await moduleFor(t, fixture);
    const root = harness.makeNode("root");
    const dispose = mod.mount(root, kind);
    cleanup(t, harness, dispose);

    const a = harness.byTestId("a");
    const b = harness.byTestId("b");
    a.callbacks.onClick(click());
    assert.equal(textOf(a), "1");

    mod.reorder();
    assert.equal(harness.byTestId("a"), a, "keyed move keeps node identity");
    assert.equal(harness.byTestId("b"), b);
    const parent = harness.host.getParentNode(a);
    assert.deepEqual(
      parent.children.map(node => node.props.get("testId")),
      ["b", "a"],
      "physical order follows the keyed move",
    );
    assert.equal(textOf(a), "1", "signal state survives the move");

    a.callbacks.onClick(click());
    assert.equal(textOf(a), "2");
  });
}
