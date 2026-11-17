import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Show, createEffect, createSignal, lockScroll, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

const [visible, setVisible] = createSignal(true);
const [choice, setChoice] = createSignal(0);
export function toggle() { setVisible(!visible()); }
export function choose(value: number) { setChoice(value); }
export function manual(node: NativeNode) { return lockScroll(node); }

export function mount(root: NativeNode) {
  return renderApp(() => {
    let first!: NativeNode;
    let second!: NativeNode;
    createEffect(() => { lockScroll(choice() === 0 ? first : second); });
    return <column>
      <Show when={visible()}><box ref={node => { lockScroll(node); }} /></Show>
      <box ref={node => { first = node; }} />
      <box ref={node => { second = node; }} />
    </column>;
  }, root);
}
`;

async function mounted(t) {
  const harness = createHost();
  const leases = harness.locks;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  assert.equal(leases.length, 0, "module import must not acquire native scroll locks");
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  return { mod, root, leases, finish: () => { const end = dispose; dispose = undefined; end?.(); } };
}

test("owner-bound scroll locks track mounts, effect reruns and disposal exactly once", async t => {
  const { mod, root, leases, finish } = await mounted(t);
  assert.equal(leases.length, 2);
  const [conditional, effect] = leases;
  const first = root.children[0].children[1];
  const second = root.children[0].children[2];
  assert.equal(effect.node, first);

  mod.toggle();
  assert.equal(conditional.disposals, 1);
  assert.equal(conditional.active, false);
  mod.toggle();
  assert.equal(leases.length, 3);
  const remounted = leases[2];
  assert.notEqual(remounted.node, conditional.node);
  assert.equal(remounted.disposals, 0);

  mod.choose(1);
  assert.equal(effect.disposals, 1);
  assert.equal(leases.length, 4);
  const switched = leases[3];
  assert.equal(switched.node, second);

  finish();
  for (const record of leases) {
    assert.equal(record.disposals, 1, "every owner-bound lease releases exactly once");
    assert.equal(record.active, false);
  }
});

test("a manual scroll lock outside an owner survives disposal until released", async t => {
  const { mod, root, leases, finish } = await mounted(t);
  const release = mod.manual(root);
  assert.equal(leases.length, 3);
  const manual = leases[2];
  assert.equal(manual.node, root);

  finish();
  assert.equal(manual.active, true, "a lease outside an owner is not released by disposal");
  assert.equal(manual.disposals, 0);
  release();
  release();
  assert.equal(manual.disposals, 1);
  assert.equal(manual.active, false);
});
