import path from "node:path";
import { test } from "node:test";
import { buildFixtureApp, createTempWorkspace, runScenario } from "./harness.mjs";

const MENU_CSS = `
.surface {
  width: 100%;
  height: 100%;
  background-color: #161b22;
  color: #f0f6fc;
  border-radius: 10px;
  padding: 6px;
}

.panel {
  position: fixed;
  width: 180px;
  background-color: #161b22;
  color: #f0f6fc;
  border: 1px solid #30363d;
  border-radius: 8px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
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
  height: 28px;
  padding: 0 10px;
  border-radius: 6px;
  color: #f0f6fc;
  background-color: transparent;
}

.item[data-highlighted="true"] {
  background-color: #1f6feb;
  color: #ffffff;
}

.item[data-state="open"] {
  background-color: #1f6feb;
  color: #ffffff;
}

.btn {
  height: 28px;
  padding: 0 10px;
  border-radius: 6px;
  background-color: #21262d;
  color: #f0f6fc;
}
`;

const THREE_LEVEL_PORTAL_SOURCE = `
import { Portal, Show, createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import {
  MenuItemPrimitive,
  MenuListPrimitive,
  MenuRootPrimitive,
  MenuSubContentPrimitive,
  MenuSubPrimitive,
  MenuSubTriggerPrimitive,
} from "@hewel/zgui/primitives";
import styles from "./menu.module.css";

export const hostApiVersion = 1;

function ThreeLevelPortalApp() {
  const [rootOpen, setRootOpen] = createSignal(true);
  const [l1Open, setL1Open] = createSignal(false);
  const [l2Open, setL2Open] = createSignal(false);
  const [l3Open, setL3Open] = createSignal(false);
  const [vetoL3, setVetoL3] = createSignal(false);
  const [l3CloseRequests, setL3CloseRequests] = createSignal(0);

  return (
    <column style={{ width: "100%", height: "100%", padding: "12px", gap: "8px", "background-color": "#0d1117", color: "#f0f6fc" }}>
      <row style={{ gap: "8px" }}>
        <control
          testId="toggle-veto"
          class={styles.btn}
          tabIndex={0}
          onClick={() => setVetoL3(v => !v)}
        >
          <text>{vetoL3() ? "Veto: ON" : "Veto: OFF"}</text>
        </control>
        <control
          testId="reopen-root"
          class={styles.btn}
          tabIndex={0}
          onClick={() => setRootOpen(true)}
        >
          <text>Reopen</text>
        </control>
      </row>
      <text testId="state-stack">{[rootOpen(), l1Open(), l2Open(), l3Open()].map(Number).join(",")}</text>
      <text testId="l3-requests">{String(l3CloseRequests())}</text>

      <MenuRootPrimitive open={rootOpen()} onOpenChange={setRootOpen}>
        <Show when={rootOpen()}>
          <Portal layer="popover">
            <MenuListPrimitive
              testId="l0-list"
              class={styles.panel}
              style={{ left: "24px", top: "96px" }}
              dismissLayer="popover"
              label="Root menu"
            >
              <MenuItemPrimitive testId="l0-before" class={styles.item} closeOnSelect={false}>
                <text>Before</text>
              </MenuItemPrimitive>
              <MenuSubPrimitive open={l1Open()} onOpenChange={setL1Open}>
                <MenuSubTriggerPrimitive testId="l1-trigger" class={styles.item}>
                  <text>Level 1</text>
                </MenuSubTriggerPrimitive>
                <Show when={l1Open()}>
                  <Portal layer="popover">
                    <MenuSubContentPrimitive
                      testId="l1-content"
                      class={styles.panel}
                      style={{ left: "212px", top: "96px" }}
                      dismissLayer="popover"
                      label="Level 1"
                    >
                      <MenuItemPrimitive testId="l1-item" class={styles.item} closeOnSelect={false}>
                        <text>L1 Action</text>
                      </MenuItemPrimitive>
                      <MenuSubPrimitive open={l2Open()} onOpenChange={setL2Open}>
                        <MenuSubTriggerPrimitive testId="l2-trigger" class={styles.item}>
                          <text>Level 2</text>
                        </MenuSubTriggerPrimitive>
                        <Show when={l2Open()}>
                          <Portal layer="popover">
                            <MenuSubContentPrimitive
                              testId="l2-content"
                              class={styles.panel}
                              style={{ left: "400px", top: "96px" }}
                              dismissLayer="popover"
                              label="Level 2"
                            >
                              <MenuItemPrimitive testId="l2-item" class={styles.item} closeOnSelect={false}>
                                <text>L2 Action</text>
                              </MenuItemPrimitive>
                              <MenuSubPrimitive
                                open={l3Open()}
                                onOpenChange={next => {
                                  if (!next) setL3CloseRequests(n => n + 1);
                                  if (!next && vetoL3()) return;
                                  setL3Open(next);
                                }}
                              >
                                <MenuSubTriggerPrimitive testId="l3-trigger" class={styles.item}>
                                  <text>Level 3</text>
                                </MenuSubTriggerPrimitive>
                                <Show when={l3Open()}>
                                  <Portal layer="popover">
                                    <MenuSubContentPrimitive
                                      testId="l3-content"
                                      class={styles.panel}
                                      style={{ left: "588px", top: "96px" }}
                                      dismissLayer="popover"
                                      label="Level 3"
                                    >
                                      <MenuItemPrimitive testId="l3-leaf" class={styles.item} closeOnSelect={false}>
                                        <text>L3 Leaf</text>
                                      </MenuItemPrimitive>
                                      <MenuItemPrimitive
                                        testId="l3-release-veto"
                                        class={styles.item}
                                        closeOnSelect={false}
                                        onSelect={() => setVetoL3(false)}
                                      >
                                        <text>Release Veto</text>
                                      </MenuItemPrimitive>
                                    </MenuSubContentPrimitive>
                                  </Portal>
                                </Show>
                              </MenuSubPrimitive>
                            </MenuSubContentPrimitive>
                          </Portal>
                        </Show>
                      </MenuSubPrimitive>
                    </MenuSubContentPrimitive>
                  </Portal>
                </Show>
              </MenuSubPrimitive>
              <MenuItemPrimitive testId="l0-after" class={styles.item} closeOnSelect={false}>
                <text>After</text>
              </MenuItemPrimitive>
            </MenuListPrimitive>
          </Portal>
        </Show>
      </MenuRootPrimitive>
    </column>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <ThreeLevelPortalApp />, root);
}
`;

