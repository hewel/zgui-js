import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button, Dialog, DialogContent, DialogTitle, DialogTrigger,
  HoverCard, HoverCardContent, HoverCardTrigger,
  Tooltip, TooltipArrow, TooltipContent, TooltipProvider, TooltipTrigger } from "@hewel/zgui/components";
export const log = { mounts: 0, cleans: 0, changes: [] as boolean[], ends: 0, calls: [] as string[] };
const [delay, setDelay] = createSignal(100);
const [closeDelay, setCloseDelay] = createSignal(80);
const [value, setValue] = createSignal(false);
const [arrow, setArrow] = createSignal(true);
const [cancel, setCancel] = createSignal(false);
const [ids, setIds] = createSignal(["a", "b"]);
export { setDelay, setCloseDelay, setValue as setOpen, setArrow, setCancel, setIds };
let bumpCounter: (() => void) | undefined;
export const bump = () => bumpCounter?.();
function Counter(props: { id?: string }) {
  log.mounts += 1;
  onCleanup(() => { log.cleans += 1; bumpCounter = undefined; });
  const [count, setCount] = createSignal(0);
  bumpCounter = () => setCount(n => n + 1);
  return <text testId={props.id ?? "counter"}>{count()}</text>;
}
const callers = (label: string) => ({
  onPointerEnter(event: { preventDefault(): void }) { log.calls.push(label + ":enter"); if (cancel()) event.preventDefault(); },
  onPointerLeave(event: { preventDefault(): void }) { log.calls.push(label + ":leave"); if (cancel()) event.preventDefault(); },
  onFocus(event: { preventDefault(): void }) { log.calls.push(label + ":focus"); if (cancel()) event.preventDefault(); },
  onBlur(event: { preventDefault(): void }) { log.calls.push(label + ":blur"); if (cancel()) event.preventDefault(); },
  onKeyDown(event: { preventDefault(): void }) { log.calls.push(label + ":key"); if (cancel()) event.preventDefault(); },
});
function TipBody() {
  return <>
    <TooltipTrigger testId="trigger" {...callers("trigger")}>
      <Button testId="button">Hover</Button>
    </TooltipTrigger>
    <TooltipContent testId="surface" class="caller" data-tone="info" style={{ width: "200px" }}
      arrow={arrow()} offset={0} onAnimationEnd={() => { log.ends += 1; }}>
      <Counter />
    </TooltipContent>
  </>;
}
export function mount(root: NativeNode) {
  return renderApp(() => <Tooltip delay={delay()} closeDelay={closeDelay()}><TipBody /></Tooltip>, root);
}
export function mountDefault(root: NativeNode) {
  return renderApp(() => <Tooltip><TipBody /></Tooltip>, root);
}
export function mountControlled(root: NativeNode) {
  return renderApp(() => <Tooltip open={value()} onOpenChange={next => { log.changes.push(next); }}
    delay={delay()} closeDelay={closeDelay()}><TipBody /></Tooltip>, root);
}
function CardBody() {
  return <>
    <HoverCardTrigger testId="trigger" {...callers("trigger")}>
      <Button testId="button">Card</Button>
    </HoverCardTrigger>
    <HoverCardContent testId="surface" class="caller" style={{ width: "200px" }}
      onAnimationEnd={() => { log.ends += 1; }}>
      <Counter />
    </HoverCardContent>
  </>;
}
export function mountCard(root: NativeNode) {
  return renderApp(() => <HoverCard delay={delay()} closeDelay={closeDelay()}><CardBody /></HoverCard>, root);
}
export function mountCardDefault(root: NativeNode) {
  return renderApp(() => <HoverCard><CardBody /></HoverCard>, root);
}
export function mountCardControlled(root: NativeNode) {
  return renderApp(() => <HoverCard open={value()} onOpenChange={next => { log.changes.push(next); setValue(next); }}
    delay={delay()} closeDelay={closeDelay()}><CardBody /></HoverCard>, root);
}
export function mountProvider(root: NativeNode) {
  return renderApp(() => <TooltipProvider delay={100} closeDelay={80}>
    <For each={ids()}>{id => <Tooltip>
      <TooltipTrigger testId={"trigger-" + id}>{"Trigger " + id}</TooltipTrigger>
      <TooltipContent testId={"surface-" + id}><Counter id={"counter-" + id} /></TooltipContent>
    </Tooltip>}</For>
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger testId="trigger-c">Trigger c</TooltipTrigger>
        <TooltipContent testId="surface-c"><Counter id="counter-c" /></TooltipContent>
      </Tooltip>
    </TooltipProvider>
  </TooltipProvider>, root);
}
export function badArrow(root: NativeNode) {
  return renderApp(() => <TooltipArrow children="unexpected" />, root);
}
export function mountCancelled(root: NativeNode) {
  return renderApp(() => {
    const [shown, setShown] = createSignal(true);
    return <>
      <Show when={shown()}><Tooltip defaultOpen><TipBody /></Tooltip></Show>
      <box ref={() => { setShown(false); }} />
    </>;
  }, root);
}
export function mountNested(root: NativeNode) {
  return renderApp(() => <Dialog defaultOpen>
    <DialogTrigger testId="dialog-trigger">Dialog</DialogTrigger>
    <DialogContent testId="dialog" dismissControl={false}>
      <DialogTitle testId="dialog-title">Dialog title</DialogTitle>
      <Tooltip defaultOpen>
        <TooltipTrigger testId="tip-trigger">Tip</TooltipTrigger>
        <TooltipContent testId="tip"><Counter id="tip-counter" /></TooltipContent>
      </Tooltip>
      <HoverCard defaultOpen>
        <HoverCardTrigger testId="card-trigger">Card</HoverCardTrigger>
        <HoverCardContent testId="card"><Counter id="card-counter" /></HoverCardContent>
      </HoverCard>
      <TooltipContent testId="stray" />
    </DialogContent>
  </Dialog>, root);
}
`;

async function mounted(t, mountName = "mount", allowScrollLock = false) {
  const h = createHost();
  const { animating: busy, traps, locks, layers, portals, geometry } = h;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  assert.equal(portals.length, 0, "module import must not allocate native nodes");
  const clock = createClock(t, h);
  let dispose;
  t.after(() => {
    try { dispose?.(); } finally { delete globalThis.__zgui; }
    if (!allowScrollLock) assert.equal(locks.length, 0, "hover surfaces must not lock scroll");
  });
  globalThis.__zgui = h.host;
  const root = h.makeNode("root");
  dispose = mod[mountName](root);
  const send = (testId, name, extras = {}) => {
    const node = h.byTestId(testId);
    assert.ok(node, `missing node ${testId}`);
    return h.dispatch(node, name, extras);
  };
  const emit = (record, value) => (record.emit ?? record.trigger)(value);
  const end = node => h.dispatch(node, "AnimationEnd");
  const flush = () => { h.flush(); clock.frame(); clock.advance(0); };
  const active = records => records.filter(record => record.active).length;
  return { h, mod, clock, busy, traps, locks, layers, portals, geometry, root, send, emit, end, flush, active,
    finish: () => { const stop = dispose; dispose = undefined; stop?.(); } };
}

test("default tooltip opens synchronously, wires relations, geometry and arrow lifecycle", async t => {
  const { h, mod, clock, traps, locks, layers, portals, geometry, send, emit, flush, finish } = await mounted(t, "mountDefault");
  const trigger = h.byTestId("trigger");
  assert.equal(mod.log.mounts, 0);
  assert.equal(h.byTestId("surface"), undefined);
  send("trigger", "onPointerEnter", { position: { x: 1, y: 1 }, pointerKind: "mouse" });
  assert.equal(h.byTestId("counter") == null, false, "zero delay must mount without timers");
  assert.equal(clock.timers.size, 0, "synchronous open leaves no pending timer");
  const surface = h.byTestId("surface");
  assert.equal(surface.props.get("role"), "tooltip");
  assert.equal(surface.props.get("ariaModal"), false);
  assert.equal(surface.props.get("tabIndex"), undefined);
  assert.equal(trigger.props.get("tabIndex"), undefined, "tooltip trigger is not an added tab stop");
  assert.equal(trigger.props.get("ariaDescribedBy"), surface);
  assert.equal(surface.props.get("ariaPopupFor"), trigger);
  assert.equal(surface.props.get("data-state"), "open");
  assert.ok(surface.props.get("class").includes("caller"));
  assert.ok(surface.props.get("class").includes("zui-tooltip"));
  assert.ok(surface.props.get("class").includes("zui-surface"));
  assert.equal(surface.props.get("data-tone"), "info");
  assert.deepEqual(surface.props.get("style"), { width: "200px" });
  assert.equal(traps.length, 0, "tooltip must not trap focus");
  assert.equal(locks.length, 0, "tooltip must not lock scroll");
  assert.equal(portals.length, 1);
  assert.equal(portals[0].layer, "popover");
  assert.equal(layers.length, 1);
  assert.equal(layers[0].options.dismissOnOutsidePress, false);
  assert.equal(layers[0].options.dismissOnEscape, true);
  assert.equal(layers[0].options.exclude, trigger);
  assert.equal(layers[0].options.leaving, false);
  const anchor = geometry.find(record => record.node === trigger);
  assert.ok(anchor && anchor.kind === "borderBox");
  const place = h.parents.get(surface);
  const floating = geometry.find(record => record.node === place && record.kind === "contentSize");
  const viewport = geometry.find(record => record.kind === "viewport");
  assert.ok(floating && viewport);
  emit(anchor, { x: 300, y: 200, width: 100, height: 30 });
  emit(floating, { width: 200, height: 100 });
  emit(viewport, { x: 0, y: 0, width: 800, height: 600, scale: 1 });
  flush();
  assert.equal(place.props.get("data-side"), "top");
  assert.equal(place.props.get("style").top, "90px", "arrow clamps the supplied zero offset to the default reach");
  assert.equal(place.props.get("style").left, "250px");
  const arrowOf = () => surface.children.find(child => child.props.get("class")?.includes("zui-tooltip__arrow"));
  const firstArrow = arrowOf();
  assert.ok(firstArrow, "default arrow is rendered");
  assert.equal(firstArrow.props.get("ariaHidden"), true);
  const counter = h.byTestId("counter");
  mod.setArrow(false);
  flush();
  assert.equal(arrowOf(), undefined);
  assert.equal(h.byTestId("counter"), counter, "arrow toggle must preserve child state");
  assert.equal(place.props.get("style").top, "100px", "without an arrow the supplied offset is preserved");
  mod.setArrow(true);
  flush();
  const secondArrow = arrowOf();
  assert.ok(secondArrow);
  assert.notEqual(secondArrow, firstArrow, "re-enabled arrow is a fresh node");
  assert.equal(h.byTestId("counter"), counter);
  assert.equal(mod.log.mounts, 1, "single lazy counter construction across arrow toggles");
  finish();
});

test("delayed tooltip cancels pending intent, retains busy exit and remounts fresh", async t => {
  const { h, mod, clock, busy, layers, geometry, send, end, flush, active, finish } = await mounted(t);
  const trigger = h.byTestId("trigger");
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  assert.equal(clock.timers.size, 1, "delayed enter queues one opening timer");
  clock.advance(99);
  assert.equal(h.byTestId("surface"), undefined);
  send("trigger", "onPointerLeave", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(500);
  assert.equal(h.byTestId("surface"), undefined, "leave cancels a pending open");
  assert.equal(clock.timers.size, 0);
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(100);
  const surface = h.byTestId("surface");
  assert.ok(surface);
  send("trigger", "onPointerLeave", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(79);
  assert.equal(surface.props.get("data-state"), "open");
  clock.advance(1);
  assert.equal(surface.props.get("data-state"), "closed");
  assert.equal(layers[0].options.leaving, true);
  busy.set(surface, true);
  flush();
  assert.equal(h.byTestId("surface"), surface, "busy animation retains the exiting surface");
  mod.bump();
  h.flush();
  assert.equal(h.byTestId("counter").children.map(node => node.props.get("text")).join(""), "1");
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(100);
  assert.equal(h.byTestId("surface"), surface, "reopening mid-exit preserves the surface");
  assert.equal(h.byTestId("counter").children.map(node => node.props.get("text")).join(""), "1");
  assert.equal(mod.log.mounts, 1);
  send("trigger", "onPointerLeave", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(80);
  flush();
  assert.equal(h.byTestId("surface"), surface, "still busy exit is retained");
  busy.set(surface, false);
  end(surface);
  flush();
  assert.equal(h.byTestId("surface"), undefined, "animation end releases the surface");
  assert.ok(trigger.props.get("ariaDescribedBy") == null, "relation clears after teardown");
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(100);
  const fresh = h.byTestId("surface");
  assert.ok(fresh && fresh !== surface, "reopen after full exit remounts");
  assert.equal(h.byTestId("counter").children.map(node => node.props.get("text")).join(""), "0");
  assert.equal(mod.log.mounts, 2);
  assert.equal(mod.log.ends, 1, "caller animation-end forwarded once for the released exit");
  finish();
  assert.equal(active(geometry), 0);
  assert.equal(active(layers), 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(clock.frames.size, 0);
});

test("tooltip focus causes, caller veto and escape policy", async t => {
  const { h, mod, clock, send, flush, finish } = await mounted(t);
  for (const cause of ["pointer", "programmatic", "window"]) {
    send("trigger", "onFocus", { cause });
    assert.equal(h.byTestId("surface"), undefined, `${cause} focus must not open`);
    assert.equal(clock.timers.size, 0, `${cause} focus must not queue timers`);
  }
  send("trigger", "onFocus", { cause: "keyboard" });
  assert.ok(h.byTestId("surface"), "keyboard focus opens immediately despite a delay");
  assert.equal(clock.timers.size, 0);
  send("trigger", "onBlur", { cause: "keyboard" });
  assert.equal(h.byTestId("surface").props.get("data-state"), "closed", "blur closes immediately");
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  assert.equal(clock.timers.size, 1);
  send("trigger", "onBlur", { cause: "pointer" });
  clock.advance(500);
  assert.equal(h.byTestId("surface"), undefined, "blur cancels a pending open");
  mod.setCancel(true);
  h.flush();
  const entered = send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  assert.equal(entered.defaultPrevented, true);
  assert.equal(h.byTestId("surface"), undefined, "caller preventDefault vetoes hover opening");
  assert.equal(clock.timers.size, 0);
  const focused = send("trigger", "onFocus", { cause: "keyboard" });
  assert.equal(focused.defaultPrevented, true);
  assert.equal(h.byTestId("surface"), undefined, "caller preventDefault vetoes focus opening");
  assert.ok(mod.log.calls.includes("trigger:enter") && mod.log.calls.includes("trigger:focus"),
    "caller handlers still run before the veto");
  mod.setCancel(false);
  h.flush();
  const closedEscape = send("trigger", "onKeyDown", { key: "Escape" });
  assert.equal(closedEscape.propagationStopped, false, "closed trigger must not stop Escape");
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(100);
  assert.ok(h.byTestId("surface"));
  const openEscape = send("trigger", "onKeyDown", { key: "Escape" });
  assert.equal(openEscape.propagationStopped, true, "open trigger stops Escape");
  assert.equal(h.byTestId("surface").props.get("data-state"), "closed");
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  finish();
});

test("hover card honours 700/300 defaults, re-entry keeps identity and dismissal cancels pending jobs", async t => {
  const { h, clock, layers, send, emit, flush, finish } = await mounted(t, "mountCardDefault");
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(699);
  assert.equal(h.byTestId("surface"), undefined);
  clock.advance(1);
  const surface = h.byTestId("surface");
  assert.ok(surface, "700ms default delay opens");
  send("trigger", "onPointerLeave", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(200);
  assert.equal(surface.props.get("data-state"), "open", "inside the 300ms close delay");
  send("surface", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(700);
  assert.equal(h.byTestId("surface"), surface, "re-entry keeps the same surface");
  send("surface", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  assert.equal(clock.timers.size, 1, "entering an open surface queues an opening job");
  emit(layers.find(record => record.node === surface), undefined);
  flush();
  assert.equal(h.byTestId("surface"), undefined, "dismiss closes the surface");
  clock.advance(1000);
  assert.equal(h.byTestId("surface"), undefined, "cancelled opening job must not resurrect");
  send("trigger", "onFocus", { cause: "programmatic" });
  assert.ok(h.byTestId("surface"), "programmatic focus opens a card immediately");
  finish();
});

test("controlled hover card refocuses without false-then-true churn and honours parent veto", async t => {
  const { h, mod, clock, traps, send, flush, finish } = await mounted(t, "mountCardControlled");
  const trigger = h.byTestId("trigger");
  assert.equal(trigger.props.get("tabIndex"), 0);
  assert.equal(trigger.props.get("ariaHasPopup"), "dialog");
  assert.equal(trigger.props.get("expanded"), false);
  send("trigger", "onFocus", { cause: "programmatic" });
  assert.deepEqual(mod.log.changes, [true], "focus requests open even for programmatic causes");
  const surface = h.byTestId("surface");
  assert.ok(surface, "echoing controlled parent mounts the surface");
  assert.equal(surface.props.get("role"), "dialog");
  assert.equal(surface.props.get("ariaModal"), false);
  assert.equal(trigger.props.get("expanded"), true);
  assert.equal(trigger.props.get("ariaControls"), surface);
  assert.equal(surface.props.get("ariaPopupFor"), trigger);
  assert.equal(traps.length, 0, "hover card must not trap focus");
  mod.log.changes.length = 0;
  send("trigger", "onFocus", { cause: "keyboard" });
  assert.deepEqual(mod.log.changes, [], "refocusing an already-open card calls openNow() without false-then-true churn");
  assert.equal(h.byTestId("surface"), surface, "same surface survives refocus");
  mod.log.changes.length = 0;
  send("surface", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  assert.equal(clock.timers.size, 1, "surface enter queues an opening job");
  mod.setOpen(false);
  h.flush();
  clock.advance(1000);
  assert.equal(h.byTestId("surface"), undefined, "parent close cancels the pending job");
  assert.equal(mod.log.changes.length, 0, "parent close issues no new requests");
  flush();
  finish();
});

test("provider handoff preserves identity, isolates nested groups and clears disposed owners", async t => {
  const { h, mod, clock, send, flush, finish } = await mounted(t, "mountProvider");
  const triggerA = h.byTestId("trigger-a");
  send("trigger-a", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(100);
  const surfaceA = h.byTestId("surface-a");
  const counterA = h.byTestId("counter-a");
  assert.ok(surfaceA);
  mod.bump();
  h.flush();
  assert.equal(counterA.children.map(node => node.props.get("text")).join(""), "1");
  mod.setIds(["b", "a"]);
  h.flush();
  assert.equal(h.byTestId("trigger-a"), triggerA, "keyed reorder keeps trigger identity");
  assert.equal(h.byTestId("surface-a"), surfaceA, "keyed reorder keeps content identity");
  assert.equal(h.byTestId("counter-a").children.map(node => node.props.get("text")).join(""), "1", "child state survives reorder");
  send("trigger-a", "onPointerLeave", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  send("trigger-b", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  const surfaceB = h.byTestId("surface-b");
  assert.ok(surfaceB, "handoff skips the opening delay while another member is open");
  assert.equal(surfaceA.props.get("data-state"), "closed");
  assert.equal(surfaceA.props.get("data-instant"), "", "replaced surface is marked instant");
  flush();
  assert.equal(h.byTestId("surface-a"), undefined, "instant exit completes on the idle styled frame");
  clock.advance(200);
  assert.equal(h.byTestId("surface-b"), surfaceB, "b persists past a's old close deadline");
  send("surface-b", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  assert.equal(clock.timers.size, 1, "surface enter queues a fresh opening job");
  send("trigger-a", "onFocus", { cause: "keyboard" });
  const revivedA = h.byTestId("surface-a");
  assert.ok(revivedA && revivedA !== surfaceA, "keyboard focus opens a immediately");
  assert.equal(revivedA.props.get("data-state"), "open");
  clock.advance(1000);
  assert.equal(h.byTestId("surface-b"), undefined, "cancelled pending job must not resurrect b");
  send("trigger-c", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  assert.ok(h.byTestId("surface-c"), "nested default provider opens with its own zero delay");
  assert.equal(h.byTestId("surface-a"), revivedA, "nested group does not disturb the outer member");
  mod.setIds(["b"]);
  h.flush();
  assert.equal(h.byTestId("trigger-a"), undefined);
  send("trigger-b", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(99);
  assert.equal(h.byTestId("surface-b"), undefined, "disposed remembered member cannot fast-track b");
  clock.advance(1);
  assert.ok(h.byTestId("surface-b"));
  finish();
  assert.equal(clock.timers.size, 0);
  assert.equal(clock.frames.size, 0);
});

test("controlled tooltip honours parent veto, retained dismiss and pending cancellation", async t => {
  const { h, mod, clock, layers, send, emit, end, flush, finish } = await mounted(t, "mountControlled");
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(100);
  assert.deepEqual(mod.log.changes, [true]);
  assert.equal(h.byTestId("surface"), undefined, "parent veto keeps the surface unmounted");
  mod.setOpen(true);
  h.flush();
  const surface = h.byTestId("surface");
  assert.ok(surface);
  emit(layers.find(record => record.node === surface), undefined);
  assert.deepEqual(mod.log.changes, [true, false], "dismiss reports the close request");
  assert.equal(h.byTestId("surface"), surface, "parent may retain the surface");
  send("surface", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  assert.equal(clock.timers.size, 1);
  mod.setOpen(false);
  h.flush();
  clock.advance(1000);
  flush();
  assert.equal(h.byTestId("surface"), undefined, "parent close cancels the pending opening job");
  assert.deepEqual(mod.log.changes, [true, false], "no resurrected requests");
  mod.setOpen(true);
  flush();
  const reopened = h.byTestId("surface");
  assert.ok(reopened);
  end(reopened);
  assert.equal(mod.log.ends, 1, "caller animation-end is forwarded");
  finish();
});

test("tooltip arrow rejects children at runtime", async t => {
  const { h, mod, finish } = await mounted(t);
  assert.throws(() => mod.badArrow(h.makeNode("root")), /TooltipArrow does not accept children/);
  finish();
});

test("removed owner cancels deferred tooltip construction entirely", async t => {
  const { h, mod, traps, layers, portals, geometry, finish } = await mounted(t, "mountCancelled");
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(mod.log.mounts, 0, "counter is never constructed");
  assert.equal(portals[0]?.node.children.length ?? 0, 0, "aborted portal placeholder builds nothing");
  assert.equal(traps.length, 0);
  assert.equal(layers.length, 0);
  assert.equal(geometry.length, 0);
  finish();
});

test("hover surfaces nest inside a dialog on the modal band without traps or foreign context", async t => {
  const { h, mod, traps, locks, portals, finish } = await mounted(t, "mountNested", true);
  const dialog = h.byTestId("dialog");
  const tip = h.byTestId("tip");
  const card = h.byTestId("card");
  assert.ok(dialog && tip && card, "dialog and both default-open hover surfaces mount");
  assert.equal(h.byTestId("stray"), undefined, "orphaned TooltipContent must not borrow the dialog state");
  assert.equal(portals.length, 3);
  assert.ok(portals.every(portal => portal.layer === "modal"), "nested surfaces inherit the modal band");
  assert.equal(traps.length, 1, "only the dialog traps focus");
  assert.equal(locks.length, 1, "only the dialog locks scroll");
  const tipTrigger = h.byTestId("tip-trigger");
  const cardTrigger = h.byTestId("card-trigger");
  assert.equal(tipTrigger.props.get("ariaDescribedBy"), tip);
  assert.equal(tip.props.get("ariaPopupFor"), tipTrigger);
  assert.equal(cardTrigger.props.get("ariaControls"), card);
  assert.equal(cardTrigger.props.get("expanded"), true);
  assert.equal(card.props.get("ariaPopupFor"), cardTrigger);
  assert.equal(tip.props.get("role"), "tooltip");
  assert.equal(card.props.get("role"), "dialog");
  assert.equal(card.props.get("ariaModal"), false);
  assert.equal(mod.log.mounts, 2);
  finish();
});

test("tooltip ignores character Escape and handles named Escape", async t => {
  const { h, clock, send, flush, finish } = await mounted(t, "mountDefault");
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(700);
  const surface = h.byTestId("surface");
  assert.ok(surface);
  assert.equal(surface.props.get("data-state"), "open");
  send("trigger", "onKeyDown", { key: "Escape", character: "Escape" });
  assert.equal(h.byTestId("surface"), surface);
  assert.equal(surface.props.get("data-state"), "open", "character text never closes");
  send("trigger", "onKeyDown", { key: "Escape", character: null });
  assert.equal(surface.props.get("data-state"), "closed");
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  finish();
  assert.equal(clock.timers.size, 0);
});

test("hover card ignores character Escape and handles named Escape", async t => {
  const { h, clock, send, flush, finish } = await mounted(t, "mountCardDefault");
  send("trigger", "onPointerEnter", { position: { x: 0, y: 0 }, pointerKind: "mouse" });
  clock.advance(700);
  const surface = h.byTestId("surface");
  assert.ok(surface);
  assert.equal(surface.props.get("data-state"), "open");
  send("trigger", "onKeyDown", { key: "Escape", character: "Escape" });
  assert.equal(h.byTestId("surface"), surface);
  assert.equal(surface.props.get("data-state"), "open", "character text never closes");
  send("trigger", "onKeyDown", { key: "Escape", character: null });
  assert.equal(surface.props.get("data-state"), "closed");
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  finish();
  assert.equal(clock.timers.size, 0);
});
