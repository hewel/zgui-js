import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Show, createEffect, createSignal, observeGeometry, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

const [size, setSize] = createSignal("none");
const [visible, setVisible] = createSignal(true);
const [choice, setChoice] = createSignal(0);
export function measuredSize() { return size(); }
export function toggle() { setVisible(!visible()); }
export function choose(index: number) { setChoice(index); }

let switchDispose: (() => void) | undefined;
export function cancelSwitch() { switchDispose?.(); }

export function mount(root: NativeNode) {
  return renderApp(() => {
    let first!: NativeNode;
    let second!: NativeNode;
    createEffect(() => {
      const node = choice() === 0 ? first : second;
      if (!node) return;
      switchDispose = observeGeometry(node, "contentSize", value => setSize("switch:" + value.width));
    });
    return <column>
      <Show when={visible()}>
        <box ref={node => { observeGeometry(node, "borderSize", value => setSize("box:" + value.width)); }}>
          <text>{size()}</text>
        </box>
      </Show>
      <box ref={node => { first = node; }} />
      <box ref={node => { second = node; }} />
    </column>;
  }, root);
}

export function watch(node: NativeNode) {
  return observeGeometry(node, "scrollPosition", value => setSize("scroll:" + value.offset.y));
}

export function mountViewport(root: NativeNode) {
  return renderApp(() => <box ref={node => {
    observeGeometry(node, "viewport", value => setSize(\`\${value.width}x\${value.height}@\${value.scale}\`));
  }} />, root);
}
`;

async function mounted(t, entry = "mount") {
  const registrations = [];
  const harness = createHost();
  harness.host.observeGeometry = (node, kind, callback) => {
    const record = { node, kind, callback, active: true, disposals: 0 };
    registrations.push(record);
    let disposed = false;
    return () => {
      if (disposed) return;
      disposed = true;
      record.disposals += 1;
      record.active = false;
    };
  };
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod[entry](root);
  const emit = (record, value) => { if (record.active) record.callback(value); };
  return { mod, root, registrations, emit, finish: () => { const end = dispose; dispose = undefined; end?.(); } };
}

test("geometry callbacks update signals, cancel on removal, and stay inert", async t => {
  const { mod, registrations, emit, finish } = await mounted(t);
  assert.equal(registrations.length, 2);
  const [boxObs, firstObs] = registrations;
  assert.equal(boxObs.kind, "borderSize");
  assert.equal(firstObs.kind, "contentSize");
  emit(boxObs, { width: 200, height: 40 });
  assert.equal(mod.measuredSize(), "box:200");

  mod.toggle();
  assert.equal(boxObs.disposals, 1);
  emit(boxObs, { width: 999, height: 1 });
  assert.equal(mod.measuredSize(), "box:200");

  mod.toggle();
  assert.equal(registrations.length, 3);
  const fresh = registrations[2];
  assert.equal(fresh.kind, "borderSize");
  assert.notEqual(fresh.node, boxObs.node);
  emit(fresh, { width: 320, height: 50 });
  assert.equal(mod.measuredSize(), "box:320");

  finish();
  assert.equal(fresh.disposals, 1);
  assert.equal(firstObs.disposals, 1);
  emit(fresh, { width: 1, height: 1 });
  assert.equal(mod.measuredSize(), "box:320");
});

test("effect reruns retire the previous geometry registration", async t => {
  const { mod, registrations, emit, finish } = await mounted(t);
  const contentObs = () => registrations.filter(record => record.kind === "contentSize");
  assert.equal(contentObs().length, 1);
  const firstObs = contentObs()[0];
  emit(firstObs, { width: 40, height: 10 });
  assert.equal(mod.measuredSize(), "switch:40");

  mod.choose(1);
  assert.equal(contentObs().length, 2);
  const secondObs = contentObs()[1];
  assert.equal(firstObs.disposals, 1);
  assert.notEqual(secondObs.node, firstObs.node);
  emit(firstObs, { width: 1, height: 1 });
  assert.equal(mod.measuredSize(), "switch:40");
  emit(secondObs, { width: 77, height: 9 });
  assert.equal(mod.measuredSize(), "switch:77");

  mod.choose(0);
  assert.equal(contentObs().length, 3);
  const backToFirst = contentObs()[2];
  assert.equal(secondObs.disposals, 1);
  assert.equal(backToFirst.node, firstObs.node);
  mod.cancelSwitch();
  assert.equal(backToFirst.disposals, 1);
  emit(backToFirst, { width: 5, height: 5 });
  assert.equal(mod.measuredSize(), "switch:77");

  finish();
  assert.equal(backToFirst.disposals, 1);
});

test("geometry registrations outside an owner require manual cancellation", async t => {
  const { mod, root, registrations, emit, finish } = await mounted(t);
  const before = registrations.length;
  const cancel = mod.watch(root);
  assert.equal(registrations.length, before + 1);
  const watchObs = registrations[registrations.length - 1];
  assert.equal(watchObs.node, root);
  assert.equal(watchObs.kind, "scrollPosition");
  emit(watchObs, {
    offset: { x: 0, y: 64 },
    contentSize: { width: 400, height: 800 },
    scrollport: { width: 400, height: 300 },
  });
  assert.equal(mod.measuredSize(), "scroll:64");

  finish();
  assert.equal(watchObs.active, true);
  assert.equal(watchObs.disposals, 0);
  cancel();
  assert.equal(watchObs.disposals, 1);
  emit(watchObs, { offset: { x: 0, y: 1 }, contentSize: { width: 1, height: 1 }, scrollport: { width: 1, height: 1 } });
  assert.equal(mod.measuredSize(), "scroll:64");
  cancel();
  assert.equal(watchObs.disposals, 1);
});

test("viewport snapshots update density and dispose with their Solid owner", async t => {
  const { mod, registrations, emit, finish } = await mounted(t, "mountViewport");
  assert.equal(registrations.length, 1);
  const record = registrations[0];
  assert.equal(record.kind, "viewport");
  emit(record, { x: 0, y: 0, width: 800, height: 600, scale: 1.75 });
  assert.equal(mod.measuredSize(), "800x600@1.75");
  emit(record, { x: 0, y: 0, width: 800, height: 600, scale: 2 });
  assert.equal(mod.measuredSize(), "800x600@2");
  finish();
  assert.equal(record.disposals, 1);
  emit(record, { x: 0, y: 0, width: 1, height: 1, scale: 1 });
  assert.equal(mod.measuredSize(), "800x600@2");
});