const TWO_POPUP_SOURCE = `
import { LayerSurface, PopupSurface, Show, createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import {
  MenuItemPrimitive,
  MenuListPrimitive,
  MenuRootPrimitive,
  MenuSubContentPrimitive,
  MenuSubPrimitive,
  MenuSubTriggerPrimitive,
} from "@hewel/zgui/primitives";
import styles from "./menu.module.css";

export const hostApiVersion = 1;

function NestedPopupSubmenuApp() {
  const [parentOpen, setParentOpen] = createSignal(true);
  const [subAOpen, setSubAOpen] = createSignal(false);
  const [subBOpen, setSubBOpen] = createSignal(false);
  const [parentDismissCount, setParentDismissCount] = createSignal(0);
  const [subADismissCount, setSubADismissCount] = createSignal(0);
  const [parentFocusAfterClose, setParentFocusAfterClose] = createSignal(0);

  return (
    <column style={{ width: "100%", height: "100%", padding: "12px", gap: "6px", "background-color": "#0d1117", color: "#f0f6fc" }}>
      <text testId="status-open">{[parentOpen(), subAOpen(), subBOpen()].map(Number).join(",")}</text>
      <text testId="status-dismiss">{parentDismissCount() + "," + subADismissCount()}</text>
      <text testId="status-late-focus">{String(parentFocusAfterClose())}</text>
      <control
        testId="reopen-parent"
        class={styles.btn}
        tabIndex={0}
        onClick={() => {
          setSubAOpen(false);
          setSubBOpen(false);
          setParentOpen(true);
        }}
      >
        <text>Reopen</text>
      </control>

      <LayerSurface name="bar" anchor={["top"]} width={480} height={32}>
        <text testId="bar-label">Shell Bar</text>
      </LayerSurface>

      <MenuRootPrimitive open={parentOpen()} onOpenChange={setParentOpen}>
        <Show when={parentOpen()}>
          <PopupSurface
            name="parent-menu"
            parent="bar"
            anchorRect={{ x: 16, y: 0, width: 32, height: 32 }}
            width={200}
            height={140}
            placement={{ anchor: "bottom-left", gravity: "bottom-right", flip: true, slide: true }}
            grab
            onDismiss={() => {
              setParentDismissCount(n => n + 1);
              setParentOpen(false);
            }}
          >
            <box class={styles.surface}>
              <MenuListPrimitive testId="parent-list" class={styles.list} label="Parent menu">
                <MenuItemPrimitive testId="parent-top" class={styles.item} closeOnSelect={false}>
                  <text>Top Action</text>
                </MenuItemPrimitive>
                <MenuSubPrimitive open={subAOpen()} onOpenChange={setSubAOpen}>
                  <MenuSubTriggerPrimitive
                    testId="sub-a-trigger"
                    class={styles.item}
                    onFocus={() => {
                      if (!parentOpen()) setParentFocusAfterClose(n => n + 1);
                    }}
                  >
                    <text>Sub A</text>
                  </MenuSubTriggerPrimitive>
                  <Show when={subAOpen()}>
                    <PopupSurface
                      name="child-sub-a"
                      parent="parent-menu"
                      anchorRect={{ x: 0, y: 34, width: 200, height: 28 }}
                      width={180}
                      height={100}
                      placement={{ anchor: "right", gravity: "right", flip: true, slide: true }}
                      onDismiss={() => {
                        setSubADismissCount(n => n + 1);
                        setSubAOpen(false);
                      }}
                    >
                      <box class={styles.surface}>
                        <MenuSubContentPrimitive testId="sub-a-content" class={styles.list} label="Sub A">
                          <MenuItemPrimitive testId="sub-a-item" class={styles.item} closeOnSelect={false}>
                            <text>Sub A Item</text>
                          </MenuItemPrimitive>
                        </MenuSubContentPrimitive>
                      </box>
                    </PopupSurface>
                  </Show>
                </MenuSubPrimitive>
                <MenuSubPrimitive open={subBOpen()} onOpenChange={setSubBOpen}>
                  <MenuSubTriggerPrimitive testId="sub-b-trigger" class={styles.item}>
                    <text>Sub B</text>
                  </MenuSubTriggerPrimitive>
                  <Show when={subBOpen()}>
                    <PopupSurface
                      name="child-sub-b"
                      parent="parent-menu"
                      anchorRect={{ x: 0, y: 64, width: 200, height: 28 }}
                      width={180}
                      height={100}
                      placement={{ anchor: "right", gravity: "right", flip: true, slide: true }}
                      onDismiss={() => setSubBOpen(false)}
                    >
                      <box class={styles.surface}>
                        <MenuSubContentPrimitive testId="sub-b-content" class={styles.list} label="Sub B">
                          <MenuItemPrimitive testId="sub-b-item" class={styles.item} closeOnSelect={false}>
                            <text>Sub B Item</text>
                          </MenuItemPrimitive>
                        </MenuSubContentPrimitive>
                      </box>
                    </PopupSurface>
                  </Show>
                </MenuSubPrimitive>
              </MenuListPrimitive>
            </box>
          </PopupSurface>
        </Show>
      </MenuRootPrimitive>
    </column>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <NestedPopupSubmenuApp />, root);
}
`;

