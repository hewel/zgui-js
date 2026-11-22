import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const FIXTURE_SOURCE = `
import { Show, createSignal, renderApp, LayerSurface, PopupSurface } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { headingToward as componentHeadingToward } from "@hewel/zgui/components";
import {
  headingToward,
  MenuItemPrimitive,
  MenuListPrimitive,
  MenuRootPrimitive,
  MenuSubContentPrimitive,
  MenuSubPrimitive,
  MenuSubTriggerPrimitive,
} from "@hewel/zgui/primitives";

export { headingToward, componentHeadingToward };

export const log = {
  selects: [] as string[],
  rootDismisses: 0,
  l3Requests: [] as boolean[],
  popupADismisses: 0,
  popupBDismisses: 0,
  parentPopupDismisses: 0,
};

export function resetLog(): void {
  log.selects.length = 0;
  log.rootDismisses = 0;
  log.l3Requests.length = 0;
  log.popupADismisses = 0;
  log.popupBDismisses = 0;
  log.parentPopupDismisses = 0;
}

const [rootOpen, setRootOpen] = createSignal(true);
const [l1Open, setL1Open] = createSignal(false);
const [l1SibOpen, setL1SibOpen] = createSignal(false);
const [l2Open, setL2Open] = createSignal(false);
const [l3Open, setL3Open] = createSignal(false);
const [l3VetoClose, setL3VetoClose] = createSignal(false);
const [l1Disabled, setL1Disabled] = createSignal(false);

export {
  rootOpen,
  setRootOpen,
  l1Open,
  setL1Open,
  l1SibOpen,
  setL1SibOpen,
  l2Open,
  setL2Open,
  l3Open,
  setL3Open,
  l3VetoClose,
  setL3VetoClose,
  l1Disabled,
  setL1Disabled,
};

export function mountThreeLevel(root: NativeNode): () => void {
  return renderApp(
    () => (
      <MenuRootPrimitive
        open={rootOpen()}
        onOpenChange={setRootOpen}
        onDismiss={() => {
          log.rootDismisses += 1;
        }}
      >
        <Show when={rootOpen()}>
          <MenuListPrimitive testId="l0-list" dismissLayer="popover">
            <MenuItemPrimitive
              testId="l0-first"
              closeOnSelect={false}
              onSelect={() => log.selects.push("l0-first")}
            >
              <text>First</text>
            </MenuItemPrimitive>
            <MenuSubPrimitive open={l1Open()} onOpenChange={setL1Open}>
              <MenuSubTriggerPrimitive testId="l1-trigger" disabled={l1Disabled()}>
                <text>Level 1</text>
              </MenuSubTriggerPrimitive>
              <Show when={l1Open()}>
                <MenuSubContentPrimitive testId="l1-content" dismissLayer="popover">
                  <MenuItemPrimitive
                    testId="l1-item"
                    closeOnSelect={false}
                    onSelect={() => log.selects.push("l1-item")}
                  >
                    <text>L1 Item</text>
                  </MenuItemPrimitive>
                  <MenuSubPrimitive open={l2Open()} onOpenChange={setL2Open}>
                    <MenuSubTriggerPrimitive testId="l2-trigger">
                      <text>Level 2</text>
                    </MenuSubTriggerPrimitive>
                    <Show when={l2Open()}>
                      <MenuSubContentPrimitive testId="l2-content">
                        <MenuItemPrimitive
                          testId="l2-item"
                          closeOnSelect={false}
                          onSelect={() => log.selects.push("l2-item")}
                        >
                          <text>L2 Item</text>
                        </MenuItemPrimitive>
                        <MenuSubPrimitive
                          open={l3Open()}
                          onOpenChange={next => {
                            log.l3Requests.push(next);
                            if (!next && l3VetoClose()) return;
                            setL3Open(next);
                          }}
                        >
                          <MenuSubTriggerPrimitive testId="l3-trigger">
                            <text>Level 3</text>
                          </MenuSubTriggerPrimitive>
                          <Show when={l3Open()}>
                            <MenuSubContentPrimitive testId="l3-content">
                              <MenuItemPrimitive
                                testId="l3-leaf"
                                onSelect={() => log.selects.push("l3-leaf")}
                              >
                                <text>Leaf</text>
                              </MenuItemPrimitive>
                            </MenuSubContentPrimitive>
                          </Show>
                        </MenuSubPrimitive>
                      </MenuSubContentPrimitive>
                    </Show>
                  </MenuSubPrimitive>
                </MenuSubContentPrimitive>
              </Show>
            </MenuSubPrimitive>
            <MenuSubPrimitive open={l1SibOpen()} onOpenChange={setL1SibOpen}>
              <MenuSubTriggerPrimitive testId="l1-sib-trigger">
                <text>Level 1 Sibling</text>
              </MenuSubTriggerPrimitive>
              <Show when={l1SibOpen()}>
                <MenuSubContentPrimitive testId="l1-sib-content" dismissLayer="popover">
                  <MenuItemPrimitive
                    testId="l1-sib-item"
                    closeOnSelect={false}
                    onSelect={() => log.selects.push("l1-sib-item")}
                  >
                    <text>L1 Sibling Item</text>
                  </MenuItemPrimitive>
                </MenuSubContentPrimitive>
              </Show>
            </MenuSubPrimitive>
          </MenuListPrimitive>
        </Show>
      </MenuRootPrimitive>
    ),
    root,
  );
}

const [parentPopupOpen, setParentPopupOpen] = createSignal(true);
const [popupAOpen, setPopupAOpen] = createSignal(false);
const [popupBOpen, setPopupBOpen] = createSignal(false);

export {
  parentPopupOpen,
  setParentPopupOpen,
  popupAOpen,
  setPopupAOpen,
  popupBOpen,
  setPopupBOpen,
};

export function mountCrossPopup(root: NativeNode): () => void {
  return renderApp(
    () => (
      <box testId="shell-root">
        <LayerSurface name="bar" anchor={["top"]} width={400} height={32}>
          <text testId="bar-label">Bar</text>
        </LayerSurface>
        <MenuRootPrimitive open={parentPopupOpen()} onOpenChange={setParentPopupOpen}>
          <Show when={parentPopupOpen()}>
            <PopupSurface
              name="parent-menu"
              parent="bar"
              anchorRect={{ x: 10, y: 0, width: 32, height: 32 }}
              width={200}
              height={160}
              onDismiss={() => {
                log.parentPopupDismisses += 1;
                setParentPopupOpen(false);
              }}
            >
              <MenuListPrimitive testId="parent-list" dismissLayer="popover">
                <MenuItemPrimitive testId="parent-item" closeOnSelect={false}>
                  <text>Parent Item</text>
                </MenuItemPrimitive>
                <MenuSubPrimitive open={popupAOpen()} onOpenChange={setPopupAOpen}>
                  <MenuSubTriggerPrimitive testId="sub-a-trigger">
                    <text>Sub A</text>
                  </MenuSubTriggerPrimitive>
                  <Show when={popupAOpen()}>
                    <PopupSurface
                      name="child-sub-a"
                      parent="parent-menu"
                      anchorRect={{ x: 0, y: 30, width: 200, height: 28 }}
                      width={180}
                      height={120}
                      onDismiss={() => {
                        log.popupADismisses += 1;
                        setPopupAOpen(false);
                      }}
                    >
                      <MenuSubContentPrimitive testId="sub-a-content" dismissLayer="popover">
                        <MenuItemPrimitive
                          testId="sub-a-leaf"
                          onSelect={() => log.selects.push("sub-a-leaf")}
                        >
                          <text>Sub A Leaf</text>
                        </MenuItemPrimitive>
                      </MenuSubContentPrimitive>
                    </PopupSurface>
                  </Show>
                </MenuSubPrimitive>
                <MenuSubPrimitive open={popupBOpen()} onOpenChange={setPopupBOpen}>
                  <MenuSubTriggerPrimitive testId="sub-b-trigger">
                    <text>Sub B</text>
                  </MenuSubTriggerPrimitive>
                  <Show when={popupBOpen()}>
                    <PopupSurface
                      name="child-sub-b"
                      parent="parent-menu"
                      anchorRect={{ x: 0, y: 60, width: 200, height: 28 }}
                      width={180}
                      height={120}
                      onDismiss={() => {
                        log.popupBDismisses += 1;
                        setPopupBOpen(false);
                      }}
                    >
                      <MenuSubContentPrimitive testId="sub-b-content" dismissLayer="popover">
                        <MenuItemPrimitive
                          testId="sub-b-leaf"
                          onSelect={() => log.selects.push("sub-b-leaf")}
                        >
                          <text>Sub B Leaf</text>
                        </MenuItemPrimitive>
                      </MenuSubContentPrimitive>
                    </PopupSurface>
                  </Show>
                </MenuSubPrimitive>
              </MenuListPrimitive>
            </PopupSurface>
          </Show>
        </MenuRootPrimitive>
      </box>
    ),
    root,
  );
}
`;

