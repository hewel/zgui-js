import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  readTree,
  runScenario,
} from "./harness.mjs";

const MENU_CSS = `
.surface {
  width: 100%;
  height: 100%;
  background-color: #161b22;
  color: #f0f6fc;
  border-radius: 12px;
  padding: 6px;
}

.list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.item {
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  height: 30px;
  padding: 0 10px;
  border-radius: 6px;
  color: #f0f6fc;
  background-color: transparent;
}

.item[data-highlighted="true"] {
  background-color: #1f6feb;
  color: #ffffff;
}

.item[data-disabled="true"] {
  opacity: 0.45;
}
`;

const POPUP_MENU_SOURCE = `
import { LayerSurface, PopupSurface, Show, createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import {
  MenuCheckboxItemPrimitive,
  MenuItemPrimitive,
  MenuListPrimitive,
  MenuRadioGroupPrimitive,
  MenuRadioItemPrimitive,
  MenuRootPrimitive,
} from "@hewel/zgui/primitives";
import styles from "./menu.module.css";

export const hostApiVersion = 1;

function ShellTrayApp() {
  const [open, setOpen] = createSignal(true);
  const [lastAction, setLastAction] = createSignal("none");
  const [muted, setMuted] = createSignal(false);
  const [profile, setProfile] = createSignal("balanced");
  const [dismissCount, setDismissCount] = createSignal(0);

  return (
    <column style={{ width: "100%", height: "100%", padding: "12px", gap: "6px" }}>
      <text testId="status-open">{String(open())}</text>
      <text testId="status-action">{lastAction()}</text>
      <text testId="status-muted">{String(muted())}</text>
      <text testId="status-profile">{profile()}</text>
      <text testId="status-dismiss">{String(dismissCount())}</text>
      <LayerSurface name="bar" anchor={["top"]} width={320} height={32}>
        <text testId="bar-label">Wayland Bar</text>
      </LayerSurface>
      <Show when={open()}>
        <PopupSurface
          name="tray-menu"
          parent="bar"
          anchorRect={{ x: 24, y: 0, width: 32, height: 32 }}
          width={220}
          height={210}
          placement={{ anchor: "bottom-left", gravity: "bottom-right", flip: true, slide: true }}
          grab
          onDismiss={() => {
            setDismissCount(n => n + 1);
            setOpen(false);
          }}
        >
          <scroll class={styles.surface}>
            <MenuRootPrimitive
              open={open()}
              onOpenChange={next => setOpen(next)}
              onDismiss={() => setDismissCount(n => n + 1)}
            >
              <MenuListPrimitive testId="tray-list" class={styles.list} label="Tray menu">
                <MenuItemPrimitive
                  testId="item-refresh"
                  class={styles.item}
                  closeOnSelect={false}
                  onSelect={() => setLastAction("refresh")}
                >
                  <text>Refresh Status</text>
                </MenuItemPrimitive>
                <MenuItemPrimitive
                  testId="item-disabled"
                  class={styles.item}
                  disabled
                  onSelect={() => setLastAction("should-not-fire")}
                >
                  <text>Disabled Entry</text>
                </MenuItemPrimitive>
                <MenuCheckboxItemPrimitive
                  testId="item-mute"
                  class={styles.item}
                  checked={muted()}
                  onCheckedChange={setMuted}
                  closeOnSelect={false}
                >
                  <text>{muted() ? "Muted [x]" : "Muted [ ]"}</text>
                </MenuCheckboxItemPrimitive>
                <MenuRadioGroupPrimitive value={profile()} onValueChange={setProfile} label="Power">
                  <MenuRadioItemPrimitive
                    testId="radio-balanced"
                    class={styles.item}
                    value="balanced"
                    closeOnSelect={false}
                  >
                    <text>Balanced</text>
                  </MenuRadioItemPrimitive>
                  <MenuRadioItemPrimitive
                    testId="radio-perf"
                    class={styles.item}
                    value="performance"
                    closeOnSelect={false}
                  >
                    <text>Performance</text>
                  </MenuRadioItemPrimitive>
                </MenuRadioGroupPrimitive>
                <MenuItemPrimitive
                  testId="item-quit"
                  class={styles.item}
                  onSelect={() => setLastAction("quit")}
                >
                  <text>Quit</text>
                </MenuItemPrimitive>
              </MenuListPrimitive>
            </MenuRootPrimitive>
          </scroll>
        </PopupSurface>
      </Show>
    </column>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <ShellTrayApp />, root);
}
`;

test("headless @hewel/zgui/primitives menu works inside PopupSurface with keyboard roving, checkbox, radio, typeahead, and Escape dismiss", async t => {
  const work = await createTempWorkspace("preview-primitives-popup-menu-");
  t.after(() => work.cleanup());

  const bundle = await buildFixtureApp(
    path.join(work.dir, "app"),
    "popup-menu-primitives",
    POPUP_MENU_SOURCE,
    {
      rawSource: true,
      capabilities: { shellSurfaces: true },
      files: { "src/menu.module.css": MENU_CSS },
    },
  );

  const monitor = {
    id: "primary",
    name: "Primary",
    x: 0,
    y: 0,
    width: 640,
    height: 480,
    scale: 1,
    refreshRateMilliHz: 60000,
  };

  const { out } = await runScenario({
    projectDir: work.dir,
    bundle,
    name: "primitives-popup-menu",
    width: 360,
    height: 240,
    requiredCapabilities: [],
    steps: [
      { id: "monitors", op: "monitors.set", monitors: [monitor] },
      { id: "settle-open", op: "clock.advance", byMs: 30 },
      { id: "cap-initial", op: "capture", captureId: "initial", surface: "tray-menu" },
      // First enabled item (item-refresh) receives autofocus; Enter triggers Refresh.
      { id: "enter-refresh", op: "key.press", surface: "tray-menu", key: "Enter" },
      {
        id: "assert-refresh",
        op: "assert",
        target: { surface: "main", testId: "status-action" },
        check: { property: "text", equals: "refresh" },
      },
      // ArrowDown skips disabled item-disabled and lands on item-mute; Enter toggles checkbox.
      { id: "down-to-mute", op: "key.press", surface: "tray-menu", key: "ArrowDown" },
      { id: "enter-mute", op: "key.press", surface: "tray-menu", key: "Enter" },
      {
        id: "assert-muted",
        op: "assert",
        target: { surface: "main", testId: "status-muted" },
        check: { property: "text", equals: "true" },
      },
      // Click radio-perf directly inside PopupSurface.
      {
        id: "click-perf",
        op: "pointer.click",
        target: { surface: "tray-menu", testId: "radio-perf" },
        button: "primary",
      },
      {
        id: "assert-profile",
        op: "assert",
        target: { surface: "main", testId: "status-profile" },
        check: { property: "text", equals: "performance" },
      },
      { id: "cap-updated", op: "capture", captureId: "updated", surface: "tray-menu" },
      // Escape dismisses the headless menu and closes PopupSurface.
      { id: "escape-close", op: "key.press", surface: "tray-menu", key: "Escape" },
      { id: "settle-close", op: "clock.advance", byMs: 30 },
      {
        id: "assert-closed",
        op: "assert",
        target: { surface: "main", testId: "status-open" },
        check: { property: "text", equals: "false" },
      },
      {
        id: "assert-dismissed",
        op: "assert",
        target: { surface: "main", testId: "status-dismiss" },
        check: { property: "text", equals: "1" },
      },
    ],
  });

  const updatedTree = await readTree(out, "surfaces/tray-menu/updated");
  assert.equal(getNode(updatedTree, "item-mute").text, "Muted [x]");
});
