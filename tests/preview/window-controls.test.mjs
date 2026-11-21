import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  readFrame,
  readTree,
  runScenario,
} from "./harness.mjs";

const windowControlsPreviewSource = `
import {
  Button,
  Show,
  WindowSurface,
  beginWindowMoveDrag,
  closeWindow,
  createSignal,
  onCloseRequest,
  renderApp,
  setWindowDecorations,
  setWindowFullscreen,
  setWindowMaximized,
  setWindowMinimized,
  setWindowTitle,
  useWindowState,
} from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { ThemeProvider } from "@hewel/zgui/components";

export const hostApiVersion = 1;

function PrefsWindow(props: { readonly dirty: () => boolean; readonly sharedLabel: () => string }) {
  const [vetoed, setVetoed] = createSignal(0);
  onCloseRequest(event => {
    if (props.dirty()) {
      setVetoed(n => n + 1);
      event.preventDefault();
    }
  });
  return (
    <column style="width: 100%; height: 100%; padding: 12px; gap: 8px; background: var(--zui-color-background)">
      <text testId="prefs-shared">{props.sharedLabel()}</text>
      <text testId="prefs-veto-count">{String(vetoed())}</text>
      <Button testId="prefs-retitle" onClick={() => setWindowTitle("Prefs Updated")}>Retitle Prefs</Button>
      <Button testId="prefs-force-close" onClick={() => closeWindow({ force: true })}>Force Close Prefs</Button>
    </column>
  );
}

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [dirty, setDirty] = createSignal(true);
    const [confirmOpen, setConfirmOpen] = createSignal(false);
    const [prefsOpen, setPrefsOpen] = createSignal(false);
    const [prefsDirty, setPrefsDirty] = createSignal(true);
    const [prefsClosedCount, setPrefsClosedCount] = createSignal(0);
    const [sharedLabel, setSharedLabel] = createSignal("theme:dark");
    const [scheme, setScheme] = createSignal<"dark" | "light">("dark");
    const winState = useWindowState();

    onCloseRequest(event => {
      if (dirty()) {
        event.preventDefault();
        setConfirmOpen(true);
      }
    });

    return (
      <ThemeProvider scheme={scheme()}>
        <column style="width: 100%; height: 100%; padding: 12px; gap: 8px">
          <text testId="state-title">{winState().title}</text>
          <text testId="state-flags">
            {\`fs=\${winState().fullscreen},max=\${winState().maximized},min=\${winState().minimized},dec=\${winState().decorations}\`}
          </text>
          <text testId="confirm-status">{confirmOpen() ? "CONFIRM_DIALOG_OPEN" : "IDLE"}</text>
          <text testId="prefs-closed-count">{String(prefsClosedCount())}</text>
          <row style="gap: 6px; flex-wrap: wrap">
            <Button testId="btn-controls" onClick={() => {
              setWindowTitle("Markdown Studio — draft.md*");
              setWindowFullscreen(true);
              setWindowMaximized(true);
              setWindowMinimized(true);
              setWindowDecorations("no-title-bar");
              beginWindowMoveDrag();
            }}>Apply Controls</Button>
            <Button testId="btn-open-prefs" onClick={() => setPrefsOpen(true)}>Open Prefs</Button>
            <Button testId="btn-flip-theme" onClick={() => {
              setScheme("light");
              setSharedLabel("theme:light");
            }}>Flip Theme</Button>
            <Button testId="btn-force-close-main" onClick={() => closeWindow({ force: true })}>Discard & Close</Button>
            <Button testId="btn-clear-dirty" onClick={() => setDirty(false)}>Save</Button>
          </row>
          <Show when={prefsOpen()}>
            <WindowSurface
              name="prefs"
              title="Preferences"
              width={360}
              height={220}
              onClose={() => {
                setPrefsClosedCount(n => n + 1);
                setPrefsOpen(false);
              }}
            >
              <PrefsWindow dirty={prefsDirty} sharedLabel={sharedLabel} />
            </WindowSurface>
          </Show>
        </column>
      </ThemeProvider>
    );
  }, root);
}
`;

