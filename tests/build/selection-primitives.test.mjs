import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const key = (key, modifiers = {}, isComposing = false) => ({
  key,
  keyWithoutModifiers: key,
  modifiers,
  isComposing,
  preventDefault() {},
});

test("selection keeps a stable range anchor, shrinks via the keyboard, and ignores IME keys", async t => {
  const { createSelectionModel, flush } = await moduleFor(
    t,
    `
    export { createSelectionModel } from "@hewel/zgui/primitives";
    export { flush } from "@hewel/zgui";
  `,
  );
  const model = createSelectionModel({ items: () => ["a", "b", "c", "d", "e"] });
  const expect = (...keys) => {
    flush();
    assert.deepEqual(model.selectedKeys(), new Set(keys));
  };
  model.selectByClick("b", {});
  expect("b");
  model.selectByClick("d", { ctrl: true });
  expect("b", "d");
  model.selectByClick("b", { shift: true });
  expect("b", "c", "d");
  model.handleKeyDown(key("ArrowRight", { shift: true }));
  expect("c", "d");
  assert.equal(model.anchorKey(), "d");
  assert.equal(model.focusedKey(), "c");
  model.handleKeyDown(key("Escape", {}, true));
  expect("c", "d");
  model.handleKeyDown(key("a", { meta: true }));
  expect("a", "b", "c", "d", "e");
  model.handleKeyDown(key("Escape"));
  expect();
});

test("controlled single selection reports changes without replacing owner state", async t => {
  const { createSelectionModel } = await moduleFor(
    t,
    `
    export { createSelectionModel } from "@hewel/zgui/primitives";
  `,
  );
  let selected = new Set([2]);
  const changes = [];
  const model = createSelectionModel({
    items: () => [1, 2, 3],
    selectedKeys: () => selected,
    selectionMode: "single",
    disallowEmpty: true,
    onSelectionChange: next => changes.push(next),
  });
  model.clear();
  model.toggle(2);
  model.selectAll();
  assert.deepEqual(changes, []);
  model.selectSingle(3);
  assert.deepEqual(changes, [new Set([3])]);
  assert.equal(model.isSelected(2), true);
  selected = changes[0];
  assert.equal(model.isSelected(3), true);
});

test("marquee starts after the threshold, selects scrolled content, and restores on cancellation", async t => {
  const host = createHost();
  globalThis.__zgui = host.host;
  const mod = await moduleFor(
    t,
    `
    import { createSignal, renderApp } from "@hewel/zgui";
    import { createMarqueeSelection, createSelectionModel } from "@hewel/zgui/primitives";
    export let selection, marquee;
    export const mount = root => renderApp(() => {
      const [container, setContainer] = createSignal(null);
      selection = createSelectionModel({ items: () => [0, 1, 2, 3], defaultSelectedKeys: [0] });
      marquee = createMarqueeSelection({
        container,
        selection,
        edgeSize: 0,
        resolveIntersectingKeys: rect => rect.y >= 100 ? [2, 3] : [1],
      });
      return <scroll testId="viewport" ref={setContainer} />;
    }, root);
  `,
  );
  const dispose = host.runTurn(() => mod.mount(host.makeNode("root")));
  t.after(() => {
    dispose();
    delete globalThis.__zgui;
  });
  const node = host.byTestId("viewport");
  host.rects.set(node, { x: 0, y: 0, width: 200, height: 120 });
  const observer = host.geometry.find(
    entry => entry.node === node && entry.kind === "scrollPosition",
  );
  host.runTurn(() =>
    observer.callback({
      offset: { x: 0, y: 100 },
      contentSize: { width: 200, height: 400 },
      scrollport: { width: 200, height: 120 },
    }),
  );
  let captured = false;
  const pointer = (x, y, pointerId = "1") => ({
    currentTarget: node,
    position: { x, y },
    pointerId,
    pointerKind: "mouse",
    primary: true,
    button: "primary",
    modifiers: { shift: true },
    capturePointer() {
      captured = true;
    },
    releasePointer() {
      captured = false;
    },
    preventDefault() {},
  });
  host.runTurn(() => mod.marquee.onPointerDown(pointer(20, 20)));
  assert.equal(captured, true);
  host.runTurn(() => mod.marquee.onPointerMove(pointer(22, 20)));
  assert.equal(mod.marquee.rect(), null);
  host.runTurn(() => mod.marquee.onPointerMove(pointer(70, 60, "2")));
  assert.equal(mod.marquee.active(), false);
  host.runTurn(() => mod.marquee.onPointerMove(pointer(70, 60)));
  assert.deepEqual(mod.marquee.rect(), { x: 20, y: 120, width: 50, height: 40 });
  assert.deepEqual(mod.selection.selectedKeys(), new Set([0, 2, 3]));
  host.runTurn(() => mod.marquee.onPointerCancel(pointer(70, 60)));
  assert.deepEqual(mod.selection.selectedKeys(), new Set([0]));
  assert.equal(mod.marquee.rect(), null);
  assert.equal(captured, false);
});
