import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Show, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Slider } from "@hewel/zgui/components";

export const log = { values: [] as number[] };

const [controlled, setControlled] = createSignal<number | undefined>(5);
const [echo, setEcho] = createSignal(false);
const [disabled, setDisabled] = createSignal(false);
const [cancelKeys, setCancelKeys] = createSignal(false);
const [cancelPointer, setCancelPointer] = createSignal<readonly string[]>([]);
const [minProp, setMinProp] = createSignal(0);
const [maxProp, setMaxProp] = createSignal(100);
const [stepProp, setStepProp] = createSignal(1);
const [defaultProp, setDefaultProp] = createSignal(10);
const [shown, setShown] = createSignal(true);

export const controls = {
  setControlled, setEcho, setDisabled, setCancelKeys, setCancelPointer,
  setMinProp, setMaxProp, setStepProp, setDefaultProp, setShown,
};

let sliderNode: NativeNode | undefined;
export const nodes = { slider: () => sliderNode };

export function mount(root: NativeNode): () => void {
  return renderApp(() => (
    <Slider testId="slider" ref={node => { sliderNode = node; }}
      class="extra" label="Volume"
      min={minProp()} max={maxProp()} step={stepProp()} defaultValue={defaultProp()}
      disabled={disabled()}
      onValueChange={value => log.values.push(value)}
      onKeyDown={event => { if (cancelKeys()) event.preventDefault(); }}
      onPointerDown={event => { if (cancelPointer().includes("down")) event.preventDefault(); }}
      onPointerMove={event => { if (cancelPointer().includes("move")) event.preventDefault(); }}
      onPointerUp={event => { if (cancelPointer().includes("up")) event.preventDefault(); }}
      onPointerCancel={event => { if (cancelPointer().includes("cancel")) event.preventDefault(); }} />
  ), root);
}

export function mountControlled(root: NativeNode): () => void {
  return renderApp(() => (
    <Slider testId="slider" value={controlled()}
      onValueChange={value => { log.values.push(value); if (echo()) setControlled(value); }} />
  ), root);
}

export function mountRange(root: NativeNode, min: number, max: number, step: number, initial?: number): () => void {
  return renderApp(() => (
    <Slider testId="slider" min={min} max={max} step={step}
      defaultValue={initial ?? min}
      onValueChange={value => log.values.push(value)}
      onKeyDown={event => { if (cancelKeys()) event.preventDefault(); }} />
  ), root);
}

export function mountInvalid(root: NativeNode, props: object): () => void {
  return renderApp(() => Slider(props as never), root);
}

export function mountOrientation(root: NativeNode, orientation: string): () => void {
  return renderApp(() => <Slider orientation={orientation as never} />, root);
}

