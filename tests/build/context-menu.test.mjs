import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import * as stylex from "@stylexjs/stylex";
import {
  ContextMenu, ContextMenuContent, ContextMenuTrigger, useContextMenuPlace,
  MenuCheckboxItem, MenuItem, MenuRadioGroup, MenuRadioItem,
  MenuSub, MenuSubContent, MenuSubTrigger,
} from "@hewel/zgui/components";

const styles = stylex.create({
  trigger: { color: "red" },
  item: { backgroundColor: "blue" },
});

export const log = {
  selects: [] as string[],
  checks: [] as boolean[],
  radios: [] as string[],
  opens: [] as boolean[],
  subOpens: [] as boolean[],
  cleans: 0,
};

const [rows, setRows] = createSignal(["alpha", "beta"]);
const [veto, setVeto] = createSignal(false);
const [disabled, setDisabled] = createSignal(false);
const [controlledOpen, setControlledOpen] = createSignal(false);
export { setRows, setVeto, setDisabled, setControlledOpen };

let activePlace: ReturnType<typeof useContextMenuPlace>;
export const getPlace = () => activePlace;

function PlaceCapture() {
  activePlace = useContextMenuPlace();
  onCleanup(() => { log.cleans += 1; activePlace = undefined; });
  return <text testId="area-copy">Area</text>;
}

export function mount(root: NativeNode) {
  return renderApp(() => <ContextMenu>
    <ContextMenuTrigger testId="trigger" {...stylex.props(styles.trigger)} disabled={disabled()}
      onContextMenu={event => { if (veto()) event.preventDefault(); }}
      onPointerDown={event => { if (veto()) event.preventDefault(); }}>
      <PlaceCapture />
      <For each={rows()}>{item => <text testId={"child-" + item}>{item}</text>}</For>
      <control testId="inner-btn">Action</control>
    </ContextMenuTrigger>
    <ContextMenuContent testId="surface">
      <MenuItem testId="item-copy" {...stylex.props(styles.item)} onSelect={() => log.selects.push("copy")}>Copy</MenuItem>
      <MenuCheckboxItem testId="item-pin" closeOnSelect={false} onCheckedChange={v => log.checks.push(v)}>Pin</MenuCheckboxItem>
      <MenuRadioGroup defaultValue="a" onValueChange={v => log.radios.push(v)}>
        <MenuRadioItem testId="radio-a" value="a" closeOnSelect={false}>A</MenuRadioItem>
        <MenuRadioItem testId="radio-b" value="b">B</MenuRadioItem>
      </MenuRadioGroup>
      <MenuSub onOpenChange={next => log.subOpens.push(next)}>
        <MenuSubTrigger testId="sub-trigger">Share</MenuSubTrigger>
        <MenuSubContent testId="sub-surface">
          <MenuItem testId="sub-item" onSelect={() => log.selects.push("sub")}>Link</MenuItem>
        </MenuSubContent>
      </MenuSub>
    </ContextMenuContent>
  </ContextMenu>, root);
}

export function mountControlled(root: NativeNode) {
  return renderApp(() => <ContextMenu open={controlledOpen()} onOpenChange={next => log.opens.push(next)}>
    <ContextMenuTrigger testId="trigger">Controlled</ContextMenuTrigger>
    <ContextMenuContent testId="surface">
      <MenuItem testId="item-close" onSelect={() => log.selects.push("close")}>Close</MenuItem>
    </ContextMenuContent>
  </ContextMenu>, root);
}

export function mountInitial(root: NativeNode) {
  return renderApp(() => <ContextMenu defaultOpen>
    <ContextMenuTrigger testId="trigger">Initial</ContextMenuTrigger>
    <ContextMenuContent testId="surface">
      <MenuItem testId="item-first">First</MenuItem>
    </ContextMenuContent>
  </ContextMenu>, root);
}

