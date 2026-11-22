import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Menubar, MenubarArrows, MenubarCheckboxItem, MenubarContent, MenubarGroup, MenubarItem,
  MenubarLabel, MenubarMenu, MenubarRadioGroup, MenubarRadioItem, MenubarSeparator,
  MenubarShortcut, MenubarTrigger,
  MenuItem, MenuSub, MenuSubContent, MenuSubTrigger,
} from "@hewel/zgui/components";

export const log = {
  selects: [] as string[],
  checks: [] as boolean[],
  radios: [] as string[],
  subOpens: [] as boolean[],
  triggers: [] as NativeNode[],
  cleans: 0,
};

const [showEdit, setShowEdit] = createSignal(true);
const [wordWrap, setWordWrap] = createSignal(false);
const [layout, setLayout] = createSignal("single");
export { setShowEdit, setWordWrap, setLayout };

export function mount(root: NativeNode) {
  return renderApp(() => <>
    <control testId="outside-btn">Outside</control>
    <Menubar testId="bar" label="Application">
      <MenubarMenu testId="file-menu" value="file">
        <MenubarTrigger testId="file-trigger" ref={node => { log.triggers.push(node); }}>File</MenubarTrigger>
        <MenubarContent testId="file-surface" class="app-content-class">
          <MenubarItem testId="file-disabled" disabled>Disabled first</MenubarItem>
          <MenubarItem testId="file-new" shortcut="Ctrl+N" class="app-item-class" onSelect={() => log.selects.push("new")}>New Tab</MenubarItem>
          <MenubarItem testId="file-open" closeOnSelect={false} onSelect={() => log.selects.push("open")}>
            Open<MenubarShortcut testId="custom-shortcut">Ctrl+O</MenubarShortcut>
          </MenubarItem>
          <MenuSub onOpenChange={next => log.subOpens.push(next)}>
            <MenuSubTrigger testId="file-share">Share</MenuSubTrigger>
            <MenuSubContent testId="file-share-sub">
              <MenuSub>
                <MenuSubTrigger testId="file-share-more">More</MenuSubTrigger>
                <MenuSubContent testId="file-share-more-sub">
                  <MenuItem testId="file-share-email" onSelect={() => log.selects.push("share-email")}>Email</MenuItem>
                </MenuSubContent>
              </MenuSub>
              <MenuItem testId="file-share-copy" onSelect={() => log.selects.push("share-copy")}>Copy link</MenuItem>
            </MenuSubContent>
          </MenuSub>
          <MenubarSeparator testId="file-sep" />
          <MenubarItem testId="file-export" closeOnSelect={false} onSelect={() => log.selects.push("export")}>Export</MenubarItem>
        </MenubarContent>
      </MenubarMenu>

      <MenubarMenu testId="disabled-menu" value="tools">
        <MenubarTrigger testId="tools-trigger" disabled>Tools</MenubarTrigger>
        <MenubarContent testId="tools-surface">
          <MenubarItem testId="tools-run">Run</MenubarItem>
        </MenubarContent>
      </MenubarMenu>

      <Show when={showEdit()}>
        <MenubarMenu testId="edit-menu" value="edit">
          <MenubarTrigger testId="edit-trigger">Edit</MenubarTrigger>
          <MenubarContent testId="edit-surface">
            <MenubarLabel testId="edit-label">Editing</MenubarLabel>
            <MenubarGroup testId="edit-group" label="History">
              <MenubarItem testId="edit-undo" destructive onSelect={() => log.selects.push("undo")}>Undo</MenubarItem>
            </MenubarGroup>
            <MenubarCheckboxItem testId="edit-wrap" checked={wordWrap()} onCheckedChange={v => {
              setWordWrap(v);
              log.checks.push(v);
            }}>Word wrap</MenubarCheckboxItem>
            <MenubarCheckboxItem testId="edit-auto" defaultChecked={false} closeOnSelect
              onCheckedChange={v => log.checks.push(v)}>Auto close</MenubarCheckboxItem>
          </MenubarContent>
        </MenubarMenu>
      </Show>

      <MenubarMenu testId="view-menu" value="view">
        <MenubarTrigger testId="view-trigger">View</MenubarTrigger>
        <MenubarContent testId="view-surface">
          <MenubarRadioGroup label="Layout" value={layout()} onValueChange={v => {
            setLayout(v);
            log.radios.push(v);
          }}>
            <MenubarRadioItem testId="radio-single" value="single">Single</MenubarRadioItem>
            <MenubarRadioItem testId="radio-split" value="split" closeOnSelect={false}>Split</MenubarRadioItem>
          </MenubarRadioGroup>
        </MenubarContent>
      </MenubarMenu>
    </Menubar>
  </>, root);
}

