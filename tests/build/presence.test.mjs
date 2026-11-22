import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Popover, PopoverContent, PopoverTrigger, ThemeProvider } from "@hewel/zgui/components";
const [open, setOpen] = createSignal(false);
export { setOpen };
export const log = { mounts: 0, cleanups: 0 };
function Body() {
  log.mounts += 1;
  onCleanup(() => { log.cleanups += 1; });
  const [count, setCount] = createSignal(0);
  return <control testId="counter" onClick={() => setCount(n => n + 1)}>{String(count())}</control>;
}
export function mount(root: NativeNode) {
  return renderApp(() => (
    <ThemeProvider>
      <Popover open={open()} onOpenChange={setOpen}>
        <PopoverTrigger testId="trigger">Toggle</PopoverTrigger>
        <PopoverContent testId="surface">
          <Body />
        </PopoverContent>
      </Popover>
    </ThemeProvider>
  ), root);
}
`;

const textOf = current =>
  current.tag === "#text"
    ? (current.props.get("text") ?? "")
    : current.children.map(textOf).join("");

async function mounted(t) {
  const harness = createHost();
  const { animating: busy, animatingQueries: queries } = harness;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  const clock = createClock(t, harness);
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  dispose = mod.mount(root);
  const surface = () => harness.byTestId("surface");
  const state = () => surface()?.props.get("data-state");
  const counter = () => textOf(harness.byTestId("counter"));
  const event = (target, type) => harness.dispatch(target, type);
  return {
    clock,
    queries,
    busy,
    harness,
    mod,
    root,
    surface,
    state,
    counter,
    event,
    finish: () => {
      const end = dispose;
      dispose = undefined;
      end?.();
    },
  };
}

test("presence mounts on open, defers its exit check past a styled frame and removes an idle surface", async t => {
  const { clock, queries, mod, surface, state, counter, event, harness } = await mounted(t);
  assert.equal(surface(), undefined);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(queries.length, 0);
  mod.setOpen(true);
  harness.flush();
  assert.ok(surface());
  assert.equal(state(), "open");
  mod.setOpen(false);
  harness.flush();
  const first = surface();
  assert.equal(state(), "closed");
  assert.equal(clock.frames.size, 1);
  assert.equal(clock.timers.size, 1, "only the deadline timer is queued before the styled frame");
  assert.equal(queries.length, 0);
  event(first, "onAnimationEnd");
  assert.ok(surface(), "lifecycle event before the styled frame must not remove");
  assert.equal(queries.length, 0);
  clock.frame();
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 2);
  event(first, "onAnimationEnd");
  assert.ok(surface(), "lifecycle event before the deferred check must not remove");
  assert.equal(queries.length, 0);
  clock.advance(0);
  assert.equal(surface(), undefined);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(queries.length, 1);
  assert.equal(mod.log.cleanups, 1);
  mod.setOpen(true);
  harness.flush();
  assert.ok(surface());
  assert.notEqual(surface(), first);
  assert.equal(counter(), "0");
  assert.equal(state(), "open");
});

test("a busy surface is retained until own lifecycle event reports all motion gone", async t => {
  const { clock, queries, busy, mod, surface, event, harness } = await mounted(t);
  mod.setOpen(true);
  harness.flush();
  const current = surface();
  busy.set(current, true);
  mod.setOpen(false);
  harness.flush();
  clock.frame();
  clock.advance(0);
  assert.equal(surface(), current, "busy surface survives the styled-frame check");
  assert.equal(queries.length, 1);
  event(current, "onAnimationEnd");
  assert.equal(surface(), current);
  assert.equal(queries.length, 2);
  busy.set(current, false);
  const counter = surface().children[0];
  harness.dispatch(counter, "AnimationEnd");
  assert.equal(surface(), current, "a descendant's event does not own the exit");
  assert.equal(queries.length, 2, "descendant event must not query");
  harness.dispatch(current, "TransitionCancel");
  assert.equal(surface(), undefined);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.log.cleanups, 1);
});

test("reopen cancels a queued exit and stale callbacks cannot close the new exit", async t => {
  const { clock, queries, busy, mod, surface, state, counter, harness } = await mounted(t);
  mod.setOpen(true);
  harness.flush();
  harness.dispatch(harness.byTestId("counter"), "Click");
  assert.equal(counter(), "1");
  mod.setOpen(false);
  harness.flush();
  const stale = [
    ...clock.frames.values(),
    ...[...clock.timers.values()].map(timer => timer.callback),
  ];
  mod.setOpen(true);
  harness.flush();
  const current = surface();
  assert.equal(state(), "open");
  assert.equal(counter(), "1");
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  for (const callback of stale) callback();
  assert.equal(surface(), current);
  assert.equal(queries.length, 0);
  mod.setOpen(false);
  harness.flush();
  busy.set(current, true);
  for (const callback of stale) callback();
  assert.equal(surface(), current, "stale callbacks must not close the new exit");
  assert.equal(clock.frames.size, 1);
  assert.equal(clock.timers.size, 1, "stale callbacks cannot schedule timers");
  clock.frame();
  clock.advance(0);
  assert.equal(surface(), current, "endless motion survives the styled-frame check");
  clock.advance(999);
  assert.equal(surface(), current);
  clock.advance(1);
  assert.equal(surface(), undefined, "the one-second deadline ends endless motion");
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(queries.length, 1);
});

test("disposal before the styled frame clears timers and saved callbacks stay inert", async t => {
  const { clock, queries, mod, surface, finish, harness } = await mounted(t);
  mod.setOpen(true);
  harness.flush();
  mod.setOpen(false);
  harness.flush();
  assert.equal(clock.frames.size, 1);
  assert.equal(clock.timers.size, 1);
  const stale = [
    ...clock.frames.values(),
    ...[...clock.timers.values()].map(timer => timer.callback),
  ];
  finish();
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.log.cleanups, 1);
  assert.equal(surface(), undefined);
  for (const callback of stale) callback();
  assert.equal(queries.length, 0);
});

test("disposal after the styled frame queued the deferred check clears timers and saved callbacks stay inert", async t => {
  const { clock, queries, mod, surface, finish, harness } = await mounted(t);
  mod.setOpen(true);
  harness.flush();
  mod.setOpen(false);
  harness.flush();
  clock.frame();
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 2);
  const stale = [...clock.timers.values()].map(timer => timer.callback);
  finish();
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.log.cleanups, 1);
  assert.equal(surface(), undefined);
  for (const callback of stale) callback();
  assert.equal(queries.length, 0);
});

test("createRetainedPresence tracks non-nullish falsy/callable payloads, transitions, batching, and atomic release", async t => {
  const harness = createHost();
  const clock = createClock(t, harness);
  const mod = await moduleFor(
    t,
    `
    import { Show, createEffect, createSignal, onCleanup, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { createRetainedPresence } from "@hewel/zgui/primitives";
    import type { RetainedPresenceController } from "@hewel/zgui/primitives";

    const [item, setItem] = createSignal<unknown>(null);
    export { setItem };
    export let rp!: RetainedPresenceController<unknown>;
    export const snapshots: Array<{ mounted: boolean; present: boolean; state: string; value: unknown }> = [];
    export let bodyMounts = 0;
    export let bodyCleanups = 0;

    function Body() {
      bodyMounts += 1;
      onCleanup(() => {
        bodyCleanups += 1;
      });
      return <box testId="retained-body" />;
    }

    export function mount(root: NativeNode) {
      return renderApp(() => {
        rp = createRetainedPresence(item, { settled: undefined });
        createEffect(
          () => ({
            mounted: rp.mounted(),
            present: rp.present(),
            state: rp.state(),
            value: rp.value(),
          }),
          snap => {
            snapshots.push(snap);
          },
        );
        return (
          <Show when={rp.mounted()}>
            <box
              testId="retained-surface"
              data-state={rp.state()}
              ref={rp.ref}
              onAnimationEnd={rp.onEnd}
              onTransitionEnd={rp.onEnd}
            >
              <Body />
            </box>
          </Show>
        );
      }, root);
    }
  `,
  );
  globalThis.__zgui = harness.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  dispose = mod.mount(harness.makeNode("root"));

  // Initially null: zero timers/frames/queries.
  assert.equal(mod.rp.present(), false);
  assert.equal(mod.rp.state(), "closed");
  assert.equal(mod.rp.mounted(), false);
  assert.equal(mod.rp.value(), undefined);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(harness.animatingQueries.length, 0);

  // Absent -> absent (null -> undefined) starts no exit/timers/frames.
  harness.runTurn(() => mod.setItem(undefined));
  assert.equal(mod.rp.mounted(), false);
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);

  // Non-nullish falsy values (0, "", false) are all present!
  for (const falsy of [0, "", false]) {
    harness.runTurn(() => mod.setItem(falsy));
    assert.equal(mod.rp.present(), true, `expected ${JSON.stringify(falsy)} to be present`);
    assert.equal(mod.rp.state(), "open");
    assert.equal(mod.rp.mounted(), true);
    assert.equal(mod.rp.value(), falsy);
  }
  assert.equal(mod.bodyMounts, 1, "A -> B updates value without exiting or remounting");

  // Callable/function payload is stored as data, never invoked as a Solid updater.
  let fnInvocations = 0;
  const payloadFn = () => {
    fnInvocations += 1;
    return "should-not-run";
  };
  harness.runTurn(() => mod.setItem(() => payloadFn));
  assert.equal(mod.rp.value(), payloadFn);
  assert.equal(fnInvocations, 0, "function payload must not be invoked when live");
  harness.runTurn(() => mod.setItem(null));
  assert.equal(mod.rp.mounted(), true);
  assert.equal(mod.rp.value(), payloadFn, "function payload retained during exit without invoking");
  assert.equal(fnInvocations, 0, "function payload must not be invoked while retained");
  harness.runTurn(() => mod.setItem(() => payloadFn));
  assert.equal(mod.rp.value(), payloadFn);
  assert.equal(fnInvocations, 0);

  // Transient Solid-batched absence (fn -> null -> "B" in same turn) starts no exit.
  harness.runTurn(() => {
    mod.setItem(null);
    mod.setItem("B");
  });
  assert.equal(mod.rp.present(), true);
  assert.equal(mod.rp.value(), "B");
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.bodyMounts, 1);

  // A -> absent retains payload until exit completion; exiting A -> C cancels old exit with zero intermediate undefined.
  harness.runTurn(() => mod.setItem(null));
  assert.equal(mod.rp.present(), false);
  assert.equal(mod.rp.state(), "closed");
  assert.equal(mod.rp.mounted(), true);
  assert.equal(mod.rp.value(), "B");
  assert.equal(clock.frames.size, 1);
  assert.equal(clock.timers.size, 1);

  const countBeforeReopen = mod.snapshots.length;
  harness.runTurn(() => mod.setItem("C"));
  assert.equal(mod.rp.present(), true);
  assert.equal(mod.rp.state(), "open");
  assert.equal(mod.rp.mounted(), true);
  assert.equal(mod.rp.value(), "C");
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.bodyMounts, 1, "reopen during exit never remounts");
  for (const snap of mod.snapshots.slice(countBeforeReopen)) {
    assert.notEqual(
      snap.value,
      undefined,
      "exiting A -> B must have no intermediate empty payload",
    );
  }

  // Exit completes while still absent: publishes mounted = false and value = undefined together.
  const countBeforeExit = mod.snapshots.length;
  harness.runTurn(() => mod.setItem(null));
  clock.frame();
  clock.advance(0);
  assert.equal(mod.rp.mounted(), false);
  assert.equal(mod.rp.value(), undefined);
  assert.equal(mod.bodyCleanups, 1);
  const exitSnap = mod.snapshots.at(-1);
  assert.ok(mod.snapshots.length > countBeforeExit);
  assert.equal(exitSnap.mounted, false);
  assert.equal(exitSnap.value, undefined, "mounted=false and value=undefined published together");
  assert.equal(clock.frames.size, 0);
  assert.equal(clock.timers.size, 0);
});

test("createRetainedPresence coordinates CSS + JS settled gates in both orders, protects slow springs past 1000ms, and prefers bound bridge rAF", async t => {
  const harness = createHost();
  const clock = createClock(t, harness);

  // Track bridge-specific rAF inside a shell LayerSurface to prove bound-node bridge preference.
  const bridgeFrames = new Map();
  let nextBridgeFrameId = 50_000;
  let globalRafCount = 0;
  const baseRaf = globalThis.requestAnimationFrame;
  globalThis.requestAnimationFrame = cb => {
    globalRafCount += 1;
    return baseRaf(cb);
  };
  const baseCancel = globalThis.cancelAnimationFrame;
  globalThis.cancelAnimationFrame = id => {
    bridgeFrames.delete(id);
    baseCancel(id);
  };
  t.after(() => {
    globalThis.requestAnimationFrame = baseRaf;
    globalThis.cancelAnimationFrame = baseCancel;
  });

  const origOpenLayer = harness.shell.openLayer.bind(harness.shell);
  harness.shell.openLayer = (name, placement, attach) =>
    origOpenLayer(name, placement, (root, bridge) => {
      bridge.requestAnimationFrame = function requestAnimationFrame(cb) {
        assert.equal(
          this,
          bridge,
          "bound bridge requestAnimationFrame must receive bridge as this",
        );
        const id = ++nextBridgeFrameId;
        bridgeFrames.set(id, cb);
        return id;
      };
      return attach(root, bridge);
    });

  const mod = await moduleFor(
    t,
    `
    import { LayerSurface, Show, createSignal, createSpring, renderShell } from "@hewel/zgui";
    import type { Spring } from "@hewel/zgui";
    import { createRetainedPresence } from "@hewel/zgui/primitives";
    import type { RetainedPresenceController } from "@hewel/zgui/primitives";

    const [item, setItem] = createSignal<string | null>(null);
    const [manualSettled, setManualSettled] = createSignal(true);
    const [useManualGate, setUseManualGate] = createSignal(false);
    export { setItem, setManualSettled, setUseManualGate };
    export let rp!: RetainedPresenceController<string>;
    export let spring!: Spring;

    function Panel() {
      // Very slow spring (low stiffness, high damping/mass) that runs well past 1000ms.
      spring = createSpring(() => (item() !== null ? 1 : 0), {
        stiffness: 18,
        damping: 12,
        mass: 2,
      });
      rp = createRetainedPresence(item, {
        settled: () => (useManualGate() ? manualSettled() : spring.settled()),
      });
      return (
        <Show when={rp.mounted()}>
          <box
            testId="dual-surface"
            data-state={rp.state()}
            ref={rp.ref}
            onAnimationEnd={rp.onEnd}
            onTransitionEnd={rp.onEnd}
          >
            <box testId="dual-child" />
          </box>
        </Show>
      );
    }

    export function mount() {
      return renderShell(() => (
        <LayerSurface name="motion-layer">
          <Panel />
        </LayerSurface>
      ));
    }
  `,
  );

  const prevShell = globalThis.__zguiShell;
  globalThis.__zguiShell = harness.shell;
  let dispose;
  t.after(() => {
    try {
      if (dispose) harness.runTurn(dispose);
    } finally {
      if (prevShell === undefined) delete globalThis.__zguiShell;
      else globalThis.__zguiShell = prevShell;
    }
  });

  dispose = harness.runTurn(() => mod.mount());
  harness.flush();

  const runBridgeFrames = (now = 16) => {
    harness.flush();
    const due = [...bridgeFrames.entries()];
    for (const [id] of due) bridgeFrames.delete(id);
    for (const [, cb] of due) harness.runTurn(() => cb(now));
  };

  // 1) Open panel, let spring converge, then close while CSS is busy.
  harness.runTurn(() => mod.setItem("toast-1"));
  harness.flush();
  let tMs = 0;
  for (let i = 0; i < 600 && bridgeFrames.size; i += 1) {
    tMs += 16;
    runBridgeFrames(tMs);
  }
  assert.equal(mod.spring.settled(), true);
  assert.equal(globalRafCount, 0, "bound bridge rAF used instead of global rAF");

  const surface1 = harness.byTestId("dual-surface");
  assert.ok(surface1);
  harness.animating.set(surface1, true);

  // Close: stale settled() === true on close turn must NOT unmount prematurely; slow spring starts.
  harness.runTurn(() => mod.setItem(null));
  assert.equal(mod.rp.mounted(), true);
  assert.equal(mod.rp.value(), "toast-1");
  assert.equal(globalRafCount, 0, "presence exit uses bound node bridge rAF");

  // Run styled frame + deferred check.
  tMs += 16;
  runBridgeFrames(tMs);
  clock.advance(0);
  assert.equal(mod.rp.mounted(), true, "both CSS and slow spring keep surface mounted");

  // CSS finishes BEFORE slow spring settles -> waits reactively for slow spring past 1000ms!
  harness.animating.set(surface1, false);
  // Descendant event ignored.
  harness.dispatch(harness.byTestId("dual-child"), "AnimationEnd");
  assert.equal(mod.rp.mounted(), true);
  // Own-node event satisfies CSS gate and clears 1000ms deadline, but JS spring is still running.
  harness.dispatch(surface1, "AnimationEnd");
  assert.equal(mod.rp.mounted(), true, "CSS finished first, must still wait for JS spring");
  assert.equal(clock.timers.size, 0, "CSS deadline cleared once CSS gate is satisfied");

  // Advance wall clock past 1,500ms while stepping slow spring partway: still mounted!
  clock.advance(1500);
  assert.equal(mod.rp.mounted(), true, "1000ms CSS deadline must never kill slow JS spring");

  // Finish slow spring: reactive settled() -> true unmounts & releases payload!
  for (let i = 0; i < 600 && mod.rp.mounted(); i += 1) {
    tMs += 16;
    runBridgeFrames(tMs);
  }
  assert.equal(mod.spring.settled(), true);
  assert.equal(mod.rp.mounted(), false);
  assert.equal(mod.rp.value(), undefined);

  // 2) Opposite order: JS settles BEFORE CSS finishes -> waits for CSS onEnd.
  harness.runTurn(() => {
    mod.setUseManualGate(true);
    mod.setManualSettled(false);
    mod.setItem("toast-2");
  });
  harness.flush();
  const surface2 = harness.byTestId("dual-surface");
  assert.ok(surface2);
  harness.animating.set(surface2, true);
  harness.runTurn(() => mod.setItem(null));
  tMs += 16;
  runBridgeFrames(tMs);
  clock.advance(0);
  assert.equal(mod.rp.mounted(), true);

  // JS settles first while CSS is still animating -> stays mounted!
  harness.runTurn(() => mod.setManualSettled(true));
  assert.equal(mod.rp.mounted(), true, "JS settled first, must still wait for CSS onEnd");

  // CSS finishes -> unmounts immediately!
  harness.animating.set(surface2, false);
  harness.dispatch(surface2, "TransitionEnd");
  assert.equal(mod.rp.mounted(), false);
  assert.equal(mod.rp.value(), undefined);

  // 3) No frame arrives + 1000ms deadline releases CSS gate & arms JS gate, but never overrides unsettled JS gate!
  harness.runTurn(() => {
    mod.setManualSettled(false);
    mod.setItem("toast-3");
  });
  harness.flush();
  harness.runTurn(() => mod.setItem(null));
  // Do NOT run rAF! Advance 1000ms so CSS deadline fires.
  clock.advance(1000);
  assert.equal(
    mod.rp.mounted(),
    true,
    "external unsettled JS gate retains past 1000ms CSS deadline",
  );
  assert.equal(mod.rp.value(), "toast-3");
  assert.equal(clock.timers.size, 0);
  assert.equal(bridgeFrames.size, 0, "CSS deadline cancelled the pending styled-frame rAF");

  // External gate that never settles retains until reopen (or disposal).
  harness.runTurn(() => mod.setItem("toast-4"));
  assert.equal(mod.rp.mounted(), true);
  assert.equal(mod.rp.value(), "toast-4");

  // Close with never-settling gate, then dispose: clears timers/frames & releases payload.
  harness.runTurn(() => mod.setItem(null));
  const stale = [
    ...bridgeFrames.values(),
    ...[...clock.timers.values()].map(timer => timer.callback),
  ];
  harness.runTurn(() => dispose());
  dispose = undefined;
  assert.equal(bridgeFrames.size, 0);
  assert.equal(clock.timers.size, 0);
  assert.equal(mod.rp.value(), undefined);
  for (const cb of stale) cb(9999);
  assert.equal(mod.rp.value(), undefined);
});

test("createRetainedPresence does not unmount prematurely on stale close-turn settled() === true and ignores replaced-ref onEnd", async t => {
  const harness = createHost();
  const clock = createClock(t, harness);
  const mod = await moduleFor(
    t,
    `
    import { Show, createSignal, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { createRetainedPresence } from "@hewel/zgui/primitives";
    import type { RetainedPresenceController } from "@hewel/zgui/primitives";

    const [item, setItem] = createSignal<string | null>("ready");
    const [settled, setSettled] = createSignal(true);
    export { setItem, setSettled };
    export let rp!: RetainedPresenceController<string>;

    export function mount(root: NativeNode) {
      return renderApp(() => {
        rp = createRetainedPresence(item, { settled });
        return (
          <Show when={rp.mounted()}>
            <box testId="stale-gate-surface" ref={rp.ref} onAnimationEnd={rp.onEnd} />
          </Show>
        );
      }, root);
    }
  `,
  );
  globalThis.__zgui = harness.host;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  dispose = mod.mount(harness.makeNode("root"));

  // Close while settled() is still true on the close turn (stale from previous target).
  harness.runTurn(() => mod.setItem(null));
  assert.equal(
    mod.rp.mounted(),
    true,
    "stale close-turn settled() === true must not unmount immediately",
  );
  assert.equal(mod.rp.value(), "ready");

  // During the styled frame, JS motion starts (settled flips to false) before setTimeout(0).
  clock.frame();
  harness.runTurn(() => mod.setSettled(false));
  clock.advance(0);
  assert.equal(mod.rp.mounted(), true, "JS gate armed after styled frame sees unsettled motion");
  assert.equal(mod.rp.value(), "ready");

  // Replaced ref: events from the old node do not satisfy the new node's CSS gate.
  const oldNode = harness.byTestId("stale-gate-surface");
  const replacement = harness.makeNode("box");
  harness.animating.set(replacement, true);
  harness.runTurn(() => mod.setItem("again"));
  mod.rp.ref(replacement);
  harness.runTurn(() => {
    mod.setSettled(true);
    mod.setItem(null);
  });
  clock.frame();
  clock.advance(0);
  assert.equal(mod.rp.mounted(), true);
  harness.animating.set(oldNode, false);
  mod.rp.onEnd({ target: oldNode });
  assert.equal(mod.rp.mounted(), true, "event from replaced ref must be ignored");
  harness.animating.set(replacement, false);
  mod.rp.onEnd({ target: replacement });
  assert.equal(mod.rp.mounted(), false);
  assert.equal(mod.rp.value(), undefined);
});