export function mountOrphanPlace(root: NativeNode) {
  return renderApp(() => <ContextMenu>
    <PlaceCapture />
    <ContextMenuContent testId="surface">
      <MenuItem testId="item-orphan">Orphan</MenuItem>
    </ContextMenuContent>
  </ContextMenu>, root);
}
`;

async function mounted(t, mountName = "mount") {
  const h = createHost();
  const { animating: busy, traps, layers, portals, geometry } = h;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  const clock = createClock(t, h);
  let dispose;
  t.after(() => {
    try { dispose?.(); } finally { delete globalThis.__zgui; }
    assert.equal(h.locks.length, 0, "context menu must not lock scroll");
  });
  globalThis.__zgui = h.host;
  const root = h.makeNode("root");
  dispose = mod[mountName](root);
  const send = (target, name, extras = {}) => {
    const node = typeof target === "string" ? h.byTestId(target) : target;
    assert.ok(node, `missing node ${target}`);
    return h.dispatch(node, name, extras);
  };
  const emit = (record, value) => record.emit(value);
  const flush = () => { clock.frame(); clock.advance(0); };
  const active = records => records.filter(record => record.active).length;
  return {
    h, mod, clock, busy, traps, layers, portals, geometry, send, emit, flush, active,
    finish: () => { const stop = dispose; dispose = undefined; stop?.(); },
  };
}

test("trigger renders zero-sized anchor after dynamic children and stays lazy", async t => {
  const { h, mod, traps, portals, layers, finish } = await mounted(t);
  const trigger = h.byTestId("trigger");
  assert.ok(trigger);
  assert.ok(h.stylesheets.has("ContextMenuStyle"));
  assert.ok(String(trigger.props.get("class")).includes("zui-context-menu__area"));
  assert.equal(trigger.props.get("ariaHasPopup"), "menu");
  assert.equal(trigger.props.get("expanded"), false);
  assert.equal(trigger.props.get("data-state"), "closed");
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(portals.length, 0);
  assert.equal(traps.length, 0);
  assert.equal(layers.length, 0);

  const anchor = trigger.children[trigger.children.length - 1];
  assert.equal(anchor.props.get("class"), "zui-context-menu__anchor");
  assert.equal(anchor.props.get("ariaHidden"), true);
  assert.deepEqual(anchor.props.get("style"), { left: "0px", top: "0px" });

  mod.setRows(["alpha", "beta", "gamma"]);
  assert.equal(trigger.children[trigger.children.length - 1], anchor, "dynamic child updates keep anchor intact at end");
  assert.ok(h.byTestId("child-gamma"));

  mod.setRows([]);
  assert.equal(trigger.children[trigger.children.length - 1], anchor, "emptying dynamic list does not clobber anchor");
  finish();
});

test("onContextMenu, secondary pointerdown, and useContextMenuPlace position anchor relative to area", async t => {
  const { h, mod, traps, layers, geometry, send, emit, flush, active, finish } = await mounted(t);
  const trigger = h.byTestId("trigger");
  const anchor = trigger.children[trigger.children.length - 1];
  h.rects.set(trigger, { x: 80, y: 60, width: 320, height: 240 });

  const primary = send("trigger", "onPointerDown", { button: "primary", position: { x: 140, y: 120 } });
  assert.equal(primary.defaultPrevented, false);
  assert.equal(h.byTestId("surface"), undefined, "primary press does not open context menu");

  const contextEvent = send("trigger", "onContextMenu", { position: { x: 145, y: 112 } });
  assert.equal(contextEvent.defaultPrevented, true);
  const surface = h.byTestId("surface");
  assert.ok(surface);
  assert.equal(trigger.props.get("expanded"), true);
  assert.equal(trigger.props.get("ariaControls"), surface);
  assert.equal(trigger.props.get("data-state"), "open");
  assert.deepEqual(anchor.props.get("style"), { left: "65px", top: "52px" });
  assert.equal(layers.length, 1);
  assert.equal(layers[0].options.exclude, anchor, "only the 0x0 anchor is excluded so clicking area dismisses");
  assert.equal(layers[0].options.dismissOnOutsidePress, true);
  assert.equal(layers[0].options.dismissOnEscape, true);
  assert.equal(surface.props.get("ariaPopupFor"), anchor);
  assert.equal(traps[0].node, surface);

  const positioner = h.parents.get(surface);
  const boxObs = geometry.find(r => r.active && r.kind === "borderBox" && r.node === anchor);
  const sizeObs = geometry.find(r => r.active && r.kind === "contentSize" && r.node === positioner);
  const viewObs = geometry.find(r => r.active && r.kind === "viewport" && r.node === positioner);
  assert.ok(boxObs && sizeObs && viewObs);
  emit(boxObs, { x: 145, y: 112, width: 0, height: 0 });
  emit(sizeObs, { width: 180, height: 120 });
  emit(viewObs, { x: 0, y: 0, width: 800, height: 600, scale: 1 });
  assert.equal(positioner.props.get("style").left, "145px");
  assert.equal(positioner.props.get("style").top, "112px");

  layers[0].callback("outside-press");
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(active(traps), 0);
  assert.equal(active(layers), 0);

  const secondary = send("trigger", "onPointerDown", { button: "secondary", position: { x: 200, y: 180 } });
  assert.equal(secondary.defaultPrevented, true);
  assert.ok(h.byTestId("surface"));
  assert.deepEqual(anchor.props.get("style"), { left: "120px", top: "120px" });

  mod.getPlace()?.openAt({ x: 95, y: 75 });
  assert.deepEqual(anchor.props.get("style"), { left: "15px", top: "15px" });
  assert.throws(() => mod.getPlace()?.openAt({ x: Number.NaN, y: 10 }), RangeError);
  assert.throws(() => mod.getPlace()?.openAt({ x: 10, y: Number.POSITIVE_INFINITY }), RangeError);

  send("item-pin", "onClick");
  assert.deepEqual(mod.log.checks, [true]);
  assert.ok(h.byTestId("surface"), "checkbox with closeOnSelect=false keeps context menu open");

  send("radio-b", "onClick");
  flush();
  assert.deepEqual(mod.log.radios, ["b"]);
  assert.equal(h.byTestId("surface"), undefined, "radio with default closeOnSelect dismisses menu");

  finish();
  assert.equal(mod.log.cleans, 1);
  assert.equal(active(traps), 0);
  assert.equal(active(layers), 0);
  assert.equal(active(geometry), 0);
});

test("caller preventDefault and disabled trigger block opening", async t => {
  const { h, mod, send, finish } = await mounted(t);
  mod.setVeto(true);
  send("trigger", "onContextMenu", { position: { x: 120, y: 90 } });
  assert.equal(h.byTestId("surface"), undefined);
  send("trigger", "onPointerDown", { button: "secondary", position: { x: 120, y: 90 } });
  assert.equal(h.byTestId("surface"), undefined);

  mod.setVeto(false);
  mod.setDisabled(true);
  const ctx = send("trigger", "onContextMenu", { position: { x: 120, y: 90 } });
  assert.equal(ctx.defaultPrevented, false);
  assert.equal(h.byTestId("surface"), undefined);
  const right = send("trigger", "onPointerDown", { button: "secondary", position: { x: 120, y: 90 } });
  assert.equal(right.defaultPrevented, false);
  assert.equal(h.byTestId("surface"), undefined);
  finish();
});

test("controlled context menu waits for caller open updates", async t => {
  const c = await mounted(t, "mountControlled");
  assert.equal(c.h.byTestId("surface"), undefined);
  c.send("trigger", "onContextMenu", { position: { x: 50, y: 40 } });
  assert.deepEqual(c.mod.log.opens, [true]);
  assert.equal(c.h.byTestId("surface"), undefined, "controlled state waits for caller");
  c.mod.setControlledOpen(true);
  assert.ok(c.h.byTestId("surface"));
  const anchor = c.h.byTestId("trigger").children[c.h.byTestId("trigger").children.length - 1];
  assert.deepEqual(anchor.props.get("style"), { left: "50px", top: "40px" });
  c.send("item-close", "onClick");
  assert.deepEqual(c.mod.log.opens, [true, false]);
  c.finish();
});

test("defaultOpen context menu opens initially at origin", async t => {
  const init = await mounted(t, "mountInitial");
  assert.ok(init.h.byTestId("surface"));
  assert.ok(init.h.byTestId("item-first"));
  init.finish();
});

test("triggerless useContextMenuPlace opens using window coordinates directly", async t => {
  const orphan = await mounted(t, "mountOrphanPlace");
  assert.equal(orphan.h.byTestId("surface"), undefined);
  orphan.mod.getPlace()?.openAt({ x: 32, y: 44 });
  assert.ok(orphan.h.byTestId("surface"));
  assert.ok(orphan.h.byTestId("item-orphan"));
  orphan.finish();
});

test("context menu forwards StyleX classes, focuses MenuSubTrigger on Escape, and cascades leaf dismissal", async t => {
  const { h, mod, layers, send, flush, finish } = await mounted(t);
  const trigger = h.byTestId("trigger");
  assert.match(String(trigger.props.get("class")), /\bzui-context-menu__area\b/);
  assert.match(String(trigger.props.get("class")), /\bx/);

  send("trigger", "onContextMenu", { position: { x: 100, y: 100 } });
  const surface = h.byTestId("surface");
  const itemCopy = h.byTestId("item-copy");
  assert.match(String(itemCopy.props.get("class")), /\bzui-menu__item\b/);
  assert.match(String(itemCopy.props.get("class")), /\bx/);

  const subTrigger = h.byTestId("sub-trigger");
  send(subTrigger, "onClick");
  assert.ok(h.byTestId("sub-surface"));
  h.host.focus(h.byTestId("sub-item"));
  layers.findLast(record => record.active).callback("escape");
  assert.equal(subTrigger.props.get("expanded"), false);
  assert.equal(h.focused.current, subTrigger, "submenu Escape inside context menu focuses MenuSubTrigger");
  flush();
  assert.equal(h.byTestId("sub-surface"), undefined);
  assert.equal(h.byTestId("surface"), surface);

  send(subTrigger, "onClick");
  mod.log.subOpens.length = 0;
  send("sub-item", "onClick");
  assert.deepEqual(mod.log.subOpens, [false], "submenu leaf selection notifies onOpenChange(false)");
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  finish();
});
