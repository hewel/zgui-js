import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, Show, createSignal, renderApp } from "@hewel/zgui";
import type { NativeChild, NativeNode } from "@hewel/zgui";
import {
  RadioGroup, RadioGroupItem, ToggleGroup, ToggleGroupItem,
} from "@hewel/zgui/components";

const log = {
  radio: [] as string[],
  toggle: [] as readonly string[][],
};

export const api = { log };

const [radioDefault, setRadioDefault] = createSignal("b");
const [disableB, setDisableB] = createSignal(false);
const [disableAll, setDisableAll] = createSignal(false);
const [showC, setShowC] = createSignal(true);
const [controlledRadio, setControlledRadio] = createSignal<string | undefined>("a");
const [echoRadio, setEchoRadio] = createSignal(false);
const [cancelRadioKeys, setCancelRadioKeys] = createSignal(false);
const [cancelItemClick, setCancelItemClick] = createSignal(false);
const [toggleOrientation, setToggleOrientation] = createSignal<"horizontal" | "vertical" | "both">("horizontal");
const [singleValues, setSingleValues] = createSignal<readonly string[]>([]);
const [controlledValues, setControlledValues] = createSignal<readonly string[]>([]);
const [echoToggle, setEchoToggle] = createSignal(false);
const [groupDisabled, setGroupDisabled] = createSignal(false);
const [groupVariant, setGroupVariant] = createSignal<"default" | "outline">("default");
const [groupSize, setGroupSize] = createSignal<"sm" | "md" | "lg">("md");
const [groupSpacing, setGroupSpacing] = createSignal(2);
const [itemOrder, setItemOrder] = createSignal<readonly string[]>(["ta", "tb", "tc"]);
const [showGroup, setShowGroup] = createSignal(true);

export const controls = {
  setRadioDefault, setDisableB, setDisableAll, setShowC,
  setControlledRadio, setEchoRadio, setCancelRadioKeys, setCancelItemClick,
  setToggleOrientation, setSingleValues, setControlledValues, setEchoToggle,
  setGroupDisabled, setGroupVariant, setGroupSize, setGroupSpacing,
  setItemOrder, setShowGroup,
};

let radioRoot: NativeNode | undefined;
export const nodes = { radioRoot: () => radioRoot };

function Rows(props: { children?: NativeChild }) {
  return <row>{props.children}</row>;
}

export function mountRadio(root: NativeNode): () => void {
  return renderApp(() => (
    <RadioGroup testId="radio" ref={node => { radioRoot = node; }}
      label="Fruit" defaultValue={radioDefault()} disabled={disableAll()}
      onValueChange={value => log.radio.push(value)}
      onKeyDown={event => { if (cancelRadioKeys()) event.preventDefault(); }}>
      <Rows>
        <RadioGroupItem testId="ra" value="a" disabled={disableB() ? false : undefined} />
        <RadioGroupItem testId="rb" value="b" disabled={disableB()}
          onClick={event => { if (cancelItemClick()) event.preventDefault(); }} />
      </Rows>
      <Rows>
        <Show when={showC()}>
          <RadioGroupItem testId="rc" value="c" />
        </Show>
      </Rows>
    </RadioGroup>
  ), root);
}

export function mountControlledRadio(root: NativeNode): () => void {
  return renderApp(() => (
    <RadioGroup testId="radio" value={controlledRadio()}
      onValueChange={value => { log.radio.push(value); if (echoRadio()) setControlledRadio(value); }}>
      <RadioGroupItem testId="ra" value="a" />
      <RadioGroupItem testId="rb" value="b" />
    </RadioGroup>
  ), root);
}

export function mountRadioOrientation(root: NativeNode, axis: "horizontal" | "vertical" | "both"): () => void {
  return renderApp(() => (
    <RadioGroup testId="radio" orientation={axis}>
      <RadioGroupItem testId="ra" value="a" />
      <RadioGroupItem testId="rb" value="b" />
    </RadioGroup>
  ), root);
}

