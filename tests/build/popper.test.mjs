import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const solverFixture = `
export { solvePopper } from "@hewel/zgui/components";
`;

const integrationFixture = `
import { createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Popper } from "@hewel/zgui/components";
import type { PopperPlacement } from "@hewel/zgui/components";
export const log = { mounts: 0, cleans: 0, positioner: undefined as NativeNode | undefined };
const [active, setActive] = createSignal(true);
const [choice, setChoice] = createSignal(0);
const [placement, setPlacement] = createSignal<PopperPlacement | undefined>(undefined);
export { setActive, setChoice, setPlacement };
function Counter() {
  log.mounts += 1;
  onCleanup(() => { log.cleans += 1; });
  const [count, setCount] = createSignal(0);
  return <control testId="counter" onClick={() => setCount(n => n + 1)}><text>{count()}</text></control>;
}
export function mount(root: NativeNode) {
  return renderApp(() => {
    let first!: NativeNode;
    let second!: NativeNode;
    return <column>
      <box testId="anchor-a" ref={node => { first = node; }} />
      <box testId="anchor-b" ref={node => { second = node; }} />
      <Popper anchor={choice() === 0 ? first : second} active={active()} placement={placement()}
        ref={node => { log.positioner = node; }}>
        <Counter />
      </Popper>
    </column>;
  }, root);
}
`;

const textOf = current =>
  current.tag === "#text" ? current.props.get("text") ?? "" : current.children.map(textOf).join("");

const stylesOf = node => {
  const style = node.props.get("style");
  if (style && typeof style === "object") return new Map(Object.entries(style));
  return new Map(String(style ?? "").split(";").map(part => part.trim()).filter(Boolean).map(part => {
    const colon = part.indexOf(":");
    return [part.slice(0, colon).trim(), part.slice(colon + 1).trim()];
  }));
};

async function mounted(t) {
  const harness = createHost();
  const registrations = harness.geometry;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, integrationFixture);
  let dispose;
  t.after(() => { try { dispose?.(); } finally { delete globalThis.__zgui; } });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  const emit = (record, value) => record.emit(value);
  return { mod, harness, registrations, emit,
    finish: () => { const end = dispose; dispose = undefined; end?.(); } };
}

test("solvePopper produces every side and alignment", async t => {
  const mod = await moduleFor(t, solverFixture);
  const anchor = { x: 300, y: 200, width: 100, height: 30 };
  const floating = { width: 60, height: 40 };
  const viewport = { x: 0, y: 0, width: 800, height: 600 };
  for (const [side, align, x, y] of [
    ["bottom", "start", 300, 234], ["bottom", "center", 320, 234], ["bottom", "end", 340, 234],
    ["top", "start", 300, 156], ["top", "center", 320, 156], ["top", "end", 340, 156],
    ["left", "start", 236, 200], ["left", "center", 236, 195], ["left", "end", 236, 190],
    ["right", "start", 404, 200], ["right", "center", 404, 195], ["right", "end", 404, 190],
  ]) {
    const solved = mod.solvePopper(anchor, floating, viewport, { placement: { side, align } });
    assert.deepEqual(solved.origin, { x, y }, `${side}/${align}`);
    assert.deepEqual(solved.placement, { side, align });
    assert.equal(solved.overflow, 0);
  }
});

