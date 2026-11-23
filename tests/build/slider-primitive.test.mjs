import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Show, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { SliderPrimitive } from "@hewel/zgui/primitives";

export const log = {
  values: [] as number[],
  dragging: [] as boolean[],
  renderedValues: [] as number[],
  renderedFractions: [] as number[],
};

const [controlled, setControlled] = createSignal<number | undefined>(25);
const [echo, setEcho] = createSignal(false);
const [disabled, setDisabled] = createSignal(false);
const [cancelChange, setCancelChange] = createSignal(false);
const [shown, setShown] = createSignal(true);
const [stepProp, setStepProp] = createSignal(5);
const [pointerStepProp, setPointerStepProp] = createSignal<number | undefined>(1);

export const controls = {
  setControlled,
  setEcho,
  setDisabled,
  setCancelChange,
  setShown,
  setStepProp,
  setPointerStepProp,
};

let rootNode: NativeNode | undefined;
let trackNode: NativeNode | undefined;
let childInputNode: NativeNode | undefined;
let draggingAccessor: (() => boolean) | undefined;
let valueAccessor: (() => number) | undefined;
let fractionAccessor: (() => number) | undefined;

export const state = {
  root: () => rootNode,
  track: () => trackNode,
  childInput: () => childInputNode,
  dragging: () => draggingAccessor?.(),
  value: () => valueAccessor?.(),
  fraction: () => fractionAccessor?.(),
};

