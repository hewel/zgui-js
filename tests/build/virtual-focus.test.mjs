import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

function makeKeyEvent(key, opts = {}) {
  let prevented = false;
  let stopped = false;
  const event = {
    type: "keydown",
    key,
    character: key.length === 1 ? key : null,
    keyWithoutModifiers: key,
    deadKey: null,
    code: null,
    physicalCode: null,
    location: "standard",
    repeat: Boolean(opts.repeat),
    isComposing: Boolean(opts.isComposing),
    modifiers: {
      shift: Boolean(opts.shift),
      control: Boolean(opts.control),
      alt: Boolean(opts.alt),
      meta: Boolean(opts.meta),
    },
    preventDefault() {
      prevented = true;
    },
    stopPropagation() {
      stopped = true;
    },
    stopImmediatePropagation() {
      stopped = true;
    },
    get prevented() {
      return prevented;
    },
    get stopped() {
      return stopped;
    },
  };
  return event;
}

const VIRTUAL_FOCUS_FIXTURE = `
import { Show, createEffect, createSignal, renderApp } from "@hewel/zgui";
import type { NativeKeyEvent, NativeNode } from "@hewel/zgui";
import { createVirtualFocus } from "@hewel/zgui/primitives";
import type { VirtualFocusController, VirtualFocusHorizontalKeys } from "@hewel/zgui/primitives";

export interface TestItem {
  readonly id: string;
  readonly disabled?: boolean;
}

export function mountController(
  root: NativeNode,
  config: {
    initialItems: readonly TestItem[];
    columns?: number;
    wrap?: boolean;
    horizontalKeys?: VirtualFocusHorizontalKeys;
    editable?: boolean;
    initialText?: string;
    bindActivate?: boolean;
  },
) {
  let api!: {
    vf: VirtualFocusController;
    setItems: (next: readonly TestItem[]) => void;
    setColumns: (next: number) => void;
    setText: (next: string) => void;
    setMountedIds: (ids: readonly string[]) => void;
    getReveals: () => readonly { id: string; itemIndex: number; activeIdAtReveal: string | null }[];
    getActivations: () => readonly string[];
    getObservedActiveNodes: () => readonly (string | null)[];
    getInputNode: () => NativeNode;
    mountExtraNode: (id: string, tag?: "box" | "control") => { node: NativeNode; dispose: () => void };
  };

  const dispose = renderApp(() => {
    const [items, setItems] = createSignal<readonly TestItem[]>(config.initialItems);
    const [columns, setColumns] = createSignal<number>(config.columns ?? 1);
    const [text, setText] = createSignal<string>(config.initialText ?? "");
    const [mountedIds, setMountedIds] = createSignal<readonly string[]>([]);

    const reveals: { id: string; itemIndex: number; activeIdAtReveal: string | null }[] = [];
    const activations: string[] = [];
    const observedActiveNodes: (string | null)[] = [];
    let inputNode!: NativeNode;

    const vf = createVirtualFocus<TestItem>({
      items,
      getId: item => item.id,
      isDisabled: item => !!item.disabled,
      columns,
      wrap: config.wrap,
      horizontalKeys: config.horizontalKeys,
      textValue: config.editable ? text : undefined,
      onReveal(id, itemIndex) {
        reveals.push({ id, itemIndex, activeIdAtReveal: vf.activeId() });
      },
      onActivate: config.bindActivate === false ? undefined : item => activations.push(item.id),
    });

    createEffect(
      () => vf.activeNode(),
      node => {
        observedActiveNodes.push(node ? (node as unknown as { testId?: string }).testId ?? "node" : null);
      },
    );

    api = {
      vf,
      setItems: next => setItems(next),
      setColumns: next => setColumns(next),
      setText: next => setText(next),
      setMountedIds: next => setMountedIds(next),
      getReveals: () => reveals.slice(),
      getActivations: () => activations.slice(),
      getObservedActiveNodes: () => observedActiveNodes.slice(),
      getInputNode: () => inputNode,
      mountExtraNode(id: string) {
        const node = globalThis.__zgui.createElement("control");
        globalThis.__zgui.setProperty(node, "testId", "extra:" + id);
        const unreg = vf.register(id, node);
        return { node, dispose: unreg };
      },
    };

    const isMounted = (id: string) => mountedIds().includes(id);

    return (
      <column>
        <field
          ref={el => { inputNode = el; }}
          role="comboBox"
          ariaActiveDescendant={vf.activeNode()}
          onKeyDown={(e: NativeKeyEvent) => vf.onKeyDown(e)}
        />
        <Show when={isMounted("a")}>
          <control testId="row:a" ref={el => { vf.register("a", el); }} />
        </Show>
        <Show when={isMounted("b")}>
          <control testId="row:b" ref={el => { vf.register("b", el); }} />
        </Show>
        <Show when={isMounted("c")}>
          <control testId="row:c" ref={el => { vf.register("c", el); }} />
        </Show>
        <Show when={isMounted("d")}>
          <control testId="row:d" ref={el => { vf.register("d", el); }} />
        </Show>
      </column>
    );
  }, root);

  return { ...api, dispose };
}
`;