test("solvePopper flips, shifts and caps against viewport edges", async t => {
  const mod = await moduleFor(t, solverFixture);
  const v = { x: 0, y: 0, width: 800, height: 600 };
  const s = mod.solvePopper;
  let p = s({ x: 100, y: 560, width: 80, height: 24 }, { width: 200, height: 160 }, v);
  assert.deepEqual(p.origin, { x: 40, y: 396 });
  assert.equal(p.placement.side, "top");
  assert.equal(p.overflow, 0);
  p = s({ x: 760, y: 100, width: 30, height: 30 }, { width: 240, height: 100 }, v);
  assert.deepEqual(p.origin, { x: 552, y: 134 });
  assert.equal(p.placement.side, "bottom");
  p = s({ x: 4, y: 100, width: 30, height: 30 }, { width: 240, height: 100 }, v);
  assert.equal(p.origin.x, 8);
  const a = { x: 100, y: 300, width: 80, height: 24 };
  const first = s(a, { width: 200, height: 700 }, v);
  const capped = s(a, { width: 200, height: first.available.height }, v);
  assert.equal(first.placement.side, "top");
  assert.deepEqual(first.origin, { x: 40, y: 8 });
  assert.equal(first.available.height, 288);
  assert.deepEqual(capped.origin, first.origin);
  assert.deepEqual(capped.placement, first.placement);
  assert.deepEqual(capped.available, first.available);
  assert.equal(capped.overflow, 0);
  p = s({ x: 100, y: 200, width: 80, height: 24 }, { width: 200, height: 700 }, v);
  assert.equal(p.placement.side, "bottom");
  assert.equal(p.available.height, 364);
  assert.deepEqual(s({ x: 100, y: 200, width: 80, height: 24 }, { width: 200, height: 364 }, v).origin, p.origin);
  p = s({ x: 700, y: 200, width: 80, height: 30 }, { width: 200, height: 100 }, v, { placement: { side: "right", align: "center" } });
  assert.equal(p.placement.side, "left");
  assert.deepEqual(p.origin, { x: 496, y: 165 });
  p = s({ x: 100, y: 560, width: 80, height: 24 }, { width: 200, height: 160 }, v, { flip: false });
  assert.equal(p.placement.side, "bottom");
  assert.equal(p.origin.y, 588);
  assert.equal(p.overflow, 148);
  p = s({ x: 760, y: 100, width: 30, height: 30 }, { width: 240, height: 100 }, v, { shift: false });
  assert.equal(p.origin.x, 655);
  p = s({ x: 100, y: 590, width: 80, height: 10 }, { width: 200, height: 100 }, v, { flip: false });
  assert.equal(p.available.height, 0);
  p = s({ x: 120, y: 320, width: 40, height: 20 }, { width: 200, height: 120 }, { x: 100, y: 50, width: 400, height: 300 });
  assert.deepEqual(p.origin, { x: 108, y: 196 });
  assert.equal(p.placement.side, "top");
});

test("solvePopper validates options and accepts negative offsets", async t => {
  const mod = await moduleFor(t, solverFixture);
  const v = { x: 0, y: 0, width: 800, height: 600 };
  const a = { x: 300, y: 200, width: 100, height: 30 };
  const f = { width: 60, height: 40 };
  assert.throws(() => mod.solvePopper(a, f, v, { offset: Number.NaN }), RangeError);
  assert.throws(() => mod.solvePopper(a, f, v, { padding: Number.POSITIVE_INFINITY }), RangeError);
  assert.throws(() => mod.solvePopper(a, f, v, { placement: { side: "middle", align: "center" } }), TypeError);
  assert.throws(() => mod.solvePopper(a, f, v, { placement: { side: "bottom" } }), TypeError);
  assert.throws(() => mod.solvePopper(a, f, v, { flip: 1 }), TypeError);
  const shifted = mod.solvePopper(a, f, v, { offset: -10 });
  assert.equal(shifted.origin.y, 220);
  const baseline = mod.solvePopper(a, f, v);
  assert.equal(baseline.origin.y, 234);
  assert.notEqual(shifted.origin.y, baseline.origin.y);
});

