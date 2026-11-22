import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import {
  For, LayerSurface, Show, createContext, createSignal, focus, getTextContent,
  mountStylesheet, onCleanup, precedes, renderShell, useContext, useMonitors,
} from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
const Theme = createContext("missing");
const [vertical, setVertical] = createSignal(false);
const [output, setOutput] = createSignal("DP-1");
const [count, setCount] = createSignal(0);
const [visible, setVisible] = createSignal(true);
const [linked, setLinked] = createSignal(true);
const [inputNode, setInputNode] = createSignal<NativeNode>();
let retainInputRef = false;
let disposed = 0;
export { inputNode, setVertical, setOutput, setVisible, setLinked };
export const retainInputNode = () => { retainInputRef = true; };
export const disposals = () => disposed;
export const read = (node: NativeNode) => getTextContent(node);
export const ordered = (first: NativeNode, second: NativeNode) => precedes(first, second);

function Panel(props: { name: string; ref?: (node: NativeNode) => void }) {
  const theme = useContext(Theme);
  onCleanup(() => { disposed++; });
  return <control testId={props.name} ref={props.ref} onClick={event => {
    focus(event.currentTarget);
    mountStylesheet("clicked", "control { opacity: .8; }");
    event.preventDefault();
    setCount(value => value + 1);
  }}>{theme}:{count()}</control>;
}

function Shell() {
  const monitors = useMonitors();
  return <>
    <LayerSurface name="panel" output={output()} layer={vertical() ? "overlay" : "top"}
      anchor={vertical() ? ["left", "top", "bottom"] : ["top", "left", "right"]}
      width={vertical() ? 28 : 0} height={vertical() ? 0 : 28}
      margin={vertical() ? { left: 4 } : { top: 2 }} exclusiveZone={28}
      keyboard={vertical() ? "on-demand" : "none"}
      inputRegion={linked() ? (inputNode() ? [inputNode()!] : []) : [{ x: 2, y: 4, width: 12, height: 10 }]}>
      <Show when={visible()}>
        <box><Panel name="counter" ref={node => {
          setInputNode(node);
          onCleanup(() => {
            if (!retainInputRef) setInputNode(current => current === node ? undefined : current);
          });
        }} /></box>
      </Show>
    </LayerSurface>
    <For each={monitors()} keyed={monitor => monitor.id}>{monitor =>
      <LayerSurface name={'overlay-' + monitor().id} output={monitor().id} inputRegion="empty">
        <Panel name={'counter:' + monitor().id} />
        <text testId={'geometry:' + monitor().id}>{monitor().name}:{monitor().scale}:{monitor().refreshRateMilliHz}</text>
      </LayerSurface>
    }</For>
  </>;
}