export function mountHeadless(root: NativeNode): () => void {
  return renderApp(() => (
    <SliderPrimitive
      testId="slider"
      ref={node => { rootNode = node; }}
      class="m3-slider"
      label="Brightness"
      min={0}
      max={100}
      step={stepProp()}
      pointerStep={pointerStepProp()}
      defaultValue={20}
      disabled={disabled()}
      onValueChange={v => log.values.push(v)}
      onDraggingChange={d => log.dragging.push(d)}
      onChange={e => { if (cancelChange()) e.preventDefault(); }}
    >
      {({ value, fraction, dragging, trackRef }) => {
        valueAccessor = value;
        fractionAccessor = fraction;
        draggingAccessor = dragging;
        return (
          <row testId="custom-layout">
            <box testId="leading-icon" />
            <box
              testId="custom-track"
              ref={node => {
                trackNode = node;
                trackRef(node);
              }}
            >
              <box
                testId="custom-fill"
                style={{ width: \`\${(fraction() * 100).toFixed(2)}%\` }}
              />
              <text testId="readout">{\`\${value()}-\${dragging() ? "drag" : "idle"}\`}</text>
            </box>
            <input testId="child-input" ref={node => { childInputNode = node; }} />
          </row>
        );
      }}
    </SliderPrimitive>
  ), root);
}

export function mountStaticFallback(root: NativeNode): () => void {
  return renderApp(() => (
    <SliderPrimitive
      testId="slider"
      ref={node => { rootNode = node; }}
      min={0}
      max={100}
      step={1}
      defaultValue={10}
      onValueChange={v => log.values.push(v)}
    >
      <box testId="static-child" />
    </SliderPrimitive>
  ), root);
}

export function mountControlled(root: NativeNode): () => void {
  return renderApp(() => (
    <SliderPrimitive
      testId="slider"
      min={0}
      max={100}
      step={5}
      value={controlled()}
      disabled={disabled()}
      onValueChange={v => {
        log.values.push(v);
        if (echo()) setControlled(v);
      }}
      onChange={e => { if (cancelChange()) e.preventDefault(); }}
    >
      {({ value, fraction, dragging, trackRef }) => {
        valueAccessor = value;
        fractionAccessor = fraction;
        draggingAccessor = dragging;
        return <box testId="custom-track" ref={node => { trackNode = node; trackRef(node); }} />;
      }}
    </SliderPrimitive>
  ), root);
}

export function mountInvalid(root: NativeNode, props: object): () => void {
  return renderApp(() => SliderPrimitive(props as never), root);
}

export function mountShown(root: NativeNode): () => void {
  return renderApp(() => (
    <Show when={shown()}>
      <SliderPrimitive
        testId="slider"
        onDraggingChange={d => log.dragging.push(d)}
        onValueChange={v => log.values.push(v)}
      />
    </Show>
  ), root);
}
`;

function sendPointer(harness, kind, target, x, options = {}) {
  const state = { captured: 0, released: 0 };
  const event = harness.dispatch(target, kind, {
    position: { x, y: 0 },
    capturePointer: () => {
      state.captured += 1;
    },
    releasePointer: () => {
      state.released += 1;
    },
    ...options,
  });
  return { event, ...state };
}

function sendKey(harness, target, name, extras = {}) {
  return harness.dispatch(target, "KeyDown", { key: name, keyWithoutModifiers: name, ...extras });
}

function sendChange(harness, target, value, extras = {}) {
  return harness.dispatch(target, "Change", { value, valueChange: "committed", ...extras });
}

async function mounted(t, mount, ...args) {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod[mount](root, ...args);
  t.after(() => {
    const previous = globalThis.__zgui;
    globalThis.__zgui = harness.host;
    try {
      dispose();
    } finally {
      if (previous && previous !== harness.host) globalThis.__zgui = previous;
      else delete globalThis.__zgui;
    }
  });
  return { mod, harness };
}

test("SliderPrimitive installs zero stylesheets, renders arbitrary children, and falls back to root when trackRef is unbound", async t => {
  const { mod, harness } = await mounted(t, "mountHeadless");
  const root = harness.byTestId("slider");
  assert.equal(
    harness.stylesheets.size,
    0,
    "SliderPrimitive must install zero built-in stylesheets",
  );
  assert.equal(root.tag, "control");
  assert.equal(root.props.get("role"), "slider");
  assert.equal(root.props.get("tabIndex"), 0);
  assert.equal(root.props.get("ariaLabel"), "Brightness");
  assert.equal(root.props.get("class"), "m3-slider");
  assert.ok(harness.byTestId("leading-icon"), "renders arbitrary custom siblings");
  assert.ok(harness.byTestId("custom-fill"), "renders render-prop descendants");
  assert.equal(mod.state.value(), 20);
  assert.equal(mod.state.fraction(), 0.2);
  assert.equal(mod.state.dragging(), false);

  // Static children without trackRef fall back to measuring the root <control> rect.
  const fallback = await mounted(t, "mountStaticFallback");
  assert.equal(fallback.harness.stylesheets.size, 0);
  const fallbackRoot = fallback.harness.byTestId("slider");
  assert.ok(fallback.harness.byTestId("static-child"));
  fallback.harness.rects.set(fallbackRoot, { x: 100, y: 0, width: 200, height: 20 });
  const down = sendPointer(fallback.harness, "PointerDown", fallbackRoot, 170);
  assert.equal(down.captured, 1);
  assert.equal(fallbackRoot.props.get("ariaValueNow"), 35);
  assert.deepEqual(fallback.mod.log.values, [35]);
  sendPointer(fallback.harness, "PointerUp", fallbackRoot, 170);
});

test("distinct pointerStep preserves fine pointer dragging while keyboard uses step", async t => {
  const { mod, harness } = await mounted(t, "mountHeadless");
  const root = harness.byTestId("slider");
  const track = harness.byTestId("custom-track");
  harness.rects.set(track, { x: 100, y: 0, width: 200, height: 12 });

  // Pointer snaps to pointerStep = 1 (37% of [100..300] is x=174 -> 37, NOT snapped to step=5).
  sendPointer(harness, "PointerDown", root, 174);
  assert.equal(mod.state.value(), 37);
  assert.equal(root.props.get("ariaValueNow"), 37);
  assert.equal(root.props.get("ariaValueStep"), 5);
  sendPointer(harness, "PointerMove", root, 175.6); // 37.8% -> 38
  assert.equal(mod.state.value(), 38);
  sendPointer(harness, "PointerUp", root, 175.6);

  // Keyboard still steps by step = 5 (and PageUp/PageDown by step * 10 = 50, Home/End exact).
  sendKey(harness, root, "ArrowRight");
  assert.equal(mod.state.value(), 45);
  sendKey(harness, root, "ArrowLeft");
  assert.equal(mod.state.value(), 40);
  sendKey(harness, root, "PageUp");
  assert.equal(mod.state.value(), 90);
  sendKey(harness, root, "PageDown");
  assert.equal(mod.state.value(), 40);
  sendKey(harness, root, "End");
  assert.equal(mod.state.value(), 100);
  sendKey(harness, root, "Home");
  assert.equal(mod.state.value(), 0);

  // pointerStep = 0 allows continuous pointer drags while keyboard still steps by step = 5.
  mod.controls.setPointerStepProp(0);
  harness.flush();
  sendPointer(harness, "PointerDown", root, 174.5); // 37.25%
  assert.ok(Math.abs(mod.state.value() - 37.25) < 1e-9);
  sendPointer(harness, "PointerUp", root, 174.5);
  sendKey(harness, root, "ArrowRight");
  assert.equal(mod.state.value(), 40);

  // Composing key events never drive the slider.
  const composing = sendKey(harness, root, "ArrowRight", { isComposing: true });
  assert.equal(mod.state.value(), 40);
  assert.equal(composing.defaultPrevented, false);
});

test("pointer capture tracks drags outside the track, blocks second pointer takeover, and cleans up on disable/cancel/unmount", async t => {
  const { mod, harness } = await mounted(t, "mountHeadless");
  const root = harness.byTestId("slider");
  const track = harness.byTestId("custom-track");
  harness.rects.set(track, { x: 50, y: 0, width: 100, height: 12 });

  const start = sendPointer(harness, "PointerDown", root, 80, { pointerId: "1" });
  assert.equal(start.captured, 1);
  assert.equal(mod.state.dragging(), true);
  assert.equal(root.props.get("data-dragging"), "true");
  assert.deepEqual(mod.log.dragging, [true]);

  // Drag outside the track bounds (beyond right edge and past left edge), then release outside.
  sendPointer(harness, "PointerMove", root, 400, { pointerId: "1" });
  assert.equal(mod.state.value(), 100);
  sendPointer(harness, "PointerMove", root, -120, { pointerId: "1" });
  assert.equal(mod.state.value(), 0);

  // Second pointer cannot steal active drag or end capture.
  const hijack = sendPointer(harness, "PointerDown", root, 100, { pointerId: "2" });
  assert.equal(hijack.captured, 0);
  sendPointer(harness, "PointerMove", root, 100, { pointerId: "2" });
  assert.equal(mod.state.value(), 0);
  const hijackUp = sendPointer(harness, "PointerUp", root, 100, { pointerId: "2" });
  assert.equal(hijackUp.released, 0);
  assert.equal(mod.state.dragging(), true);

  // Release matching pointer outside track ends capture and notifies onDraggingChange(false).
  const finish = sendPointer(harness, "PointerUp", root, 120, { pointerId: "1" });
  assert.equal(finish.released, 1);
  assert.equal(mod.state.value(), 70);
  assert.equal(mod.state.dragging(), false);
  assert.equal(root.props.get("data-dragging"), undefined);
  assert.deepEqual(mod.log.dragging, [true, false]);

  // Disabling mid-drag immediately ends dragging and releases capture on next event.
  sendPointer(harness, "PointerDown", root, 90, { pointerId: "1" });
  assert.equal(mod.state.dragging(), true);
  mod.controls.setDisabled(true);
  harness.flush();
  assert.equal(mod.state.dragging(), false);
  assert.equal(root.props.get("data-disabled"), "true");
  assert.deepEqual(mod.log.dragging, [true, false, true, false]);
  const cleanupMove = sendPointer(harness, "PointerMove", root, 140, { pointerId: "1" });
  assert.equal(cleanupMove.released, 1);
  assert.equal(mod.state.value(), 40);

  // PointerCancel mid-drag ends dragging and releases capture.
  mod.controls.setDisabled(false);
  harness.flush();
  sendPointer(harness, "PointerDown", root, 100, { pointerId: "1" });
  const cancelled = sendPointer(harness, "PointerCancel", root, 100, { pointerId: "1" });
  assert.equal(cancelled.released, 1);
  assert.equal(mod.state.dragging(), false);
  assert.deepEqual(mod.log.dragging, [true, false, true, false, true, false]);

  // Unmounting mid-drag notifies onDraggingChange(false).
  const unmountCase = await mounted(t, "mountShown");
  const shownRoot = unmountCase.harness.byTestId("slider");
  unmountCase.harness.rects.set(shownRoot, { x: 0, y: 0, width: 100, height: 10 });
  sendPointer(unmountCase.harness, "PointerDown", shownRoot, 40);
  assert.deepEqual(unmountCase.mod.log.dragging, [true]);
  unmountCase.mod.controls.setShown(false);
  unmountCase.harness.flush();
  assert.deepEqual(unmountCase.mod.log.dragging, [true, false]);

  // Same-turn PointerDown + PointerUp before flush never strands dragging or blocks a later pointer.
  const sameTurn = await mounted(t, "mountHeadless");
  const stRoot = sameTurn.harness.byTestId("slider");
  const stTrack = sameTurn.harness.byTestId("custom-track");
  sameTurn.harness.rects.set(stTrack, { x: 0, y: 0, width: 100, height: 10 });
  sendPointer(sameTurn.harness, "PointerDown", stRoot, 50, { pointerId: "1" });
  sendPointer(sameTurn.harness, "PointerUp", stRoot, 55, { pointerId: "1" });
  sameTurn.harness.flush();
  assert.equal(sameTurn.mod.state.dragging(), false);
  assert.equal(sameTurn.mod.state.value(), 55);
  assert.deepEqual(sameTurn.mod.log.dragging, [true, false]);
  const nextPen = sendPointer(sameTurn.harness, "PointerDown", stRoot, 30, {
    pointerId: "7",
    pointerKind: "pen",
  });
  assert.equal(nextPen.captured, 1, "subsequent pen press after same-turn click must work");
  assert.equal(sameTurn.mod.state.value(), 30);
});

test("AccessKit Change events normalize and update both uncontrolled and controlled sliders", async t => {
  // Uncontrolled with step=5, pointerStep=1
  const { mod, harness } = await mounted(t, "mountHeadless");
  const root = harness.byTestId("slider");
  sendChange(harness, root, "63");
  assert.equal(mod.state.value(), 63);
  assert.equal(root.props.get("ariaValueNow"), 63);
  sendChange(harness, root, "250");
  assert.equal(mod.state.value(), 100, "AT Change clamps to max");
  sendChange(harness, root, "-15");
  assert.equal(mod.state.value(), 0, "AT Change clamps to min");

  // Non-finite, empty, bubbled descendant, disabled, and caller-prevented Change events are ignored.
  const countBefore = mod.log.values.length;
  sendChange(harness, root, "");
  sendChange(harness, root, "   ");
  sendChange(harness, root, "NaN");
  sendChange(harness, root, "Infinity");
  sendChange(harness, root, "not-a-number");
  sendChange(harness, mod.state.childInput(), "50");
  assert.equal(mod.log.values.length, countBefore, "invalid or bubbled Change events do nothing");

  mod.controls.setCancelChange(true);
  harness.flush();
  sendChange(harness, root, "40");
  assert.equal(mod.log.values.length, countBefore, "caller-prevented Change does nothing");
  mod.controls.setCancelChange(false);

  mod.controls.setDisabled(true);
  harness.flush();
  sendChange(harness, root, "40");
  assert.equal(mod.log.values.length, countBefore, "disabled slider ignores AT Change");

  // Controlled slider: AT Change reports normalized value and waits for parent echo.
  const controlled = await mounted(t, "mountControlled");
  const cRoot = controlled.harness.byTestId("slider");
  assert.equal(cRoot.props.get("ariaValueNow"), 25);
  sendChange(controlled.harness, cRoot, "43"); // step=5, pointerStep defaults to 5 -> snaps to 45
  assert.deepEqual(controlled.mod.log.values, [45]);
  assert.equal(cRoot.props.get("ariaValueNow"), 25, "controlled display waits for echo");

  controlled.mod.controls.setEcho(true);
  controlled.harness.flush();
  sendChange(controlled.harness, cRoot, "68"); // snaps to 70
  assert.deepEqual(controlled.mod.log.values, [45, 70]);
  assert.equal(
    cRoot.props.get("ariaValueNow"),
    70,
    "echoed AT Change updates display and ariaValueNow",
  );
});

test("invalid pointerStep fails with the same finite non-negative validation as step", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  t.after(() => delete globalThis.__zgui);
  const root = harness.makeNode("root");
  for (const [props, pattern] of [
    [{ pointerStep: Number.NaN }, /finite/],
    [{ pointerStep: Number.POSITIVE_INFINITY }, /finite/],
    [{ pointerStep: -0.5 }, /non-negative/],
    [{ min: 0, max: 1e308, pointerStep: Number.MIN_VALUE }, /representable step/],
  ]) {
    assert.throws(() => mod.mountInvalid(root, props), pattern, JSON.stringify(props));
    harness.host.clearRoot(root);
  }
});