export function mountToggle(root: NativeNode, selection: "single" | "multiple"): () => void {
  return renderApp(() => (
    <ToggleGroup testId="toggle" selection={selection} orientation={toggleOrientation()}
      variant={groupVariant()} size={groupSize()} spacing={groupSpacing()}
      disabled={groupDisabled()}
      onValueChange={value => log.toggle.push(value)}>
      <ToggleGroupItem testId="ta" value="a" label="A" />
      <ToggleGroupItem testId="tb" value="b" size="lg" disabled={false} />
      <ToggleGroupItem testId="tc" value="c" />
    </ToggleGroup>
  ), root);
}

export function mountControlledToggle(root: NativeNode): () => void {
  return renderApp(() => (
    <ToggleGroup testId="toggle" value={controlledValues()}
      onValueChange={value => { log.toggle.push(value); if (echoToggle()) setControlledValues(value); }}>
      <ToggleGroupItem testId="ta" value="a" />
      <ToggleGroupItem testId="tb" value="b" />
    </ToggleGroup>
  ), root);
}

export function mountReorder(root: NativeNode): () => void {
  return renderApp(() => (
    <ToggleGroup testId="toggle" selection="multiple">
      <For each={[...itemOrder()]}>
        {name => <ToggleGroupItem testId={name} value={name.slice(1)} />}
      </For>
    </ToggleGroup>
  ), root);
}

export function mountShown(root: NativeNode): () => void {
  return renderApp(() => (
    <Show when={showGroup()}>
      <RadioGroup testId="radio">
        <RadioGroupItem testId="ra" value="a" />
      </RadioGroup>
    </Show>
  ), root);
}

export function mountNestedPair(root: NativeNode): () => void {
  return renderApp(() => (
    <RadioGroup testId="outer">
      <RadioGroupItem testId="o1" value="o1" />
      <RadioGroup testId="inner"
        onValueChange={value => log.radio.push(value)}>
        <RadioGroupItem testId="i1" value="i1" />
        <RadioGroupItem testId="i2" value="i2" />
      </RadioGroup>
    </RadioGroup>
  ), root);
}

export function mountStandalone(root: NativeNode): () => void {
  return renderApp(() => (
    <box>
      <RadioGroupItem testId="sr" value="s" />
      <ToggleGroupItem testId="st" value="s" />
    </box>
  ), root);
}

export function mountInvalidItem(root: NativeNode, value: string): () => void {
  return renderApp(() => (
    <RadioGroup testId="radio">
      <RadioGroupItem value={value} />
    </RadioGroup>
  ), root);
}

export function mountInvalidSpacing(root: NativeNode, spacing: number): () => void {
  return renderApp(() => <ToggleGroup spacing={spacing} />, root);
}