test("popper measures anchor content and viewport before placing", async t => {
  const { mod, registrations, emit, finish } = await mounted(t);
  const positioner = () => mod.log.positioner;
  const style = () => positioner().props.get("style");
  assert.equal(registrations.length, 3);
  assert.deepEqual(registrations.map(r => r.kind), ["borderBox", "contentSize", "viewport"]);
  assert.equal(style().visibility, "hidden");
  const [borderBox, contentSize, viewport] = registrations;
  emit(borderBox, { x: 300.2, y: 200.2, width: 100.2, height: 30.2 });
  emit(viewport, { x: 0, y: 0, width: 800, height: 600, scale: 1.75 });
  assert.equal(style().visibility, "hidden");
  assert.equal(stylesOf(positioner()).get("--zui-popper-anchor-width"), "100px");
  emit(contentSize, { width: 0, height: 0 });
  assert.equal(style().visibility, "hidden");
  assert.equal(stylesOf(positioner()).get("--zui-popper-anchor-width"), "100px");
  emit(contentSize, { width: 200.1, height: 120.2 });
  assert.equal("visibility" in style(), false);
  assert.equal(style().left, `${438 / 1.75}px`);
  assert.equal(style().top, `${410 / 1.75}px`);
  assert.equal(positioner().props.get("data-side"), "bottom");
  assert.equal(stylesOf(positioner()).get("--zui-popper-anchor-width"), "100px");
  assert.equal(stylesOf(positioner()).get("--zui-popper-available-height"), "357px");
  assert.equal(stylesOf(positioner()).get("--zui-popper-anchor-x"), "100px");
  assert.equal(stylesOf(positioner()).get("--zui-popper-anchor-y"), "0px");
  emit(viewport, { x: 0, y: 0, width: 800, height: 600, scale: 2 });
  assert.equal(style().left, "250.5px");
  assert.equal(style().top, "234.5px");
  mod.setPlacement({ side: "top", align: "end" });
  assert.equal(stylesOf(positioner()).get("left"), "200.5px");
  assert.equal(stylesOf(positioner()).get("top"), "76px");
  assert.equal(stylesOf(positioner()).get("--zui-popper-anchor-width"), "100px");
  assert.equal(stylesOf(positioner()).get("--zui-popper-available-height"), "188px");
  assert.equal(positioner().props.get("data-side"), "top");
  assert.equal(positioner().props.get("data-align"), "end");
  assert.equal(registrations.length, 3, "no re-registration from measurements or placement");
  finish();
  assert.equal(mod.log.cleans, 1);
  for (const record of registrations) assert.equal(record.disposals, 1);
});

test("popper re-anchors and deactivates without losing content state", async t => {
  const { mod, harness, registrations, emit, finish } = await mounted(t);
  const style = () => mod.log.positioner.props.get("style");
  const counter = harness.byTestId("counter");
  harness.dispatch(counter, "Click");
  assert.equal(textOf(counter), "1");
  mod.setChoice(1);
  assert.equal(registrations.length, 4);
  const retired = registrations[0];
  assert.equal(retired.kind, "borderBox");
  assert.equal(retired.active, false);
  assert.equal(retired.disposals, 1);
  const fresh = registrations[3];
  assert.equal(fresh.kind, "borderBox");
  assert.equal(registrations.filter(r => r.active).length, 3);
  assert.equal(style().visibility, "hidden");
  emit(retired, { x: 1, y: 1, width: 1, height: 1 });
  mod.setPlacement({ side: "right", align: "center" });
  emit(fresh, { x: 700, y: 200, width: 80, height: 30 });
  emit(registrations[1], { width: 200.1, height: 120.2 });
  emit(registrations[2], { x: 0, y: 0, width: 800, height: 600, scale: 1.75 });
  assert.equal(mod.log.positioner.props.get("data-side"), "left");
  assert.equal(harness.byTestId("counter"), counter);
  assert.equal(textOf(counter), "1");
  mod.setActive(false);
  assert.equal(registrations.filter(r => r.active).length, 0);
  assert.equal(style().visibility, "hidden");
  assert.equal(harness.byTestId("counter"), counter);
  assert.equal(textOf(counter), "1");
  mod.setActive(true);
  assert.equal(registrations.length, 7);
  assert.equal(registrations.filter(r => r.active).length, 3);
  assert.equal(style().visibility, "hidden");
  const latest = registrations.slice(4);
  emit(latest[0], { x: 700, y: 200, width: 80, height: 30 });
  emit(latest[1], { width: 200.1, height: 120.2 });
  emit(latest[2], { x: 0, y: 0, width: 800, height: 600, scale: 1.75 });
  assert.equal("visibility" in style(), false);
  assert.equal(textOf(counter), "1");
  assert.equal(mod.log.mounts, 1);
  finish();
  assert.equal(mod.log.cleans, 1);
  for (const record of registrations) assert.equal(record.disposals, 1);
});