export function mountShown(root: NativeNode): () => void {
  return renderApp(() => (
    <Show when={shown()}>
      <Slider testId="slider" />
    </Show>
  ), root);
}
`;

const counters = () => ({ captured: 0, released: 0, activated: 0, prevented: 0, stopped: 0 });

const pointer = (type, target, options = {}) => ({
  type, target, currentTarget: target,
  phase: "target", timestamp: 0,
  modifiers: { shift: false, control: false, alt: false, meta: false },
  position: { x: 0, y: 0 },
  pointerId: "0", pointerKind: "mouse", primary: true,
  button: "primary", pressure: null, coalesced: [],
  preventDefault() { this.prevented = (this.prevented ?? 0) + 1; },
  stopPropagation() { this.stopped = (this.stopped ?? 0) + 1; },
  stopImmediatePropagation() {},
  activateNow() { this.activated = (this.activated ?? 0) + 1; },
  capturePointer() { this.captured = (this.captured ?? 0) + 1; },
  releasePointer() { this.released = (this.released ?? 0) + 1; },
  ...options,
});

const down = (target, x, options) => pointer("pointerdown", target, { position: { x, y: 0 }, ...options });
const move = (target, x, options) => pointer("pointermove", target, { position: { x, y: 0 }, ...options });
const up = (target, x, options) => pointer("pointerup", target, { position: { x, y: 0 }, ...options });
const cancel = (target, options) => pointer("pointercancel", target, options);

const key = (target, name, currentTarget = target) => ({
  type: "keydown", key: name, keyWithoutModifiers: name, target, currentTarget,
  prevented: 0, stopped: 0,
  modifiers: { shift: false, control: false, alt: false, meta: false },
  preventDefault() { this.prevented = (this.prevented ?? 0) + 1; },
  stopPropagation() { this.stopped = (this.stopped ?? 0) + 1; },
  stopImmediatePropagation() {},
});

function cleanup(t, harness, dispose) {
  t.after(() => {
    const previous = globalThis.__zgui;
    globalThis.__zgui = harness.host;
    try { dispose(); }
    finally {
      if (previous && previous !== harness.host) globalThis.__zgui = previous;
      else delete globalThis.__zgui;
    }
  });
}

async function mounted(t, mount, ...args) {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod[mount](root, ...args);
  cleanup(t, harness, dispose);
  return { mod, harness };
}

const slider = harness => harness.byTestId("slider");
const attached = (harness, node) => {
  let current = node;
  while (current) {
    if (current.tag === "root") return true;
    current = harness.host.getParentNode(current);
  }
  return false;
};
const part = (harness, className) =>
  [...harness.nodes.values()].find(node =>
    attached(harness, node) && (node.props.get("class") ?? "").includes(className));
const track = harness => part(harness, "zui-slider__track");
const prop = (harness, name) => slider(harness)?.props.get(name);
const ariaValueNow = harness => prop(harness, "ariaValueNow");

function press(harness, name, target = slider(harness)) {
  const event = key(target, name);
  slider(harness).callbacks.onKeyDown(event);
  return event;
}

test("stepped keyboard navigation snaps and clamps at endpoints", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 3, 23, 5, 9);
  assert.equal(ariaValueNow(harness), 8, "defaultValue snaps to the nearest step");
  assert.equal(prop(harness, "--zui-slider-fraction"), "25.0000%");
  assert.equal(prop(harness, "ariaValueMin"), 3);
  assert.equal(prop(harness, "ariaValueMax"), 23);
  assert.equal(prop(harness, "ariaValueStep"), 5);

  press(harness, "ArrowRight");
  assert.equal(ariaValueNow(harness), 13);
  press(harness, "ArrowDown");
  assert.equal(ariaValueNow(harness), 8);
  press(harness, "PageUp");
  assert.equal(ariaValueNow(harness), 23, "big page steps saturate at max instead of throwing");
  press(harness, "PageDown");
  assert.equal(ariaValueNow(harness), 3);
  press(harness, "End");
  assert.equal(ariaValueNow(harness), 23, "End reaches the endpoint despite step 5");
  press(harness, "Home");
  assert.equal(ariaValueNow(harness), 3);
  assert.deepEqual(mod.log.values, [13, 8, 23, 3, 23, 3]);

  for (const name of ["Tab", "Escape", "a", " "]) {
    const event = press(harness, name);
    assert.equal(event.prevented, 0, `${name} is untouched`);
  }
  assert.equal(mod.log.values.length, 6);

  mod.controls.setCancelKeys(true);
  press(harness, "ArrowRight");
  assert.equal(ariaValueNow(harness), 3, "a caller-prevented key does not change");
  assert.equal(mod.log.values.length, 6);
});

test("character text never drives named slider keys", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 0, 100, 1, 50);
  const root = slider(harness);
  for (const character of ["Home", "ArrowRight", "PageDown"]) {
    const event = key(root, character);
    event.character = character;
    root.callbacks.onKeyDown(event);
    assert.equal(ariaValueNow(harness), 50, `character "${character}" never moves the thumb`);
    assert.equal(event.prevented, 0, `character "${character}" is not consumed`);
    assert.equal(event.stopped, 0);
  }
  assert.deepEqual(mod.log.values, []);
  const named = key(root, "Home");
  named.character = null;
  root.callbacks.onKeyDown(named);
  assert.equal(ariaValueNow(harness), 0);
  assert.deepEqual(mod.log.values, [0]);
  assert.equal(named.prevented, 1, "named Home consumes the event");
  assert.equal(named.stopped, 1);
});

test("step zero is continuous for pointer but steps by one for keys", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 0, 1, 0, 0);
  const root = slider(harness);
  harness.rects.set(track(harness), { x: 0, y: 0, width: 100, height: 10 });
  const start = down(root, 37.1);
  root.callbacks.onPointerDown(start);
  assert.equal(start.captured, 1);
  assert.ok(Math.abs(ariaValueNow(harness) - 0.371) < 1e-9);
  root.callbacks.onPointerUp(up(root, 37.1));
  press(harness, "ArrowRight");
  assert.equal(ariaValueNow(harness), 1, "keyboard still increments by one and clamps");
  assert.equal(mod.log.values.at(-1), 1);
});

test("endpoints stay exact when step does not divide the interval", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 0, 10, 3, 0);
  const root = slider(harness);
  harness.rects.set(track(harness), { x: 0, y: 0, width: 100, height: 10 });
  press(harness, "End");
  assert.equal(ariaValueNow(harness), 10);
  root.callbacks.onPointerDown(down(root, 50));
  assert.equal(ariaValueNow(harness), 6, "interior point snaps to the nearest step");
  root.callbacks.onPointerMove(move(root, 400));
  assert.equal(ariaValueNow(harness), 10, "far-right point reaches exact max");
  root.callbacks.onPointerUp(up(root, 400));
  press(harness, "Home");
  assert.equal(ariaValueNow(harness), 0);
  assert.deepEqual(mod.log.values, [10, 6, 10, 0]);
});

test("degenerate and fractional ranges behave", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 7, 7, 1, 7);
  const root = slider(harness);
  harness.rects.set(track(harness), { x: 0, y: 0, width: 100, height: 10 });
  assert.equal(ariaValueNow(harness), 7);
  assert.equal(prop(harness, "--zui-slider-fraction"), "0.0000%");
  press(harness, "ArrowRight");
  press(harness, "Home");
  root.callbacks.onPointerDown(down(root, 50));
  assert.equal(mod.log.values.length, 0, "min===max emits nothing");

  const second = createHost();
  globalThis.__zgui = second.host;
  const mod2 = await moduleFor(t, fixture);
  const root2 = second.makeNode("root");
  const dispose2 = mod2.mountRange(root2, -0.5, 0.5, 0.25, 0);
  cleanup(t, second, dispose2);
  assert.equal(ariaValueNow(second), 0);
  press(second, "ArrowLeft");
  assert.equal(ariaValueNow(second), -0.25);
});

test("invalid bounds and values fail clearly", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  t.after(() => delete globalThis.__zgui);
  const root = harness.makeNode("root");
  for (const [props, pattern] of [
    [{ min: Number.NaN }, /finite/],
    [{ max: Number.POSITIVE_INFINITY }, /finite/],
    [{ step: Number.NaN }, /finite/],
    [{ min: 10, max: 5 }, /ordered finite bounds/],
    [{ step: -1 }, /non-negative/],
    [{ defaultValue: Number.NaN }, /finite/],
    [{ value: Number.POSITIVE_INFINITY }, /finite/],
    [{ min: 0, max: 1e308, step: Number.MIN_VALUE }, /representable step/],
  ]) {
    assert.throws(() => mod.mountInvalid(root, props), pattern, JSON.stringify(props));
    harness.host.clearRoot(root);
  }
  assert.throws(() => mod.mountOrientation(root, "vertical"), TypeError);
});

test("pointer gestures map positions through the track rect, not the root", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 0, 100, 1, 0);
  const root = slider(harness);
  const thumb = part(harness, "zui-slider__thumb");
  harness.rects.set(root, { x: 10, y: 0, width: 1000, height: 40 });
  harness.rects.set(track(harness), { x: 300, y: 0, width: 200, height: 10 });

  const start = down(thumb, 350, { currentTarget: root, phase: "bubble" });
  root.callbacks.onPointerDown(start);
  assert.equal(ariaValueNow(harness), 25, "geometry resolves against the track rect");
  assert.equal(start.captured, 1);
  assert.equal(harness.focused.current, root, "the root takes focus for the drag");
  assert.equal(harness.focused.reveal, false);

  root.callbacks.onPointerMove(move(root, 800));
  assert.equal(ariaValueNow(harness), 100);
  root.callbacks.onPointerMove(move(root, -50));
  assert.equal(ariaValueNow(harness), 0);
  assert.equal(start.captured, 1, "capture is issued once for the gesture");

  const finish = up(root, 450);
  root.callbacks.onPointerUp(finish);
  assert.equal(ariaValueNow(harness), 75, "release uses the final pointer position");
  assert.equal(finish.released, 1);

  root.callbacks.onPointerMove(move(root, 400));
  assert.equal(ariaValueNow(harness), 75, "moves after release are ignored");
});

test("missing or degenerate geometry aborts capture but never strands a drag", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 0, 100, 1, 0);
  const root = slider(harness);
  const trackNode = track(harness);

  const zeroWidth = down(root, 10);
  harness.rects.set(trackNode, { x: 0, y: 0, width: 0, height: 10 });
  root.callbacks.onPointerDown(zeroWidth);
  assert.equal(zeroWidth.captured, undefined, "zero-width track cannot start a capture");
  harness.rects.delete(trackNode);
  const noRect = down(root, 10);
  root.callbacks.onPointerDown(noRect);
  assert.equal(noRect.captured, undefined, "a missing rect cannot start a capture");

  harness.rects.set(trackNode, { x: 0, y: 0, width: 100, height: 10 });
  const start = down(root, 25);
  root.callbacks.onPointerDown(start);
  assert.equal(ariaValueNow(harness), 25);
  harness.rects.delete(trackNode);
  root.callbacks.onPointerMove(move(root, 80));
  assert.equal(ariaValueNow(harness), 25, "a missing rect during drag skips the update");
  const finish = up(root, 80);
  root.callbacks.onPointerUp(finish);
  assert.equal(finish.released, 1, "release still ends the drag");
});

test("pointer identity keeps concurrent pointers out of the active drag", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 0, 100, 1, 0);
  const root = slider(harness);
  harness.rects.set(track(harness), { x: 0, y: 0, width: 100, height: 10 });
  const pen = { pointerId: "7", pointerKind: "pen", button: null };

  const start = down(root, 10, pen);
  root.callbacks.onPointerDown(start);
  assert.equal(start.captured, 1);
  assert.equal(ariaValueNow(harness), 10);

  const foreign = { pointerId: "9", pointerKind: "pen", button: null };
  root.callbacks.onPointerMove(move(root, 80, foreign));
  assert.equal(ariaValueNow(harness), 10, "another pointer's move does nothing");
  const foreignUp = up(root, 80, foreign);
  root.callbacks.onPointerUp(foreignUp);
  assert.equal(foreignUp.released, undefined, "another pointer's up releases nothing");
  root.callbacks.onPointerCancel(cancel(root, foreign));
  assert.equal(ariaValueNow(harness), 10);

  const takeover = down(root, 60, foreign);
  root.callbacks.onPointerDown(takeover);
  assert.equal(takeover.captured, undefined, "a second primary down cannot take over");
  assert.equal(ariaValueNow(harness), 10);

  root.callbacks.onPointerMove(move(root, 40, pen));
  assert.equal(ariaValueNow(harness), 40, "the matching pointer updates");
  const stop = cancel(root, pen);
  root.callbacks.onPointerCancel(stop);
  assert.equal(stop.released, 1);
  root.callbacks.onPointerMove(move(root, 90, pen));
  assert.equal(ariaValueNow(harness), 40, "cancel ends the gesture");

  const touch = down(root, 30, { pointerId: "11", pointerKind: "touch", button: null, primary: true });
  root.callbacks.onPointerDown(touch);
  assert.equal(touch.captured, 1, "touch with a null button works");
  root.callbacks.onPointerUp(up(root, 30, { pointerId: "11", pointerKind: "touch", button: null }));

  const notPrimary = down(root, 30, { primary: false });
  root.callbacks.onPointerDown(notPrimary);
  assert.equal(notPrimary.captured, undefined);
  const secondary = down(root, 30, { button: "secondary" });
  root.callbacks.onPointerDown(secondary);
  assert.equal(secondary.captured, undefined, "a secondary mouse button does not start");

  root.callbacks.onPointerDown(down(root, 10));
  const secondaryUp = up(root, 10, { button: "secondary" });
  root.callbacks.onPointerUp(secondaryUp);
  assert.equal(secondaryUp.released, undefined, "a secondary release cannot end the primary drag");
  assert.equal(mod.log.values.at(-1), 10);
});

test("a fresh down from the same pointer restarts a lost capture", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 0, 100, 1, 0);
  const root = slider(harness);
  harness.rects.set(track(harness), { x: 0, y: 0, width: 100, height: 10 });
  root.callbacks.onPointerDown(down(root, 10));
  assert.equal(ariaValueNow(harness), 10);

  const restart = down(root, 65);
  root.callbacks.onPointerDown(restart);
  assert.equal(restart.captured, 1, "the same pointer re-arms a lost capture");
  assert.equal(ariaValueNow(harness), 65);

  const other = down(root, 40, { pointerId: "9" });
  root.callbacks.onPointerDown(other);
  assert.equal(other.captured, undefined, "another pointer still cannot take over");
  assert.equal(ariaValueNow(harness), 65);
  assert.equal(mod.log.values.at(-1), 65);
});

test("finite-overflow page keys saturate instead of throwing", async t => {
  const { mod, harness } = await mounted(t, "mountRange", 0, 100, Number.MAX_VALUE, 0);
  press(harness, "PageUp");
  assert.equal(ariaValueNow(harness), 100);
  press(harness, "PageDown");
  assert.equal(ariaValueNow(harness), 0);
  assert.deepEqual(mod.log.values, [100, 0]);
});

test("disabled and caller prevention gate gestures but never strand cleanup", async t => {
  const { mod, harness } = await mounted(t, "mount");
  const root = slider(harness);
  harness.rects.set(track(harness), { x: 0, y: 0, width: 100, height: 10 });
  assert.match(prop(harness, "class"), /\bextra\b/);
  assert.match(prop(harness, "class"), /\bzui-slider\b/);
  assert.equal(prop(harness, "ariaLabel"), "Volume");
  assert.ok(mod.nodes.slider() === root);

  mod.controls.setDisabled(true);
  const blocked = down(root, 50);
  root.callbacks.onPointerDown(blocked);
  assert.equal(blocked.captured, undefined, "disabled cannot start a gesture");
  press(harness, "ArrowRight");
  assert.equal(mod.log.values.length, 0);
  mod.controls.setDisabled(false);

  mod.controls.setCancelPointer(["down"]);
  const prevented = down(root, 50);
  root.callbacks.onPointerDown(prevented);
  assert.equal(prevented.captured, undefined, "caller-prevented down issues no capture");
  assert.equal(ariaValueNow(harness), 10);
  mod.controls.setCancelPointer([]);

  const start = down(root, 20);
  root.callbacks.onPointerDown(start);
  assert.equal(ariaValueNow(harness), 20);

  mod.controls.setDisabled(true);
  const stuck = move(root, 90);
  root.callbacks.onPointerMove(stuck);
  assert.equal(ariaValueNow(harness), 20, "disabling mid-drag stops updates");
  assert.equal(stuck.released, 1, "the next matching event releases the capture");
  mod.controls.setDisabled(false);
  root.callbacks.onPointerMove(move(root, 90));
  assert.equal(ariaValueNow(harness), 20, "re-enabling does not resume a dead drag");

  root.callbacks.onPointerDown(down(root, 20));
  mod.controls.setDisabled(true);
  mod.controls.setDisabled(false);
  const released = move(root, 90);
  root.callbacks.onPointerMove(released);
  assert.equal(released.released, 1, "disable and re-enable still ends the dead drag");
  assert.equal(ariaValueNow(harness), 20);

  const restart = down(root, 20);
  root.callbacks.onPointerDown(restart);
  mod.controls.setCancelPointer(["move"]);
  root.callbacks.onPointerMove(move(root, 60));
  assert.equal(ariaValueNow(harness), 20, "caller-prevented move skips only that move");
  mod.controls.setCancelPointer(["up"]);
  const finish = up(root, 60);
  root.callbacks.onPointerUp(finish);
  assert.equal(finish.released, 1, "caller-prevented up still cleans up");
  assert.equal(ariaValueNow(harness), 20);
  mod.controls.setCancelPointer(["cancel"]);
  root.callbacks.onPointerDown(down(root, 20));
  const aborted = cancel(root);
  root.callbacks.onPointerCancel(aborted);
  assert.equal(aborted.released, 1, "caller-prevented cancel still cleans up");
  mod.controls.setCancelPointer([]);
});

test("controlled value waits for an echo and prop changes never reset owned state", async t => {
  const { mod, harness } = await mounted(t, "mountControlled");
  const root = slider(harness);
  harness.rects.set(track(harness), { x: 0, y: 0, width: 100, height: 10 });
  press(harness, "ArrowRight");
  assert.deepEqual(mod.log.values, [6]);
  assert.equal(ariaValueNow(harness), 5, "controlled display waits for the echo");
  press(harness, "ArrowRight");
  assert.deepEqual(mod.log.values, [6, 6], "a repeated attempt is reported again");
  mod.controls.setEcho(true);
  press(harness, "ArrowRight");
  assert.equal(ariaValueNow(harness), 6, "the echoed attempt becomes the display");
  press(harness, "ArrowRight");
  assert.equal(ariaValueNow(harness), 7);
  mod.controls.setControlled(4);
  assert.equal(ariaValueNow(harness), 4);
});

test("bound prop changes normalize display without emitting a change", async t => {
  const { mod, harness } = await mounted(t, "mount");
  assert.equal(ariaValueNow(harness), 10);
  press(harness, "ArrowRight");
  assert.equal(ariaValueNow(harness), 11);
  const calls = mod.log.values.length;
  mod.controls.setMaxProp(8);
  assert.equal(ariaValueNow(harness), 8, "shrinking max clamps the display");
  assert.equal(mod.log.values.length, calls, "prop normalization emits no callback");
  mod.controls.setStepProp(4);
  mod.controls.setMinProp(0);
  mod.controls.setMaxProp(40);
  assert.equal(ariaValueNow(harness), 12, "the stored value resnaps onto the new step");
  assert.equal(mod.log.values.length, calls);
  mod.controls.setDefaultProp(30);
  assert.equal(ariaValueNow(harness), 12, "defaultValue updates never reset owned state");
});

test("remount produces fresh nodes with no stale gesture state", async t => {
  const { mod, harness } = await mounted(t, "mountShown");
  const first = slider(harness);
  const firstTrack = track(harness);
  harness.rects.set(firstTrack, { x: 0, y: 0, width: 100, height: 10 });
  first.callbacks.onPointerDown(down(first, 30));
  assert.equal(ariaValueNow(harness), 30);
  mod.controls.setShown(false);
  assert.equal(slider(harness), undefined);
  mod.controls.setShown(true);
  const second = slider(harness);
  assert.ok(second);
  assert.notEqual(second, first);
  assert.notEqual(track(harness), firstTrack);
  assert.equal(ariaValueNow(harness), 0, "a remounted slider starts fresh");
});