export function mount() {
  return renderShell(() => <Theme value="shared"><Shell /></Theme>);
}
`;

const monitor = (id, name, x) => ({
  id,
  name,
  x,
  y: 0,
  width: 1920,
  height: 1080,
  scale: 2,
  refreshRateMilliHz: 60000,
});

async function mounted(
  t,
  { attach = true, source = fixture, monitors = [monitor("DP-1", "Desk", 0)], sessionLock } = {},
) {
  const harness = createHost({ monitors, sessionLock });
  const mod = await moduleFor(t, source);
  const previous = globalThis.__zguiShell;
  globalThis.__zguiShell = harness.shell;
  let dispose;
  t.after(() => {
    try {
      dispose?.();
      harness.flush();
    } finally {
      if (previous === undefined) delete globalThis.__zguiShell;
      else globalThis.__zguiShell = previous;
    }
  });
  dispose = harness.runTurn(mod.mount);
  assert.equal(harness.surfaces.size, 0, "shell opens attach asynchronously");
  if (attach) harness.flush();
  return {
    harness,
    mod,
    stop: () => {
      dispose();
      dispose = undefined;
      harness.flush();
    },
  };
}

const textOf = (harness, testId) => {
  const node = harness.byTestId(testId);
  assert.ok(node, `missing ${testId}`);
  return node.children.map(child => child.props.get("text")).join("");
};

test("pure-shell mount shares state and routes events across output hotplug", async t => {
  const { harness, mod, stop } = await mounted(t);
  const panel = harness.byTestId("counter");
  const panelId = harness.surfaces.get("panel").id;
  const first = harness.byTestId("counter:DP-1");
  assert.equal(textOf(harness, "geometry:DP-1"), "Desk:2:60000");
  assert.equal(textOf(harness, "counter"), "shared:0");
  const event = harness.dispatch(first, "click");
  assert.equal(event.defaultPrevented, true);
  assert.equal(harness.focused.current, first);
  assert.equal(harness.surfaces.get("overlay-DP-1").region.stylesheets.has("clicked"), true);
  assert.equal(harness.surfaces.get("panel").region.stylesheets.has("clicked"), false);
  assert.equal(textOf(harness, "counter"), "shared:1");
  assert.equal(textOf(harness, "counter:DP-1"), "shared:1");
  assert.throws(() => harness.runTurn(() => mod.ordered(panel, first)), /foreign mount region/);

  harness.emitMonitors([
    { ...monitor("DP-1", "Desk", 0), scale: 1.5 },
    monitor("HDMI-1", "Side", 1920),
  ]);
  harness.flush();
  assert.equal(
    harness.byTestId("counter:DP-1"),
    first,
    "geometry changes preserve the keyed surface",
  );
  assert.equal(harness.byTestId("counter"), panel);
  assert.equal(textOf(harness, "geometry:DP-1"), "Desk:1.5:60000");
  assert.equal(textOf(harness, "counter:HDMI-1"), "shared:1");

  harness.emitMonitors([monitor("HDMI-1", "Side", 1920)]);
  harness.flush();
  assert.equal(harness.byTestId("counter"), undefined);
  assert.equal(harness.byTestId("counter:DP-1"), undefined);
  assert.equal(textOf(harness, "counter:HDMI-1"), "shared:1");
  assert.throws(() => harness.runTurn(() => mod.read(first)), /stale detached/);
  assert.throws(() => harness.runTurn(() => mod.read(panel)), /stale detached/);
  assert.equal(mod.disposals(), 2);

  harness.emitMonitors([monitor("DP-1", "Desk", 0), monitor("HDMI-1", "Side", 1920)]);
  harness.flush();
  assert.equal(harness.surfaces.get("panel").id, panelId, "the retained request reattaches");
  assert.notEqual(harness.byTestId("counter"), panel);
  assert.equal(textOf(harness, "counter"), "shared:1");

  stop();
  assert.equal(harness.surfaces.size, 0);
  assert.equal(harness.monitorSubscribers.size, 0);
  assert.equal(mod.disposals(), 5);
  assert.equal(
    harness.geometry.some(record => record.active),
    false,
  );
});

test("the first-output layer retargets before host reconciliation disposes its old region", async t => {
  const { harness, mod } = await mounted(t, {
    monitors: [desk, side],
    source: `
    import { LayerSurface, onCleanup, renderShell, useMonitors } from "@hewel/zgui";
    export const retired = [];
    function Content(props) {
      const output = props.output;
      onCleanup(() => retired.push(output));
      return <text testId="overlay-content">{output}</text>;
    }
    export function mount() {
      return renderShell(() => {
        const monitors = useMonitors();
        return <LayerSurface name="overlay" output={monitors()[0]?.id}>
          <Content output={monitors()[0]?.id} />
        </LayerSurface>;
      });
    }
  `,
  });
  const old = harness.surfaces.get("overlay");
  const oldContent = harness.byTestId("overlay-content");
  assert.equal(old.placement.output, desk.id);
  assert.equal(textOf(harness, "overlay-content"), desk.id);

  harness.emitMonitors([side]);
  assert.deepEqual(mod.retired, [desk.id]);
  assert.equal(harness.invalidated.has(oldContent), true);
  assert.equal(harness.shellLog.findLast(event => event.op === "open").placement.output, side.id);
  harness.flush();
  assert.notEqual(harness.surfaces.get("overlay").id, old.id);
  assert.equal(textOf(harness, "overlay-content"), side.id);
});

test("monitor subscribers see one snapshot without replacing an unchanged layer", async t => {
  const { harness, mod, stop } = await mounted(t, {
    monitors: [desk],
    source: `
    import { LayerSurface, Show, createEffect, renderShell, useMonitors } from "@hewel/zgui";
    export const observed = [];
    export function mount() {
      return renderShell(() => {
        const first = useMonitors();
        const second = useMonitors();
        const names = () => [first()[0]?.name, second()[0]?.name];
        createEffect(names, pair => { observed.push(pair); });
        return <Show when={first()[0]?.name === second()[0]?.name}>
          <LayerSurface name="stable" output="DP-1" />
        </Show>;
      });
    }
  `,
  });
  const { id } = harness.surfaces.get("stable");
  harness.emitMonitors([{ ...desk, name: "Renamed" }]);
  assert.deepEqual(mod.observed, [
    ["Desk", "Desk"],
    ["Renamed", "Renamed"],
  ]);
  harness.flush();
  assert.equal(harness.surfaces.get("stable").id, id);
  stop();
  assert.equal(harness.monitorSubscribers.size, 0);
});

test("host popup dismissal updates the parent and opens its replacement in the same turn", async t => {
  const { harness, mod } = await mounted(t, {
    source: `
    import { LayerSurface, PopupSurface, Show, createSignal, onCleanup, renderShell } from "@hewel/zgui";
    const [queued, setQueued] = createSignal(false);
    export const queueUpdate = () => setQueued(true);
    let retire;
    export function onRetire(callback) { retire = callback; }
    function Menu() {
      onCleanup(() => queueMicrotask(() => retire()));
      return <text>menu</text>;
    }
    export function mount() {
      return renderShell(() => {
        const [dismissed, setDismissed] = createSignal(false);
        return <>
          <LayerSurface name="parent"><text testId="status">{dismissed() ? "dismissed" : "open"}</text></LayerSurface>
          <PopupSurface name="menu" parent="parent" width={20} height={20}
            anchorRect={{ x: 0, y: 0, width: 10, height: 10 }} onDismiss={() => setDismissed(true)}>
            <Menu />
          </PopupSurface>
          <Show when={queued()}><LayerSurface name="queued" /></Show>
          <Show when={dismissed()}><LayerSurface name="replacement" /></Show>
        </>;
      });
    }
  `,
  });
  assert.equal(textOf(harness, "status"), "open");
  let rejected = 0;
  mod.onRetire(() => {
    for (const open of [
      () => harness.shell.openLayer("forbidden-layer", {}, () => {}),
      () =>
        harness.shell.openPopup(
          "forbidden-popup",
          { parent: "parent" },
          () => {},
          () => {},
        ),
    ]) {
      assert.throws(open, /shell work is unavailable during disposal/);
      rejected++;
    }
  });
  harness.runTurn(mod.queueUpdate, false);
  assert.equal(
    harness.shellLog.some(event => event.name === "queued"),
    false,
  );
  harness.popupDone("menu");
  assert.equal(harness.surfaces.has("menu"), false);
  assert.equal(textOf(harness, "status"), "dismissed");
  assert.equal(rejected, 2);
  harness.flush();
  assert.equal(harness.surfaces.has("replacement"), true);
  assert.equal(harness.surfaces.has("queued"), true);
});

test("host lock state transitions are observable before the job checkpoint", async t => {
  const { harness, mod } = await mounted(t, {
    sessionLock: "locked",
    source: `
    import { SessionLockSurface, createEffect, renderShell, requestSessionLock } from "@hewel/zgui";
    let lock;
    export const observed = [];
    export const unlock = () => lock.unlock();
    export function mount() {
      return renderShell(() => {
        lock = requestSessionLock();
        createEffect(lock.state, state => { observed.push(state); });
        return <SessionLockSurface name="locker" output="DP-1"><text testId="locked">{lock.state()}</text></SessionLockSurface>;
      });
    }
  `,
  });
  assert.deepEqual(mod.observed, ["unlocked", "pending", "locked"]);
  assert.equal(textOf(harness, "locked"), "locked");
  harness.runTurn(mod.unlock);
  harness.flush();
  assert.deepEqual(mod.observed, ["unlocked", "pending", "locked", "unlocked"]);
  assert.equal(harness.byTestId("locked"), undefined);
});

test("region cleanup releases its own resources without flushing an outer effect or admitting direct shell work", async t => {
  const { harness, mod } = await mounted(t, {
    monitors: [desk],
    source: `
    import { LayerSurface, createEffect, createSignal, lockScroll, mountStylesheet, onCleanup, renderShell } from "@hewel/zgui";
    export const observed = [];
    let attemptCreation = () => {};
    export function onRetire(callback) { attemptCreation = callback; }
    let inspectLease;
    export function onLeaseRelease(callback) { inspectLease = callback; }
    function Content(props) {
      const [node, setNode] = createSignal();
      createEffect(node, target => {
        if (!target) return;
        const release = lockScroll(target);
        return () => { release(); props.leaseReleased(); };
      });
      mountStylesheet("local", "text { opacity: .8; }");
      onCleanup(() => {
        attemptCreation();
        props.retired();
      });
      return <text testId="retiring" ref={setNode}>retiring</text>;
    }
    export function mount() {
      return renderShell(() => {
        const [state, setState] = createSignal("mounted");
        createEffect(state, value => { observed.push(value); });
        return <LayerSurface name="retiring" output="DP-1">
          <Content retired={() => setState("retired")} leaseReleased={() => inspectLease()} />
        </LayerSurface>;
      });
    }
  `,
  });
  const { region: retiring, root } = harness.surfaces.get("retiring");
  const lease = harness.locks.find(record => record.node === harness.byTestId("retiring"));
  assert.equal(lease.active, true);
  const released = [];
  mod.onLeaseRelease(() => released.push(lease.active));
  assert.equal(retiring.stylesheets.has("local"), true);
  const failures = [];
  mod.onRetire(() => {
    for (const open of [
      () => harness.shell.openLayer("forbidden", {}, () => {}),
      () =>
        harness.shell.openPopup(
          "forbidden",
          { parent: "retiring" },
          () => {},
          () => {},
        ),
    ]) {
      try {
        open();
      } catch (error) {
        failures.push(error.message);
      }
    }
  });
  harness.emitMonitors([]);
  assert.deepEqual(failures, Array(2).fill("shell work is unavailable during disposal"));
  assert.equal(harness.byTestId("retiring"), undefined);
  assert.equal(retiring.stylesheets.size, 0);
  assert.ok(harness.clearCalls.includes(root), "the disposer must clear the retiring root");
  assert.deepEqual(
    released,
    [false],
    "the effect's returned cleanup must release the lease before forced teardown",
  );
  assert.deepEqual(mod.observed, ["mounted"]);
  harness.flush();
  assert.deepEqual(mod.observed, ["mounted", "retired"]);
});

test("live placement preserves the mount region and updates layout-driven input rectangles", async t => {
  const { harness, mod } = await mounted(t);
  const surface = harness.surfaces.get("panel");
  const panel = harness.byTestId("counter");
  const observer = harness.geometry.find(
    record => record.node === panel && record.kind === "borderBox",
  );
  assert.ok(observer, "input nodes are observed in the surface's mount region");
  assert.deepEqual(surface.inputRegion, []);
  const rect = { x: 10, y: 4, width: 24, height: 12 };
  observer.emit(rect);
  harness.flush();
  assert.deepEqual(surface.inputRegion, [rect]);

  mod.setVertical(true);
  harness.flush();
  assert.equal(harness.surfaces.get("panel"), surface);
  assert.equal(harness.byTestId("counter"), panel);
  assert.deepEqual(surface.placement, {
    output: "DP-1",
    layer: "overlay",
    anchor: ["left", "top", "bottom"],
    width: 28,
    height: 0,
    margin: { top: 0, right: 0, bottom: 0, left: 4 },
    exclusiveZone: 28,
    keyboard: "on-demand",
  });
  assert.ok(harness.shellLog.some(entry => entry.op === "update" && entry.id === surface.id));

  mod.setVisible(false);
  harness.flush();
  assert.equal(observer.active, false);
  assert.deepEqual(surface.inputRegion, []);
  mod.setLinked(false);
  harness.flush();
  assert.deepEqual(surface.inputRegion, [{ x: 2, y: 4, width: 12, height: 10 }]);

  mod.setVisible(true);
  harness.flush();
  const oldNode = harness.byTestId("counter");
  harness.emitMonitors([monitor("DP-1", "Desk", 0), monitor("HDMI-1", "Side", 1920)]);
  mod.setOutput("HDMI-1");
  harness.flush();
  const replacement = harness.surfaces.get("panel");
  assert.notEqual(replacement, surface);
  assert.equal(replacement.placement.output, "HDMI-1");
  assert.notEqual(harness.byTestId("counter"), oldNode);
  assert.throws(() => harness.runTurn(() => mod.read(oldNode)), /stale detached/);
});

test("a retained input node stops intercepting when its branch detaches", async t => {
  const { harness, mod } = await mounted(t);
  const surface = harness.surfaces.get("panel");
  const panel = harness.byTestId("counter");
  const observer = harness.geometry.find(
    record => record.node === panel && record.kind === "borderBox",
  );
  assert.ok(observer);
  const rect = { x: 10, y: 4, width: 24, height: 12 };
  observer.emit(rect);
  assert.deepEqual(surface.inputRegion, [rect]);

  harness.runTurn(() => {
    mod.retainInputNode();
    mod.setVisible(false);
  });
  assert.equal(mod.inputNode(), panel, "the input-region ref was not cleared by the app");
  assert.equal(harness.byTestId("counter"), undefined);
  assert.deepEqual(surface.inputRegion, [], "no stale hit rectangle at the end of the detach turn");
});

test("disposing before an asynchronous attach cancels the pending surface", async t => {
  const { harness, mod, stop } = await mounted(t, { attach: false });
  stop();
  assert.equal(harness.surfaces.size, 0);
  assert.equal(harness.monitorSubscribers.size, 0);
  assert.equal(mod.disposals(), 0);
  const opened = harness.shellLog.filter(entry => entry.op === "open").map(entry => entry.id);
  const closed = harness.shellLog.filter(entry => entry.op === "close").map(entry => entry.id);
  assert.equal(opened.length, 2);
  assert.deepEqual(new Set(closed), new Set(opened));
});

const stylesFixture = `
import {
  For, LayerSurface, Show, createEffect, createSignal, getOwner,
  mountShellStylesheet, mountStylesheet, renderShell, useMonitors,
} from "@hewel/zgui";
import type { Owner } from "@hewel/zgui";