async function mountCase(t, config) {
  const harness = createHost();
  const mod = await moduleFor(t, VIRTUAL_FOCUS_FIXTURE);
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const handle = mod.mountController(root, config);
  dispose = handle.dispose;
  harness.flush();
  return { harness, ...handle };
}

test("createVirtualFocus 1D navigation updates activeId before onReveal, resolves virtual mounts reactively, and protects cleanup ownership", async t => {
  const ctx = await mountCase(t, {
    initialItems: [
      { id: "disabled-first", disabled: true },
      { id: "a" },
      { id: "b", disabled: true },
      { id: "c" },
      { id: "d" },
    ],
  });

  // Initial activeId skips disabled-first and lands on "a". Unmounted -> activeNode is null.
  assert.equal(ctx.vf.activeId(), "a");
  assert.equal(ctx.vf.activeNode(), null);
  assert.equal(ctx.getInputNode().props.get("ariaActiveDescendant"), undefined);

  // Navigate Down -> skips disabled "b" to unmounted "c"; activeId updates BEFORE onReveal.
  const down = makeKeyEvent("ArrowDown");
  ctx.vf.onKeyDown(down);
  ctx.harness.flush();
  assert.equal(down.prevented, true);
  assert.equal(down.stopped, true);
  assert.equal(ctx.vf.activeId(), "c");
  assert.equal(ctx.vf.activeNode(), null);
  assert.deepEqual(ctx.getReveals(), [{ id: "c", itemIndex: 3, activeIdAtReveal: "c" }]);

  // Virtualizer subsequently mounts row "c": activeNode() and ariaActiveDescendant update reactively without another keypress!
  ctx.setMountedIds(["c"]);
  ctx.harness.flush();
  const mountedC = ctx.vf.activeNode();
  assert.ok(mountedC, "activeNode must resolve once row 'c' registers");
  assert.equal(ctx.getInputNode().props.get("ariaActiveDescendant"), mountedC);

  // Overlapping remount: new node mounts for "c" BEFORE old row unmounts; old cleanup must NOT clobber new node!
  const newerMount = ctx.mountExtraNode("c");
  ctx.harness.flush();
  assert.equal(ctx.vf.activeNode(), newerMount.node);
  ctx.setMountedIds([]);
  ctx.harness.flush();
  assert.equal(
    ctx.vf.activeNode(),
    newerMount.node,
    "stale owner cleanup must only remove matching (id, node) pair",
  );
  assert.equal(ctx.vf.activeId(), "c", "unmounting old row must never clear logical activeId");

  // Disposing the current mounted node sets activeNode -> null while preserving activeId and NOT forcing reveal.
  const revealsBeforeUnmount = ctx.getReveals().length;
  newerMount.dispose();
  ctx.harness.flush();
  assert.equal(ctx.vf.activeNode(), null);
  assert.equal(ctx.vf.activeId(), "c");
  assert.equal(ctx.getReveals().length, revealsBeforeUnmount);

  // Pointer highlight setActive updates logical activeId without forcing scroll or stealing native focus.
  ctx.harness.focus(ctx.getInputNode());
  ctx.vf.setActive("d");
  ctx.harness.flush();
  assert.equal(ctx.vf.activeId(), "d");
  assert.equal(ctx.getReveals().length, revealsBeforeUnmount);
  assert.equal(ctx.harness.focused.current, ctx.getInputNode());

  // Disabled or missing setActive is ignored.
  ctx.vf.setActive("b");
  ctx.vf.setActive("missing");
  ctx.harness.flush();
  assert.equal(ctx.vf.activeId(), "d");

  // Removing or disabling active item falls back to first enabled ("a"); re-adding "d" does NOT jump back.
  ctx.setItems([{ id: "a" }, { id: "c" }, { id: "d", disabled: true }]);
  ctx.harness.flush();
  assert.equal(ctx.vf.activeId(), "a");
  ctx.setItems([{ id: "a" }, { id: "c" }, { id: "d" }]);
  ctx.harness.flush();
  assert.equal(ctx.vf.activeId(), "a", "re-enabling previously lost activeId must not jump back");

  // All-disabled / empty falls back to null.
  ctx.setItems([{ id: "x", disabled: true }]);
  ctx.harness.flush();
  assert.equal(ctx.vf.activeId(), null);
  ctx.setItems([]);
  ctx.harness.flush();
  assert.equal(ctx.vf.activeId(), null);

  // ArrowLeft / ArrowRight are untouched in 1D mode.
  const left1d = makeKeyEvent("ArrowLeft");
  const right1d = makeKeyEvent("ArrowRight");
  ctx.vf.onKeyDown(left1d);
  ctx.vf.onKeyDown(right1d);
  assert.equal(left1d.prevented, false);
  assert.equal(right1d.prevented, false);
});