export function mountSingle(root: NativeNode) {
  return renderApp(() => <>
    <Menubar testId="bar">
      <MenubarMenu testId="only-menu" value="only">
        <MenubarTrigger testId="only-trigger">Only</MenubarTrigger>
        <MenubarContent testId="only-surface">
          <MenubarItem testId="only-item">Only item</MenubarItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu value="idle">
        <MenubarTrigger testId="idle-trigger" disabled>Idle</MenubarTrigger>
        <MenubarContent><MenubarItem>Nope</MenubarItem></MenubarContent>
      </MenubarMenu>
    </Menubar>
    <MenubarArrows testId="standalone-arrows">
      <control testId="standalone-child">Standalone</control>
    </MenubarArrows>
  </>, root);
}

export function mountBadMenu(root: NativeNode) {
  return renderApp(() => <Menubar><MenubarMenu value=""><MenubarTrigger>Bad</MenubarTrigger></MenubarMenu></Menubar>, root);
}
export function mountBadRadio(root: NativeNode) {
  return renderApp(() => <Menubar><MenubarMenu value="v"><MenubarTrigger>V</MenubarTrigger><MenubarRadioGroup><MenubarRadioItem value="">X</MenubarRadioItem></MenubarRadioGroup></MenubarMenu></Menubar>, root);
}
export function mountBadCheck(root: NativeNode) {
  return renderApp(() => <Menubar><MenubarMenu value="v"><MenubarTrigger>V</MenubarTrigger><MenubarCheckboxItem checked={"yes" as unknown as boolean}>X</MenubarCheckboxItem></MenubarMenu></Menubar>, root);
}
export function mountBadSeparator(root: NativeNode) {
  return renderApp(() => <MenubarSeparator children="bad" />, root);
}
`;

async function mounted(t, mountName = "mount") {
  const h = createHost();
  const { traps, layers, portals, geometry } = h;
  const mod = await moduleFor(t, fixture);
  const clock = createClock(t, h);
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
    assert.equal(h.locks.length, 0, "menubar must not lock scroll");
  });
  globalThis.__zgui = h.host;
  const root = h.makeNode("root");
  dispose = mountName === "none" ? undefined : mod[mountName](root);
  const send = (target, name, extras = {}) => {
    const node = typeof target === "string" ? h.byTestId(target) : target;
    assert.ok(node, `missing node ${target}`);
    return h.dispatch(node, name, extras);
  };
  const key = (target, extras = {}) => {
    const node = typeof target === "string" ? h.byTestId(target) : target;
    assert.ok(node, `missing node ${target}`);
    return h.dispatch(node, "KeyDown", extras);
  };
  const active = records => records.filter(r => r.active).length;
  const flush = () => {
    h.flush();
    clock.frame();
    clock.advance(0);
  };
  return {
    h,
    mod,
    clock,
    traps,
    layers,
    portals,
    geometry,
    send,
    key,
    active,
    flush,
    finish: () => {
      const stop = dispose;
      dispose = undefined;
      stop?.();
    },
  };
}

test("menubar roles, horizontal roving tabindex, and closed bar behavior", async t => {
  const { h, mod, traps, send, key, finish } = await mounted(t);
  const bar = h.byTestId("bar");
  const fileTrigger = h.byTestId("file-trigger");
  const toolsTrigger = h.byTestId("tools-trigger");
  const editTrigger = h.byTestId("edit-trigger");
  const viewTrigger = h.byTestId("view-trigger");

  assert.ok(h.stylesheets.has("MenubarStyle"));
  assert.equal(mod.log.triggers.length, 1, "trigger ref runs once");
  assert.equal(mod.log.triggers[0], fileTrigger);
  assert.equal(bar.props.get("role"), "menuBar");
  assert.equal(bar.props.get("orientation"), "horizontal");
  assert.equal(bar.props.get("ariaLabel"), "Application");
  assert.ok(String(bar.props.get("class")).includes("zui-menubar"));

  assert.equal(fileTrigger.props.get("role"), "menuItem");
  assert.equal(fileTrigger.props.get("ariaHasPopup"), "menu");
  assert.equal(fileTrigger.props.get("expanded"), false);
  assert.equal(fileTrigger.props.get("tabIndex"), 0);
  assert.equal(toolsTrigger.props.get("tabIndex"), -1);
  assert.equal(editTrigger.props.get("tabIndex"), -1);
  assert.equal(viewTrigger.props.get("tabIndex"), -1);
  assert.equal(h.byTestId("file-surface"), undefined);

  h.host.focus(editTrigger);
  send("view-trigger", "onPointerEnter");
  assert.equal(
    h.byTestId("edit-surface"),
    undefined,
    "focusing trigger on closed bar does not open",
  );
  assert.equal(
    h.byTestId("view-surface"),
    undefined,
    "hovering trigger on closed bar does not open",
  );

  key("file-trigger", { key: "ArrowRight", character: "ArrowRight" });
  assert.equal(
    h.focused.current,
    editTrigger,
    "prior focus stays untouched on character-producing ArrowRight",
  );
  h.host.focus(fileTrigger);
  key("file-trigger", { key: "ArrowRight" });
  assert.equal(h.focused.current, editTrigger, "ArrowRight skips disabled Tools trigger");
  key("edit-trigger", { key: "End" });
  assert.equal(h.focused.current, viewTrigger);
  key("view-trigger", { key: "Home" });
  assert.equal(h.focused.current, fileTrigger);
  key("file-trigger", { key: "ArrowDown", character: "ArrowDown" });
  assert.equal(h.byTestId("file-surface"), undefined, "character ArrowDown does not open menu");

  key("file-trigger", { key: "ArrowDown" });
  const fileSurface = h.byTestId("file-surface");
  assert.ok(fileSurface);
  assert.equal(traps.length, 0, "menubar menus never install a focus trap");
  assert.equal(fileSurface.props.get("role"), "menu");
  assert.ok(
    !String(fileSurface.props.get("class")).includes("zui-surface"),
    "menubar content omits zui-surface class",
  );
  assert.ok(String(fileSurface.props.get("class")).includes("zui-menubar__content"));
  assert.equal(
    h.focused.current,
    h.byTestId("file-new"),
    "opening focuses first enabled item, skipping disabled item",
  );
  finish();
});

test("open bar switches cleanly via hover, focus, and MenubarArrows without leftover surfaces", async t => {
  const { h, send, key, flush, finish } = await mounted(t);
  send("file-trigger", "onClick");
  assert.ok(h.byTestId("file-surface"));
  assert.equal(h.byTestId("file-menu").props.get("data-state"), "open");
  assert.equal(h.byTestId("file-trigger").props.get("expanded"), true);
  assert.equal(h.byTestId("file-trigger").props.get("ariaControls"), h.byTestId("file-surface"));
  assert.equal(h.focused.current, h.byTestId("file-new"));

  // Vertical roving & typeahead inside MenubarContent
  key("file-new", { key: "ArrowDown" });
  assert.equal(h.focused.current, h.byTestId("file-open"));
  key("file-open", { key: "e", character: "e", keyWithoutModifiers: "e" });
  assert.equal(h.focused.current, h.byTestId("file-export"), "typeahead jumps inside menubar menu");

  // MenubarArrows ArrowRight moves across the bar into Edit, unmounting File cleanly
  const arrowRight = key("file-export", { key: "ArrowRight" });
  assert.equal(arrowRight.defaultPrevented, true);
  assert.equal(arrowRight.propagationStopped, true);
  flush();
  assert.equal(
    h.byTestId("file-surface"),
    undefined,
    "previous menu closes and unmounts immediately",
  );
  assert.equal(h.byTestId("file-menu").props.get("data-state"), "closed");
  assert.ok(h.byTestId("edit-surface"), "next menu opens");
  assert.equal(h.byTestId("edit-menu").props.get("data-state"), "open");
  assert.equal(h.focused.current, h.byTestId("edit-undo"));

  // Pointer enter on View while Edit is open swaps to View; hovering disabled Tools does nothing
  send("tools-trigger", "onPointerEnter");
  assert.ok(h.byTestId("edit-surface"));
  send("view-trigger", "onPointerEnter");
  flush();
  assert.equal(h.byTestId("edit-surface"), undefined);
  assert.ok(h.byTestId("view-surface"));
  assert.equal(h.focused.current, h.byTestId("radio-single"));

  // ArrowLeft from View wraps past disabled Tools? No: file <- edit <- view
  const arrowLeft = key("radio-single", { key: "ArrowLeft" });
  assert.equal(arrowLeft.defaultPrevented, true);
  flush();
  assert.equal(h.byTestId("view-surface"), undefined);
  assert.ok(h.byTestId("edit-surface"));

  // Clicking already-open trigger toggles closed
  send("edit-trigger", "onClick");
  flush();
  assert.equal(h.byTestId("edit-surface"), undefined);
  finish();
});

test("single-menu bar and standalone MenubarArrows leave ArrowLeft and ArrowRight unhandled", async t => {
  const { h, send, key, finish } = await mounted(t, "mountSingle");
  send("only-trigger", "onClick");
  assert.equal(h.focused.current, h.byTestId("only-item"));

  const right = key("only-item", { key: "ArrowRight" });
  assert.equal(right.defaultPrevented, false);
  assert.equal(right.propagationStopped, false);
  assert.equal(
    h.focused.current,
    h.byTestId("only-item"),
    "focus stays on menu item in 1-enabled-menu bar",
  );

  const left = key("only-item", { key: "ArrowLeft" });
  assert.equal(left.defaultPrevented, false);
  assert.equal(left.propagationStopped, false);

  const standalone = key("standalone-child", { key: "ArrowRight" });
  assert.equal(standalone.defaultPrevented, false);
  assert.equal(standalone.propagationStopped, false);
  finish();
});

test("item structure, check/radio defaults, dismiss focus rules, and unmounting open menu", async t => {
  const { h, mod, layers, send, flush, active, finish } = await mounted(t);
  send("file-trigger", "onClick");

  const itemNew = h.byTestId("file-new");
  assert.equal(itemNew.props.get("ariaKeyShortcuts"), "Ctrl+N");
  assert.ok(String(itemNew.props.get("class")).includes("zui-menubar__item"));
  assert.equal(itemNew.children[0].props.get("class"), "zui-menubar__item-label");
  assert.equal(itemNew.children[1].props.get("class"), "zui-menubar__shortcut");
  assert.equal(h.host.getTextContent(itemNew.children[1]), "Ctrl+N");

  const sep = h.byTestId("file-sep");
  assert.equal(sep.props.get("ariaHidden"), true);
  assert.equal(sep.props.get("class"), "zui-menubar__separator");

  // Submenu inside menubar closes root bar menu when leaf item selects
  send("file-share", "onClick");
  assert.ok(h.byTestId("file-share-sub"));
  send("file-share-copy", "onClick");
  flush();
  assert.deepEqual(mod.log.selects, ["share-copy"]);
  assert.equal(h.byTestId("file-surface"), undefined);
  assert.equal(h.focused.current, h.byTestId("file-trigger"));

  // MenubarCheckboxItem defaults to closeOnSelect = false
  send("edit-trigger", "onClick");
  assert.ok(h.byTestId("edit-label").props.get("class").includes("zui-menubar__label"));
  assert.equal(h.byTestId("edit-group").props.get("role"), "group");
  const wrap = h.byTestId("edit-wrap");
  assert.equal(wrap.props.get("role"), "menuItemCheckBox");
  assert.equal(wrap.props.get("data-state"), "unchecked");
  send("edit-wrap", "onClick");
  assert.deepEqual(mod.log.checks, [true]);
  assert.equal(wrap.props.get("data-state"), "checked");
  assert.ok(h.byTestId("edit-surface"), "checkbox stays open by default");

  // Unmounting the currently open MenubarMenu must reset bar open state
  mod.setShowEdit(false);
  flush();
  assert.equal(h.byTestId("edit-surface"), undefined);
  send("view-trigger", "onPointerEnter");
  assert.equal(
    h.byTestId("view-surface"),
    undefined,
    "hovering remaining menu stays closed after open menu unmounted",
  );

  // MenubarRadioItem defaults to closeOnSelect = true and restores trigger focus
  send("view-trigger", "onClick");
  const split = h.byTestId("radio-split");
  const single = h.byTestId("radio-single");
  assert.equal(single.props.get("role"), "menuItemRadio");
  assert.equal(single.props.get("data-value"), "single");
  assert.equal(single.props.get("checked"), true);
  send("radio-split", "onClick");
  assert.deepEqual(mod.log.radios, ["split"]);
  assert.ok(h.byTestId("view-surface"), "radio with closeOnSelect=false keeps menu open");
  assert.equal(split.props.get("checked"), true);

  send("radio-single", "onClick");
  flush();
  assert.equal(
    h.byTestId("view-surface"),
    undefined,
    "radio with default closeOnSelect=true closes menu",
  );
  assert.equal(h.focused.current, h.byTestId("view-trigger"));

  // Escape dismisses and focuses trigger
  send("file-trigger", "onClick");
  assert.ok(h.byTestId("file-surface"));
  const escLayer = layers.find(l => l.active);
  escLayer.callback("escape");
  flush();
  assert.equal(h.byTestId("file-surface"), undefined);
  assert.equal(h.focused.current, h.byTestId("file-trigger"));

  // Outside press closes WITHOUT stealing focus back to trigger
  send("file-trigger", "onClick");
  const outsideBtn = h.byTestId("outside-btn");
  h.host.focus(outsideBtn);
  const outLayer = layers.find(l => l.active);
  outLayer.callback("outside-press");
  flush();
  assert.equal(h.byTestId("file-surface"), undefined);
  assert.equal(
    h.focused.current,
    outsideBtn,
    "outside press does not steal focus back to menubar trigger",
  );

  finish();
  assert.equal(active(layers), 0);
});

test("menubar validates required values, boolean checked states, and childless separator", async t => {
  await assert.rejects(() => mounted(t, "mountBadMenu"), TypeError);
  await assert.rejects(() => mounted(t, "mountBadRadio"), TypeError);
  await assert.rejects(() => mounted(t, "mountBadCheck"), TypeError);
  await assert.rejects(() => mounted(t, "mountBadSeparator"), TypeError);
});

test("menubar forwards application classes, returns Escape focus to MenuSubTrigger, and cascades nested leaf dismissal", async t => {
  const { h, mod, layers, send, key, flush, finish } = await mounted(t);
  send("file-trigger", "onClick");
  const fileSurface = h.byTestId("file-surface");
  const fileNew = h.byTestId("file-new");
  assert.match(String(fileSurface.props.get("class")), /\bzui-menubar__content\b/);
  assert.match(String(fileSurface.props.get("class")), /\bapp-content-class\b/);
  assert.match(String(fileNew.props.get("class")), /\bzui-menubar__item\b/);
  assert.match(String(fileNew.props.get("class")), /\bapp-item-class\b/);

  const shareTrigger = h.byTestId("file-share");
  send(shareTrigger, "onClick");
  const shareSub = h.byTestId("file-share-sub");
  assert.ok(shareSub);
  h.host.focus(h.byTestId("file-share-copy"));
  layers.findLast(record => record.active).callback("escape");
  assert.equal(shareTrigger.props.get("expanded"), false);
  assert.equal(
    h.focused.current,
    shareTrigger,
    "submenu Escape inside menubar focuses MenuSubTrigger",
  );
  flush();
  assert.equal(h.byTestId("file-share-sub"), undefined);
  assert.equal(h.byTestId("file-surface"), fileSurface);

  send(shareTrigger, "onClick");
  key(h.byTestId("file-share-more"), { key: "ArrowRight" });
  assert.ok(h.byTestId("file-share-more-sub"));
  mod.log.subOpens.length = 0;
  send("file-share-email", "onClick");
  assert.deepEqual(
    mod.log.subOpens,
    [false],
    "nested menubar leaf selection cascades onOpenChange(false)",
  );
  flush();
  assert.equal(h.byTestId("file-surface"), undefined);
  assert.equal(h.focused.current, h.byTestId("file-trigger"));
  finish();
});