const [effectEnabled, setEffectEnabled] = createSignal(false);
const [effectCss, setEffectCss] = createSignal(".panel { color: purple; }");
let effectOwner: Owner | null | undefined;
export { mountShellStylesheet, setEffectEnabled, setEffectCss };
export const getEffectOwner = () => effectOwner;

function ThemeEffect() {
  createEffect(effectCss, css => {
    effectOwner = getOwner();
    return mountShellStylesheet("effect-theme", css);
  });
  return null;
}

export function mount() {
  return renderShell(() => {
    const monitors = useMonitors();
    return <>
      <Show when={effectEnabled()}><ThemeEffect /></Show>
      <For each={monitors()} keyed={monitor => monitor.id}>{monitor =>
        <LayerSurface name={'styled-' + monitor().id} output={monitor().id}>
          <box class="panel" testId={'styled:' + monitor().id} onClick={() => {
            mountStylesheet("theme", ".panel { color: orange; }");
          }} />
        </LayerSurface>
      }</For>
    </>;
  });
}
`;

const desk = monitor("DP-1", "Desk", 0);
const side = monitor("HDMI-1", "Side", 1920);
const late = monitor("USB-C-1", "Late", 3840);
const sheetTexts = (harness, output) => {
  const surface = harness.surfaces.get(`styled-${output}`);
  assert.ok(surface, `missing surface for ${output}`);
  return [...surface.region.stylesheets.values()];
};

test("mountShellStylesheet replaces in place and reaches late and reattached surfaces", async t => {
  const { harness, mod } = await mounted(t, { source: stylesFixture, monitors: [desk, side] });
  assert.equal(globalThis.__zgui, undefined);
  const red = ".panel { color: red; }";
  const blue = ".panel { color: blue; }";
  const green = ".panel { color: green; }";
  const override = ".panel { color: gray; }";
  const deskNode = harness.byTestId("styled:DP-1");
  const sideNode = harness.byTestId("styled:HDMI-1");
  assert.ok(deskNode);
  assert.ok(sideNode);

  const disposeRed = harness.runTurn(() => mod.mountShellStylesheet("theme", red));
  const disposeOverride = harness.runTurn(() => mod.mountShellStylesheet("override", override));
  assert.deepEqual(sheetTexts(harness, desk.id), [red, override]);
  assert.deepEqual(sheetTexts(harness, side.id), [red, override]);

  const disposeBlue = harness.runTurn(() => mod.mountShellStylesheet("theme", blue));
  assert.deepEqual(
    sheetTexts(harness, desk.id),
    [blue, override],
    "replacement preserves cascade order",
  );
  assert.deepEqual(sheetTexts(harness, side.id), [blue, override]);
  assert.equal(harness.byTestId("styled:DP-1"), deskNode);
  assert.equal(harness.byTestId("styled:HDMI-1"), sideNode);
  harness.runTurn(disposeRed);
  assert.deepEqual(
    sheetTexts(harness, side.id),
    [blue, override],
    "old disposer cannot remove the replacement",
  );

  harness.emitMonitors([desk, side, late]);
  harness.flush();
  assert.deepEqual(sheetTexts(harness, late.id), [blue, override]);

  harness.emitMonitors([side, late]);
  harness.flush();
  assert.equal(harness.byTestId("styled:DP-1"), undefined);
  const disposeGreen = harness.runTurn(() => mod.mountShellStylesheet("theme", green));
  harness.runTurn(disposeBlue);
  harness.emitMonitors([desk, side, late]);
  harness.flush();
  assert.ok(harness.byTestId("styled:DP-1"));
  assert.notEqual(harness.byTestId("styled:DP-1"), deskNode);
  assert.equal(harness.byTestId("styled:HDMI-1"), sideNode);
  for (const output of [desk, side, late])
    assert.deepEqual(sheetTexts(harness, output.id), [green, override]);

  harness.runTurn(disposeGreen);
  for (const output of [desk, side, late])
    assert.deepEqual(sheetTexts(harness, output.id), [override]);
  harness.runTurn(disposeOverride);
  for (const output of [desk, side, late]) assert.deepEqual(sheetTexts(harness, output.id), []);

  harness.emitMonitors([side, late]);
  harness.flush();
  harness.emitMonitors([desk, side, late]);
  harness.flush();
  assert.deepEqual(sheetTexts(harness, desk.id), [], "disposed sheets are not inherited on attach");
});

test("mountShellStylesheet isolates same-named region-local sheets in either install order", async t => {
  const { harness, mod } = await mounted(t, { source: stylesFixture, monitors: [desk, side] });
  const local = ".panel { color: orange; }";
  const shell = ".panel { color: teal; }";
  const replacement = ".panel { color: navy; }";

  harness.dispatch(harness.byTestId("styled:DP-1"), "click");
  assert.deepEqual(sheetTexts(harness, desk.id), [local]);
  assert.deepEqual(sheetTexts(harness, side.id), []);
  harness.runTurn(() => mod.mountShellStylesheet("theme", shell));
  assert.deepEqual(sheetTexts(harness, side.id), [shell]);
  harness.dispatch(harness.byTestId("styled:HDMI-1"), "click");
  for (const output of [desk, side])
    assert.deepEqual(sheetTexts(harness, output.id).sort(), [local, shell]);

  const dispose = harness.runTurn(() => mod.mountShellStylesheet("theme", replacement));
  for (const output of [desk, side])
    assert.deepEqual(sheetTexts(harness, output.id).sort(), [replacement, local]);
  harness.runTurn(dispose);
  for (const output of [desk, side]) assert.deepEqual(sheetTexts(harness, output.id), [local]);
});

test("mountShellStylesheet works in an ownerless Solid createEffect apply callback", async t => {
  const { harness, mod } = await mounted(t, { attach: false, source: stylesFixture });
  assert.equal(globalThis.__zgui, undefined);
  harness.runTurn(() => mod.setEffectEnabled(true));
  assert.equal(mod.getEffectOwner(), null);
  harness.flush();
  const node = harness.byTestId("styled:DP-1");
  assert.ok(node);
  assert.deepEqual(sheetTexts(harness, desk.id), [".panel { color: purple; }"]);

  const css = ".panel { color: gold; }";
  harness.runTurn(() => mod.setEffectCss(css));
  assert.deepEqual(sheetTexts(harness, desk.id), [css]);
  harness.runTurn(() => mod.setEffectEnabled(false));
  assert.deepEqual(sheetTexts(harness, desk.id), []);
  assert.equal(harness.byTestId("styled:DP-1"), node);
});
