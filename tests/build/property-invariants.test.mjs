import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

function mulberry32(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function intRange(rng, lo, hi) {
  return lo + Math.floor(rng() * (hi - lo + 1));
}

test("property: solvePopper always produces finite non-negative bounds and fits when space permits", async t => {
  const mod = await moduleFor(t, 'export { solvePopper } from "@hewel/zgui/components";');
  const rng = mulberry32(0x10010001);
  const sides = ["top", "right", "bottom", "left"];
  const aligns = ["start", "center", "end"];

  for (let i = 0; i < 250; i++) {
    const vx = intRange(rng, -50, 50);
    const vy = intRange(rng, -50, 50);
    const vw = intRange(rng, 240, 1400);
    const vh = intRange(rng, 240, 1000);
    const viewport = { x: vx, y: vy, width: vw, height: vh };

    const aw = intRange(rng, 16, Math.min(200, vw - 20));
    const ah = intRange(rng, 16, Math.min(120, vh - 20));
    const anchor = {
      x: intRange(rng, vx, vx + vw - aw),
      y: intRange(rng, vy, vy + vh - ah),
      width: aw,
      height: ah,
    };

    const fw = intRange(rng, 40, 260);
    const fh = intRange(rng, 30, 220);
    const offset = intRange(rng, 0, 16);
    const padding = intRange(rng, 0, 16);
    const side = sides[intRange(rng, 0, sides.length - 1)];
    const align = aligns[intRange(rng, 0, aligns.length - 1)];
    const flip = rng() < 0.85;
    const shift = rng() < 0.85;

    const solution = mod.solvePopper(anchor, { width: fw, height: fh }, viewport, {
      placement: { side, align },
      offset,
      padding,
      flip,
      shift,
    });

    assert.ok(Number.isFinite(solution.origin.x), `trial ${i}: origin.x finite`);
    assert.ok(Number.isFinite(solution.origin.y), `trial ${i}: origin.y finite`);
    assert.ok(solution.available.width >= 0, `trial ${i}: available.width >= 0`);
    assert.ok(solution.available.height >= 0, `trial ${i}: available.height >= 0`);
    assert.ok(solution.overflow >= 0, `trial ${i}: overflow >= 0`);
    assert.ok(["top", "right", "bottom", "left"].includes(solution.placement.side));
    assert.equal(solution.placement.align, align);

    const verticalSpace = Math.max(anchor.y - vy, vy + vh - (anchor.y + ah));
    const horizontalSpace = Math.max(anchor.x - vx, vx + vw - (anchor.x + aw));
    const isVert = solution.placement.side === "top" || solution.placement.side === "bottom";
    const mainFit = isVert
      ? verticalSpace >= fh + offset + padding && vw - 2 * padding >= fw
      : horizontalSpace >= fw + offset + padding && vh - 2 * padding >= fh;
    if (flip && shift && mainFit) {
      assert.equal(
        solution.overflow,
        0,
        `trial ${i}: expected zero overflow when side and cross axis fit`,
      );
    }
  }
});

test("property: Slider clamps values, aligns steps, and never strands pointer capture", async t => {
  const fixture = `
import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Slider } from "@hewel/zgui/components";
const [min, setMin] = createSignal(0);
const [max, setMax] = createSignal(100);
const [step, setStep] = createSignal(1);
export const log = { values: [] as number[] };
export function setBounds(nextMin: number, nextMax: number, nextStep: number) {
  setMin(nextMin);
  setMax(nextMax);
  setStep(nextStep);
}
export function mount(root: NativeNode) {
  return renderApp(() => (
    <Slider
      testId="slider"
      min={min()}
      max={max()}
      step={step()}
      onValueChange={v => { log.values.push(v); }}
    />
  ), root);
}
`;
  const h = createHost();
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  globalThis.__zgui = h.host;
  const mod = await moduleFor(t, fixture);
  const root = h.makeNode("root");
  dispose = mod.mount(root);

  const slider = h.byTestId("slider");
  const track = [...h.nodes.values()].find(n =>
    (n.props.get("class") ?? "").includes("zui-slider__track"),
  );
  assert.ok(slider && track);
  h.rects.set(track, { x: 100, y: 20, width: 200, height: 12 });

  const rng = mulberry32(0x20020002);
  const keys = [
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
    "PageUp",
    "PageDown",
    "Home",
    "End",
  ];
  let activePointer = null;
  let activeStep = 1;

  for (let i = 0; i < 220; i++) {
    if (i % 25 === 0) {
      if (activePointer !== null) {
        h.dispatch(slider, "PointerUp", {
          pointerId: String(activePointer),
          position: { x: 150, y: 26 },
        });
        activePointer = null;
      }
      const nextMin = intRange(rng, -50, 20);
      const nextMax = nextMin + intRange(rng, 10, 120);
      activeStep = rng() < 0.2 ? 0 : [0.5, 1, 5, 7][intRange(rng, 0, 3)];
      mod.setBounds(nextMin, nextMax, activeStep);
      h.flush();
    }

    const prevEmitted = mod.log.values.length;
    const op = intRange(rng, 0, 3);
    if (op === 0 && activePointer === null) {
      activePointer = 1;
      const x = intRange(rng, 60, 340);
      h.dispatch(slider, "PointerDown", { pointerId: "1", position: { x, y: 26 } });
    } else if (op === 1 && activePointer !== null) {
      const x = intRange(rng, 60, 340);
      h.dispatch(slider, "PointerMove", { pointerId: "1", position: { x, y: 26 } });
    } else if (op === 2 && activePointer !== null) {
      const cancel = rng() < 0.35;
      const x = intRange(rng, 60, 340);
      h.dispatch(slider, cancel ? "PointerCancel" : "PointerUp", {
        pointerId: "1",
        position: { x, y: 26 },
      });
      activePointer = null;
      const before = slider.props.get("ariaValueNow");
      h.dispatch(slider, "PointerMove", {
        pointerId: "1",
        position: { x: x === 340 ? 60 : 340, y: 26 },
      });
      assert.equal(
        slider.props.get("ariaValueNow"),
        before,
        "stray move after up/cancel must not change value",
      );
    } else {
      const key = keys[intRange(rng, 0, keys.length - 1)];
      h.dispatch(slider, "KeyDown", { key, keyWithoutModifiers: key });
    }

    const lo = slider.props.get("ariaValueMin");
    const hi = slider.props.get("ariaValueMax");
    const val = slider.props.get("ariaValueNow");
    assert.ok(
      Number.isFinite(val) && val >= lo && val <= hi,
      `trial ${i}: ${val} outside [${lo}, ${hi}]`,
    );
    if (activeStep > 0) {
      const stepsFromMin = (val - lo) / activeStep;
      const aligned = Math.abs(stepsFromMin - Math.round(stepsFromMin)) < 1e-9 || val === hi;
      assert.ok(
        aligned,
        `trial ${i}: value ${val} not aligned to step=${activeStep} from min=${lo} (max=${hi})`,
      );
    }
    for (let j = prevEmitted; j < mod.log.values.length; j++) {
      const emitted = mod.log.values[j];
      assert.ok(
        Number.isFinite(emitted) && emitted >= lo && emitted <= hi,
        `trial ${i}: emitted ${emitted} outside [${lo}, ${hi}]`,
      );
    }
  }
});

test("property: nested Dialog + Popover release all traps, scroll locks, layers, and timers across random transitions", async t => {
  const fixture = `
import { Show, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button, Dialog, DialogClose, DialogContent, DialogTrigger,
  Popover, PopoverClose, PopoverContent, PopoverTrigger, ThemeProvider,
} from "@hewel/zgui/components";
const [mounted, setMounted] = createSignal(true);
const [dialogOpen, setDialogOpen] = createSignal(false);
const [popoverOpen, setPopoverOpen] = createSignal(false);
const [dismissible, setDismissible] = createSignal(true);
export { setMounted, setDialogOpen, setPopoverOpen, setDismissible, dialogOpen, popoverOpen };
export function mount(root: NativeNode) {
  return renderApp(() => (
    <ThemeProvider scheme="light">
      <Show when={mounted()}>
        <Dialog open={dialogOpen()} onOpenChange={open => { setDialogOpen(open); if (!open) setPopoverOpen(false); }}>
          <DialogTrigger testId="dlg-trigger">Open dialog</DialogTrigger>
          <DialogContent testId="dlg-surface" dismissOnEscape={dismissible()} dismissOnOutsidePress={dismissible()}>
            <Popover open={popoverOpen()} onOpenChange={setPopoverOpen}>
              <PopoverTrigger testId="pop-trigger">Open popover</PopoverTrigger>
              <PopoverContent testId="pop-surface" dismissOnEscape={dismissible()} dismissOnOutsidePress={dismissible()}>
                <PopoverClose testId="pop-close">Close popover</PopoverClose>
              </PopoverContent>
            </Popover>
            <DialogClose testId="dlg-close">Close dialog</DialogClose>
          </DialogContent>
        </Dialog>
      </Show>
    </ThemeProvider>
  ), root);
}
`;
  const h = createHost();
  const clock = createClock(t, h);
  const { traps, locks, layers } = h;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  globalThis.__zgui = h.host;
  const mod = await moduleFor(t, fixture);
  const root = h.makeNode("root");
  dispose = mod.mount(root);

  const countActive = list => list.filter(entry => entry.active).length;
  const rng = mulberry32(0x30030003);

  for (let step = 0; step < 220; step++) {
    const action = intRange(rng, 0, 6);
    if (action === 0) {
      mod.setMounted(true);
      mod.setDialogOpen(v => {
        const next = !v;
        if (!next) mod.setPopoverOpen(false);
        return next;
      });
      h.flush();
    } else if (action === 1 && mod.dialogOpen()) {
      mod.setPopoverOpen(v => !v);
      h.flush();
    } else if (action === 2) {
      const topLayer = layers.findLast(record => record.active);
      topLayer?.callback(rng() < 0.5 ? "escape" : "outsidePointer");
    } else if (action === 3) {
      mod.setDismissible(v => !v);
      h.flush();
    } else if (action === 4) {
      mod.setMounted(false);
      mod.setDialogOpen(false);
      mod.setPopoverOpen(false);
      h.flush();
    } else if (action === 5) {
      mod.setMounted(true);
      h.flush();
    } else {
      clock.settle();
    }
    clock.frame();
    clock.advance(25);
  }

  mod.setDialogOpen(false);
  mod.setPopoverOpen(false);
  h.flush();
  clock.settle();
  assert.equal(countActive(traps), 0, "all focus traps released after close");
  assert.equal(countActive(locks), 0, "all scroll locks released after close");
  assert.equal(countActive(layers), 0, "all dismiss layers released after close");

  dispose();
  dispose = undefined;
  clock.settle();
  assert.equal(clock.timers.size, 0, "zero timers remain after disposal");
  assert.equal(clock.frames.size, 0, "zero frames remain after disposal");
  for (const record of [...traps, ...locks, ...layers]) {
    assert.equal(record.active, false);
    assert.equal(record.disposals, 1);
  }
});

test("property: Menubar + MenuSub preserve single-open, roving tabIndex, and clean teardown", async t => {
  const fixture = `
import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Menubar, MenubarContent, MenubarItem, MenubarMenu, MenubarTrigger,
  MenuItem, MenuSub, MenuSubContent, MenuSubTrigger, ThemeProvider,
} from "@hewel/zgui/components";
export function mount(root: NativeNode) {
  return renderApp(() => (
    <ThemeProvider scheme="light">
      <Menubar testId="bar" label="Property bar">
        <MenubarMenu value="one">
          <MenubarTrigger testId="t-one">One</MenubarTrigger>
          <MenubarContent testId="c-one">
            <MenubarItem testId="i-one">Alpha</MenubarItem>
          </MenubarContent>
        </MenubarMenu>
        <MenubarMenu value="two">
          <MenubarTrigger testId="t-two">Two</MenubarTrigger>
          <MenubarContent testId="c-two">
            <MenuSub>
              <MenuSubTrigger testId="sub-trigger">More</MenuSubTrigger>
              <MenuSubContent testId="sub-content">
                <MenuItem testId="i-sub">Nested</MenuItem>
              </MenuSubContent>
            </MenuSub>
          </MenubarContent>
        </MenubarMenu>
        <MenubarMenu value="three">
          <MenubarTrigger testId="t-three">Three</MenubarTrigger>
          <MenubarContent testId="c-three">
            <MenubarItem testId="i-three">Gamma</MenubarItem>
          </MenubarContent>
        </MenubarMenu>
      </Menubar>
    </ThemeProvider>
  ), root);
}
`;
  const h = createHost();
  const clock = createClock(t, h);
  const { traps, layers, geometry } = h;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  globalThis.__zgui = h.host;
  const mod = await moduleFor(t, fixture);
  const root = h.makeNode("root");
  dispose = mod.mount(root);

  const clickNode = node => {
    if (node) h.dispatch(node, "Click");
  };
  const keyNode = (node, key) => {
    if (node) h.dispatch(node, "KeyDown", { key, keyWithoutModifiers: key });
  };

  const rng = mulberry32(0x40040004);
  const triggers = ["t-one", "t-two", "t-three"];
  const navKeys = ["ArrowLeft", "ArrowRight", "ArrowDown", "ArrowUp", "Home", "End", "Escape"];

  for (let step = 0; step < 220; step++) {
    const op = intRange(rng, 0, 5);
    if (op <= 1) {
      const target = h.byTestId(triggers[intRange(rng, 0, 2)]);
      clickNode(target);
    } else if (op === 2) {
      const focused =
        h.focused.current && !h.invalidated.has(h.focused.current)
          ? h.focused.current
          : h.byTestId("t-one");
      keyNode(focused, navKeys[intRange(rng, 0, navKeys.length - 1)]);
    } else if (op === 3) {
      const subTrigger = h.byTestId("sub-trigger");
      if (subTrigger) clickNode(subTrigger);
    } else if (op === 4) {
      const top = layers.findLast(record => record.active);
      top?.callback("outsidePointer");
    } else {
      clock.frame();
      clock.advance(120);
    }

    const openMenus = ["c-one", "c-two", "c-three"].filter(
      id => h.byTestId(id)?.props.get("data-state") === "open",
    );
    assert.ok(
      openMenus.length <= 1,
      `step ${step}: expected at most 1 open menubar menu, got ${openMenus}`,
    );
    clock.settle();
    const mountedMenus = ["c-one", "c-two", "c-three"].filter(id => Boolean(h.byTestId(id)));
    assert.ok(
      mountedMenus.length <= 1,
      `step ${step}: exiting menu leaked after settle: ${mountedMenus}`,
    );

    const activeStops = triggers
      .map(id => h.byTestId(id)?.props.get("tabIndex"))
      .filter(v => v === 0);
    assert.equal(activeStops.length, 1, `step ${step}: expected exactly 1 roving tabIndex=0 stop`);
    for (const id of triggers) {
      const tabIndex = h.byTestId(id)?.props.get("tabIndex");
      assert.ok(tabIndex === 0 || tabIndex === -1, `step ${step}: invalid tabIndex ${tabIndex}`);
    }
  }

  dispose();
  dispose = undefined;
  clock.settle();
  assert.equal(clock.timers.size, 0);
  assert.equal(clock.frames.size, 0);
  for (const record of [...traps, ...layers, ...geometry]) {
    assert.equal(record.active, false);
    assert.equal(record.disposals, 1);
  }
});