test("headingToward export identity matches between @hewel/zgui/primitives and @hewel/zgui/components", async t => {
  const mod = await moduleFor(t, FIXTURE_SOURCE);
  assert.equal(
    mod.headingToward,
    mod.componentHeadingToward,
    "primitives and components must export the exact same headingToward function",
  );
  const rect = { x: 200, y: 100, width: 140, height: 180 };
  assert.equal(mod.headingToward({ x: 100, y: 120 }, { x: 160, y: 160 }, rect), true);
  assert.equal(mod.headingToward({ x: 100, y: 120 }, { x: 100, y: 350 }, rect), false);
});

test("3-level same-document headless submenu installs zero stylesheets, closes one level per Escape/ArrowLeft, enforces veto, corridor DPI, and sibling replacement", async t => {
  const h = createHost();
  const clock = createClock(t, h);
  const mod = await moduleFor(t, FIXTURE_SOURCE);
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
  });
  globalThis.__zgui = h.host;
  mod.resetLog();
  mod.setRootOpen(true);
  mod.setL1Open(false);
  mod.setL1SibOpen(false);
  mod.setL2Open(false);
  mod.setL3Open(false);
  mod.setL3VetoClose(false);
  mod.setL1Disabled(false);

  const root = h.makeNode("root");
  dispose = mod.mountThreeLevel(root);
  h.flush();

  const key = (node, extras) => h.dispatch(node, "KeyDown", extras);
  const send = (node, name, extras = {}) => h.dispatch(node, name, extras);

  assert.equal(h.stylesheets.size, 0, "headless submenu primitives install zero stylesheets");

  const l0List = h.byTestId("l0-list");
  const l1Trigger = h.byTestId("l1-trigger");
  const l1SibTrigger = h.byTestId("l1-sib-trigger");

  // Hover open waits 120ms and MUST NOT steal focus from l1-trigger.
  send(l1Trigger, "onPointerEnter");
  assert.equal(h.focused.current, l1Trigger);
  clock.advance(119);
  h.flush();
  assert.equal(h.byTestId("l1-content"), undefined);
  clock.advance(1);
  h.flush();
  const l1Content = h.byTestId("l1-content");
  assert.ok(l1Content, "120ms hover opens L1");
  assert.equal(
    h.focused.current,
    l1Trigger,
    "hover open must not steal/demand focus into L1 content",
  );
  assert.equal(l1Trigger.props.get("ariaControls"), l1Content);
  assert.equal(l1Content.props.get("ariaPopupFor"), l1Trigger);
  const l1Layer = h.layers.findLast(record => record.active && record.node === l1Content);
  assert.equal(l1Layer.options.exclude, l1Trigger, "same-document dismiss excludes trigger");

  // Pressing ArrowRight on an already hover-opened trigger focuses first enabled child in L1.
  key(l1Trigger, { key: "ArrowRight" });
  assert.equal(
    h.focused.current,
    h.byTestId("l1-item"),
    "ArrowRight on already-hover-opened trigger focuses L1 first enabled item",
  );

  // Keyboard open L2 and L3 (3-level stack).
  const l2Trigger = h.byTestId("l2-trigger");
  h.host.focus(l2Trigger);
  key(l2Trigger, { key: "ArrowRight" });
  const l2Content = h.byTestId("l2-content");
  assert.ok(l2Content, "ArrowRight opens L2");
  assert.equal(h.focused.current, h.byTestId("l2-item"), "keyboard open focuses L2 first item");

  const l3Trigger = h.byTestId("l3-trigger");
  h.host.focus(l3Trigger);
  key(l3Trigger, { key: "Enter" });
  const l3Content = h.byTestId("l3-content");
  const l3Leaf = h.byTestId("l3-leaf");
  assert.ok(l3Content, "Enter opens L3");
  assert.equal(h.focused.current, l3Leaf, "keyboard open focuses L3 leaf");

  // Controlled close veto on L3 keeps focus on l3-leaf for BOTH ArrowLeft and Escape.
  mod.setL3VetoClose(true);
  h.flush();
  key(l3Leaf, { key: "ArrowLeft" });
  assert.equal(mod.l3Open(), true, "vetoed ArrowLeft keeps L3 open");
  assert.equal(h.focused.current, l3Leaf, "vetoed ArrowLeft keeps focus inside L3");
  const l0Layer = h.layers.find(record => record.active && record.node === l0List);
  l0Layer.callback("escape");
  h.flush();
  assert.equal(mod.l3Open(), true, "root-capture Escape routes to L3 and honors veto");
  assert.equal(mod.l2Open(), true, "L2 stays open when L3 vetoed Escape");
  assert.equal(mod.l1Open(), true, "L1 stays open when L3 vetoed Escape");
  assert.equal(mod.rootOpen(), true, "L0 stays open when L3 vetoed Escape");
  assert.equal(h.focused.current, l3Leaf, "vetoed Escape keeps focus inside L3");

  // Release veto: root-capture Escape on L0 closes strictly ONE level at a time (L3 -> L2 -> L1 -> L0).
  mod.setL3VetoClose(false);
  h.flush();
  l0Layer.callback("escape");
  h.flush();
  assert.equal(mod.l3Open(), false, "1st Escape closes only L3");
  assert.equal(mod.l2Open(), true, "L2 stays open after 1st Escape");
  assert.equal(mod.l1Open(), true, "L1 stays open after 1st Escape");
  assert.equal(h.focused.current, l3Trigger, "1st Escape restores focus to L3 trigger");

  // 2nd Escape via local keydown on L2 closes strictly L2 and restores focus to L2 trigger.
  key(l3Trigger, { key: "Escape" });
  assert.equal(mod.l2Open(), false, "2nd Escape closes only L2");
  assert.equal(mod.l1Open(), true, "L1 stays open after 2nd Escape");
  assert.equal(h.focused.current, l2Trigger, "2nd Escape restores focus to L2 trigger");

  // Hover corridor across DPI change (scale 1 -> 2) on L1.
  const l1Viewport = h.geometry.find(
    record => record.active && record.node === l1Trigger && record.kind === "viewport",
  );
  l1Viewport.emit({ x: 0, y: 0, width: 800, height: 600, scale: 1 });
  send(l1Trigger, "onPointerMove", { position: { x: 100, y: 110 } });
  l1Viewport.emit({ x: 0, y: 0, width: 800, height: 600, scale: 2 });
  h.rects.set(l1Content, { x: 100, y: 50, width: 80, height: 100 });
  send(l1Trigger, "onPointerLeave", { position: { x: 70, y: 80 } });
  clock.advance(200);
  h.flush();
  assert.equal(mod.l1Open(), true, "device-space corridor survives DPI change");

  // Sibling submenu replacement: hovering sibling trigger for 120ms closes L1 cleanly.
  send(l1SibTrigger, "onPointerEnter");
  clock.advance(120);
  h.flush();
  assert.equal(mod.l1SibOpen(), true, "sibling opens after 120ms");
  assert.equal(mod.l1Open(), false, "sibling replacement closes L1");

  // Outside-press on sibling submenu closes it WITHOUT forcing focus onto l1-sib-trigger.
  const outsideFocus = h.byTestId("l0-first");
  h.host.focus(outsideFocus);
  const sibLayer = h.layers.findLast(
    record => record.active && record.node === h.byTestId("l1-sib-content"),
  );
  sibLayer.callback("outside-press");
  h.flush();
  assert.equal(mod.l1SibOpen(), false, "outside-press closes sibling submenu");
  assert.equal(
    h.focused.current,
    outsideFocus,
    "outside-press never steals focus back to submenu trigger",
  );

  // Disabling trigger cancels pending hover open timer.
  send(l1Trigger, "onPointerEnter");
  clock.advance(60);
  mod.setL1Disabled(true);
  h.flush();
  clock.advance(120);
  h.flush();
  assert.equal(mod.l1Open(), false, "disabling trigger cancels pending open timer");
  mod.setL1Disabled(false);
  h.flush();

  // Leaf closeOnSelect cascades all 3 levels cleanly.
  key(l1Trigger, { key: "ArrowRight" });
  key(h.byTestId("l2-trigger"), { key: "ArrowRight" });
  key(h.byTestId("l3-trigger"), { key: "ArrowRight" });
  send(h.byTestId("l3-leaf"), "onClick");
  assert.deepEqual(mod.log.selects, ["l3-leaf"]);
  assert.equal(mod.l3Open(), false);
  assert.equal(mod.l2Open(), false);
  assert.equal(mod.l1Open(), false);
  assert.equal(mod.rootOpen(), false);
  assert.equal(mod.log.rootDismisses, 1);
  assert.equal(h.stylesheets.size, 0, "still zero stylesheets after full lifecycle");

  dispose();
  dispose = undefined;
  h.flush();
  assert.equal(clock.timers.size, 0, "disposal leaves zero pending timers");
});

