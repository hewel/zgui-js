import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button, Dialog, DialogContent, DialogTitle,
  DropdownMenu, DropdownMenuContent, DropdownMenuTrigger,
  MenuCheckboxItem, MenuContent, MenuGroup, MenuItem, MenuLabel,
  MenuRadioGroup, MenuRadioItem, MenuSeparator, MenuTypeahead,
  MenuSub, MenuSubContent, MenuSubTrigger } from "@hewel/zgui/components";
export { headingToward } from "@hewel/zgui/components";
export const log = {
  selects: [] as string[], checks: [] as boolean[], radios: [] as string[],
  opens: [] as boolean[], subOpens: [] as boolean[], calls: [] as string[], mounts: 0, cleans: 0,
};
const [loose, setLoose] = createSignal(["print", "pending", "props"]);
const [keyOrder, setKeyOrder] = createSignal(["new", "save", "sendnow", "nel", "dsx", "dsy", "zulu"]);
const [off, setOff] = createSignal([] as string[]);
const [overrides, setOverrides] = createSignal({} as Record<string, string>);
const [veto, setVeto] = createSignal(false);
const [open, setOpen] = createSignal(false);
const [checkedA, setCheckedA] = createSignal(undefined as boolean | undefined);
const [radioValue, setRadioValue] = createSignal(undefined as string | undefined);
const [shortcut, setShortcut] = createSignal("⌘P" as string | null);
const [trigOff, setTrigOff] = createSignal(false);
const [subShown, setSubShown] = createSignal(true);
const [subTriggerShown, setSubTriggerShown] = createSignal(true);
const [subOpen, setSubOpen] = createSignal(undefined as boolean | undefined);
const [subOff, setSubOff] = createSignal(false);
const [removeOnFocus, setRemoveOnFocus] = createSignal(false);
const [itemDestructive, setItemDestructive] = createSignal(false);
const [itemInset, setItemInset] = createSignal(false);
export { setLoose, setKeyOrder, setOff, setOverrides, setVeto, setOpen, setCheckedA, setRadioValue, setShortcut, setTrigOff,
  setSubShown, setSubTriggerShown, setSubOpen, setSubOff, setRemoveOnFocus, setItemDestructive, setItemInset };
