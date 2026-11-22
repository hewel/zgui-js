import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import { render } from "@hewel/zgui/renderer";
import type { NativeNode } from "@hewel/zgui";
import { RadioGroup, RadioGroupItem } from "@hewel/zgui/components";

export let cleanups = 0;

export function mount(root: NativeNode, mode: "app" | "raw", tail: number) {
  cleanups = 0;
  const renderRoot = mode === "app" ? renderApp : render;
  return renderRoot(() => <column>
    <RadioGroup defaultValue="b"><For each={["a", "b", "c"]}>{value => {
      onCleanup(() => { cleanups++; });
      return <RadioGroupItem testId={value} value={value} label={value} />;
    }}</For></RadioGroup>
    <For each={Array.from({ length: tail }, (_, index) => index)}>{value => <text>{value}</text>}</For>
  </column>, root);
}

const [shown, setShown] = createSignal(true);
export function hide() { setShown(false); }
export function mountConditional(root: NativeNode, onDetach?: (node: NativeNode) => void) {
  setShown(true);
  return renderApp(() => <column><Show when={shown()}>
    <box testId="ephemeral" onClick={() => {}} ref={node => onCleanup(() => onDetach?.(node))} />
  </Show></column>, root);
}

export function mountFailing(root: NativeNode, cleanupError?: string) {
  cleanups = 0;
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
  t.after(() => {
    globalThis.__zgui = harness.host;
    harness.host.clearRoot(root);
    delete globalThis.__zgui;
  });
  return { mod, harness, root };
}

const tabIndex = (harness, testId) => harness.byTestId(testId)?.props.get("tabIndex");

for (const mode of ["app", "raw"]) {
  test(`${mode} disposal does not re-register events on a stopping session`, async t => {
    const { mod, harness, root } = await mounted(t);
    const dispose = mod.mount(root, mode, 0);
    assert.equal(tabIndex(harness, "b"), 0);
    assert.equal(tabIndex(harness, "a"), -1);
    assert.equal(tabIndex(harness, "c"), -1);
    harness.beginStop();
    assert.doesNotThrow(() => dispose());
    assert.deepEqual(harness.registrationsAfterStop, []);
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
      const dispose = mod.mount(root, mode, tail);
      t.after(() => {
        globalThis.__zgui = harness.host;
        try {
          dispose();
        } finally {
          harness.host.clearRoot(root);
          delete globalThis.__zgui;
        }
      });
      runs.push(harness.queries.precedes.length);
      assert.equal(tabIndex(harness, "b"), 0);
      assert.equal(tabIndex(harness, "a"), -1);
      assert.equal(tabIndex(harness, "c"), -1);
    }
    assert.equal(runs[1], runs[0], "trailing nodes multiplied tree-order queries");
  });
}

test("renderApp cleans up and preserves the original mount error", async t => {
  const { mod, harness, root } = await mounted(t);
  assert.throws(
    () => mod.mountFailing(root),
    error => error instanceof Error && error.message === "mount failed",
  );
  assert.equal(mod.cleanups, 1, "cleanup ran exactly once");
  assert.equal(harness.clearCalls.length, 1, "clearRoot ran on mount failure");
});

test("renderApp still clears the root when cleanup throws", async t => {
  const { mod, harness, root } = await mounted(t);
  assert.throws(
    () => mod.mountFailing(root, "cleanup failed"),
    error =>
      error instanceof AggregateError &&
      error.errors.length === 2 &&
      error.errors[0].message === "mount failed" &&
      error.errors[1].message === "cleanup failed",
  );
  assert.equal(harness.clearCalls.length, 1, "clearRoot still attempted");
});

test("detached node allows same-turn cleanup but invalidates after the Bridge turn ends", async t => {
  const { mod, harness, root } = await mounted(t);
  let cleanedInTurn = false;
  const dispose = mod.mountConditional(root, node => {
    harness.host.setProperty(node, "data-cleaned", true);
    cleanedInTurn = node.props.get("data-cleaned") === true;
  });
  try {
    const captured = harness.byTestId("ephemeral");
    assert.ok(captured);
    harness.runTurn(() => mod.hide());
    assert.equal(cleanedInTurn, true, "same-turn cleanup succeeds before the Bridge turn ends");
    assert.equal(harness.byTestId("ephemeral"), undefined);
    assert.throws(() => harness.host.setProperty(captured, "data-stale", true), TypeError);
  } finally {
    globalThis.__zgui = harness.host;
    dispose();
  }
});

test("beginStop rejects new nodes, leases, stylesheets, and listeners with ReferenceError while allowing disposal", async t => {
  const { mod, harness, root } = await mounted(t);
  const child = harness.host.createElement("box");
  harness.host.insertNode(root, child, null);
  const dismiss = harness.host.registerDismissLayer(root, () => {}, {});
  const dispose = mod.mount(root, "app", 0);
  harness.beginStop();
  assert.throws(() => harness.host.createElement("box"), ReferenceError);
  assert.throws(() => harness.host.createPortal("popover"), ReferenceError);
  assert.throws(() => harness.host.createTextNode("hi"), ReferenceError);
  assert.throws(() => harness.host.insertNode(root, child, null), ReferenceError);
  assert.throws(() => harness.host.installStylesheet("late", ".a {}"), ReferenceError);
  assert.throws(() => harness.host.trapFocus(root, true, true, true), ReferenceError);
  assert.throws(() => harness.host.lockScroll(root), ReferenceError);
  assert.throws(() => harness.host.registerDismissLayer(root, () => {}, {}), ReferenceError);
  assert.throws(() => dismiss.update({}), ReferenceError);
  assert.throws(() => harness.host.observeGeometry(root, "borderBox", () => {}), ReferenceError);
  assert.throws(() => harness.host.setProperty(root, "onClick", () => {}), ReferenceError);
  assert.throws(() => harness.host.setProperty(child, "src", "foo.png"), ReferenceError);
  assert.doesNotThrow(() => dismiss.dispose());
  assert.doesNotThrow(() => dispose());
  assert.equal(mod.cleanups, 3);
});
