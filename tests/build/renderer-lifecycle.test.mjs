import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, onCleanup, renderApp } from "@hewel/zgui";
import { render } from "@hewel/zgui/renderer";
import type { NativeNode } from "@hewel/zgui";
import { RadioGroup, RadioGroupItem } from "@hewel/zgui/components";

export let cleanups = 0;

export function mount(root: NativeNode, mode: "app" | "raw", tail: number) {
  const renderRoot = mode === "app" ? renderApp : render;
  return renderRoot(() => <column>
    <RadioGroup defaultValue="b"><For each={["a", "b", "c"]}>{value => {
      onCleanup(() => { cleanups++; });
      return <RadioGroupItem testId={value} value={value} label={value} />;
    }}</For></RadioGroup>
    <For each={Array.from({ length: tail }, (_, index) => index)}>{value => <text>{value}</text>}</For>
  </column>, root);
}

export function mountFailing(root: NativeNode, cleanupError?: string) {
  return renderApp(() => {
    onCleanup(() => { if (cleanupError) throw new Error(cleanupError); cleanups++; });
    throw new Error("mount failed");
  }, root);
}
`;

async function mounted(t) {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  t.after(() => { globalThis.__zgui = harness.host; harness.host.clearRoot(root); delete globalThis.__zgui; });
  return { mod, harness, root };
}

const tabIndex = (harness, testId) => harness.byTestId(testId)?.props.get("tabIndex");

for (const mode of ["app", "raw"]) {
  test(`${mode} disposal does not re-register events on a stopping session`, async t => {
    const { mod, harness, root } = await mounted(t);
    const registrationsAfterStop = [];
    let stopping = false;
    const original = harness.host.setProperty;
    harness.host.setProperty = (node, name, value, previous) => {
      if (stopping && name.startsWith("on") && value != null) {
        registrationsAfterStop.push(name);
        throw new ReferenceError("native session is stopping");
      }
      return original(node, name, value, previous);
    };
    const dispose = mod.mount(root, mode, 0);
    assert.equal(tabIndex(harness, "b"), 0);
    assert.equal(tabIndex(harness, "a"), -1);
    assert.equal(tabIndex(harness, "c"), -1);
    stopping = true;
    assert.doesNotThrow(() => dispose());
    assert.deepEqual(registrationsAfterStop, []);
    assert.equal(mod.cleanups, 3);
    assert.doesNotThrow(() => dispose());
    assert.equal(mod.cleanups, 3);
    if (mode === "app") assert.deepEqual(root.children, []);
    if (mode === "raw") harness.host.clearRoot(root);
  });

  test(`${mode} mount does not multiply roving order queries with unrelated nodes`, async t => {
    const runs = [];
    for (const tail of [0, 100]) {
      const { mod, harness, root } = await mounted(t);
      let precedesCalls = 0;
      const original = harness.host.precedes;
      harness.host.precedes = (first, second) => { precedesCalls++; return original(first, second); };
      const dispose = mod.mount(root, mode, tail);
      t.after(() => {
        globalThis.__zgui = harness.host;
        try { dispose(); } finally { harness.host.clearRoot(root); delete globalThis.__zgui; }
      });
      runs.push(precedesCalls);
      assert.equal(tabIndex(harness, "b"), 0);
      assert.equal(tabIndex(harness, "a"), -1);
      assert.equal(tabIndex(harness, "c"), -1);
    }
    assert.equal(runs[1], runs[0], "trailing nodes multiplied tree-order queries");
  });
}

test("renderApp cleans up and preserves the original mount error", async t => {
  const { mod, harness, root } = await mounted(t);
  let clearCalls = 0;
  const original = harness.host.clearRoot;
  harness.host.clearRoot = target => { clearCalls++; return original(target); };
  assert.throws(() => mod.mountFailing(root), (error) => error instanceof Error && error.message === "mount failed");
  assert.equal(mod.cleanups, 1, "cleanup ran exactly once");
  assert.equal(clearCalls, 1, "clearRoot ran on mount failure");
});

test("renderApp still clears the root when cleanup throws", async t => {
  const { mod, harness, root } = await mounted(t);
  let clearCalls = 0;
  const original = harness.host.clearRoot;
  harness.host.clearRoot = target => { clearCalls++; return original(target); };
  assert.throws(() => mod.mountFailing(root, "cleanup failed"), (error) =>
    error instanceof AggregateError &&
    error.errors.length === 2 &&
    error.errors[0].message === "mount failed" &&
    error.errors[1].message === "cleanup failed");
  assert.equal(clearCalls, 1, "clearRoot still attempted");
});