const bumpers: Record<string, () => void> = {};
export const bump = (id = "counter") => bumpers[id]?.();
function Counter(props: { id?: string }) {
  log.mounts += 1;
  const [count, setCount] = createSignal(0);
  const id = props.id ?? "counter";
  bumpers[id] = () => setCount(n => n + 1);
  onCleanup(() => { log.cleans += 1; delete bumpers[id]; });
  return <text testId={id}>{count()}</text>;
}
const base: Record<string, string> = { print: "Print", pending: "Pending", props: "Properties", save: "Save", send: "Send" };
const keyBase: Record<string, string> = { new: "New file", save: "Save", sendnow: "Send Now", nel: "Édit", dsx: "𐐷x", dsy: "𐐷y", zulu: "Zulu" };
const label = (id: string) => overrides()[id] ?? base[id];
const keyLabel = (id: string) => overrides()[id] ?? keyBase[id];
const vetoable = {
  onClick(event: { preventDefault(): void }) { log.calls.push("click"); if (veto()) event.preventDefault(); },
  onPointerDown(event: { preventDefault(): void }) { log.calls.push("down"); if (veto()) event.preventDefault(); },
  onPointerEnter(event: { preventDefault(): void }) { log.calls.push("enter"); if (veto()) event.preventDefault(); },
  onPointerLeave(event: { preventDefault(): void }) { log.calls.push("leave"); if (veto()) event.preventDefault(); },
  onFocus(event: { preventDefault(): void }) { log.calls.push("focus"); if (veto()) event.preventDefault(); },
  onBlur() { log.calls.push("blur"); },
  onKeyDown(event: { preventDefault(): void }) { log.calls.push("key"); if (veto()) event.preventDefault(); },
};
function row(id: string) {
  return <MenuItem testId={"item-" + id} disabled={id === "pending" || off().includes(id)}
    closeOnSelect={id === "send" ? false : undefined}
    onSelect={() => log.selects.push(id)} {...(id === "print" ? vetoable : {})}>
    {id === "send" ? <><Counter id="menu-counter" />{label(id)}</> : label(id)}</MenuItem>;
}
function Body() {
  return <>
    <DropdownMenuTrigger testId="trigger" disabled={trigOff()} {...vetoable}>Menu</DropdownMenuTrigger>
    <DropdownMenuContent testId="surface">
      <For each={loose()}>{id => id === "props" ? <box>{row(id)}</box> : row(id)}</For>
      <MenuGroup label="File"><For each={["save", "send"]}>{id => row(id)}</For></MenuGroup>
    </DropdownMenuContent>
  </>;
}
export function mount(root: NativeNode) {
  return renderApp(() => <DropdownMenu><Body /></DropdownMenu>, root);
}
export function mountControlled(root: NativeNode) {
  return renderApp(() => {
    const [shown, setShown] = createSignal(true);
    return <Show when={shown()}>
      <DropdownMenu open={open()} onOpenChange={next => log.opens.push(next)}>
        <DropdownMenuTrigger testId="trigger" {...vetoable}>Menu</DropdownMenuTrigger>
        <DropdownMenuContent testId="surface">
          <MenuItem testId="item-print" onSelect={() => log.selects.push("print")}>Print</MenuItem>
          <MenuItem testId="item-gone" onSelect={() => { log.selects.push("gone"); setShown(false); }}>Gone</MenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </Show>;
  }, root);
}
export function mountKeys(root: NativeNode) {
  return renderApp(() => <DropdownMenu>
    <DropdownMenuTrigger testId="trigger">Keys</DropdownMenuTrigger>
    <DropdownMenuContent testId="surface">
      <For each={keyOrder()}>{id => <MenuItem testId={"item-" + id} disabled={id === "zulu"}
        onSelect={() => log.selects.push(id)}>{keyLabel(id)}</MenuItem>}</For>
    </DropdownMenuContent>
  </DropdownMenu>, root);
}
export function mountChecks(root: NativeNode) {
  return renderApp(() => <DropdownMenu>
    <DropdownMenuTrigger testId="trigger">Checks</DropdownMenuTrigger>
    <DropdownMenuContent testId="surface">
      <MenuCheckboxItem testId="check-a" checked={checkedA()} onCheckedChange={value => log.checks.push(value)}>Alpha</MenuCheckboxItem>
      <MenuCheckboxItem testId="check-b" defaultChecked closeOnSelect={false}
        onCheckedChange={value => log.checks.push(value)}><Counter id="check-counter" />Beta</MenuCheckboxItem>
      <MenuRadioGroup testId="group-a" label="Tone" value={radioValue()} onValueChange={value => log.radios.push(value)}>
        <MenuRadioItem testId="radio-x" value="x" closeOnSelect={false}><Counter id="radio-counter" />X</MenuRadioItem>
        <MenuRadioItem testId="radio-y" value="y">Y</MenuRadioItem>
      </MenuRadioGroup>
      <MenuRadioGroup testId="group-b">
        <MenuRadioItem testId="radio-p" value="p" closeOnSelect={false}>P</MenuRadioItem>
      </MenuRadioGroup>
    </DropdownMenuContent>
  </DropdownMenu>, root);
}
export function mountNested(root: NativeNode) {
  return renderApp(() => <DropdownMenu>
    <DropdownMenuTrigger testId="trigger">Outer</DropdownMenuTrigger>
    <DropdownMenuContent testId="outer-surface">
      <MenuItem testId="outer-item" onSelect={() => log.selects.push("outer")}>Outer item</MenuItem>
      <DropdownMenu>
        <DropdownMenuTrigger testId="inner-trigger">Inner</DropdownMenuTrigger>
        <DropdownMenuContent testId="inner-surface">
          <MenuItem testId="inner-item" onSelect={() => log.selects.push("inner")}>Inner item</MenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </DropdownMenuContent>
  </DropdownMenu>, root);
}
export function mountDialog(root: NativeNode) {
  return renderApp(() => <Dialog defaultOpen>
    <DialogContent testId="dialog">
      <DialogTitle>Dialog</DialogTitle>
      <DropdownMenu defaultOpen>
        <DropdownMenuTrigger testId="d-trigger">Menu</DropdownMenuTrigger>
        <DropdownMenuContent testId="d-surface"><MenuItem testId="d-item">Item</MenuItem></DropdownMenuContent>
      </DropdownMenu>
      <MenuContent testId="orphan"><MenuItem testId="orphan-item">Orphan</MenuItem></MenuContent>
    </DialogContent>
  </Dialog>, root);
}
export function mountMeta(root: NativeNode) {
  return renderApp(() => <DropdownMenu defaultOpen>
    <DropdownMenuTrigger testId="trigger">Meta</DropdownMenuTrigger>
    <DropdownMenuContent testId="surface">
      <MenuLabel testId="label" inset class="caller-label">Heading</MenuLabel>
      <MenuGroup testId="group" label="Section" class="caller-group">
        <MenuItem testId="meta-item" shortcut={shortcut()} destructive={itemDestructive()} inset={itemInset()}
          onSelect={() => log.selects.push("meta")}>
          <Counter id="meta-counter" />Rename</MenuItem>
      </MenuGroup>
      <MenuSeparator testId="sep" />
    </DropdownMenuContent>
  </DropdownMenu>, root);
}
export function mountDescendants(root: NativeNode) {
  return renderApp(() => <DropdownMenu>
    <DropdownMenuTrigger testId="trigger">Descendants</DropdownMenuTrigger>
    <DropdownMenuContent testId="surface">
      <MenuLabel testId="label">Actions</MenuLabel>
      <MenuItem testId="row-a" closeOnSelect={false}><text>Alpha</text><control testId="child" tabIndex={0}>Inner control</control></MenuItem>
      <MenuItem testId="disabled" disabled>Disabled</MenuItem>
      <MenuItem testId="row-b" closeOnSelect={false}>Beta</MenuItem>
      <MenuItem testId="row-c" closeOnSelect={false}>Charlie</MenuItem>
      <control testId="loose" tabIndex={0}>Loose focus</control>
      <MenuTypeahead onKeyDown={event => event.stopPropagation()}><control testId="blocked" tabIndex={0}>Blocked propagation</control></MenuTypeahead>
    </DropdownMenuContent>
  </DropdownMenu>, root);
}
export function mountSub(root: NativeNode) {
  return renderApp(() => <DropdownMenu onOpenChange={value => log.opens.push(value)}>
    <DropdownMenuTrigger testId="trigger">Menu</DropdownMenuTrigger>
    <DropdownMenuContent testId="surface">
      <MenuItem testId="before" closeOnSelect={false}>Before</MenuItem>
      <Show when={subShown()}><MenuSub open={subOpen()} onOpenChange={value => log.subOpens.push(value)}>
        <Show when={subTriggerShown()}><MenuSubTrigger testId="sub-trigger" disabled={subOff()} inset {...vetoable}
          onFocus={event => { vetoable.onFocus(event); if (removeOnFocus()) setSubShown(false); }}>Export<Counter id="trigger-counter" /></MenuSubTrigger></Show>
        <MenuSubContent testId="sub-surface" onPointerEnter={vetoable.onPointerEnter} onKeyDown={vetoable.onKeyDown}>
          <MenuItem testId="keep" closeOnSelect={false}>Keep<Counter id="sub-counter" /></MenuItem>
          <MenuSub><MenuSubTrigger testId="deep-trigger">More</MenuSubTrigger>
            <MenuSubContent testId="deep-surface"><MenuItem testId="leaf" onSelect={() => log.selects.push("leaf")}>Leaf</MenuItem></MenuSubContent>
          </MenuSub>
        </MenuSubContent>
      </MenuSub></Show>
      <MenuItem testId="after" closeOnSelect={false}>After</MenuItem>
    </DropdownMenuContent>
  </DropdownMenu>, root);
}
export function mountBadSeparator(root: NativeNode) {
  return renderApp(() => <MenuSeparator children="unexpected" />, root);
}
export function mountBadRadio(root: NativeNode) {
  return renderApp(() => <DropdownMenu defaultOpen><DropdownMenuTrigger testId="trigger" />
    <DropdownMenuContent><MenuRadioItem value="">{""}</MenuRadioItem></DropdownMenuContent></DropdownMenu>, root);
}
export function mountBadCheck(root: NativeNode) {
  return renderApp(() => <DropdownMenu defaultOpen><DropdownMenuTrigger testId="trigger" />
    <DropdownMenuContent><MenuCheckboxItem checked={"yes" as unknown as boolean}>Bad</MenuCheckboxItem></DropdownMenuContent></DropdownMenu>, root);
}
`;

async function mounted(t, mountName = "mount", allowScrollLock = false) {
  const activations = [];
  const h = createHost();
  const { animating: busy, traps, locks, layers, portals, geometry } = h;
  assert.equal(globalThis.__zgui, undefined);
  const mod = await moduleFor(t, fixture);
  assert.equal(portals.length, 0, "module import must not allocate native nodes");
  const clock = createClock(t, h);
  let dispose;
  t.after(() => {
    try {
      dispose?.();
    } finally {
      delete globalThis.__zgui;
    }
    if (!allowScrollLock) assert.equal(locks.length, 0, "menu surfaces must not lock scroll");
  });
  globalThis.__zgui = h.host;
  const root = h.makeNode("root");
  dispose = mountName === "none" ? undefined : mod[mountName](root);
  const send = (target, name, extras = {}) => {
    const node = typeof target === "string" ? h.byTestId(target) : target;
    assert.ok(node, `missing node ${target}`);
    return h.dispatch(node, name, { activateNow: () => activations.push(node), ...extras });
  };
  const key = (target, extras = {}) => {
    const node = typeof target === "string" ? h.byTestId(target) : target;
    assert.ok(node, `missing node ${target}`);
    return h.dispatch(node, "KeyDown", { activateNow: () => activations.push(node), ...extras });
  };
  const emit = (record, value) => record.emit(value);
  const end = node => h.dispatch(node, "AnimationEnd");
  const flush = () => {
    h.flush();
    clock.frame();
    clock.advance(0);
  };
  const active = records => records.filter(record => record.active).length;
  const textOf = node =>
    node.tag === "#text" ? (node.props.get("text") ?? "") : node.children.map(textOf).join("");
  return {
    h,
    mod,
    clock,
    busy,
    traps,
    locks,
    layers,
    portals,
    geometry,
    activations,
    root,
    send,
    key,
    emit,
    end,
    flush,
    active,
    textOf,
    finish: () => {
      const stop = dispose;
      dispose = undefined;
      stop?.();
    },
  };
}

test("dropdown stays lazy, wires trigger relations, contracts and vetoable opens", async t => {
  const { h, mod, traps, locks, layers, portals, geometry, send, key, flush, finish } =
    await mounted(t);
  const trigger = h.byTestId("trigger");
  assert.equal(h.byTestId("surface"), undefined, "content children stay lazy before open");
  assert.equal(traps.length, 0);
  key("trigger", { key: "ArrowDown", character: "ArrowDown", keyWithoutModifiers: "ArrowDown" });
  assert.equal(h.byTestId("surface"), undefined, "a character-producing ArrowDown must not open");
  key("trigger", { key: "ArrowUp" });
  const surface = h.byTestId("surface");
  assert.ok(surface, "named ArrowUp opens");
  assert.equal(trigger.props.get("ariaHasPopup"), "menu");
  assert.equal(trigger.props.get("expanded"), true);
  assert.equal(trigger.props.get("ariaControls"), surface);
  assert.equal(trigger.props.get("data-state"), "open");
  assert.equal(surface.props.get("role"), "menu");
  assert.equal(surface.props.get("ariaModal"), true);
  assert.equal(surface.props.get("ariaPopupFor"), trigger);
  assert.equal(traps.length, 1, "menu surface traps focus");
  assert.equal(traps[0].node, surface);
  assert.equal(locks.length, 0, "menus never lock scroll");
  assert.equal(portals.length, 1);
  assert.equal(portals[0].layer, "popover");
  assert.equal(layers.length, 1);
  assert.equal(layers[0].options.dismissOnOutsidePress, true);
  assert.equal(layers[0].options.dismissOnEscape, true);
  assert.equal(layers[0].options.exclude, trigger);
  assert.equal(layers[0].options.leaving, false);
  assert.ok(geometry.find(record => record.node === trigger && record.kind === "borderBox"));
  assert.ok(geometry.find(record => record.kind === "contentSize"));
  assert.ok(geometry.find(record => record.kind === "viewport"));
  layers[0].callback("outside-press");
  flush();
  assert.equal(h.byTestId("surface"), undefined, "dismissal closes the root");
  assert.equal(trigger.props.get("expanded"), false);
  key("trigger", { key: "ArrowDown" });
  assert.ok(h.byTestId("surface"), "named ArrowDown also opens");
  layers.at(-1).callback("escape");
  flush();
  mod.setVeto(true);
  h.flush();
  send("trigger", "onClick");
  key("trigger", { key: "ArrowDown" });
  assert.equal(
    h.byTestId("surface"),
    undefined,
    "caller preventDefault vetoes click and arrow open",
  );
  mod.setVeto(false);
  h.flush();
  mod.setTrigOff(true);
  h.flush();
  send("trigger", "onClick");
  key("trigger", { key: "ArrowDown" });
  assert.equal(h.byTestId("surface"), undefined, "a disabled trigger never opens");
  mod.setTrigOff(false);
  h.flush();
  send("trigger", "onPointerDown", { button: "primary" });
  send("trigger", "onClick");
  assert.ok(h.byTestId("surface"), "click opens after veto released");
  layers.at(-1).callback("escape");
  flush();
  finish();
  assert.equal(h.byTestId("surface"), undefined);
});

test("selection requests close and presence keeps busy exits", async t => {
  const { h, mod, busy, send, end, flush, finish, textOf } = await mounted(t);
  send("trigger", "onClick");
  const surface = h.byTestId("surface");
  busy.set(surface, true);
  h.host.focus(h.byTestId("item-print"));
  send("item-print", "onClick");
  assert.deepEqual(mod.log.selects, ["print"], "onSelect runs once");
  flush();
  assert.equal(h.byTestId("surface"), surface, "busy exit keeps the surface mounted");
  assert.equal(surface.props.get("data-state"), "closed");
  assert.equal(
    h.byTestId("trigger").props.get("ariaControls"),
    surface,
    "ariaControls survives a busy exit",
  );
  send("trigger", "onClick");
  assert.equal(h.byTestId("surface"), surface, "reopen while leaving preserves surface identity");
  mod.bump("menu-counter");
  h.flush();
  assert.equal(textOf(h.byTestId("menu-counter")), "1", "child state survives the reopen");
  send("item-send", "onClick");
  assert.deepEqual(mod.log.selects, ["print", "send"]);
  assert.equal(h.byTestId("surface"), surface, "closeOnSelect false keeps the menu open");
  send("item-print", "onClick");
  flush();
  assert.equal(h.byTestId("surface"), surface, "busy exit still retained after second close");
  busy.set(surface, false);
  end(surface);
  assert.equal(h.byTestId("surface"), undefined, "completed exit unmounts");
  assert.equal(h.byTestId("trigger").props.get("ariaControls"), undefined);
  send("trigger", "onClick");
  const fresh = h.byTestId("surface");
  assert.notEqual(fresh, surface, "completed exit mounts a fresh surface");
  assert.equal(textOf(h.byTestId("menu-counter")), "0", "fresh surface resets child state");
  send("item-print", "onClick");
  flush();
  finish();
});

test("controlled root only requests and onSelect unmounting emits no late request", async t => {
  const { h, mod, send, finish } = await mounted(t, "mountControlled");
  send("trigger", "onClick");
  assert.deepEqual(mod.log.opens, [true], "click requests open");
  assert.equal(h.byTestId("surface"), undefined, "controlled surface waits for acceptance");
  mod.setOpen(true);
  h.flush();
  const surface = h.byTestId("surface");
  assert.ok(surface);
  send("item-print", "onClick");
  assert.deepEqual(mod.log.selects, ["print"]);
  assert.deepEqual(mod.log.opens, [true, false], "select requests close");
  assert.equal(h.byTestId("surface"), surface, "controlled true keeps surface until accepted");
  send("item-gone", "onClick");
  assert.deepEqual(mod.log.selects, ["print", "gone"]);
  assert.deepEqual(
    mod.log.opens,
    [true, false],
    "root disposal during onSelect emits no late request",
  );
  assert.equal(h.byTestId("surface"), undefined);
  finish();
});

test("roving keeps native tree order, wraps, skips disabled and survives keyed edits", async t => {
  const { h, mod, send, key, finish } = await mounted(t);
  send("trigger", "onClick");
  const print = h.byTestId("item-print"),
    props = h.byTestId("item-props");
  const save = h.byTestId("item-save"),
    sendItem = h.byTestId("item-send"),
    pending = h.byTestId("item-pending");
  assert.equal(print.props.get("tabIndex"), 0);
  assert.equal(props.props.get("tabIndex"), -1);
  assert.equal(pending.props.get("tabIndex"), -1, "disabled rows are never tab stops");
  h.host.focus(print);
  key(print, { key: "ArrowDown" });
  assert.equal(h.focused.current, props, "ArrowDown skips the disabled row");
  assert.equal(props.props.get("tabIndex"), 0);
  assert.equal(print.props.get("tabIndex"), -1);
  key(props, { key: "ArrowDown" });
  assert.equal(h.focused.current, save);
  key(save, { key: "ArrowDown" });
  assert.equal(h.focused.current, sendItem);
  key(sendItem, { key: "ArrowDown" });
  assert.equal(h.focused.current, print, "ArrowDown wraps to the first enabled row");
  key(print, { key: "ArrowUp" });
  assert.equal(h.focused.current, sendItem, "ArrowUp wraps to the last enabled row");
  key(sendItem, { key: "Home" });
  assert.equal(h.focused.current, print);
  key(print, { key: "End" });
  assert.equal(h.focused.current, sendItem);
  const lateral = key(sendItem, { key: "ArrowRight" });
  assert.equal(lateral.defaultPrevented, false, "horizontal keys are ignored by a vertical menu");
  assert.equal(h.focused.current, sendItem);
  key(sendItem, { key: "ArrowDown", character: "ArrowDown", keyWithoutModifiers: "ArrowDown" });
  assert.equal(
    h.focused.current,
    sendItem,
    "character keys never fall through to named-key roving",
  );
  assert.deepEqual(mod.log.selects, [], "focus alone never selects");
  mod.setLoose(["pending", "props", "print"]);
  h.flush();
  assert.equal(h.byTestId("item-print"), print, "keyed reorder preserves row identity");
  h.host.focus(props);
  key(props, { key: "ArrowDown" });
  assert.equal(h.focused.current, print, "roving follows native tree order after reorder");
  mod.setOff(["print"]);
  h.flush();
  assert.equal(print.props.get("tabIndex"), -1, "disabling the active row retires its tab stop");
  assert.equal(props.props.get("tabIndex"), 0, "next enabled row takes the tab stop");
  mod.setOff(["print", "props", "save", "send", "pending"]);
  h.flush();
  for (const id of ["item-print", "item-props", "item-save", "item-send"])
    assert.equal(h.byTestId(id).props.get("tabIndex"), -1, `${id} tab stop gone when all disabled`);
  mod.setOff([]);
  h.flush();
  mod.setLoose(["pending"]);
  h.flush();
  assert.equal(h.byTestId("item-print"), undefined);
  assert.equal(
    h.byTestId("item-save").props.get("tabIndex"),
    0,
    "remaining enabled row keeps a valid tab stop",
  );
  finish();
});

test("typeahead queries live native text, cycles, resets and ignores non-character keys", async t => {
  const { h, mod, clock, key, send, finish } = await mounted(t, "mountKeys");
  send("trigger", "onClick");
  const focusOf = id => h.byTestId("item-" + id);
  h.host.focus(focusOf("new"));
  assert.equal(h.focused.current, focusOf("new"));
  key(focusOf("new"), { key: "s", character: "s" });
  assert.equal(h.focused.current, focusOf("save"), "s finds Save after New file");
  key(focusOf("save"), { key: "s", character: "s" });
  assert.equal(h.focused.current, focusOf("sendnow"), "repeated s cycles to Send Now");
  key(focusOf("sendnow"), { key: "s", character: "s" });
  assert.equal(h.focused.current, focusOf("save"), "repeated s wraps back to Save");
  clock.advance(1500);
  for (const character of ["n", "e", "w", " "])
    key(h.focused.current, { key: character, character });
  assert.equal(h.focused.current, focusOf("new"), "prefix 'new ' matches the multiword item");
  clock.advance(1500);
  key(h.focused.current, { key: "É", character: "É" });
  assert.equal(h.focused.current, focusOf("nel"), "casefolded É skips leading native whitespace");
  clock.advance(1500);
  key(h.focused.current, { key: "𐐷", character: "𐐷" });
  assert.equal(h.focused.current, focusOf("dsx"));
  key(focusOf("dsx"), { key: "𐐷", character: "𐐷" });
  assert.equal(h.focused.current, focusOf("dsy"), "supplementary characters cycle too");
  clock.advance(1000);
  const zulu = key(focusOf("dsy"), { key: "z", character: "z" });
  assert.equal(zulu.defaultPrevented, false, "disabled matching rows are excluded");
  assert.equal(h.focused.current, focusOf("dsy"));
  clock.advance(1500);
  const blank = key(focusOf("dsy"), { key: " ", character: " " });
  assert.equal(blank.defaultPrevented, false, "an empty prefix never consumes character space");
  const named = key(focusOf("dsy"), { key: " ", character: null });
  assert.equal(named.defaultPrevented, false, "named Space is not typeahead text");
  assert.equal(h.focused.current, focusOf("dsy"));
  key(focusOf("dsy"), {
    key: "s",
    character: "s",
    modifiers: { shift: true, control: true, alt: false, meta: false },
  });
  assert.equal(h.focused.current, focusOf("save"), "modifiers do not filter character matching");
  mod.setOverrides({ save: "Upload" });
  h.flush();
  clock.advance(1500);
  key(focusOf("save"), { key: "u", character: "u" });
  assert.equal(h.focused.current, focusOf("save"), "reactive native text drives later queries");
  mod.setOverrides({});
  h.flush();
  mod.setKeyOrder(["dsx", "dsy", "new", "sendnow", "save", "nel", "zulu"]);
  h.flush();
  clock.advance(1500);
  key(focusOf("save"), { key: "s", character: "s" });
  assert.equal(h.focused.current, focusOf("sendnow"), "search order follows the keyed reorder");
  key(focusOf("sendnow"), { key: "s", character: "s" });
  assert.equal(h.focused.current, focusOf("save"), "repeated characters cycle through matches");
  clock.advance(1500);
  key(focusOf("save"), { key: "s", character: "s" });
  key(h.focused.current, { key: "e", character: "e" });
  assert.equal(h.focused.current, focusOf("sendnow"), "'se' prefix matches Send Now");
  clock.advance(999);
  key(focusOf("sendnow"), { key: "r", character: "r" });
  assert.equal(h.focused.current, focusOf("sendnow"), "'ser' matches nothing within the window");
  clock.advance(1000);
  key(focusOf("sendnow"), { key: "n", character: "n" });
  assert.equal(h.focused.current, focusOf("new"), "expired prefix restarts the buffer");
  clock.advance(1500);
  key(focusOf("new"), { key: "s", character: "s" });
  assert.equal(h.focused.current, focusOf("sendnow"));
  const stale = [...clock.timers.values()].at(-1)?.callback;
  assert.ok(stale, "reset timer scheduled");
  key(h.focused.current, { key: "e", character: "e" });
  assert.equal(h.focused.current, focusOf("sendnow"), "prefix 'se' keeps Send Now");
  stale();
  key(focusOf("sendnow"), { key: "n", character: "n" });
  assert.equal(
    h.focused.current,
    focusOf("sendnow"),
    "a cancelled reset never clears a live prefix",
  );
  finish();
  assert.equal(clock.timers.size, 0, "disposal cancels the reset timer");
});

test("pointer boundaries track focus and primary activation without stealing", async t => {
  const { h, mod, send, key, activations, finish } = await mounted(t);
  send("trigger", "onClick");
  const surface = h.byTestId("surface");
  const print = h.byTestId("item-print"),
    props = h.byTestId("item-props"),
    pending = h.byTestId("item-pending");
  send(print, "onPointerEnter");
  assert.equal(h.focused.current, print, "pointer enter focuses the row");
  assert.equal(h.focused.reveal, false, "hover focus does not reveal");
  assert.equal(print.props.get("tabIndex"), 0);
  send(props, "onPointerEnter");
  assert.equal(h.focused.current, props);
  send(print, "onPointerLeave");
  assert.equal(h.focused.current, props, "a stale leave cannot steal focus back");
  send(props, "onPointerLeave");
  assert.equal(h.focused.current, surface, "leaving the focused row returns focus to the surface");
  send(print, "onPointerEnter");
  key(print, { key: "ArrowDown" });
  assert.equal(h.focused.current, props);
  send(print, "onPointerLeave");
  assert.equal(h.focused.current, props, "blurred rows cannot steal focus on leave");
  send(pending, "onPointerEnter");
  assert.equal(h.focused.current, props, "disabled rows ignore pointer enter");
  mod.setVeto(true);
  h.flush();
  send(print, "onPointerEnter");
  assert.equal(h.focused.current, props, "caller veto skips hover focus");
  send(print, "onPointerDown", { button: "primary", activateNow: () => activations.push(print) });
  assert.equal(activations.length, 0, "caller veto skips activateNow");
  mod.setVeto(false);
  h.flush();
  send(print, "onPointerDown", { button: "secondary", activateNow: () => activations.push(print) });
  assert.equal(activations.length, 0, "secondary buttons never activateNow");
  send(print, "onPointerDown", { button: "primary", activateNow: () => activations.push(print) });
  assert.deepEqual(activations, [print], "primary down requests activation once");
  mod.setVeto(true);
  h.flush();
  send(print, "onClick");
  assert.deepEqual(mod.log.selects, [], "caller veto skips selection and closing");
  assert.ok(h.byTestId("surface"));
  mod.setVeto(false);
  h.flush();
  send(print, "onPointerEnter");
  assert.equal(h.focused.current, print);
  mod.setVeto(true);
  h.flush();
  send(print, "onPointerLeave");
  assert.equal(h.focused.current, print, "caller veto skips the leave handoff");
  mod.setVeto(false);
  h.flush();
  send(print, "onPointerLeave");
  assert.equal(h.focused.current, surface, "unvetoed leave returns focus to the surface");
  finish();
});

test("checkbox and radio items keep native state, indicators and group isolation", async t => {
  const { h, mod, send, flush, finish, textOf } = await mounted(t, "mountChecks");
  send("trigger", "onClick");
  const checkA = h.byTestId("check-a");
  assert.equal(checkA.props.get("role"), "menuItemCheckBox");
  assert.equal(checkA.props.get("checked"), false);
  assert.equal(checkA.props.get("data-state"), "unchecked");
  send("check-a", "onClick");
  assert.deepEqual(mod.log.checks, [true]);
  flush();
  assert.equal(h.byTestId("surface"), undefined, "checkbox defaults closeOnSelect true");
  send("trigger", "onClick");
  assert.equal(
    h.byTestId("check-a").props.get("checked"),
    false,
    "uncontrolled state is owned by the fresh mount",
  );
  const checkB = h.byTestId("check-b");
  assert.equal(checkB.props.get("checked"), true);
  const indicator = checkB.children[0];
  const counter = h.byTestId("check-counter");
  mod.bump("check-counter");
  h.flush();
  const firstIcon = indicator.children[0];
  assert.ok(firstIcon, "defaultChecked draws the tick icon");
  send("check-b", "onClick");
  send("check-b", "onClick");
  send("check-b", "onClick");
  send("check-b", "onClick");
  assert.deepEqual(mod.log.checks, [true, false, true, false, true]);
  assert.ok(h.byTestId("surface"), "closeOnSelect false keeps the menu open");
  assert.equal(
    h.byTestId("check-b").children[0],
    indicator,
    "indicator container survives toggles",
  );
  assert.notEqual(indicator.children[0], firstIcon, "each check draws a fresh tick icon");
  assert.equal(h.byTestId("check-counter"), counter, "body counter survives indicator swaps");
  assert.equal(textOf(h.byTestId("check-counter")), "1", "counter state preserved");
  const radioX = h.byTestId("radio-x"),
    radioY = h.byTestId("radio-y");
  assert.equal(h.byTestId("group-a").props.get("role"), "group");
  assert.equal(h.byTestId("group-a").props.get("ariaLabel"), "Tone");
  assert.equal(radioX.props.get("role"), "menuItemRadio");
  assert.equal(radioX.props.get("data-value"), "x");
  assert.equal(radioX.props.get("checked"), false);
  assert.equal(radioX.props.get("ariaRadioGroup"), undefined, "no fabricated radio relations");
  h.host.focus(radioY);
  assert.deepEqual(mod.log.radios, [], "focus alone never picks a radio value");
  send("radio-x", "onClick");
  send("radio-x", "onClick");
  assert.deepEqual(mod.log.radios, ["x"], "repeating the same value stays silent");
  assert.equal(h.byTestId("radio-x").props.get("checked"), true);
  send("radio-y", "onClick");
  assert.deepEqual(mod.log.radios, ["x", "y"]);
  flush();
  assert.equal(h.byTestId("surface"), undefined, "radio defaults closeOnSelect true");
  send("trigger", "onClick");
  assert.equal(
    h.byTestId("radio-y").props.get("checked"),
    false,
    "uncontrolled radio resets per mount",
  );
  mod.setRadioValue("x");
  h.flush();
  assert.equal(h.byTestId("radio-x").props.get("checked"), true);
  send("radio-y", "onClick");
  send("trigger", "onClick");
  assert.deepEqual(mod.log.radios, ["x", "y", "y"], "controlled denial still logs the request");
  assert.equal(
    h.byTestId("radio-x").props.get("checked"),
    true,
    "controlled parent vetoes the change",
  );
  mod.setRadioValue("y");
  h.flush();
  assert.equal(h.byTestId("radio-y").props.get("checked"), true);
  send("radio-p", "onClick");
  assert.deepEqual(mod.log.radios, ["x", "y", "y"], "a second group stays independent");
  assert.equal(h.byTestId("radio-p").props.get("checked"), true);
  mod.setCheckedA(false);
  h.flush();
  send("check-a", "onClick");
  send("trigger", "onClick");
  assert.equal(mod.log.checks.at(-1), true);
  assert.equal(
    h.byTestId("check-a").props.get("checked"),
    false,
    "controlled checked vetoed until accepted",
  );
  finish();
});

test("nested roots isolate dismissal, dialogs own the scroll lock and orphans stay shut", async t => {
  const nested = await mounted(t, "mountNested");
  nested.send("trigger", "onClick");
  nested.send("inner-trigger", "onClick");
  assert.ok(nested.h.byTestId("inner-surface"));
  assert.equal(nested.portals.length, 2);
  nested.send("inner-item", "onClick");
  assert.deepEqual(nested.mod.log.selects, ["inner"]);
  nested.flush();
  assert.equal(
    nested.h.byTestId("inner-surface"),
    undefined,
    "inner select closes the inner root only",
  );
  assert.ok(nested.h.byTestId("outer-surface"), "outer root stays open");
  nested.send("outer-item", "onClick");
  assert.deepEqual(nested.mod.log.selects, ["inner", "outer"]);
  nested.finish();
  delete globalThis.__zgui;

  const dialog = await mounted(t, "mountDialog", true);
  assert.ok(dialog.h.byTestId("dialog"));
  assert.ok(dialog.h.byTestId("d-surface"), "default-open menu inside a dialog mounts");
  assert.equal(dialog.locks.length, 1, "only the dialog owns the scroll lock");
  assert.equal(dialog.locks[0].node, dialog.h.byTestId("dialog"));
  assert.equal(dialog.portals.length, 2, "dialog and menu surfaces each portal");
  assert.equal(dialog.portals[1].layer, "modal", "menu inherits the dialog elevation");
  assert.equal(dialog.h.byTestId("orphan"), undefined, "orphan MenuContent never opens");
  assert.equal(dialog.h.byTestId("orphan-item"), undefined);
  assert.equal(dialog.layers.length, 2, "only live surfaces own dismiss layers");
  dialog.send("d-item", "onClick");
  dialog.flush();
  assert.equal(dialog.h.byTestId("d-surface"), undefined, "select closes the menu root");
  assert.ok(dialog.h.byTestId("dialog"), "dialog survives menu dismissal");
  dialog.finish();
  assert.equal(dialog.active(dialog.traps), 0);
  assert.equal(dialog.active(dialog.layers), 0);
  assert.equal(dialog.active(dialog.geometry), 0);
  assert.equal(dialog.clock.timers.size, 0, "no timers survive disposal");
});

test("metadata parts keep caller attrs and runtime rejects invalid props", async t => {
  const { h, mod, finish, textOf } = await mounted(t, "mountMeta");
  const label = h.byTestId("label"),
    group = h.byTestId("group"),
    sep = h.byTestId("sep"),
    item = h.byTestId("meta-item");
  assert.ok(label.props.get("class").includes("zui-menu__label"));
  assert.ok(label.props.get("class").includes("zui-menu__label--inset"));
  assert.ok(label.props.get("class").includes("caller-label"));
  assert.equal(group.props.get("role"), "group");
  assert.equal(group.props.get("ariaLabel"), "Section");
  assert.ok(group.props.get("class").includes("caller-group"));
  assert.equal(sep.props.get("ariaHidden"), true);
  assert.ok(sep.props.get("class").includes("zui-menu__separator"));
  assert.equal(sep.children.length, 0);
  assert.equal(item.props.get("ariaKeyShortcuts"), "⌘P");
  const shortcutOf = node => {
    let found;
    const walk = current => {
      for (const child of current.children) {
        if (child.props.get("class")?.includes("zui-menu__shortcut")) found = child;
        walk(child);
      }
    };
    walk(node);
    return found;
  };
  const first = shortcutOf(item);
  assert.ok(first, "shortcut text renders inside the row");
  assert.equal(first.props.get("ariaHidden"), true);
  const counter = h.byTestId("meta-counter");
  mod.bump("meta-counter");
  h.flush();
  mod.setShortcut("Ctrl+N");
  h.flush();
  const next = shortcutOf(h.byTestId("meta-item"));
  assert.equal(next, first, "shortcut text updates in place");
  assert.equal(first.children.map(child => child.props.get("text") ?? "").join(""), "Ctrl+N");
  assert.equal(h.byTestId("meta-counter"), counter, "shortcut swaps preserve body children");
  assert.equal(textOf(counter), "1");
  mod.setShortcut(null);
  h.flush();
  assert.equal(shortcutOf(h.byTestId("meta-item")), undefined);
  assert.equal(
    h.byTestId("meta-item").props.get("ariaKeyShortcuts"),
    undefined,
    "removed shortcut clears the semantic",
  );
  assert.equal(h.byTestId("meta-counter"), counter);
  assert.equal(item.props.get("data-destructive"), undefined);
  assert.equal(item.props.get("data-inset"), undefined);
  mod.setItemDestructive(true);
  h.flush();
  mod.setItemInset(true);
  h.flush();
  assert.equal(
    item.props.get("data-destructive"),
    "true",
    "destructive data attribute updates reactively",
  );
  assert.equal(item.props.get("data-inset"), "true", "inset data attribute updates reactively");
  assert.ok(String(item.props.get("class")).includes("zui-menu__item--destructive"));
  assert.ok(String(item.props.get("class")).includes("zui-menu__item--inset"));
  mod.setItemDestructive(false);
  h.flush();
  mod.setItemInset(false);
  h.flush();
  assert.equal(item.props.get("data-destructive"), undefined);
  assert.equal(item.props.get("data-inset"), undefined);
  finish();
  delete globalThis.__zgui;

  const bad = await mounted(t, "none");
  assert.throws(
    () => bad.mod.mountBadSeparator(bad.root),
    /MenuSeparator does not accept children/,
  );
  assert.throws(() => bad.mod.mountBadRadio(bad.root), /non-empty string/);
  assert.throws(() => bad.mod.mountBadCheck(bad.root), /must be a boolean/);
  bad.finish();
});

test("roving steps from the active member under descendants and honors propagation", async t => {
  const { h, clock, send, key, finish } = await mounted(t, "mountDescendants");
  send("trigger", "onClick");
  const surface = h.byTestId("surface");
  const rowA = h.byTestId("row-a"),
    rowB = h.byTestId("row-b"),
    rowC = h.byTestId("row-c");
  const child = h.byTestId("child"),
    loose = h.byTestId("loose"),
    blocked = h.byTestId("blocked");
  h.host.focus(loose);
  key(loose, { key: "ArrowDown" });
  assert.equal(h.focused.current, rowB, "with no member focus the first row seeds the step");
  h.host.focus(child);
  assert.equal(h.focused.current, child, "descendants of a row can hold real focus");
  key(child, { key: "ArrowDown" });
  assert.equal(h.focused.current, rowB, "descendant key events step from the containing row");
  key(child, { key: "Home" });
  assert.equal(h.focused.current, rowA);
  key(child, { key: "End" });
  assert.equal(h.focused.current, rowC);
  h.host.focus(rowA);
  h.host.focus(loose);
  key(loose, { key: "ArrowDown" });
  assert.equal(h.focused.current, rowB, "a nonmember target still steps from the last active row");
  h.host.focus(child);
  send(rowA, "onPointerLeave");
  assert.equal(
    h.focused.current,
    surface,
    "a row focused through a descendant still leaves to the surface",
  );
  h.host.focus(blocked);
  const stopped = key(blocked, { key: "ArrowDown" });
  assert.equal(stopped.propagationStopped, true);
  assert.equal(stopped.defaultPrevented, false);
  assert.equal(h.focused.current, blocked, "caller stopPropagation blocks ancestor navigation");
  h.host.focus(child);
  const character = key(child, {
    key: "ArrowDown",
    character: "ArrowDown",
    keyWithoutModifiers: "ArrowDown",
  });
  assert.equal(character.defaultPrevented, false);
  assert.equal(h.focused.current, child, "character-producing arrows never navigate");
  clock.advance(1000);
  finish();
});

test("submenus open by click and named keys, keep identity and right placement", async t => {
  const {
    h,
    mod,
    busy,
    activations,
    traps,
    locks,
    geometry,
    send,
    key,
    emit,
    flush,
    active,
    finish,
    textOf,
  } = await mounted(t, "mountSub");
  send("trigger", "onClick");
  const surface = h.byTestId("surface"),
    trigger = h.byTestId("trigger"),
    subTrigger = h.byTestId("sub-trigger");
  for (const name of ["ArrowRight", "Enter", " "])
    key(subTrigger, { key: name, character: name, keyWithoutModifiers: name });
  assert.equal(
    h.byTestId("sub-surface"),
    undefined,
    "character-producing keys never open a submenu",
  );
  const opened = key(subTrigger, { key: "ArrowRight" });
  const subSurface = h.byTestId("sub-surface");
  assert.ok(subSurface, "named ArrowRight opens the submenu");
  assert.equal(opened.defaultPrevented, true);
  assert.equal(opened.propagationStopped, true);
  assert.equal(subTrigger.props.get("role"), "menuItem");
  assert.equal(subTrigger.props.get("ariaHasPopup"), "menu");
  assert.equal(subTrigger.props.get("expanded"), true);
  assert.equal(subTrigger.props.get("data-inset"), "true");
  assert.equal(subTrigger.props.get("ariaControls"), subSurface);
  assert.equal(subSurface.props.get("ariaPopupFor"), subTrigger);
  assert.equal(
    trigger.props.get("ariaControls"),
    surface,
    "the root trigger keeps its own relation",
  );
  assert.ok(subSurface.props.get("class").includes("zui-menu--sub"));
  const chevron = (() => {
    let found;
    const walk = current => {
      for (const child of current.children) {
        if (child.props.get("class")?.includes("zui-menu__chevron")) found = child;
        walk(child);
      }
    };
    walk(subTrigger);
    return found;
  })();
  assert.ok(chevron, "the trigger renders a chevron affordance");
  send(subTrigger, "onPointerDown", { button: "primary" });
  send(subTrigger, "onPointerDown", { button: "secondary" });
  assert.deepEqual(
    activations.filter(node => node === subTrigger),
    [],
    "submenu triggers never activate on press",
  );
  send(subTrigger, "onClick");
  assert.equal(h.byTestId("sub-surface"), subSurface, "click on an open submenu stays mounted");
  const positioner = h.parents.get(subSurface);
  emit(
    geometry.find(record => record.node === subTrigger && record.kind === "borderBox"),
    { x: 200, y: 100, width: 160, height: 28 },
  );
  emit(
    geometry.find(record => record.node === positioner && record.kind === "viewport"),
    { x: 0, y: 0, width: 800, height: 600, scale: 1 },
  );
  emit(
    geometry.find(record => record.node === positioner && record.kind === "contentSize"),
    { width: 180, height: 100 },
  );
  const style = positioner.props.get("style");
  assert.equal(
    style.left,
    "356px",
    `right placement applies the -4 offset: ${JSON.stringify(style)}`,
  );
  assert.equal(style.top, "100px", `start alignment follows the anchor: ${JSON.stringify(style)}`);
  assert.equal(positioner.props.get("data-side"), "right");
  assert.equal(traps.length, 2, "each mounted surface owns a focus trap");
  assert.equal(locks.length, 0, "menus never lock scroll");
  mod.bump("sub-counter");
  h.flush();
  assert.equal(textOf(h.byTestId("sub-counter")), "1");
  const keep = h.byTestId("keep");
  h.host.focus(keep);
  busy.set(subSurface, true);
  key(keep, { key: "ArrowLeft" });
  assert.equal(subTrigger.props.get("expanded"), false, "ArrowLeft closes the current branch");
  assert.equal(h.focused.current, subTrigger, "ArrowLeft focuses the branch trigger");
  assert.equal(h.byTestId("sub-surface"), subSurface, "a busy exit retains the surface");
  send(subTrigger, "onClick");
  assert.equal(h.byTestId("sub-surface"), subSurface, "reopening keeps the busy surface");
  assert.equal(textOf(h.byTestId("sub-counter")), "1", "child state survives the busy reopen");
  busy.set(subSurface, false);
  h.host.focus(keep);
  key(keep, { key: "ArrowLeft" });
  flush();
  assert.equal(h.byTestId("sub-surface"), undefined, "a completed exit unmounts the surface");
  send(subTrigger, "onClick");
  assert.notEqual(h.byTestId("sub-surface"), subSurface, "a completed exit mounts a fresh surface");
  assert.equal(textOf(h.byTestId("sub-counter")), "0", "a fresh surface resets child state");
  assert.equal(h.byTestId("sub-trigger"), subTrigger, "the trigger identity survives remounts");
  key(h.byTestId("deep-trigger"), { key: "ArrowRight" });
  assert.ok(h.byTestId("deep-surface"), "nested submenus open independently");
  send("leaf", "onClick");
  assert.deepEqual(mod.log.selects, ["leaf"]);
  assert.equal(trigger.props.get("expanded"), false, "a leaf selection closes the outer menu root");
  flush();
  assert.equal(h.byTestId("surface"), undefined);
  assert.equal(h.byTestId("sub-surface"), undefined);
  assert.equal(h.byTestId("deep-surface"), undefined);
  assert.equal(h.byTestId("trigger"), trigger, "the outer trigger survives root dismissal");
  assert.equal(active(traps), 0);
  assert.equal(active(geometry), 0);
  finish();
});

test("submenu hover corridor opens, defers and cancels closes", async t => {
  const { h, mod, clock, geometry, send, emit, flush, finish } = await mounted(t, "mountSub");
  send("trigger", "onClick");
  const surface = h.byTestId("surface"),
    subTrigger = h.byTestId("sub-trigger");
  send(subTrigger, "onPointerEnter");
  clock.advance(119);
  assert.equal(h.byTestId("sub-surface"), undefined, "hover open waits the full 120 ms");
  send(subTrigger, "onPointerLeave");
  assert.equal(h.focused.current, surface, "a closed leave defocuses to the parent surface");
  clock.advance(1000);
  assert.equal(h.byTestId("sub-surface"), undefined, "leaving before the delay cancels the open");
  send(subTrigger, "onPointerEnter");
  clock.advance(120);
  const subSurface = h.byTestId("sub-surface");
  assert.ok(subSurface, "120 ms of hover opens the submenu");
  emit(
    geometry.find(record => record.node === subTrigger && record.kind === "viewport"),
    { x: 0, y: 0, width: 800, height: 600, scale: 1 },
  );
  h.rects.set(subSurface, { x: 200, y: 100, width: 160, height: 200 });
  const keep = h.byTestId("keep");
  h.host.focus(keep);
  send(subTrigger, "onPointerMove", { position: { x: 100, y: 110 } });
  send(subTrigger, "onPointerLeave", { position: { x: 140, y: 160 } });
  clock.advance(500);
  assert.equal(subTrigger.props.get("expanded"), true, "a corridor exit keeps the submenu open");
  assert.equal(h.focused.current, keep, "a corridor exit preserves panel focus");
  send(subTrigger, "onPointerEnter");
  send(subTrigger, "onPointerMove", { position: { x: 100, y: 110 } });
  send(subTrigger, "onPointerLeave", { position: { x: 100, y: 400 } });
  clock.advance(179);
  assert.equal(subTrigger.props.get("expanded"), true);
  clock.advance(1);
  assert.equal(subTrigger.props.get("expanded"), false, "leaving away closes after 180 ms");
  flush();
  assert.equal(h.byTestId("sub-surface"), undefined);
  assert.ok(h.byTestId("surface"), "closing the branch keeps the parent open");
  send(subTrigger, "onClick");
  const reopened = h.byTestId("sub-surface");
  h.rects.set(reopened, { x: 200, y: 100, width: 160, height: 200 });
  send(subTrigger, "onPointerMove", { position: { x: 100, y: 110 } });
  send(subTrigger, "onPointerLeave", { position: { x: 100, y: 400 } });
  clock.advance(100);
  send(reopened, "onPointerEnter");
  clock.advance(200);
  assert.equal(
    subTrigger.props.get("expanded"),
    true,
    "entering the panel cancels the armed close",
  );
  send(subTrigger, "onPointerMove", { position: { x: 100, y: 110 } });
  send(subTrigger, "onPointerLeave", { position: { x: 100, y: 400 } });
  send(subTrigger, "onPointerEnter");
  send(subTrigger, "onPointerLeave", { position: { x: 100, y: 400 } });
  mod.setSubTriggerShown(false);
  h.flush();
  assert.equal(h.byTestId("sub-trigger"), undefined);
  clock.advance(180);
  assert.equal(mod.log.subOpens.at(-1), false, "the root-owned close survives trigger removal");
  flush();
  assert.equal(h.byTestId("sub-surface"), undefined);
  assert.ok(h.byTestId("surface"), "the outer menu stays open");
  finish();
});

test("headingToward corridors and device-space tracking survive DPI changes", async t => {
  const { h, mod, clock, geometry, send, emit, flush, finish } = await mounted(t, "mountSub");
  const toward = mod.headingToward;
  const right = { x: 200, y: 100, width: 160, height: 200 };
  for (const to of [
    [140, 150],
    [170, 200],
    [190, 280],
  ])
    assert.equal(toward({ x: 100, y: 110 }, { x: to[0], y: to[1] }, right), true, `to ${to}`);
  for (const to of [
    [100, 400],
    [120, 500],
  ])
    assert.equal(toward({ x: 100, y: 110 }, { x: to[0], y: to[1] }, right), false, `to ${to}`);
  const left = { x: 0, y: 100, width: 160, height: 200 };
  assert.equal(toward({ x: 300, y: 110 }, { x: 220, y: 180 }, left), true);
  assert.equal(toward({ x: 300, y: 110 }, { x: 300, y: 400 }, left), false);
  const edge = { x: 10, y: 0, width: 10, height: 10 };
  for (const to of [
    [10, 5],
    [5, 0],
    [5, 4],
  ])
    assert.equal(toward({ x: 0, y: 0 }, { x: to[0], y: to[1] }, edge), true, `to ${to}`);
  assert.equal(toward({ x: 0, y: 0 }, { x: 5, y: 6 }, edge), false);
  send("trigger", "onClick");
  const subTrigger = h.byTestId("sub-trigger");
  send(subTrigger, "onClick");
  const subSurface = h.byTestId("sub-surface");
  const viewport = geometry.find(
    record => record.node === subTrigger && record.kind === "viewport",
  );
  emit(viewport, { x: 0, y: 0, width: 800, height: 600, scale: 1 });
  send(subTrigger, "onPointerMove", { position: { x: 100, y: 110 } });
  emit(viewport, { x: 0, y: 0, width: 800, height: 600, scale: 2 });
  h.rects.set(subSurface, { x: 100, y: 50, width: 80, height: 100 });
  send(subTrigger, "onPointerLeave", { position: { x: 70, y: 80 } });
  clock.advance(180);
  assert.equal(
    subTrigger.props.get("expanded"),
    true,
    "the stored device point survives a DPI change",
  );
  h.rects.set(subSurface, { x: 0, y: 50, width: 80, height: 100 });
  send(subTrigger, "onPointerMove", { position: { x: 150, y: 55 } });
  send(subTrigger, "onPointerLeave", { position: { x: 110, y: 90 } });
  clock.advance(180);
  assert.equal(subTrigger.props.get("expanded"), true, "the left corridor still defers the close");
  send(subTrigger, "onPointerLeave", { position: { x: 150, y: 200 } });
  clock.advance(180);
  assert.equal(subTrigger.props.get("expanded"), false, "leaving away closes the branch");
  flush();
  finish();
});

test("controlled submenu requests, vetoes, disabled state and stale callbacks", async t => {
  const { h, mod, clock, activations, send, key, flush, finish } = await mounted(t, "mountSub");
  mod.setSubOpen(false);
  h.flush();
  send("trigger", "onClick");
  const subTrigger = h.byTestId("sub-trigger");
  send(subTrigger, "onClick");
  key(subTrigger, { key: "ArrowRight" });
  assert.deepEqual(mod.log.subOpens, [true, true], "click and named keys only request");
  assert.equal(h.byTestId("sub-surface"), undefined, "a controlled refusal mounts nothing");
  mod.setVeto(true);
  h.flush();
  send(subTrigger, "onClick");
  send(subTrigger, "onPointerEnter");
  key(subTrigger, { key: "ArrowRight" });
  clock.advance(120);
  assert.deepEqual(mod.log.subOpens, [true, true], "caller vetoes add no request");
  mod.setVeto(false);
  h.flush();
  mod.setSubOff(true);
  h.flush();
  send(subTrigger, "onClick");
  send(subTrigger, "onPointerEnter");
  key(subTrigger, { key: "ArrowRight" });
  send(subTrigger, "onPointerDown", { button: "primary" });
  assert.deepEqual(mod.log.subOpens, [true, true], "a disabled trigger requests nothing");
  assert.deepEqual(
    activations.filter(node => node === subTrigger),
    [],
    "a disabled trigger never activates",
  );
  mod.setSubOff(false);
  h.flush();
  mod.setSubOpen(true);
  h.flush();
  const subSurface = h.byTestId("sub-surface");
  assert.ok(subSurface, "accepting the request mounts the surface");
  const keep = h.byTestId("keep");
  h.host.focus(keep);
  key(keep, { key: "ArrowLeft" });
  assert.deepEqual(mod.log.subOpens, [true, true, false], "ArrowLeft requests the branch close");
  assert.equal(h.byTestId("sub-surface"), subSurface, "a refused request keeps the surface");
  assert.equal(
    h.focused.current,
    keep,
    "a refused controlled close keeps focus inside the submenu",
  );
  mod.setVeto(true);
  h.flush();
  h.host.focus(keep);
  key(keep, { key: "ArrowLeft" });
  assert.equal(h.focused.current, keep, "a vetoed ArrowLeft keeps panel focus");
  assert.deepEqual(mod.log.subOpens, [true, true, false]);
  mod.setVeto(false);
  h.flush();
  key(keep, { key: "ArrowLeft", character: "ArrowLeft", keyWithoutModifiers: "ArrowLeft" });
  assert.equal(h.byTestId("sub-surface"), subSurface, "a character ArrowLeft never closes");
  assert.deepEqual(mod.log.subOpens, [true, true, false]);
  mod.setSubOpen(false);
  flush();
  assert.equal(h.byTestId("sub-surface"), undefined, "an accepted close retires the branch");
  const staleKey = subTrigger.callbacks.onKeyDown,
    staleMove = subTrigger.callbacks.onPointerMove;
  finish();
  staleKey({ key: "ArrowRight", character: null, preventDefault() {}, stopPropagation() {} });
  staleMove({ position: { x: 10, y: 10 }, preventDefault() {}, stopPropagation() {} });
  assert.deepEqual(mod.log.subOpens, [true, true, false], "stale handlers request nothing");
  assert.equal(clock.timers.size, 0, "no timers survive disposal");
});

test("submenu disposal cancels its own timers but never the root-owned close", async t => {
  const { h, mod, clock, geometry, traps, layers, send, active, finish } = await mounted(
    t,
    "mountSub",
  );
  send("trigger", "onClick");
  mod.setRemoveOnFocus(true);
  h.flush();
  const subTrigger = h.byTestId("sub-trigger");
  const viewportLease = () =>
    geometry.find(record => record.node === subTrigger && record.kind === "viewport");
  assert.ok(viewportLease()?.active, "a mounted trigger owns a viewport subscription");
  send(subTrigger, "onPointerEnter");
  assert.equal(
    h.byTestId("sub-trigger"),
    undefined,
    "a focus callback may remove the whole submenu",
  );
  clock.advance(120);
  assert.deepEqual(mod.log.subOpens, [], "no open is armed after disposal");
  assert.equal(h.byTestId("sub-surface"), undefined);
  assert.equal(viewportLease().active, false, "trigger removal retires the viewport lease");
  mod.setRemoveOnFocus(false);
  h.flush();
  mod.setSubShown(true);
  h.flush();
  const remounted = h.byTestId("sub-trigger");
  send(remounted, "onPointerEnter");
  mod.setSubShown(false);
  h.flush();
  clock.advance(120);
  assert.equal(h.byTestId("sub-surface"), undefined, "removal before the delay cancels the open");
  assert.deepEqual(mod.log.subOpens, []);
  finish();
  assert.equal(active(traps), 0);
  assert.equal(active(layers), 0);
  assert.equal(active(geometry), 0);
  assert.equal(clock.timers.size, 0, "disposal leaves no pending timers");
});

test("submenu Escape returns focus to trigger while outside-press does not", async t => {
  const { h, layers, send, flush, finish } = await mounted(t, "mountSub");
  send("trigger", "onClick");
  const surface = h.byTestId("surface"),
    subTrigger = h.byTestId("sub-trigger");
  send(subTrigger, "onClick");
  const keep = h.byTestId("keep");
  h.host.focus(keep);
  const subEscLayer = layers.findLast(record => record.active);
  subEscLayer.callback("escape");
  h.flush();
  assert.equal(subTrigger.props.get("expanded"), false, "Escape closes only the submenu branch");
  assert.equal(h.focused.current, subTrigger, "Escape returns focus to MenuSubTrigger");
  flush();
  assert.equal(h.byTestId("sub-surface"), undefined);
  assert.equal(h.byTestId("surface"), surface, "parent menu stays open after submenu Escape");

  send(subTrigger, "onClick");
  const keepAgain = h.byTestId("keep");
  h.host.focus(keepAgain);
  const subOutsideLayer = layers.findLast(record => record.active);
  subOutsideLayer.callback("outside-press");
  h.flush();
  assert.equal(subTrigger.props.get("expanded"), false);
  assert.notEqual(
    h.focused.current,
    subTrigger,
    "outside-press does not force focus onto MenuSubTrigger",
  );
  flush();
  finish();
});

test("leaf selection cascades outward through nested and controlled submenus before root dismissal", async t => {
  const { h, mod, busy, send, key, flush, finish } = await mounted(t, "mountSub");
  send("trigger", "onClick");
  const surface = h.byTestId("surface"),
    subTrigger = h.byTestId("sub-trigger");
  busy.set(surface, true);
  send(subTrigger, "onClick");
  const deepTrigger = h.byTestId("deep-trigger");
  key(deepTrigger, { key: "ArrowRight" });
  assert.ok(h.byTestId("deep-surface"));
  mod.log.subOpens.length = 0;
  mod.log.opens.length = 0;

  send("leaf", "onClick");
  assert.equal(
    deepTrigger.props.get("expanded"),
    false,
    "innermost submenu closes synchronously on leaf selection",
  );
  assert.equal(
    subTrigger.props.get("expanded"),
    false,
    "enclosing submenu closes synchronously even when root exit is busy",
  );
  assert.deepEqual(
    mod.log.subOpens,
    [false],
    "enclosing MenuSub notifies onOpenChange(false) during cascade",
  );
  assert.deepEqual(mod.log.opens, [false], "root menu dismisses after branch cascade");

  busy.set(surface, false);
  flush();
  assert.equal(h.byTestId("surface"), undefined);

  mod.setSubOpen(true);
  h.flush();
  send("trigger", "onClick");
  key(h.byTestId("deep-trigger"), { key: "ArrowRight" });
  mod.log.subOpens.length = 0;
  mod.log.opens.length = 0;
  send("leaf", "onClick");
  assert.equal(h.byTestId("deep-trigger").props.get("expanded"), false);
  assert.deepEqual(
    mod.log.subOpens,
    [false],
    "controlled MenuSub receives onOpenChange(false) on leaf selection",
  );
  assert.deepEqual(mod.log.opens, [false]);
  mod.setSubOpen(false);
  flush();
  finish();
});