test("renderApp primary window controls, synchronous close veto, force close, and auxiliary WindowSurface work without shellSurfaces (#58)", async t => {
  const temp = await createTempWorkspace("preview-window-controls-");
  t.after(() => temp.cleanup());
  // Intentionally NO capabilities.shellSurfaces: renderApp gets auxiliary WindowSurface natively!
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "window-controls", windowControlsPreviewSource, {
    rawSource: true,
  });

  const click = (id, testId, surface = "main") => ({
    id,
    op: "pointer.click",
    button: "primary",
    target: { surface, testId },
  });
  const check = (id, testId, equals, surface = "main") => ({
    id,
    op: "assert",
    target: { surface, testId },
    check: { property: "text", equals },
  });

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "window-controls",
    requiredCapabilities: [],
    width: 640,
    height: 320,
    scale: 1,
    assertEmptyResources: true,
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      // 1. Primary window runtime controls update reactive JS state and OffscreenSurface state.
      click("apply-controls", "btn-controls"),
      { id: "settle-controls", op: "clock.advance", byMs: 32 },
      check("check-title", "state-title", "Markdown Studio — draft.md*"),
      check("check-flags", "state-flags", "fs=true,max=true,min=true,dec=no-title-bar"),
      { id: "cap-main-controls", op: "capture", captureId: "main-controls", surface: "main" },

      // 2. Primary window close request is vetoed synchronously -> confirmation UI mounts, window stays open.
      { id: "close-main-veto", op: "surface.closeRequest", surface: "main" },
      check("check-confirm-open", "confirm-status", "CONFIRM_DIALOG_OPEN"),

      // 3. Open auxiliary <WindowSurface> inside renderApp (without shellSurfaces or renderShell).
      click("open-prefs", "btn-open-prefs"),
      { id: "settle-prefs", op: "clock.advance", byMs: 32 },
      check("check-prefs-shared-dark", "prefs-shared", "theme:dark", "prefs"),
      click("flip-theme", "btn-flip-theme"),
      { id: "settle-theme", op: "clock.advance", byMs: 32 },
      check("check-prefs-shared-light", "prefs-shared", "theme:light", "prefs"),
      click("retitle-prefs", "prefs-retitle", "prefs"),
      { id: "settle-prefs-title", op: "clock.advance", byMs: 32 },
      { id: "cap-prefs", op: "capture", captureId: "prefs-window", surface: "prefs" },

      // 4. Auxiliary WindowSurface close request is vetoed synchronously (fixing prior close-before-JS bug!).
      { id: "close-prefs-veto", op: "surface.closeRequest", surface: "prefs" },
      check("check-prefs-veto-count", "prefs-veto-count", "1", "prefs"),

      // 5. Force-closing auxiliary WindowSurface bypasses its close guard and fires onClose.
      click("force-close-prefs", "prefs-force-close", "prefs"),
      { id: "settle-prefs-closed", op: "clock.advance", byMs: 32 },
      check("check-prefs-closed-count", "prefs-closed-count", "1"),

      // 6. Force-closing primary window bypasses its close guard cleanly.
      click("force-close-main", "btn-force-close-main"),
    ],
  });

  const mainFrame = await readFrame(out, "main-controls");
  assert.deepEqual(mainFrame.window, {
    title: "Markdown Studio — draft.md*",
    fullscreen: true,
    maximized: true,
    minimized: true,
    decorations: "no-title-bar",
    moveDragCount: 1,
  });

  const prefsFrame = await readFrame(out, "surfaces/prefs/prefs-window");
  assert.equal(prefsFrame.window.title, "Prefs Updated");
  const prefsTree = await readTree(out, "surfaces/prefs/prefs-window");
  assert.equal(getNode(prefsTree, "prefs-shared").text, "theme:light");

  const trace = (await readFile(path.join(out, "steps.jsonl"), "utf8"))
    .trim()
    .split("\n")
    .map(JSON.parse);
  const mainCloseStep = trace.find(row => row.id === "close-main-veto");
  assert.equal(mainCloseStep.result.vetoed, true);
  assert.equal(mainCloseStep.result.closed, false);
  const prefsCloseStep = trace.find(row => row.id === "close-prefs-veto");
  assert.equal(prefsCloseStep.result.vetoed, true);
  assert.equal(prefsCloseStep.result.closed, false);

  assert.equal(result.teardown?.clean, true);
});