export function mountRadioChildren(root: NativeNode, child: () => NativeChild): () => void {
  return renderApp(() => (
    <RadioGroup>
      <RadioGroupItem value="a" {...({ get children() { return child() as never; } })} />
    </RadioGroup>
  ), root);
}
`;

const counters = () => ({ prevented: 0, stopped: 0 });

const click = target => ({
  type: "click", target, prevented: 0, stopped: 0,
  preventDefault() { this.prevented = (this.prevented ?? 0) + 1; },
  stopPropagation() { this.stopped = (this.stopped ?? 0) + 1; },
  stopImmediatePropagation() {},
});

const key = (target, name) => ({
  type: "keydown", key: name, keyWithoutModifiers: name, target, prevented: 0, stopped: 0,
  modifiers: { shift: false, control: false, alt: false, meta: false },
  preventDefault() { this.prevented = (this.prevented ?? 0) + 1; },
  stopPropagation() { this.stopped = (this.stopped ?? 0) + 1; },
  stopImmediatePropagation() {},
});

const focusEvent = target => ({
  type: "focusin", target, cause: "program",
  preventDefault() {}, stopPropagation() {}, stopImmediatePropagation() {},
});

function cleanup(t, harness, dispose) {
  t.after(() => {
    const previous = globalThis.__zgui;
    globalThis.__zgui = harness.host;
    try { dispose(); }
    finally {
      if (previous && previous !== harness.host) globalThis.__zgui = previous;
      else delete globalThis.__zgui;
    }
  });
}

async function mounted(t, mount, ...args) {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod[mount](root, ...args);
  cleanup(t, harness, dispose);
  return { mod, harness };
}

const prop = (harness, testId, name) => harness.byTestId(testId)?.props.get(name);
const tabIndex = (harness, testId) => prop(harness, testId, "tabIndex");
const checked = (harness, testId) => prop(harness, testId, "checked");

test("radio group selects on click and focus with roving tab stops and live order", async t => {
  const { mod, harness } = await mounted(t, "mountRadio");
  const group = harness.byTestId("radio");
  const items = ["ra", "rb", "rc"];
  assert.equal(prop(harness, "radio", "role"), "radioGroup");
  assert.equal(prop(harness, "radio", "ariaLabel"), "Fruit");
  assert.ok(mod.nodes.radioRoot() === group);
  assert.match(prop(harness, "radio", "class"), /\bzui-radio-group\b/);
  for (const id of items) assert.equal(prop(harness, id, "ariaRadioGroup"), group);
  assert.equal(tabIndex(harness, "ra"), -1);
  assert.equal(tabIndex(harness, "rb"), 0, "the selected radio is the initial tab stop");
  assert.equal(tabIndex(harness, "rc"), -1);
  assert.equal(checked(harness, "rb"), true);
  assert.equal(checked(harness, "ra"), false);

  const b = harness.byTestId("rb");
  const c = harness.byTestId("rc");
  const down = key(b, "ArrowDown");
  group.callbacks.onKeyDown(down);
  assert.equal(harness.focused.current, c);
  assert.equal(harness.focused.reveal, true);
  assert.equal(down.prevented, 1);
  assert.equal(down.stopped, 1);
  assert.equal(checked(harness, "rc"), false, "arrow navigation only focuses");

  c.callbacks.onFocus(focusEvent(c));
  assert.deepEqual(mod.api.log.radio, ["c"]);
  assert.equal(checked(harness, "rc"), true);
  assert.equal(tabIndex(harness, "rc"), 0);

  c.callbacks.onClick(click(c));
  c.callbacks.onClick(click(c));
  assert.deepEqual(mod.api.log.radio, ["c"], "repeating the selected item emits nothing");

  const wrap = key(harness.byTestId("ra"), "ArrowUp");
  group.callbacks.onKeyDown(wrap);
  assert.equal(harness.focused.current, c, "arrow up from the first item wraps to the last");

  harness.byTestId("ra").callbacks.onFocus(focusEvent(harness.byTestId("ra")));
  const end = key(harness.byTestId("ra"), "End");
  group.callbacks.onKeyDown(end);
  assert.equal(harness.focused.current, c);
  const home = key(harness.byTestId("rc"), "Home");
  group.callbacks.onKeyDown(home);
  assert.equal(harness.focused.current, harness.byTestId("ra"));

  mod.controls.setCancelRadioKeys(true);
  const blocked = key(harness.byTestId("ra"), "ArrowDown");
  group.callbacks.onKeyDown(blocked);
  assert.notEqual(harness.focused.current, harness.byTestId("rb"), "a caller-prevented key does not navigate");
  mod.controls.setCancelRadioKeys(false);

  mod.controls.setCancelItemClick(true);
  b.callbacks.onClick(click(b));
  assert.notEqual(checked(harness, "rb"), true, "a caller-prevented click does not select");
  assert.deepEqual(mod.api.log.radio, ["c", "a"], "prevented clicks emit nothing");
});

test("radio orientation gates arrow axes", async t => {
  const { harness } = await mounted(t, "mountRadioOrientation", "horizontal");
  const group = harness.byTestId("radio");
  const a = harness.byTestId("ra");
  harness.focused.current = null;
  group.callbacks.onKeyDown(key(a, "ArrowDown"));
  assert.equal(harness.focused.current, null, "vertical arrows are ignored horizontally");
  group.callbacks.onKeyDown(key(a, "ArrowRight"));
  assert.equal(harness.focused.current, harness.byTestId("rb"));

  const second = createHost();
  globalThis.__zgui = second.host;
  const mod2 = await moduleFor(t, fixture);
  const root2 = second.makeNode("root");
  const dispose2 = mod2.mountRadioOrientation(root2, "both");
  cleanup(t, second, dispose2);
  const group2 = second.byTestId("radio");
  group2.callbacks.onKeyDown(key(second.byTestId("ra"), "ArrowDown"));
  assert.equal(second.focused.current, second.byTestId("rb"), "both accepts every arrow");
});

test("disabled and removed radio items recover the tab stop and skip navigation", async t => {
  const { mod, harness } = await mounted(t, "mountRadio");
  const group = harness.byTestId("radio");
  const a = harness.byTestId("ra");
  const b = harness.byTestId("rb");
  const c = harness.byTestId("rc");
  assert.equal(checked(harness, "rb"), true);

  mod.controls.setDisableB(true);
  assert.equal(checked(harness, "rb"), true, "disabling does not clear its own checked flag");
  assert.equal(tabIndex(harness, "rb"), -1);
  assert.equal(tabIndex(harness, "ra"), 0, "the tab stop falls back to the first enabled item");
  const skip = key(a, "ArrowDown");
  group.callbacks.onKeyDown(skip);
  assert.equal(harness.focused.current, c, "navigation skips disabled items");
  b.callbacks.onClick(click(b));
  assert.equal(checked(harness, "rb"), true);
  assert.deepEqual(mod.api.log.radio, [], "a disabled item cannot be chosen");

  a.callbacks.onClick(click(a));
  assert.deepEqual(mod.api.log.radio, ["a"]);
  assert.equal(checked(harness, "rb"), false, "the disabled item only stays checked until a new choice");
  mod.controls.setRadioDefault("c");
  assert.equal(checked(harness, "ra"), true, "defaultValue changes never reset owned state");

  mod.controls.setDisableAll(true);
  assert.equal(tabIndex(harness, "ra"), -1);
  assert.equal(tabIndex(harness, "rc"), -1);
  const stuck = key(a, "ArrowDown");
  group.callbacks.onKeyDown(stuck);
  assert.equal(stuck.prevented, 0, "a disabled group does not handle keys");
  mod.controls.setDisableAll(false);
  mod.controls.setDisableB(false);

  c.callbacks.onClick(click(c));
  mod.controls.setShowC(false);
  assert.equal(tabIndex(harness, "ra"), 0, "removing the active item recovers a surviving tab stop");
  const fresh = harness.byTestId("rc");
  assert.equal(fresh, undefined);
  mod.controls.setShowC(true);
  const remounted = harness.byTestId("rc");
  assert.ok(remounted);
  assert.notEqual(remounted, c, "hide and show produce fresh item nodes");
});

test("controlled radio only changes when the caller echoes the value", async t => {
  const { mod, harness } = await mounted(t, "mountControlledRadio");
  const b = harness.byTestId("rb");
  b.callbacks.onClick(click(b));
  assert.deepEqual(mod.api.log.radio, ["b"]);
  assert.equal(checked(harness, "rb"), false, "controlled value stays until echoed");
  assert.equal(checked(harness, "ra"), true);
  mod.controls.setEchoRadio(true);
  b.callbacks.onClick(click(b));
  assert.deepEqual(mod.api.log.radio, ["b", "b"]);
  assert.equal(checked(harness, "rb"), true);
});

test("keyed reorder before first focus fixes the initial tab stop", async t => {
  const { mod, harness } = await mounted(t, "mountReorder");
  const group = harness.byTestId("toggle");
  const ta = harness.byTestId("ta");
  assert.equal(prop(harness, "ta", "tabIndex"), 0);
  assert.equal(prop(harness, "tb", "tabIndex"), -1);

  mod.controls.setItemOrder(["tc", "ta", "tb"]);
  assert.equal(prop(harness, "tc", "tabIndex"), 0, "first tab stop follows live tree order");
  assert.equal(prop(harness, "ta", "tabIndex"), -1);
  assert.equal(harness.byTestId("ta"), ta, "keyed moves keep node identities");

  const tc = harness.byTestId("tc");
  group.callbacks.onKeyDown(key(tc, "ArrowRight"));
  assert.equal(harness.focused.current, ta, "arrows after a reorder use the new order");

  tc.callbacks.onFocus(focusEvent(tc));
  mod.controls.setItemOrder(["tb", "tc", "ta"]);
  group.callbacks.onKeyDown(key(tc, "ArrowRight"));
  assert.equal(harness.focused.current, ta);

  mod.controls.setItemOrder(["tb", "tc"]);
  assert.equal(harness.byTestId("ta"), undefined, "removed items unregister");
  assert.equal(prop(harness, "tb", "tabIndex"), 0, "the surviving item takes the tab stop");
  group.callbacks.onKeyDown(key(harness.byTestId("tb"), "ArrowRight"));
  assert.equal(harness.focused.current, harness.byTestId("tc"));
  group.callbacks.onKeyDown(key(harness.byTestId("tc"), "ArrowRight"));
  assert.equal(harness.focused.current, harness.byTestId("tb"), "wrapping skips the removed item");
});

test("toggle group single and multiple selection semantics", async t => {
  const { mod, harness } = await mounted(t, "mountToggle", "single");
  const clickItem = id => harness.byTestId(id).callbacks.onClick(click(harness.byTestId(id)));
  assert.equal(prop(harness, "toggle", "role"), "toolbar");
  assert.equal(prop(harness, "ta", "role"), "radioButton");

  clickItem("ta");
  assert.deepEqual(mod.api.log.toggle, [["a"]]);
  clickItem("tb");
  assert.deepEqual(mod.api.log.toggle, [["a"], ["b"]], "single replaces the selection");
  clickItem("tb");
  assert.deepEqual(mod.api.log.toggle, [["a"], ["b"], []]);

  const second = createHost();
  globalThis.__zgui = second.host;
  const mod2 = await moduleFor(t, fixture);
  const root2 = second.makeNode("root");
  const dispose2 = mod2.mountToggle(root2, "multiple");
  cleanup(t, second, dispose2);
  const click2 = id => second.byTestId(id).callbacks.onClick(click(second.byTestId(id)));
  assert.equal(prop(second, "ta", "role"), "button");
  click2("tb");
  click2("ta");
  assert.deepEqual(mod2.api.log.toggle, [["b"], ["a", "b"]], "multiple keeps a sorted unique set");
  click2("tb");
  assert.deepEqual(mod2.api.log.toggle, [["b"], ["a", "b"], ["a"]]);
  second.byTestId("ta").callbacks.onFocus(focusEvent(second.byTestId("ta")));
  const before = mod2.api.log.toggle.length;
  second.byTestId("toggle").callbacks.onKeyDown(key(second.byTestId("ta"), "ArrowRight"));
  assert.equal(mod2.api.log.toggle.length, before, "arrow focus never toggles");
  assert.equal(second.focused.current, second.byTestId("tb"));
});

test("controlled toggle reports attempts while visual state stays until echoed", async t => {
  const { mod, harness } = await mounted(t, "mountControlledToggle");
  const a = harness.byTestId("ta");
  a.callbacks.onClick(click(a));
  assert.deepEqual(mod.api.log.toggle, [["a"]]);
  assert.equal(prop(harness, "ta", "data-state"), "off");
  mod.controls.setEchoToggle(true);
  a.callbacks.onClick(click(a));
  assert.equal(prop(harness, "ta", "data-state"), "on");
});

test("group disabled and inherited variant, size, spacing stay reactive", async t => {
  const { mod, harness } = await mounted(t, "mountToggle", "single");
  const b = harness.byTestId("tb");
  assert.equal(prop(harness, "toggle", "data-spacing"), "2");
  assert.equal(prop(harness, "toggle", "--zui-toggle-group-gap"), "2");
  assert.equal(prop(harness, "toggle", "data-variant"), "default");
  assert.equal(prop(harness, "tb", "data-size"), "lg", "item size overrides the group");
  assert.equal(prop(harness, "ta", "data-size"), "md");

  mod.controls.setGroupVariant("outline");
  mod.controls.setGroupSize("sm");
  assert.equal(prop(harness, "ta", "data-variant"), "outline");
  assert.match(prop(harness, "ta", "class"), /zui-toggle--outline/);
  assert.equal(prop(harness, "ta", "data-size"), "sm", "group size inherits when the item does not override");

  mod.controls.setGroupDisabled(true);
  assert.equal(prop(harness, "tb", "disabled"), true, "item disabled=false cannot override the group");
  const length = mod.api.log.toggle.length;
  b.callbacks.onClick(click(b));
  assert.equal(mod.api.log.toggle.length, length);
  const keyEvent = key(harness.byTestId("ta"), "ArrowRight");
  harness.byTestId("toggle").callbacks.onKeyDown(keyEvent);
  assert.equal(keyEvent.prevented, 0, "a disabled group does not navigate");
});

test("nested groups only own their items and standalone items are inert", async t => {
  const { mod, harness } = await mounted(t, "mountNestedPair");
  const outer = harness.byTestId("outer");
  const inner = harness.byTestId("inner");
  const i1 = harness.byTestId("i1");
  assert.equal(tabIndex(harness, "o1"), 0, "each group keeps its own tab stop");
  assert.equal(tabIndex(harness, "i1"), 0);
  inner.callbacks.onKeyDown(key(i1, "ArrowDown"));
  assert.equal(harness.focused.current, harness.byTestId("i2"));
  const foreign = key(i1, "ArrowUp");
  outer.callbacks.onKeyDown(foreign);
  assert.equal(foreign.prevented, 0, "a group ignores keys targeting another group");
  i1.callbacks.onClick(click(i1));
  assert.deepEqual(mod.api.log.radio, ["i1"]);
  assert.equal(checked(harness, "o1"), false);

  const second = createHost();
  globalThis.__zgui = second.host;
  const mod2 = await moduleFor(t, fixture);
  const root2 = second.makeNode("root");
  const dispose2 = mod2.mountStandalone(root2);
  cleanup(t, second, dispose2);
  assert.equal(prop(second, "sr", "tabIndex"), 0, "standalone items stay focusable");
  second.byTestId("sr").callbacks.onClick(click(second.byTestId("sr")));
  assert.equal(checked(second, "sr"), false, "a standalone item is unchecked and inert");
  assert.equal(prop(second, "sr", "ariaRadioGroup") ?? null, null);
});

test("invalid group inputs fail clearly and radio children are rejected lazily", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  t.after(() => delete globalThis.__zgui);
  assert.throws(() => mod.mountInvalidItem(root, ""), /non-empty string/);
  assert.throws(() => mod.mountInvalidSpacing(root, Number.NaN), /finite non-negative/);
  assert.throws(() => mod.mountInvalidSpacing(root, -1), /finite non-negative/);
  let ran = false;
  assert.throws(() => mod.mountRadioChildren(root, () => { ran = true; return "x"; }), /does not accept children/);
  assert.equal(ran, false, "children are rejected before the getter runs");
});

test("hide and show remount produces fresh item nodes", async t => {
  const { mod, harness } = await mounted(t, "mountShown");
  const first = harness.byTestId("ra");
  assert.ok(first);
  mod.controls.setShowGroup(false);
  assert.equal(harness.byTestId("ra"), undefined);
  mod.controls.setShowGroup(true);
  const second = harness.byTestId("ra");
  assert.ok(second);
  assert.notEqual(second, first, "remounting creates fresh item nodes");
});