test("createVirtualFocus 1D wrap: false stops at list boundaries", async t => {
  const ctx = await mountCase(t, {
    initialItems: [{ id: "a" }, { id: "b" }],
    wrap: false,
  });
  assert.equal(ctx.vf.activeId(), "a");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowUp"));
  assert.equal(ctx.vf.activeId(), "a");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowDown"));
  assert.equal(ctx.vf.activeId(), "b");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowDown"));
  assert.equal(ctx.vf.activeId(), "b");
});

test("createVirtualFocus 2D grid handles short final row clamping, desired column retention, disabled cells, and no horizontal wrap", async t => {
  // 6-column grid, 8 items:
  // Row 0: c0 c1 c2(disabled) c3 c4 c5
  // Row 1: c6 c7   (short final row of 2 items)
  const items6x8 = [
    { id: "c0" },
    { id: "c1" },
    { id: "c2", disabled: true },
    { id: "c3" },
    { id: "c4" },
    { id: "c5" },
    { id: "c6" },
    { id: "c7" },
  ];
  const ctx = await mountCase(t, { initialItems: items6x8, columns: 6 });

  // Horizontal move skips disabled c2 within row 0 and never wraps at row ends.
  ctx.vf.setActive("c1");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowRight"));
  assert.equal(ctx.vf.activeId(), "c3", "ArrowRight skips disabled cell c2");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowLeft"));
  assert.equal(ctx.vf.activeId(), "c1", "ArrowLeft skips disabled cell c2");
  ctx.vf.setActive("c0");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowLeft"));
  assert.equal(ctx.vf.activeId(), "c0", "ArrowLeft never wraps to previous row");
  ctx.vf.setActive("c5");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowRight"));
  assert.equal(ctx.vf.activeId(), "c5", "ArrowRight never wraps to next row");

  // Vertical move from col 5 (c5) into 2-item last row clamps to col 1 (c7), retaining desiredColumn = 5.
  ctx.vf.onKeyDown(makeKeyEvent("ArrowDown"));
  assert.equal(ctx.vf.activeId(), "c7", "col 5 clamps to last existing col (c7) on 2-item row");
  // Reverse vertical move uses retained desiredColumn = 5 to return to c5!
  ctx.vf.onKeyDown(makeKeyEvent("ArrowUp"));
  assert.equal(ctx.vf.activeId(), "c5", "ArrowUp restores retained desiredColumn = 5");

  // Horizontal move resets desiredColumn: c5 -> Down -> c7 -> Left -> c6 -> Up -> c0 (col 0, NOT col 5!).
  ctx.vf.onKeyDown(makeKeyEvent("ArrowDown"));
  assert.equal(ctx.vf.activeId(), "c7");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowLeft"));
  assert.equal(ctx.vf.activeId(), "c6");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowUp"));
  assert.equal(ctx.vf.activeId(), "c0", "horizontal move must reset desiredColumn");

  // Disabled clamped candidate on final row with no further rows stays put (NO sideways search to c6!).
  ctx.setItems(items6x8.map(item => (item.id === "c7" ? { ...item, disabled: true } : item)));
  ctx.harness.flush();
  ctx.vf.setActive("c5");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowDown"));
  assert.equal(
    ctx.vf.activeId(),
    "c5",
    "disabled clamped candidate on final row must stay put without sideways search",
  );

  // Home / End move to current row endpoints; Ctrl+Home / Ctrl+End move to entire grid endpoints.
  ctx.setItems(items6x8);
  ctx.harness.flush();
  ctx.vf.setActive("c4");
  ctx.vf.onKeyDown(makeKeyEvent("Home"));
  assert.equal(ctx.vf.activeId(), "c0");
  ctx.vf.onKeyDown(makeKeyEvent("End"));
  assert.equal(ctx.vf.activeId(), "c5");
  ctx.vf.onKeyDown(makeKeyEvent("End", { control: true }));
  assert.equal(ctx.vf.activeId(), "c7");
  ctx.vf.onKeyDown(makeKeyEvent("Home", { control: true }));
  assert.equal(ctx.vf.activeId(), "c0");

  // Column count change resets desiredColumn: c5 -> Down -> c7 (desired=5), switch to 4 cols -> Up from c7 (row 1, col 3) goes to c3!
  ctx.vf.setActive("c5");
  ctx.vf.onKeyDown(makeKeyEvent("ArrowDown"));
  assert.equal(ctx.vf.activeId(), "c7");
  ctx.setColumns(4);
  ctx.harness.flush();
  ctx.vf.onKeyDown(makeKeyEvent("ArrowUp"));
  assert.equal(ctx.vf.activeId(), "c3", "column count change must reset desiredColumn");
});

