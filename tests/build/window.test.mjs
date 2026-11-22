import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { PopupSurface, Show, WindowSurface, createSignal, onCleanup, renderShell } from "@hewel/zgui";
import type { NativeNode, WindowDecorations } from "@hewel/zgui";

const [open, setOpen] = createSignal(true);
const [title, setTitle] = createSignal("Settings");
const [width, setWidth] = createSignal(820);
const [decorations, setDecorations] = createSignal<WindowDecorations>("full");
const [transparent, setTransparent] = createSignal(false);
const [anchor, setAnchor] = createSignal<NativeNode>();
const closed: string[] = [];
const cleaned: string[] = [];
export { setOpen, setTitle, setWidth, setDecorations, setTransparent, closed, cleaned };

function Tracked(props: { name: string }) {
  onCleanup(() => cleaned.push(props.name));
  return <text testId={props.name}>{props.name}</text>;
}

export function mount() {
  return renderShell(() => <>
    <Show when={open()}>
      <WindowSurface
        name="settings"
        title={title()}
        appId="z-shell-settings"
        width={width()}
        height={600}
        minWidth={640}
        minHeight={480}
        maxWidth={1440}
        maxHeight={1080}
        resizable
        decorations={decorations()}
        transparent={transparent()}
        onClose={() => closed.push("settings")}
      >
        <Tracked name="settings-body" />
        <box testId="window-anchor" ref={node => {
          setAnchor(node);
          onCleanup(() => setAnchor(current => current === node ? undefined : current));
        }} />
      </WindowSurface>
    </Show>
    <PopupSurface
      name="settings-menu"
      parent="settings"
      anchorNode={anchor()}
      width={240}
      height="auto"
      minHeight={48}
      maxHeight={420}
    >
      <Tracked name="menu-body" />
    </PopupSurface>
  </>);
}
`;

async function setup(t, options) {
  const mod = await moduleFor(t, fixture);
  const harness = createHost(options);
  const previous = globalThis.__zguiShell;
  globalThis.__zguiShell = harness.shell;
  let dispose;
  const stop = () => {
    if (dispose) harness.runTurn(dispose);
    dispose = undefined;
    harness.flush();
  };
  t.after(() => {
    try {
      stop();
    } finally {
      if (previous === undefined) delete globalThis.__zguiShell;
      else globalThis.__zguiShell = previous;
    }
  });
  const mount = () => {
    dispose = harness.runTurn(() => mod.mount());
  };
  return { harness, mod, mount, stop };
}

test("WindowSurface opens, updates in place, hosts node-anchored auto-height popups, and handles compositor close", async t => {
  const { harness, mod, mount, stop } = await setup(t);
  mount();
  harness.flush();
  assert.deepEqual([...harness.surfaces.keys()], ["settings"]);
  const win = harness.surfaces.get("settings");
  const body = harness.byTestId("settings-body");
  assert.ok(body);
  assert.deepEqual(win.options, {
    title: "Settings",
    appId: "z-shell-settings",
    width: 820,
    height: 600,
    minWidth: 640,
    minHeight: 480,
    maxWidth: 1440,
    maxHeight: 1080,
    resizable: true,
    decorations: "full",
    transparent: false,
  });

  // Child PopupSurface anchors to a NativeNode inside WindowSurface via popupParentRegions.
  const anchorNode = harness.byTestId("window-anchor");
  const observer = harness.geometry.find(
    record => record.active && record.node === anchorNode && record.kind === "borderBox",
  );
  assert.ok(observer);
  observer.emit({ x: 20, y: 30, width: 100, height: 28 });
  harness.flush();
  assert.deepEqual([...harness.surfaces.keys()], ["settings", "settings-menu"]);
  assert.equal(harness.surfaces.get("settings-menu").placement.height, "auto");
  assert.equal(harness.surfaces.get("settings-menu").placement.minHeight, 48);
  assert.equal(harness.surfaces.get("settings-menu").placement.maxHeight, 420);

  // Reactive title/width/decorations update in place without remounting.
  harness.runTurn(() => {
    mod.setTitle("Settings — Audio");
    mod.setWidth(900);
    mod.setDecorations("no-title-bar");
  });
  harness.flush();
  assert.equal(harness.surfaces.get("settings"), win);
  assert.equal(harness.byTestId("settings-body"), body);
  assert.equal(win.options.title, "Settings — Audio");
  assert.equal(win.options.width, 900);
  assert.equal(win.options.decorations, "no-title-bar");

  // Changing transparent reopens cleanly.
  harness.runTurn(() => mod.setTransparent(true));
  harness.flush();
  const reopened = harness.surfaces.get("settings");
  assert.notEqual(reopened, win);
  assert.equal(reopened.options.transparent, true);

  // Compositor close tears down child popup + window, fires onClose, and does not reopen on reactive updates.
  harness.windowClose("settings");
  harness.flush();
  assert.deepEqual(mod.closed, ["settings"]);
  assert.equal(harness.surfaces.size, 0);
  harness.runTurn(() => mod.setTitle("Ignored After Close"));
  harness.flush();
  assert.equal(harness.surfaces.size, 0);

  // Toggling Show remounts a fresh window.
  harness.runTurn(() => mod.setOpen(false));
  harness.runTurn(() => mod.setOpen(true));
  harness.flush();
  assert.deepEqual([...harness.surfaces.keys()], ["settings"]);
  stop();
  assert.equal(harness.surfaces.size, 0);
});

const appWindowControlsFixture = `
import {
  Show,
  WindowSurface,
  beginWindowMoveDrag,
  closeWindow,
  createSignal,
  getWindowState,
  onCloseRequest,
  renderApp,
  setWindowDecorations,
  setWindowFullscreen,
  setWindowMaximized,
  setWindowMinimized,
  setWindowTitle,
  useWindowState,
} from "@hewel/zgui";
import { ThemeProvider } from "@hewel/zgui/components";

