import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { isAnimating, renderApp } from "@hewel/zgui";
import type { NativeAnimationEvent, NativeNode, NativeTransitionEvent } from "@hewel/zgui";

const seen = [];
export function taken() { return seen.slice(); }
let box!: NativeNode;
export function busy() { return isAnimating(box); }

export function mount(root: NativeNode) {
  return renderApp(() => {
    const animation = (event: NativeAnimationEvent) =>
      seen.push("a:" + event.type + "|" + event.animationName + "|" + event.elapsedTime + "|" + event.pseudoElement);
    const transition = (event: NativeTransitionEvent) =>
      seen.push("t:" + event.type + "|" + event.propertyName + "|" + event.elapsedTime + "|" + event.pseudoElement);
    return <column><box ref={node => { box = node; }}
      onAnimationEnd={animation} onAnimationCancel={animation}
      onTransitionEnd={transition} onTransitionRun={transition}
      onAnimationStartCapture={animation} onTransitionCancelCapture={transition} /></column>;
  }, root);
}
`;

test("lifecycle handlers forward name, seconds and pseudo; isAnimating reflects the host", async t => {
  const queries = [];
  let running = false;
  const harness = createHost();
  harness.host.isAnimating = node => { queries.push(node); return running; };
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  assert.equal(queries.length, 0, "module import must not query the host");
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  const box = root.children[0].children[0];
  for (const name of ["onAnimationEnd", "onAnimationCancel", "onTransitionEnd", "onTransitionRun",
    "onAnimationStartCapture", "onTransitionCancelCapture"]) {
    assert.equal(typeof box.callbacks[name], "function", name);
  }
  box.callbacks.onAnimationEnd({ type: "animationend", animationName: "fade", elapsedTime: 0.125, pseudoElement: "::before" });
  box.callbacks.onTransitionEnd({ type: "transitionend", propertyName: "opacity", elapsedTime: 0.25, pseudoElement: "" });
  box.callbacks.onAnimationStartCapture({ type: "animationstart", animationName: "pulse", elapsedTime: 0, pseudoElement: "::after" });
  box.callbacks.onTransitionCancelCapture({ type: "transitioncancel", propertyName: "width", elapsedTime: 0.05, pseudoElement: "" });
  assert.deepEqual(mod.taken(), [
    "a:animationend|fade|0.125|::before",
    "t:transitionend|opacity|0.25|",
    "a:animationstart|pulse|0|::after",
    "t:transitioncancel|width|0.05|",
  ]);
  assert.equal(mod.busy(), false);
  running = true;
  assert.equal(mod.busy(), true);
  assert.equal(queries.length, 2);
  assert.equal(queries[0], box);
  dispose();
});
