import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Dialog, DialogContent, DialogTitle, DialogTrigger,
  Popover, PopoverTrigger, PopoverContent, PopoverHeader, PopoverTitle, PopoverDescription, PopoverClose,
  ThemeProvider } from "@hewel/zgui/components";
export const log = { mounts: 0, cleans: 0, changes: [] as boolean[], ends: 0 };
const [value, setValue] = createSignal(false);
const [escape, setEscape] = createSignal(true);
const [outside, setOutside] = createSignal(true);
const [cancel, setCancel] = createSignal(false);
export { setValue as setOpen, setEscape, setOutside, setCancel };
function Counter(props: { id: string }) {
  log.mounts += 1;
  onCleanup(() => { log.cleans += 1; });
  const [count, setCount] = createSignal(0);
  return <control testId={props.id} onClick={() => setCount(n => n + 1)}><text>{count()}</text></control>;
}
function Body() {
  return <>
    <PopoverTrigger testId="trigger" onClick={e => { if (cancel()) e.preventDefault(); }}>Open</PopoverTrigger>
    <PopoverContent testId="surface" class="caller" style={{ width: "400px" }}
      dismissOnEscape={escape()} dismissOnOutsidePress={outside()}
      onAnimationEnd={() => { log.ends += 1; }}>
      <PopoverHeader><PopoverTitle testId="title">Share</PopoverTitle><PopoverDescription testId="description">Send a link</PopoverDescription></PopoverHeader>
      <Counter id="counter" />
      <PopoverClose testId="close" onClick={e => { if (cancel()) e.preventDefault(); }}>Done</PopoverClose>
    </PopoverContent>
  </>;
}
export function mount(root: NativeNode) {
  return renderApp(() => <ThemeProvider scheme="light"><ThemeProvider scheme="dark">
    <Popover><Body /></Popover>
  </ThemeProvider></ThemeProvider>, root);
}
export function mountControlled(root: NativeNode) {
  return renderApp(() => <Popover open={value()} onOpenChange={next => log.changes.push(next)}><Body /></Popover>, root);
}
export function mountNested(root: NativeNode) {
  return renderApp(() => <Dialog defaultOpen><DialogTrigger testId="dialog-trigger">Dialog</DialogTrigger>
    <DialogContent testId="dialog" dismissControl={false}>
      <DialogTitle testId="dialog-title">Dialog title</DialogTitle>
      <Popover defaultOpen><PopoverTrigger testId="outer-trigger">Outer</PopoverTrigger>
        <PopoverContent testId="outer">
          <PopoverTitle testId="outer-title">Outer title</PopoverTitle>
          <Popover defaultOpen><PopoverTrigger testId="inner-trigger">Inner</PopoverTrigger>
            <PopoverContent testId="inner">
              <PopoverTitle testId="inner-title">Inner title</PopoverTitle>
            </PopoverContent>
          </Popover>
        </PopoverContent>
      </Popover>
    </DialogContent>
  </Dialog>, root);
}
export function mountCancelled(root: NativeNode) {
  return renderApp(() => {
    const [shown, setShown] = createSignal(true);
    return <>
      <Show when={shown()}><Popover defaultOpen>
        <PopoverContent testId="surface"><Counter id="counter" /></PopoverContent>
      </Popover></Show>
      <box ref={() => { setShown(false); }} />
    </>;
  }, root);
}
`;

const textOf = current =>
  current.tag === "#text"
    ? (current.props.get("text") ?? "")
    : current.children.map(textOf).join("");

async function mounted(t, mountName = "mount", allowScrollLock = false) {
  const h = createHost();
  const { animating: busy, traps, locks, layers, portals, geometry } = h;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  assert.equal(portals.length, 0, "module import must not allocate native nodes");
  const clock = createClock(t, h);
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
    if (!allowScrollLock) assert.equal(locks.length, 0, "Popover must not lock scroll");
  });
  globalThis.__zgui = h.host;
  const root = h.makeNode("root");
  dispose = mod[mountName](root);
  const click = testId => {
    const node = h.byTestId(testId);
    assert.ok(node, `missing node ${testId}`);
    return h.dispatch(node, "Click").defaultPrevented;
  };
  const emit = (record, value) => record.emit(value);
  const end = node => h.dispatch(node, "AnimationEnd");
  const flush = () => {
    h.flush();
    clock.frame();
    clock.advance(0);
  };
  const active = records => records.filter(record => record.active).length;
  return {
    h,
    mod,
    clock,
    busy,
    traps,
    locks,
    layers,
    portals,
    geometry,
    root,
    click,
    emit,
    end,
    flush,
    active,
    finish: () => {
      const stop = dispose;
      dispose = undefined;
      stop?.();
    },
  };
}

test("popover mounts lazily, wires relations and acquires trap dismiss and geometry leases without a scrim", async t => {
  const { h, mod, traps, layers, portals, geometry, click, emit, finish } = await mounted(t);
  assert.equal(mod.log.mounts, 0);
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(portals.length, 0);
  mod.setCancel(true);
  h.flush();
  assert.equal(click("trigger"), true);
  assert.equal(
    h.byTestId("surface"),
    undefined,
    "cancelled trigger click keeps the popover closed",
  );
  mod.setCancel(false);
  h.flush();
  click("trigger");
  assert.equal(mod.log.mounts, 1);
  const surface = h.byTestId("surface");
  const trigger = h.byTestId("trigger");
  assert.ok(surface);
  assert.equal(surface.props.get("role"), "dialog");
  assert.equal(surface.props.get("ariaModal"), true);
  assert.equal(surface.props.get("tabIndex"), -1);
  assert.equal(trigger.props.get("ariaHasPopup"), "dialog");
  assert.equal(trigger.props.get("expanded"), true);
  assert.equal(trigger.props.get("ariaControls"), surface);
  assert.equal(surface.props.get("ariaPopupFor"), trigger);
  assert.equal(surface.props.get("ariaLabelledBy"), h.byTestId("title"));
  assert.equal(surface.props.get("ariaDescribedBy"), h.byTestId("description"));
  const names = surface.props.get("class").split(/\s+/);
  assert.ok(names.includes("caller"));
  assert.ok(names.includes("zui-popover"));
  assert.ok(names.includes("zui-surface"));
  assert.deepEqual(surface.props.get("style"), { width: "400px" });
  assert.equal(traps.length, 1);
  assert.equal(traps[0].node, surface);
  assert.deepEqual([traps[0].wrap, traps[0].autoFocus, traps[0].restore], [true, true, true]);
  assert.equal(layers.length, 1);
  assert.equal(layers[0].node, surface);
  assert.deepEqual(layers[0].options, {
    layer: "popover",
    dismissOnOutsidePress: true,
    dismissOnEscape: true,
    exclude: trigger,
    leaving: false,
  });
  assert.equal(portals.length, 1);
  assert.equal(portals[0].layer, "popover");
  const scrims = [];
  const walk = node => {
    if ((node.props.get("class") ?? "").includes("scrim")) scrims.push(node);
    for (const child of node.children) walk(child);
  };
  walk(portals[0].node);
  assert.equal(scrims.length, 0, "popover renders no scrim");
  assert.equal(geometry.length, 3);
  assert.deepEqual(
    geometry.map(r => r.kind),
    ["borderBox", "contentSize", "viewport"],
  );
  assert.equal(geometry[0].node, trigger);
  const positioner = h.parents.get(surface);
  assert.equal(positioner.props.get("style").visibility, "hidden");
  emit(geometry[0], { x: 300, y: 200, width: 100, height: 30 });
  emit(geometry[2], { x: 0, y: 0, width: 800, height: 600, scale: 1 });
  assert.equal(positioner.props.get("style").visibility, "hidden", "hidden until every sample");
  emit(geometry[1], { width: 200, height: 120 });
  const style = positioner.props.get("style");
  assert.equal("visibility" in style, false);
  assert.equal(style.left, "250px");
  assert.equal(style.top, "234px");
  assert.equal(positioner.props.get("data-side"), "bottom");
  finish();
  for (const record of [...traps, ...layers, ...geometry]) {
    assert.equal(record.active, false);
    assert.equal(record.disposals, 1);
  }
  assert.equal(mod.log.cleans, 1);
});

test("popover exit retains a busy surface and reopening preserves identity", async t => {
  const { h, mod, clock, busy, traps, layers, geometry, click, end, flush, active, finish } =
    await mounted(t);
  click("trigger");
  const surface = h.byTestId("surface");
  const counter = h.byTestId("counter");
  click("counter");
  assert.equal(textOf(counter), "1");
  busy.set(surface, true);
  click("close");
  flush();
  assert.equal(h.byTestId("surface"), surface, "animating exit retains the surface");
  assert.equal(surface.props.get("data-state"), "closed");
  assert.equal(layers[0].options.leaving, true);
  assert.equal(active(geometry), 3);
  assert.equal(active(traps), 1);
  click("trigger");
  assert.equal(h.byTestId("surface"), surface, "reopen during exit keeps the same surface");
  assert.equal(h.byTestId("counter"), counter);
  assert.equal(textOf(counter), "1");
  assert.equal(geometry.length, 3, "no new subscriptions on reopen");
  assert.equal(layers[0].options.leaving, false);
  click("close");
  flush();
  busy.set(surface, false);
  end(surface);
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(h.byTestId("trigger").props.has("ariaControls"), false);
  assert.equal(active(geometry), 0);
  assert.equal(active(traps), 0);
  assert.equal(mod.log.ends, 1, "caller handler observes the animation end");
  click("trigger");
  const second = h.byTestId("surface");
  assert.ok(second);
  assert.notEqual(second, surface);
  assert.equal(textOf(h.byTestId("counter")), "0");
  assert.equal(mod.log.mounts, 2);
  assert.equal(active(geometry), 3);
  finish();
  assert.equal(clock.timers.size, 0);
  assert.equal(clock.frames.size, 0);
});

test("controlled popover lets the parent veto every transition", async t => {
  const { h, mod, layers, click, flush, finish } = await mounted(t, "mountControlled");
  click("trigger");
  assert.deepEqual(mod.log.changes, [true]);
  assert.equal(h.byTestId("surface"), undefined, "parent veto keeps the surface unmounted");
  mod.setOpen(true);
  h.flush();
  const surface = h.byTestId("surface");
  assert.ok(surface);
  mod.setEscape(false);
  h.flush();
  mod.setOutside(false);
  h.flush();
  assert.equal(layers.length, 1, "option changes reuse the same dismiss record");
  assert.equal(layers[0].options.dismissOnEscape, false);
  assert.equal(layers[0].options.dismissOnOutsidePress, false);
  layers[0].callback("outside-press");
  assert.deepEqual(mod.log.changes, [true, false]);
  assert.equal(h.byTestId("surface"), surface, "controlled close stays until the parent commits");
  mod.setCancel(true);
  h.flush();
  assert.equal(click("close"), true);
  assert.deepEqual(mod.log.changes, [true, false], "caller cancellation emits no request");
  mod.setCancel(false);
  h.flush();
  mod.setOpen(false);
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  finish();
});

test("default-open popovers nest inside a dialog with shared modal band and per-layer relations", async t => {
  const { h, traps, locks, layers, portals, geometry, finish } = await mounted(
    t,
    "mountNested",
    true,
  );
  const dialog = h.byTestId("dialog");
  const outer = h.byTestId("outer");
  const inner = h.byTestId("inner");
  assert.ok(dialog);
  assert.ok(outer);
  assert.ok(inner);
  assert.deepEqual(
    traps.map(record => record.node.props.get("testId")),
    ["dialog", "outer", "inner"],
  );
  assert.deepEqual(
    layers.map(record => record.node.props.get("testId")),
    ["dialog", "outer", "inner"],
  );
  assert.equal(locks.length, 1, "only the dialog locks scroll");
  assert.equal(locks[0].node, dialog);
  assert.deepEqual(
    portals.map(record => record.layer),
    ["modal", "modal", "modal"],
  );
  assert.deepEqual(
    layers.map(record => record.options.layer),
    ["modal", "modal", "modal"],
  );
  const depthOf = node => {
    for (let current = h.parents.get(node); current; current = h.parents.get(current)) {
      const names = (current.props.get("class") ?? "").split(/\s+/);
      if (names.includes("zui-overlay-depth")) return current.props.get("--zui-overlay-depth");
    }
  };
  assert.equal(depthOf(dialog), "0");
  assert.equal(depthOf(outer), "1");
  assert.equal(depthOf(inner), "2");
  assert.equal(dialog.props.get("ariaLabelledBy"), h.byTestId("dialog-title"));
  assert.equal(dialog.props.get("ariaPopupFor"), h.byTestId("dialog-trigger"));
  assert.equal(outer.props.get("ariaLabelledBy"), h.byTestId("outer-title"));
  assert.equal(outer.props.get("ariaPopupFor"), h.byTestId("outer-trigger"));
  assert.equal(inner.props.get("ariaLabelledBy"), h.byTestId("inner-title"));
  assert.equal(inner.props.get("ariaPopupFor"), h.byTestId("inner-trigger"));
  assert.equal(geometry.length, 6, "two poppers share the window observation path");
  finish();
  for (const record of [...traps, ...locks, ...layers, ...geometry]) {
    assert.equal(record.active, false);
    assert.equal(record.disposals, 1);
  }
});

test("removing an owner before its deferred overlay mounts cancels popover construction", async t => {
  const { h, mod, clock, traps, layers, geometry, active, finish } = await mounted(
    t,
    "mountCancelled",
  );
  assert.equal(mod.log.mounts, 0);
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(h.byTestId("counter"), undefined);
  for (const records of [traps, layers, geometry]) assert.equal(active(records), 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(clock.frames.size, 0);
  finish();
});
