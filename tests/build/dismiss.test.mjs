import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Show, createEffect, createSignal, registerDismissLayer, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

const [visible, setVisible] = createSignal(true);
const [choice, setChoice] = createSignal(0);
const reasons = [];
export function toggle() { setVisible(!visible()); }
export function choose(value: number) { setChoice(value); }
export function taken() { return reasons.slice(); }
export function manual(node: NativeNode) {
  return registerDismissLayer(node, reason => reasons.push("manual:" + reason), { layer: "modal" });
}

export function mount(root: NativeNode) {
  return renderApp(() => {
    let first!: NativeNode;
    let second!: NativeNode;
    createEffect(() => { registerDismissLayer(choice() === 0 ? first : second, reason => reasons.push("effect:" + reason)); });
    return <column>
      <Show when={visible()}><box ref={node => { registerDismissLayer(node, reason => reasons.push("conditional:" + reason), { layer: "toast", exclude: root }); }} /></Show>
      <box ref={node => { first = node; }} />
      <box ref={node => { second = node; }} />
    </column>;
  }, root);
}
`;

async function mounted(t) {
  const registrations = [];
  const harness = createHost();
  harness.host.registerDismissLayer = (node, callback, options) => {
    const record = { node, callback, options, updates: [], disposals: 0, active: true };
    registrations.push(record);
    let disposed = false;
    return {
      update: options => { record.updates.push(options); },
      dispose: () => {
        if (disposed) return;
        disposed = true;
        record.disposals += 1;
        record.active = false;
      },
    };
  };
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  assert.equal(registrations.length, 0, "module import must not register native dismiss layers");
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  return { mod, root, registrations, finish: () => { const end = dispose; dispose = undefined; end?.(); } };
}

test("owner-bound leases track mounts, effects and disposal exactly once", async t => {
  const { mod, root, registrations, finish } = await mounted(t);
  assert.equal(registrations.length, 2);
  const [conditional, effect] = registrations;
  assert.deepEqual(conditional.options, { layer: "toast", dismissOnOutsidePress: true, dismissOnEscape: true, exclude: root, leaving: false });
  assert.deepEqual(effect.options, { layer: "popover", dismissOnOutsidePress: true, dismissOnEscape: true, exclude: null, leaving: false });
  const first = root.children[0].children[1];
  const second = root.children[0].children[2];
  assert.equal(effect.node, first);

  mod.toggle();
  assert.equal(conditional.disposals, 1);
  assert.equal(conditional.active, false);
  mod.toggle();
  assert.equal(registrations.length, 3);
  const remounted = registrations[2];
  assert.notEqual(remounted.node, conditional.node);
  assert.equal(remounted.disposals, 0);
  assert.equal(remounted.options.layer, "toast");

  mod.choose(1);
  assert.equal(effect.disposals, 1);
  assert.equal(registrations.length, 4);
  const switched = registrations[3];
  assert.equal(switched.node, second);

  finish();
  for (const record of registrations) {
    assert.equal(record.disposals, 1, "every owner-bound lease releases exactly once");
    assert.equal(record.active, false);
  }
});

test("a manual lease outside an owner survives disposal until released", async t => {
  const { mod, root, registrations, finish } = await mounted(t);
  const lease = mod.manual(root);
  assert.equal(registrations.length, 3);
  const manual = registrations[2];
  assert.equal(manual.node, root);
  assert.deepEqual([manual.options.layer, manual.options.dismissOnOutsidePress, manual.options.dismissOnEscape], ["modal", true, true]);

  finish();
  assert.equal(manual.active, true, "a lease outside an owner is not released by disposal");
  assert.equal(manual.disposals, 0);
  lease.dispose();
  lease.dispose();
  assert.equal(manual.disposals, 1);
  assert.equal(manual.active, false);
});

test("update replaces the option snapshot and reasons reach the callback", async t => {
  const { mod, root, registrations } = await mounted(t);
  const lease = mod.manual(root);
  const record = registrations[2];

  lease.update({ layer: "toast", dismissOnEscape: false });
  lease.update();
  assert.deepEqual(record.updates, [
    { layer: "toast", dismissOnOutsidePress: true, dismissOnEscape: false, exclude: null, leaving: false },
    { layer: "popover", dismissOnOutsidePress: true, dismissOnEscape: true, exclude: null, leaving: false },
  ]);

  record.callback("escape");
  record.callback("outside-press");
  assert.deepEqual(mod.taken(), ["manual:escape", "manual:outside-press"]);
  lease.dispose();
  lease.dispose();
  assert.equal(record.disposals, 1);
});
