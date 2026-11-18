import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Show, createEffect, createSignal, renderApp, trapFocus } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

const [visible, setVisible] = createSignal(true);
const [choice, setChoice] = createSignal(0);
export function toggle() { setVisible(!visible()); }
export function choose(value: number) { setChoice(value); }
export function manual(node: NativeNode) { return trapFocus(node, { autoFocus: false, restore: false }); }

export function mount(root: NativeNode) {
  return renderApp(() => {
    let first!: NativeNode;
    let second!: NativeNode;
    createEffect(() => choice(), current => trapFocus(current === 0 ? first : second, { wrap: false }));
    return <column>
      <Show when={visible()}><box ref={node => { trapFocus(node); }} /></Show>
      <box ref={node => { first = node; }} />
      <box ref={node => { second = node; }} />
    </column>;
  }, root);
}
`;

async function mounted(t) {
  const harness = createHost();
  const registrations = harness.traps;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  assert.equal(registrations.length, 0, "module import must not register native traps");
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  return { mod, harness, root, registrations, finish: () => { const end = dispose; dispose = undefined; end?.(); } };
}

test("owner-bound traps track mounts, effects and disposal exactly once", async t => {
  const { mod, harness, root, registrations, finish } = await mounted(t);
  assert.equal(registrations.length, 2);
  const [conditional, effect] = registrations;
  assert.deepEqual([conditional.wrap, conditional.autoFocus, conditional.restore], [true, true, true]);
  assert.deepEqual([effect.wrap, effect.autoFocus, effect.restore], [false, true, true]);
  const first = root.children[0].children[1];
  const second = root.children[0].children[2];
  assert.equal(effect.node, first);

  mod.toggle();
  harness.flush();
  assert.equal(conditional.disposals, 1);
  assert.equal(conditional.active, false);
  mod.toggle();
  harness.flush();
  assert.equal(registrations.length, 3);
  const remounted = registrations[2];
  assert.notEqual(remounted.node, conditional.node);
  assert.equal(remounted.disposals, 0);

  mod.choose(1);
  harness.flush();
  assert.equal(effect.disposals, 1);
  assert.equal(registrations.length, 4);
  const switched = registrations[3];
  assert.equal(switched.node, second);
  assert.deepEqual([switched.wrap, switched.autoFocus, switched.restore], [false, true, true]);

  finish();
  for (const record of registrations) {
    assert.equal(record.disposals, 1, "every owner-bound lease releases exactly once");
    assert.equal(record.active, false);
  }
});

test("a manual lease outside an owner survives disposal until released", async t => {
  const { mod, root, registrations, finish } = await mounted(t);
  const release = mod.manual(root);
  assert.equal(registrations.length, 3);
  const manual = registrations[2];
  assert.equal(manual.node, root);
  assert.deepEqual([manual.wrap, manual.autoFocus, manual.restore], [true, false, false]);

  finish();
  assert.equal(manual.active, true, "a lease outside an owner is not released by disposal");
  assert.equal(manual.disposals, 0);
  release();
  release();
  assert.equal(manual.disposals, 1);
  assert.equal(manual.active, false);
});