test("cross-PopupSurface submenu omits foreign handles, uses 180ms grace without getWindowRect, replaces siblings, handles parent Escape, and reconciles popup_done idempotently", async t => {
  const h = createHost();
  const clock = createClock(t, h);
  const mod = await moduleFor(t, FIXTURE_SOURCE);
  let dispose;
  t.after(() => {
    try {
      if (dispose) h.runTurn(dispose);
      dispose = undefined;
      h.flush();
    } finally {
      delete globalThis.__zgui;
      delete globalThis.__zguiShell;
    }
  });
  globalThis.__zgui = h.host;
  globalThis.__zguiShell = h.shell;
  mod.resetLog();
  mod.setParentPopupOpen(true);
  mod.setPopupAOpen(false);
  mod.setPopupBOpen(false);

  const root = h.makeNode("root");
  dispose = h.runTurn(() => mod.mountCrossPopup(root));
  h.flush();

  assert.ok(h.surfaces.get("parent-menu"), "parent-menu PopupSurface mounted");
  const subATrigger = h.byTestId("sub-a-trigger");
  const subBTrigger = h.byTestId("sub-b-trigger");
  assert.ok(subATrigger && subBTrigger);

  // 1. Hover-open Sub A across distinct PopupSurface documents.
  h.dispatch(subATrigger, "onPointerEnter");
  clock.advance(120);
  h.flush();
  assert.equal(mod.popupAOpen(), true, "120ms hover opens child-sub-a");
  assert.ok(h.surfaces.get("child-sub-a"), "child-sub-a PopupSurface mounted");
  const subAContent = h.byTestId("sub-a-content");
  assert.ok(subAContent);

  // Verify all three cross-document node links are omitted (null/unset) — no foreign-handle error thrown.
  assert.equal(
    subATrigger.props.get("ariaControls") ?? null,
    null,
    "cross-doc ariaControls omitted",
  );
  assert.equal(
    subAContent.props.get("ariaPopupFor") ?? null,
    null,
    "cross-doc ariaPopupFor omitted",
  );
  const subALayer = h.layers.findLast(r => r.active && r.node === subAContent);
  assert.equal(subALayer?.options.exclude, null, "cross-doc dismiss exclude omitted");

  // Hover-open MUST NOT focus inside child-sub-a (focus remains on sub-a-trigger in parent).
  assert.equal(
    h.focused.current,
    subATrigger,
    "hover open must not demand focus on child PopupSurface",
  );

  // 2. Parent leave -> child pointerenter cancels 180ms grace timer without calling getWindowRect on child.
  const rectQueriesBefore = h.queries.rect.length;
  h.dispatch(subATrigger, "onPointerLeave", { position: { x: 180, y: 40 } });
  const rectQueriesAfter = h.queries.rect.length;
  assert.equal(
    rectQueriesAfter,
    rectQueriesBefore,
    "never calls getWindowRect across PopupSurface documents",
  );
  clock.advance(100);
  h.flush();
  assert.equal(mod.popupAOpen(), true, "still open inside 180ms grace window");
  h.dispatch(subAContent, "onPointerEnter");
  clock.advance(250);
  h.flush();
  assert.equal(mod.popupAOpen(), true, "child pointerenter cancels 180ms grace timer");

  // 3. Sibling submenu replacement: hovering sub-b-trigger for 120ms replaces child-sub-a with child-sub-b.
  h.dispatch(subBTrigger, "onPointerEnter");
  clock.advance(120);
  h.flush();
  assert.equal(mod.popupBOpen(), true, "sub-b opens after 120ms");
  assert.equal(mod.popupAOpen(), false, "sibling replacement closes sub-a");
  assert.equal(h.surfaces.has("child-sub-a"), false);
  assert.equal(h.surfaces.has("child-sub-b"), true);

  // 4. Escape delivered to parent PopupSurface closes ONLY child-sub-b (parent stays open).
  const parentList = h.byTestId("parent-list");
  const parentLayer = h.layers.findLast(r => r.active && r.node === parentList);
  parentLayer.callback("escape");
  h.flush();
  assert.equal(mod.popupBOpen(), false, "parent Escape closes deepest child-sub-b");
  assert.equal(mod.parentPopupOpen(), true, "parent PopupSurface stays open");

  // Re-open sub-b via keyboard and verify local parent keydown Escape ALSO closes only child-sub-b.
  h.dispatch(subBTrigger, "KeyDown", { key: "ArrowRight" });
  h.flush();
  assert.equal(mod.popupBOpen(), true);
  assert.equal(
    h.focused.current,
    h.byTestId("sub-b-leaf"),
    "keyboard open focuses child-sub-b first enabled item",
  );
  h.dispatch(subBTrigger, "KeyDown", { key: "Escape" });
  h.flush();
  assert.equal(mod.popupBOpen(), false, "parent local Escape closes only child-sub-b");
  assert.equal(mod.parentPopupOpen(), true, "parent still open");

  // 5. Idempotent popup_done reconciliation without restoring focus onto a closing parent.
  h.dispatch(subATrigger, "KeyDown", { key: "ArrowRight" });
  h.flush();
  assert.equal(mod.popupAOpen(), true);
  h.focused.current = null;
  h.popupDone("child-sub-a");
  h.flush();
  assert.equal(mod.popupAOpen(), false, "child popup_done closes child-sub-a");
  assert.equal(mod.parentPopupOpen(), true, "parent stays open after child-only popup_done");
  assert.equal(mod.log.popupADismisses, 1);
  assert.equal(h.focused.current, null, "child popup_done does not steal focus onto parent");

  // Re-open child-sub-a and simulate Wayland popup_done sequence (child then parent),
  // verifying idempotent reconciliation without restoring focus onto a closing parent.
  h.dispatch(subATrigger, "KeyDown", { key: "ArrowRight" });
  h.flush();
  assert.equal(mod.popupAOpen(), true);
  h.focused.current = null;
  h.popupDone("child-sub-a");
  h.popupDone("parent-menu");
  h.flush();
  assert.equal(mod.parentPopupOpen(), false);
  assert.equal(mod.popupAOpen(), false);
  assert.equal(mod.log.popupADismisses, 2);
  assert.equal(mod.log.parentPopupDismisses, 1);
  assert.equal(
    h.focused.current,
    null,
    "popup_done must never restore focus onto a closing parent",
  );

  h.runTurn(dispose);
  dispose = undefined;
  h.flush();
  assert.equal(clock.timers.size, 0, "disposal leaves zero pending timers");
});
