import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { renderApp } from "@hewel/zgui";
import type { NativeNode, NativeWheelEvent } from "@hewel/zgui";

export const seen: string[] = [];
export let saved: NativeWheelEvent;

export function mount(root: NativeNode) {
  return renderApp(() => (
    <scroll onWheelCapture={event => {
      saved = event;
      seen.push(event.phase + ":" + event.scrollPhase + ":" + event.delta.unit + ":" + event.delta.y);
      event.preventDefault();
    }} onWheel={event => seen.push(event.phase)}>
      <box testId="target" onWheelCapture={event => {
        seen.push(event.phase + ":capture");
        if (event.delta.x === 1) event.stopPropagation();
        if (event.delta.x === 2) event.stopImmediatePropagation();
      }} onWheel={event => seen.push(event.phase + ":bubble")} />
      <box testId="bubble-first"
        onWheel={event => seen.push(event.phase + ":bubble")}
        onWheelCapture={event => seen.push(event.phase + ":capture")} />
    </scroll>
  ), root);
}
`;

test("wheel handlers receive snapshots and cancel only the current dispatch", async t => {
  const mod = await moduleFor(t, fixture);
  const h = createHost();
  const root = h.makeNode("root");
  let dispose;
  globalThis.__zgui = h.host;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  dispose = mod.mount(root);
  const target = h.byTestId("target");
  assert.equal(h.dispatch(target, "Wheel").defaultPrevented, true);
  const first = mod.saved;
  assert.deepEqual(mod.seen, [
    "capture:discrete:lines:0",
    "target:capture",
    "target:bubble",
    "bubble",
  ]);
  mod.seen.length = 0;
  h.dispatch(h.byTestId("bubble-first"), "Wheel");
  assert.deepEqual(mod.seen, [
    "capture:discrete:lines:0",
    "target:bubble",
    "target:capture",
    "bubble",
  ]);
  for (const [x, expected, immediate] of [
    [1, ["capture:moved:pixels:15", "target:capture", "target:bubble"], false],
    [2, ["capture:moved:pixels:15", "target:capture"], true],
  ]) {
    mod.seen.length = 0;
    const result = h.dispatch(target, "Wheel", {
      delta: { x, y: 15, unit: "pixels" },
      scrollPhase: "moved",
    });
    assert.equal(result.propagationStopped, true);
    assert.equal(result.immediatePropagationStopped, immediate);
    assert.deepEqual(mod.seen, expected);
  }
  assert.equal(first.phase, "capture");
  assert.equal(first.currentTarget, root.children[0]);
  assert.throws(() => first.preventDefault(), /expired-dispatch/);
});
