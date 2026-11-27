import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Dialog, DialogTrigger, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter, DialogClose, DialogDismiss, ThemeProvider } from "@hewel/zgui/components";
import { AlertDialog, AlertDialogTrigger, AlertDialogContent, AlertDialogHeader, AlertDialogMedia, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogAction, AlertDialogCancel } from "@hewel/zgui/components";
export const log = { mounts: 0, cleans: 0, changes: [] as boolean[], ends: 0, ref: undefined as NativeNode | undefined };
const [value, setValue] = createSignal(false);
const [initial, setInitial] = createSignal(false);
const [cancel, setCancel] = createSignal(false);
const [disabled, setDisabled] = createSignal(false);
const [dismiss, setDismiss] = createSignal(true);
const [footer, setFooter] = createSignal(false);
const [escape, setEscape] = createSignal(true);
const [outside, setOutside] = createSignal(true);
const [scheme, setScheme] = createSignal<"light" | "dark">("dark");
export { setValue, setInitial, setCancel, setDisabled, setDismiss, setFooter, setEscape, setOutside, setScheme };
function Counter(props: { id: string }) {
  log.mounts += 1;
  onCleanup(() => { log.cleans += 1; });
  const [count, setCount] = createSignal(0);
  return <control testId={props.id} onClick={() => setCount(n => n + 1)}><text>{count()}</text></control>;
}
function Body() {
  return <>
    <DialogTrigger testId="trigger" disabled={disabled()} onClick={e => { if (cancel()) e.preventDefault(); }}>Open</DialogTrigger>
    <DialogContent testId="surface" class="caller" style={{ width: "400px" }} ref={node => { log.ref = node; }}
      dismissControl={dismiss()} dismissOnEscape={escape()} dismissOnOutsidePress={outside()}
      onAnimationEnd={() => { log.ends += 1; }}>
      <DialogHeader><DialogTitle testId="title">Rename</DialogTitle><DialogDescription testId="description">A project name</DialogDescription></DialogHeader>
      <Counter id="counter" />
      <DialogFooter testId="footer" closeControl={footer()}>
        <DialogClose testId="close" disabled={disabled()} onClick={e => { if (cancel()) e.preventDefault(); }}>Cancel</DialogClose>
      </DialogFooter>
    </DialogContent>
  </>;
}
export function mount(root: NativeNode) {
  return renderApp(() => <ThemeProvider scheme="light"><ThemeProvider scheme={scheme()}>
    <Dialog defaultOpen={initial()} onOpenChange={next => log.changes.push(next)}><Body /></Dialog>
  </ThemeProvider></ThemeProvider>, root);
}
export function mountAlert(root: NativeNode) {
  return renderApp(() => <AlertDialog onOpenChange={next => log.changes.push(next)}>
    <AlertDialogTrigger testId="trigger">Delete</AlertDialogTrigger>
    <AlertDialogContent testId="surface" size="sm" dismissOnEscape={escape()}>
      <AlertDialogHeader>
        <AlertDialogMedia>!</AlertDialogMedia>
        <AlertDialogTitle testId="title">Delete item?</AlertDialogTitle>
        <AlertDialogDescription testId="description">This cannot be undone.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel testId="cancel">Cancel</AlertDialogCancel>
        <AlertDialogAction testId="action" disabled={disabled()} onClick={event => { if (cancel()) event.preventDefault(); }}>Delete</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>, root);
}
export function mountControlled(root: NativeNode) {
  return renderApp(() => <Dialog open={value()} onOpenChange={next => log.changes.push(next)}><Body /></Dialog>, root);
}
export function mountNested(root: NativeNode) {
  return renderApp(() => <Dialog defaultOpen><DialogTrigger testId="outer-trigger">Outer</DialogTrigger>
    <DialogContent testId="outer" dismissControl={false}>
      <DialogTitle testId="outer-title">Outer title</DialogTitle>
      <DialogClose testId="outer-close">Close outer</DialogClose>
      <Dialog defaultOpen><DialogTrigger testId="inner-trigger">Inner</DialogTrigger>
        <DialogContent testId="inner" dismissControl={false}>
          <DialogTitle testId="inner-title">Inner title</DialogTitle>
          <DialogDismiss testId="inner-close" ariaLabel="Close inner" />
        </DialogContent>
      </Dialog>
    </DialogContent>
  </Dialog>, root);
}
export function mountStandalone(root: NativeNode) {
  return renderApp(() => <column><DialogTrigger testId="trigger">No root</DialogTrigger>
    <DialogClose testId="close">No root</DialogClose><DialogContent><Counter id="counter" /></DialogContent></column>, root);
}
export function mountCancelled(root: NativeNode) {
  return renderApp(() => {
    const [shown, setShown] = createSignal(true);
    return <>
      <Show when={shown()}><Dialog defaultOpen>
        <DialogContent testId="surface"><Counter id="counter" /></DialogContent>
      </Dialog></Show>
      <box ref={() => { setShown(false); }} />
    </>;
  }, root);
}
`;

const textOf = current =>
  current.tag === "#text"
    ? (current.props.get("text") ?? "")
    : current.children.map(textOf).join("");

async function mounted(t, mountName = "mount") {
  const h = createHost();
  const { animating: busy, traps, locks, layers, portals } = h;
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
  });
  globalThis.__zgui = h.host;
  const root = h.makeNode("root");
  dispose = mod[mountName](root);
  const click = testId => {
    const node = h.byTestId(testId);
    assert.ok(node, `missing node ${testId}`);
    return h.dispatch(node, "Click").defaultPrevented;
  };
  const end = node => h.dispatch(node, "AnimationEnd");
  const flush = () => {
    h.flush();
    clock.frame();
    clock.advance(0);
  };
  const active = records => records.filter(record => record.active).length;
  const settled = () => {
    for (const record of [...traps, ...locks, ...layers]) {
      assert.equal(record.active, false);
      assert.equal(record.disposals, 1);
    }
  };
  return {
    h,
    mod,
    clock,
    busy,
    traps,
    locks,
    layers,
    portals,
    root,
    click,
    end,
    flush,
    active,
    settled,
    finish: () => {
      const stop = dispose;
      dispose = undefined;
      stop?.();
    },
  };
}

test("dialog mounts lazily, wires relations and acquires trap lock and dismiss leases", async t => {
  const { h, mod, traps, locks, layers, portals, root, click, settled, finish } = await mounted(t);
  assert.equal(mod.log.mounts, 0);
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(portals.length, 0);
  mod.setInitial(true);
  h.flush();
  assert.equal(h.byTestId("surface"), undefined, "defaultOpen is read once at mount");
  click("trigger");
  assert.equal(mod.log.mounts, 1);
  const surface = h.byTestId("surface");
  const trigger = h.byTestId("trigger");
  assert.equal(surface, mod.log.ref);
  assert.equal(surface.props.get("role"), "dialog");
  assert.equal(surface.props.get("ariaModal"), true);
  assert.equal(surface.props.get("tabIndex"), -1);
  assert.ok(surface.props.get("class").includes("caller"));
  assert.ok(surface.props.get("class").split(/\s+/).includes("zui-dialog"));
  assert.deepEqual(surface.props.get("style"), { width: "400px" });
  assert.equal(trigger.props.get("ariaHasPopup"), "dialog");
  assert.equal(trigger.props.get("expanded"), true);
  assert.equal(trigger.props.get("ariaControls"), surface);
  assert.equal(surface.props.get("ariaPopupFor"), trigger);
  assert.equal(surface.props.get("ariaLabelledBy"), h.byTestId("title"));
  assert.equal(surface.props.get("ariaDescribedBy"), h.byTestId("description"));
  const dismiss = surface.children.find(child =>
    (child.props.get("class") ?? "").includes("zui-dialog__dismiss"),
  );
  assert.ok(dismiss, "default dismiss control is a direct child");
  assert.equal(dismiss.props.get("ariaLabel"), "Close");
  const vectors = [];
  const walk = node => {
    for (const child of node.children) {
      if (child.tag === "vector") vectors.push(child);
      walk(child);
    }
  };
  walk(dismiss);
  assert.equal(vectors.length, 1);
  assert.equal(traps.length, 1);
  assert.equal(traps[0].node, surface);
  assert.deepEqual([traps[0].wrap, traps[0].autoFocus, traps[0].restore], [true, true, true]);
  assert.equal(locks.length, 1);
  assert.equal(locks[0].node, surface);
  assert.equal(layers.length, 1);
  assert.equal(layers[0].node, surface);
  assert.deepEqual(layers[0].options, {
    layer: "modal",
    dismissOnOutsidePress: true,
    dismissOnEscape: true,
    exclude: trigger,
    leaving: false,
  });
  finish();
  settled();
  assert.equal(mod.log.cleans, 1);
  assert.equal(root.children.length, 0);
});

test("controlled open is parent authoritative", async t => {
  const { h, mod, click, flush, finish } = await mounted(t, "mountControlled");
  click("trigger");
  assert.deepEqual(mod.log.changes, [true]);
  assert.equal(h.byTestId("surface"), undefined, "parent veto keeps the surface unmounted");
  mod.setValue(true);
  h.flush();
  const surface = h.byTestId("surface");
  assert.ok(surface);
  click("close");
  assert.deepEqual(mod.log.changes, [true, false]);
  assert.equal(surface.props.get("data-state"), "open");
  assert.equal(h.byTestId("surface"), surface);
  mod.setValue(false);
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(h.byTestId("trigger").props.has("ariaControls"), false);
  finish();
});

test("cancellation disabled and live dismiss options", async t => {
  const { h, mod, traps, layers, click, flush, finish } = await mounted(t);
  mod.setCancel(true);
  h.flush();
  assert.equal(click("trigger"), true);
  assert.equal(h.byTestId("surface"), undefined);
  mod.setCancel(false);
  h.flush();
  click("trigger");
  const surface = h.byTestId("surface");
  assert.ok(surface);
  mod.setDisabled(true);
  h.flush();
  assert.equal(click("close"), false);
  assert.equal(surface.props.get("data-state"), "open");
  mod.setDisabled(false);
  h.flush();
  mod.setCancel(true);
  h.flush();
  assert.equal(click("close"), true);
  assert.equal(surface.props.get("data-state"), "open");
  mod.setCancel(false);
  h.flush();
  mod.setEscape(false);
  h.flush();
  mod.setOutside(false);
  h.flush();
  assert.equal(layers.length, 1, "option changes reuse the same dismiss record");
  assert.equal(traps.length, 1, "option changes do not rebuild the focus trap");
  assert.equal(layers[0].options.dismissOnEscape, false);
  assert.equal(layers[0].options.dismissOnOutsidePress, false);
  mod.setEscape(true);
  h.flush();
  assert.equal(layers[0].options.dismissOnEscape, true);
  layers[0].callback("escape");
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  finish();
});

test("exit retains surface while animating and reopening keeps its identity", async t => {
  const { h, mod, clock, busy, traps, locks, layers, click, end, flush, active, settled, finish } =
    await mounted(t);
  click("trigger");
  const surface = h.byTestId("surface");
  click("counter");
  assert.equal(textOf(h.byTestId("counter")), "1");
  busy.set(surface, true);
  click("close");
  assert.equal(h.byTestId("surface"), surface);
  assert.equal(surface.props.get("data-state"), "closed");
  assert.equal(active(locks), 0, "scroll lock follows open, not presence lifetime");
  assert.equal(active(traps), 1, "focus trap lives until actual unmount");
  assert.equal(layers[0].options.leaving, true);
  flush();
  assert.equal(h.byTestId("surface"), surface, "animating exit retains the surface");
  click("trigger");
  assert.equal(h.byTestId("surface"), surface, "reopen during exit keeps the same surface");
  assert.equal(textOf(h.byTestId("counter")), "1");
  assert.equal(traps.length, 1);
  assert.equal(active(locks), 1);
  assert.equal(layers[0].options.leaving, false);
  end(surface);
  assert.equal(mod.log.ends, 1, "caller handler still observes the event");
  clock.advance(1200);
  assert.equal(h.byTestId("surface"), surface);
  click("close");
  busy.set(surface, false);
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(mod.log.cleans, 1);
  assert.equal(h.byTestId("trigger").props.has("ariaControls"), false);
  click("trigger");
  const second = h.byTestId("surface");
  assert.ok(second);
  assert.notEqual(second, surface);
  assert.equal(textOf(h.byTestId("counter")), "0");
  assert.equal(mod.log.mounts, 2);
  finish();
  settled();
  assert.equal(clock.timers.size, 0);
  assert.equal(clock.frames.size, 0);
});

test("optional controls update without rebuilding user children", async t => {
  const { h, mod, portals, click, settled, finish } = await mounted(t);
  click("trigger");
  const surface = h.byTestId("surface");
  const counter = h.byTestId("counter");
  click("counter");
  assert.equal(textOf(counter), "1");
  mod.setDismiss(false);
  h.flush();
  mod.setFooter(true);
  h.flush();
  assert.equal(h.byTestId("surface"), surface);
  assert.equal(h.byTestId("counter"), counter);
  assert.equal(textOf(counter), "1");
  const hasDismiss = () =>
    surface.children.some(child =>
      (child.props.get("class") ?? "").includes("zui-dialog__dismiss"),
    );
  assert.equal(hasDismiss(), false);
  const footer = h.byTestId("footer");
  const cancel = footer.children.find(child => textOf(child) === "Cancel");
  const close = footer.children.find(child => textOf(child) === "Close");
  assert.ok(cancel);
  assert.equal(cancel.props.get("data-variant"), "ghost");
  assert.ok(close, "closeControl adds the default footer close");
  assert.equal(close.props.get("data-variant"), "outline");
  mod.setDismiss(true);
  h.flush();
  mod.setFooter(false);
  h.flush();
  assert.equal(h.byTestId("counter"), counter);
  assert.equal(textOf(counter), "1");
  assert.equal(mod.log.mounts, 1);
  assert.ok(hasDismiss());
  assert.equal(footer.children.length, 1);
  assert.equal(portals.length, 1, "no new portal allocation");
  finish();
  settled();
});

test("nested initial-open dialogs register parents before descendants with depth", async t => {
  const { h, traps, locks, layers, portals, click, flush, active, finish } = await mounted(
    t,
    "mountNested",
  );
  const outer = h.byTestId("outer");
  const inner = h.byTestId("inner");
  assert.ok(outer);
  assert.ok(inner);
  assert.deepEqual(
    traps.map(record => record.node.props.get("testId")),
    ["outer", "inner"],
  );
  assert.deepEqual(
    layers.map(record => record.node.props.get("testId")),
    ["outer", "inner"],
  );
  assert.equal(active(locks), 2);
  assert.deepEqual(
    portals.map(record => record.layer),
    ["modal", "modal"],
  );
  const depthOf = node => {
    for (let current = h.parents.get(node); current; current = h.parents.get(current)) {
      const names = (current.props.get("class") ?? "").split(/\s+/);
      if (names.includes("zui-overlay-depth")) return current.props.get("--zui-overlay-depth");
    }
  };
  assert.equal(depthOf(outer), "0");
  assert.equal(depthOf(inner), "1");
  assert.equal(outer.props.get("ariaLabelledBy"), h.byTestId("outer-title"));
  assert.equal(outer.props.get("ariaPopupFor"), h.byTestId("outer-trigger"));
  assert.equal(inner.props.get("ariaLabelledBy"), h.byTestId("inner-title"));
  assert.equal(inner.props.get("ariaPopupFor"), h.byTestId("inner-trigger"));
  click("inner-close");
  flush();
  assert.equal(h.byTestId("inner"), undefined);
  assert.equal(h.byTestId("outer"), outer);
  assert.equal(active(locks), 1);
  assert.equal(active(traps), 1);
  click("outer-close");
  flush();
  assert.equal(h.byTestId("outer"), undefined);
  assert.equal(active(locks), 0);
  assert.equal(active(traps), 0);
  assert.equal(active(layers), 0);
  finish();
});

test("scoped theme reacts across the portal without rebuilding content", async t => {
  const { h, mod, click, finish } = await mounted(t);
  click("trigger");
  const surface = h.byTestId("surface");
  const counter = h.byTestId("counter");
  let themeClass;
  for (let current = h.parents.get(surface); current; current = h.parents.get(current)) {
    themeClass = (current.props.get("class") ?? "")
      .split(/\s+/)
      .find(name => name.startsWith("zui-theme-"));
    if (themeClass) break;
  }
  assert.ok(themeClass, "surface sits under a re-scoped theme wrapper");
  const entry = [...h.stylesheets.entries()].find(([, css]) => css.includes(`.${themeClass}`));
  assert.ok(entry, "theme stylesheet exists for the re-scoped class");
  const [sheetName] = entry;
  assert.match(entry[1], /color-scheme:\s*dark/);
  mod.setScheme("light");
  h.flush();
  assert.ok(h.stylesheets.has(sheetName), "scheme switch keeps the same sheet name");
  assert.match(h.stylesheets.get(sheetName), /color-scheme:\s*light/);
  assert.equal(h.byTestId("surface"), surface);
  assert.equal(h.byTestId("counter"), counter);
  finish();
  assert.equal(h.stylesheets.has(sheetName), false);
});

test("removing an owner before its deferred overlay mounts cancels construction", async t => {
  const { h, mod, clock, traps, locks, layers, active, finish } = await mounted(
    t,
    "mountCancelled",
  );
  assert.equal(mod.log.mounts, 0);
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(h.byTestId("counter"), undefined);
  for (const records of [traps, locks, layers]) assert.equal(active(records), 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(clock.frames.size, 0);
  finish();
});

test("dialog parts outside a root are inert", async t => {
  const { h, mod, traps, locks, layers, portals, root, click, finish } = await mounted(
    t,
    "mountStandalone",
  );
  assert.equal(click("trigger"), false);
  assert.equal(click("close"), false);
  assert.equal(h.byTestId("counter"), undefined);
  assert.equal(mod.log.mounts, 0);
  assert.equal(portals.length, 0);
  assert.equal(traps.length, 0);
  assert.equal(locks.length, 0);
  assert.equal(layers.length, 0);
  finish();
  assert.equal(root.children.length, 0);
});

test("AlertDialog is labelled, has no implicit dismiss control, and honors action cancellation", async t => {
  const { h, mod, layers, click, finish } = await mounted(t, "mountAlert");
  click("trigger");
  const surface = h.byTestId("surface");
  assert.equal(surface.props.get("role"), "alertDialog");
  assert.equal(surface.props.get("ariaLabelledBy"), h.byTestId("title"));
  assert.equal(surface.props.get("ariaDescribedBy"), h.byTestId("description"));
  assert.equal(surface.props.get("data-size"), "sm");
  assert.equal(h.byTestId("cancel").props.get("data-variant"), "outline");
  assert.ok(!surface.children.some(child => child.props.get("ariaLabel") === "Close"));
  assert.equal(layers[0].options.dismissOnOutsidePress, false);
  assert.equal(layers[0].options.dismissOnEscape, true);
  mod.setEscape(false);
  mod.setDisabled(true);
  h.flush();
  assert.equal(layers[0].options.dismissOnEscape, false);
  click("action");
  assert.deepEqual(mod.log.changes, [true]);
  mod.setDisabled(false);
  mod.setCancel(true);
  h.flush();
  assert.equal(click("action"), true);
  assert.deepEqual(mod.log.changes, [true]);
  mod.setCancel(false);
  h.flush();
  click("action");
  assert.deepEqual(mod.log.changes, [true, false]);
  finish();
});