test("3-level headless Portal submenu closes one level per Escape, enforces controlled close veto, and preserves hover corridor across DPI", async t => {
  const work = await createTempWorkspace("preview-submenu-portal-primitives-");
  t.after(() => work.cleanup());

  const bundle = await buildFixtureApp(
    path.join(work.dir, "app"),
    "submenu-portal-primitives",
    THREE_LEVEL_PORTAL_SOURCE,
    {
      rawSource: true,
      files: { "src/menu.module.css": MENU_CSS },
    },
  );

  let serial = 0;
  const id = () => `step-${++serial}`;
  const key = name => ({ id: id(), op: "key.press", surface: "main", key: name });
  const check = (testId, property, equals) => ({
    id: id(),
    op: "assert",
    target: { surface: "main", testId },
    check: { property, equals },
  });

  await runScenario({
    projectDir: work.dir,
    bundle,
    name: "submenu-portal-primitives",
    width: 820,
    height: 360,
    requiredCapabilities: [],
    steps: [
      { id: id(), op: "clock.advance", byMs: 30 },
      // Enable controlled close veto on L3 first, then open L1 -> L2 -> L3 via keyboard.
      {
        id: id(),
        op: "pointer.click",
        target: { surface: "main", testId: "toggle-veto" },
        button: "primary",
      },
      {
        id: id(),
        op: "pointer.click",
        target: { surface: "main", testId: "reopen-root" },
        button: "primary",
      },
      check("l0-before", "focused", true),
      key("ArrowDown"),
      check("l1-trigger", "focused", true),
      key("ArrowRight"),
      check("l1-item", "focused", true),
      key("ArrowDown"),
      check("l2-trigger", "focused", true),
      key("Enter"),
      check("l2-item", "focused", true),
      key("ArrowDown"),
      check("l3-trigger", "focused", true),
      key("Space"),
      check("l3-leaf", "focused", true),
      check("state-stack", "text", "1,1,1,1"),

      // Controlled close veto on L3 keeps focus on l3-leaf for BOTH ArrowLeft and Escape.
      key("ArrowLeft"),
      check("state-stack", "text", "1,1,1,1"),
      check("l3-leaf", "focused", true),
      key("Escape"),
      check("state-stack", "text", "1,1,1,1"),
      check("l3-leaf", "focused", true),

      // Release veto inside L3, return focus to l3-leaf, and press Escape 4 times:
      // strictly ONE level closes per Escape with focus restored to each still-live trigger.
      key("ArrowDown"),
      check("l3-release-veto", "focused", true),
      key("Enter"),
      key("ArrowUp"),
      check("l3-leaf", "focused", true),
      key("Escape"),
      check("state-stack", "text", "1,1,1,0"),
      check("l3-trigger", "focused", true),
      key("Escape"),
      check("state-stack", "text", "1,1,0,0"),
      check("l2-trigger", "focused", true),
      key("Escape"),
      check("state-stack", "text", "1,0,0,0"),
      check("l1-trigger", "focused", true),
      key("Escape"),
      check("state-stack", "text", "0,0,0,0"),

      // Reopen root, hover-open L1 (120ms, must NOT steal focus from l1-trigger), and verify corridor across DPI 1 -> 2.
      {
        id: id(),
        op: "pointer.click",
        target: { surface: "main", testId: "reopen-root" },
        button: "primary",
      },
      { id: id(), op: "pointer.move", target: { surface: "main", testId: "l1-trigger" } },
      { id: id(), op: "clock.advance", byMs: 120 },
      check("state-stack", "text", "1,1,0,0"),
      check("l1-trigger", "focused", true),
      // Record device point inside l1-trigger at scale 1, switch to scale 2, and move diagonally toward l1-content.
      { id: id(), op: "pointer.move", surface: "main", position: { x: 60, y: 140 } },
      { id: id(), op: "surface.scale", surface: "main", scale: 2 },
      { id: id(), op: "clock.advance", byMs: 40 },
      { id: id(), op: "pointer.move", surface: "main", position: { x: 140, y: 164 } },
      { id: id(), op: "clock.advance", byMs: 250 },
      check("state-stack", "text", "1,1,0,0"),
    ],
  });
});

