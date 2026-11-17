import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeChild, NativeNode } from "@hewel/zgui";
const [count, setCount] = createSignal(0);
export { setCount };
function Body(props: { children?: NativeChild }) {
  return <column testId="mixed-numbers">{props.children}</column>;
}
export function mount(root: NativeNode) {
  return renderApp(() => <Body>Count: {count()}{" / "}{["[", 1, ",", [2.5, null, false, ",", -4], "]"]}</Body>, root);
}
`;

const textOf = node => {
  if (node.tag === "#text") return node.props.get("text") ?? "";
  return node.children.map(textOf).join("");
};

test("numeric children reach the host as text", async t => {
  const harness = createHost();
  const previous = globalThis.__zgui;
  globalThis.__zgui = harness.host;
  let dispose;
  t.after(() => {
    globalThis.__zgui = harness.host;
    try { dispose?.(); }
    finally {
      if (previous && previous !== harness.host) globalThis.__zgui = previous;
      else delete globalThis.__zgui;
    }
  });
  const mod = await moduleFor(t, fixture);
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  const panel = harness.byTestId("mixed-numbers");
  assert.ok(panel);
  assert.equal(textOf(panel), "Count: 0 / [1,2.5,-4]");
  mod.setCount(5);
  assert.equal(harness.byTestId("mixed-numbers"), panel);
  assert.equal(textOf(panel), "Count: 5 / [1,2.5,-4]");
  mod.setCount(-2.5);
  assert.equal(textOf(panel), "Count: -2.5 / [1,2.5,-4]");
  mod.setCount(0);
  assert.equal(textOf(panel), "Count: 0 / [1,2.5,-4]");
});