const [dirty, setDirty] = createSignal(true);
const [confirmOpen, setConfirmOpen] = createSignal(false);
const [prefsOpen, setPrefsOpen] = createSignal(false);
const [prefsDirty, setPrefsDirty] = createSignal(true);
const [scheme, setScheme] = createSignal("dark");
export const log = [];

export { setDirty, setPrefsOpen, setPrefsDirty, setScheme };
export function updatePrimary() {
  setWindowTitle("Studio — Untitled*");
  setWindowFullscreen(true);
  setWindowMaximized(true);
  setWindowMinimized(false);
  setWindowDecorations("no-title-bar");
  beginWindowMoveDrag();
}
export function forceClosePrimary() {
  closeWindow({ force: true });
}
export function readPrimaryState() {
  return getWindowState();
}

export function mount(root) {
  return renderApp(() => {
    const winState = useWindowState();
    onCloseRequest(event => {
      log.push("primary-close-req");
      if (dirty()) {
        event.preventDefault();
        setConfirmOpen(true);
      }
    });
    return (
      <ThemeProvider scheme={scheme()}>
        <column testId="app-root">
          <text testId="win-title">{winState().title}</text>
          <text testId="confirm-state">{confirmOpen() ? "confirm-open" : "confirm-closed"}</text>
          <Show when={prefsOpen()}>
            <WindowSurface name="prefs" title="Preferences" width={420} height={300} onClose={() => log.push("prefs-closed")}>
              {(() => {
                onCloseRequest(event => {
                  log.push("prefs-close-req");
                  if (prefsDirty()) event.preventDefault();
                });
                return <text testId="prefs-content">Prefs</text>;
              })()}
            </WindowSurface>
          </Show>
        </column>
      </ThemeProvider>
    );
  }, root);
}
`;

test("renderApp primary window controls, close veto, force close, and auxiliary WindowSurface theme sync (#58)", async t => {
  const harness = createHost();
  const mod = await moduleFor(t, appWindowControlsFixture);
  globalThis.__zgui = harness.host;
  globalThis.__zguiShell = harness.shell;
  let stop;
  t.after(() => {
    try {
      stop?.();
    } finally {
      delete globalThis.__zgui;
      delete globalThis.__zguiShell;
    }
  });

  stop = harness.runTurn(() => mod.mount(harness.makeNode("root")));

  // Runtime window controls update state reactively and enqueue commands.
  harness.runTurn(() => mod.updatePrimary());
  assert.equal(harness.host.getTextContent(harness.byTestId("win-title")), "Studio — Untitled*");
  assert.deepEqual(mod.readPrimaryState(), {
    title: "Studio — Untitled*",
    fullscreen: true,
    maximized: true,
    minimized: false,
    decorations: "no-title-bar",
  });
  assert.ok(harness.windowCommands.some(c => c.op === "beginWindowMoveDrag"));

  // Synchronous onCloseRequest vetoes close and opens confirmation state.
  assert.equal(harness.requestClose("main"), false, "dirty guard vetoes primary close");
  assert.equal(harness.windowClosed("main"), false);
  assert.equal(harness.host.getTextContent(harness.byTestId("confirm-state")), "confirm-open");

  // Auxiliary WindowSurface inside renderApp inherits ThemeProvider tokens & has its own close veto.
  harness.runTurn(() => mod.setPrefsOpen(true));
  harness.flush();
  const prefs = harness.surfaces.get("prefs");
  assert.ok(prefs, "auxiliary WindowSurface opens inside renderApp");
  assert.ok(
    prefs.region.stylesheets.has("zui-theme-inherited"),
    "WindowSurface inherits ThemeProvider stylesheet",
  );
  const darkThemeCss = prefs.region.stylesheets.get("zui-theme-inherited");
  harness.runTurn(() => mod.setScheme("light"));
  harness.flush();
  assert.notEqual(
    prefs.region.stylesheets.get("zui-theme-inherited"),
    darkThemeCss,
    "theme updates propagate live to WindowSurface",
  );

  assert.equal(harness.requestClose("prefs"), false, "WindowSurface onCloseRequest vetoes close");
  assert.ok(harness.surfaces.has("prefs"), "vetoed WindowSurface stays open");

  harness.runTurn(() => mod.setPrefsDirty(false));
  assert.equal(harness.requestClose("prefs"), true, "unvetoed WindowSurface closes");
  harness.flush();
  assert.equal(harness.surfaces.has("prefs"), false);
  assert.ok(mod.log.includes("prefs-closed"));

  // Force close bypasses primary close guard.
  harness.runTurn(() => mod.forceClosePrimary());
  assert.equal(harness.windowClosed("main"), true);
  stop();
  stop = undefined;
});