test("createVirtualFocus editable vs non-editable key routing, raw empty check, and IME isComposing guard", async t => {
  // Editable 2D grid with default horizontalKeys: "whenEmpty".
  const editable = await mountCase(t, {
    initialItems: [{ id: "a" }, { id: "b" }, { id: "c" }, { id: "d" }],
    columns: 2,
    editable: true,
    initialText: "",
  });

  // 1. IME isComposing === true ignores EVERY key completely without consuming it.
  for (const key of [
    "ArrowDown",
    "ArrowUp",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "Enter",
    "Space",
  ]) {
    const ev = makeKeyEvent(key, { isComposing: true });
    editable.vf.onKeyDown(ev);
    assert.equal(ev.prevented, false, `composing ${key} must not call preventDefault`);
    assert.equal(ev.stopped, false, `composing ${key} must not call stopPropagation`);
  }
  assert.equal(editable.vf.activeId(), "a");
  assert.deepEqual(editable.getActivations(), []);

  // 2. Empty raw text (length === 0): ArrowRight navigates grid, Home/End navigate.
  const rightEmpty = makeKeyEvent("ArrowRight");
  editable.vf.onKeyDown(rightEmpty);
  assert.equal(rightEmpty.prevented, true);
  assert.equal(editable.vf.activeId(), "b");

  // 3. Whitespace-only text (" ") has raw length 1 (NOT trimmed!) -> ArrowLeft/Right & Home/End belong to editor!
  editable.setText(" ");
  editable.harness.flush();
  for (const key of ["ArrowLeft", "ArrowRight", "Home", "End"]) {
    const ev = makeKeyEvent(key);
    editable.vf.onKeyDown(ev);
    assert.equal(ev.prevented, false, `${key} must be left to editor when textValue is " "`);
  }
  assert.equal(editable.vf.activeId(), "b");

  // 4. Space NEVER intercepted in editable mode; Enter activates once (repeat consumed without re-invoking).
  const spaceEdit = makeKeyEvent("Space");
  editable.vf.onKeyDown(spaceEdit);
  assert.equal(spaceEdit.prevented, false);
  const enterOnce = makeKeyEvent("Enter");
  editable.vf.onKeyDown(enterOnce);
  assert.equal(enterOnce.prevented, true);
  assert.deepEqual(editable.getActivations(), ["b"]);
  const enterRepeat = makeKeyEvent("Enter", { repeat: true });
  editable.vf.onKeyDown(enterRepeat);
  assert.equal(enterRepeat.prevented, true);
  assert.deepEqual(editable.getActivations(), ["b"]);

  // 5. Modified arrows and unrelated keys (Tab, Escape, PageUp, PageDown) untouched.
  for (const ev of [
    makeKeyEvent("ArrowDown", { shift: true }),
    makeKeyEvent("ArrowUp", { control: true }),
    makeKeyEvent("Tab"),
    makeKeyEvent("Escape"),
    makeKeyEvent("PageUp"),
    makeKeyEvent("PageDown"),
  ]) {
    editable.vf.onKeyDown(ev);
    assert.equal(ev.prevented, false, `${ev.key} must not be consumed`);
  }

  // 6. horizontalKeys: "never" vs "always" on editable grid.
  const neverHoriz = await mountCase(t, {
    initialItems: [{ id: "a" }, { id: "b" }],
    columns: 2,
    editable: true,
    horizontalKeys: "never",
    initialText: "",
  });
  const rightNever = makeKeyEvent("ArrowRight");
  neverHoriz.vf.onKeyDown(rightNever);
  assert.equal(rightNever.prevented, false);
  assert.equal(neverHoriz.vf.activeId(), "a");

  const alwaysHoriz = await mountCase(t, {
    initialItems: [{ id: "a" }, { id: "b" }],
    columns: 2,
    editable: true,
    horizontalKeys: "always",
    initialText: "hello",
  });
  const rightAlways = makeKeyEvent("ArrowRight");
  alwaysHoriz.vf.onKeyDown(rightAlways);
  assert.equal(rightAlways.prevented, true);
  assert.equal(alwaysHoriz.vf.activeId(), "b");

  // 7. Non-editable owner: Space and Enter both activate once (repeat ignored).
  const nonEditable = await mountCase(t, {
    initialItems: [{ id: "a" }, { id: "b" }],
    columns: 2,
    editable: false,
  });
  const spaceNonEdit = makeKeyEvent("Space");
  nonEditable.vf.onKeyDown(spaceNonEdit);
  assert.equal(spaceNonEdit.prevented, true);
  assert.deepEqual(nonEditable.getActivations(), ["a"]);
  const spaceRepeat = makeKeyEvent("Space", { repeat: true });
  nonEditable.vf.onKeyDown(spaceRepeat);
  assert.equal(spaceRepeat.prevented, true);
  assert.deepEqual(nonEditable.getActivations(), ["a"]);
});