test("two-PopupSurface nested headless submenu avoids foreign handles, cancels 180ms leave grace, replaces siblings, handles parent Escape, and reconciles popup_done idempotently", async t => {
  const work = await createTempWorkspace("preview-submenu-popup-primitives-");
  t.after(() => work.cleanup());

  const bundle = await buildFixtureApp(
    path.join(work.dir, "app"),
    "submenu-popup-primitives",
    TWO_POPUP_SOURCE,
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
    width: 800,
    height: 600,
    scale: 1,
    refreshRateMilliHz: 60000,
  };

  let serial = 0;
  const id = () => `step-${++serial}`;
  const checkMain = (testId, property, equals) => ({
    id: id(),
    op: "assert",
    target: { surface: "main", testId },
    check: { property, equals },
  });

  await runScenario({
    projectDir: work.dir,
    bundle,
    name: "submenu-popup-primitives",
    width: 360,
    height: 220,
    requiredCapabilities: [],
    steps: [
      { id: id(), op: "monitors.set", monitors: [monitor] },
      { id: id(), op: "clock.advance", byMs: 40 },
      checkMain("status-open", "text", "1,0,0"),

      // 1. Hover sub-a-trigger for 120ms -> child-sub-a opens without stealing focus from sub-a-trigger.
      {
        id: id(),
        op: "pointer.move",
        target: { surface: "parent-menu", testId: "sub-a-trigger" },
      },
      { id: id(), op: "clock.advance", byMs: 120 },
      { id: id(), op: "clock.advance", byMs: 30 },
      checkMain("status-open", "text", "1,1,0"),
      {
        id: id(),
        op: "assert",
        target: { surface: "parent-menu", testId: "sub-a-trigger" },
        check: { property: "focused", equals: true },
      },

      // 2. Parent leave -> child pointerenter within 180ms cancels the leave grace timer.
      {
        id: id(),
        op: "pointer.move",
        target: { surface: "parent-menu", testId: "parent-top" },
      },
      { id: id(), op: "clock.advance", byMs: 100 },
      checkMain("status-open", "text", "1,1,0"),
      {
        id: id(),
        op: "pointer.move",
        target: { surface: "child-sub-a", testId: "sub-a-item" },
      },
      { id: id(), op: "clock.advance", byMs: 250 },
      checkMain("status-open", "text", "1,1,0"),

      // 3. Sibling replacement: hovering sub-b-trigger for 120ms opens child-sub-b and closes child-sub-a.
      {
        id: id(),
        op: "pointer.move",
        target: { surface: "parent-menu", testId: "sub-b-trigger" },
      },
      { id: id(), op: "clock.advance", byMs: 120 },
      { id: id(), op: "clock.advance", byMs: 30 },
      checkMain("status-open", "text", "1,0,1"),

      // 4. Escape delivered to parent-menu closes ONLY deepest child-sub-b (parent stays open).
      { id: id(), op: "key.press", surface: "parent-menu", key: "Escape" },
      { id: id(), op: "clock.advance", byMs: 30 },
      checkMain("status-open", "text", "1,0,0"),

      // 5. Re-open child-sub-a via keyboard, then reconcile popupDone on child-sub-a and parent-menu idempotently.
      {
        id: id(),
        op: "pointer.move",
        target: { surface: "parent-menu", testId: "sub-a-trigger" },
      },
      { id: id(), op: "key.press", surface: "parent-menu", key: "ArrowRight" },
      { id: id(), op: "clock.advance", byMs: 30 },
      checkMain("status-open", "text", "1,1,0"),
      { id: id(), op: "popupDone", surface: "child-sub-a" },
      { id: id(), op: "clock.advance", byMs: 30 },
      checkMain("status-open", "text", "1,0,0"),
      { id: id(), op: "popupDone", surface: "parent-menu" },
      { id: id(), op: "clock.advance", byMs: 30 },
      checkMain("status-open", "text", "0,0,0"),
      checkMain("status-dismiss", "text", "1,1"),
      checkMain("status-late-focus", "text", "0"),
    ],
  });
});
