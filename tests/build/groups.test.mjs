import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, Show, createSignal, renderApp } from "@hewel/zgui";
import type { NativeChild, NativeNode } from "@hewel/zgui";
import {
  RadioGroup, RadioGroupItem, ToggleGroup, ToggleGroupItem,
} from "@hewel/zgui/components";
import { RadioGroupPrimitive, RadioItemPrimitive } from "@hewel/zgui/primitives";

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

const [primDisabledAll, setPrimDisabledAll] = createSignal(false);
const [primDisableB, setPrimDisableB] = createSignal(false);
const [primControlled, setPrimControlled] = createSignal<string | undefined>("a");
const [primEcho, setPrimEcho] = createSignal(false);
const [primOrientation, setPrimOrientation] = createSignal<"horizontal" | "vertical" | "both">("horizontal");
export const primControls = {
  setPrimDisabledAll,
  setPrimDisableB,
  setPrimControlled,
  setPrimEcho,
  setPrimOrientation,
};

export function mountPrimitiveRadio(root: NativeNode): () => void {
  return renderApp(() => (
    <RadioGroupPrimitive
      testId="prg"
      ariaLabel="Segments"
      orientation={primOrientation()}
      defaultValue="b"
      disabled={primDisabledAll()}
      onValueChange={value => log.radio.push(value)}
    >
      <RadioItemPrimitive testId="pa" value="a" ariaLabel="Alpha">
        <row testId="pa-row">
          <box testId="pa-dot" />
          <text testId="pa-text">Alpha</text>
        </row>
      </RadioItemPrimitive>
      <RadioItemPrimitive testId="pb" value="b" disabled={primDisableB()}>
        <text testId="pb-text">Beta</text>
      </RadioItemPrimitive>
      <RadioItemPrimitive testId="pc" value="c">
        <text testId="pc-text">Gamma</text>
      </RadioItemPrimitive>
    </RadioGroupPrimitive>
  ), root);
}

export function mountPrimitiveControlledRadio(root: NativeNode): () => void {
  return renderApp(() => (
    <RadioGroupPrimitive
      testId="prg"
      value={primControlled()}
      onValueChange={value => {
        log.radio.push(value);
        if (primEcho()) setPrimControlled(value);
      }}
    >
      <RadioItemPrimitive testId="pa" value="a">
        <text>Alpha</text>
      </RadioItemPrimitive>
      <RadioItemPrimitive testId="pb" value="b">
        <text>Beta</text>
      </RadioItemPrimitive>
      <RadioItemPrimitive testId="pc" value="c">
        <text>Gamma</text>
      </RadioItemPrimitive>
    </RadioGroupPrimitive>
  ), root);
}

export function mountPrimitiveInvalid(root: NativeNode, value: string): () => void {
  return renderApp(() => (
    <RadioGroupPrimitive>
      <RadioItemPrimitive value={value} />
    </RadioGroupPrimitive>
  ), root);
}
`;

function cleanup(t, harness, dispose) {
  t.after(() => {
    const previous = globalThis.__zgui;
    globalThis.__zgui = harness.host;
    try {
      dispose();
    } finally {
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
const sendKey = (harness, target, keyName) =>
  harness.dispatch(target, "KeyDown", { key: keyName, keyWithoutModifiers: keyName });

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
  const down = sendKey(harness, b, "ArrowDown");
  assert.equal(harness.focused.current, c);
  assert.equal(harness.focused.reveal, true);
  assert.equal(down.defaultPrevented, true);
  assert.equal(down.propagationStopped, true);
  assert.deepEqual(mod.api.log.radio, ["c"], "arrow navigation focuses and activates via FocusIn");
  assert.equal(checked(harness, "rc"), true);
  assert.equal(tabIndex(harness, "rc"), 0);

  harness.dispatch(c, "Click");
  harness.dispatch(c, "Click");
  assert.deepEqual(mod.api.log.radio, ["c"], "repeating the selected item emits nothing");

  sendKey(harness, harness.byTestId("ra"), "ArrowUp");
  assert.equal(harness.focused.current, c, "arrow up from the first item wraps to the last");

  harness.dispatch(harness.byTestId("ra"), "FocusIn");
  sendKey(harness, harness.byTestId("ra"), "End");
  assert.equal(harness.focused.current, c);
  sendKey(harness, harness.byTestId("rc"), "Home");
  assert.equal(harness.focused.current, harness.byTestId("ra"));

  mod.controls.setCancelRadioKeys(true);
  harness.flush();
  sendKey(harness, harness.byTestId("ra"), "ArrowDown");
  assert.notEqual(
    harness.focused.current,
    harness.byTestId("rb"),
    "a caller-prevented key does not navigate",
  );
  mod.controls.setCancelRadioKeys(false);

  const countBeforeCancel = mod.api.log.radio.length;
  mod.controls.setCancelItemClick(true);
  harness.flush();
  harness.dispatch(b, "Click");
  assert.notEqual(checked(harness, "rb"), true, "a caller-prevented click does not select");
  assert.equal(mod.api.log.radio.length, countBeforeCancel, "prevented clicks emit nothing");
});

test("radio orientation gates arrow axes", async t => {
  const { harness } = await mounted(t, "mountRadioOrientation", "horizontal");
  const a = harness.byTestId("ra");
  harness.focused.current = null;
  sendKey(harness, a, "ArrowDown");
  assert.equal(harness.focused.current, null, "vertical arrows are ignored horizontally");
  sendKey(harness, a, "ArrowRight");
  assert.equal(harness.focused.current, harness.byTestId("rb"));

  const second = createHost();
  globalThis.__zgui = second.host;
  const mod2 = await moduleFor(t, fixture);
  const root2 = second.makeNode("root");
  const dispose2 = mod2.mountRadioOrientation(root2, "both");
  cleanup(t, second, dispose2);
  sendKey(second, second.byTestId("ra"), "ArrowDown");
  assert.equal(second.focused.current, second.byTestId("rb"), "both accepts every arrow");
});

test("disabled and removed radio items recover the tab stop and skip navigation", async t => {
  const { mod, harness } = await mounted(t, "mountRadio");
  const a = harness.byTestId("ra");
  const b = harness.byTestId("rb");
  const c = harness.byTestId("rc");
  assert.equal(checked(harness, "rb"), true);

  mod.controls.setDisableB(true);
  harness.flush();
  assert.equal(checked(harness, "rb"), true, "disabling does not clear its own checked flag");
  assert.equal(tabIndex(harness, "rb"), -1);
  assert.equal(tabIndex(harness, "ra"), 0, "the tab stop falls back to the first enabled item");
  harness.dispatch(b, "Click");
  assert.equal(checked(harness, "rb"), true);
  assert.deepEqual(mod.api.log.radio, [], "a disabled item cannot be chosen");

  harness.dispatch(a, "Click");
  assert.deepEqual(mod.api.log.radio, ["a"]);
  assert.equal(
    checked(harness, "rb"),
    false,
    "the disabled item only stays checked until a new choice",
  );
  mod.controls.setRadioDefault("c");
  harness.flush();
  assert.equal(checked(harness, "ra"), true, "defaultValue changes never reset owned state");

  sendKey(harness, a, "ArrowDown");
  assert.equal(harness.focused.current, c, "navigation skips disabled items");

  mod.controls.setDisableAll(true);
  harness.flush();
  assert.equal(tabIndex(harness, "ra"), -1);
  assert.equal(tabIndex(harness, "rc"), -1);
  const stuck = sendKey(harness, a, "ArrowDown");
  assert.equal(stuck.defaultPrevented, false, "a disabled group does not handle keys");
  mod.controls.setDisableAll(false);
  mod.controls.setDisableB(false);
  harness.flush();

  harness.dispatch(c, "Click");
  mod.controls.setShowC(false);
  harness.flush();
  assert.equal(
    tabIndex(harness, "ra"),
    0,
    "removing the active item recovers a surviving tab stop",
  );
  const fresh = harness.byTestId("rc");
  assert.equal(fresh, undefined);
  mod.controls.setShowC(true);
  harness.flush();
  const remounted = harness.byTestId("rc");
  assert.ok(remounted);
  assert.notEqual(remounted, c, "hide and show produce fresh item nodes");
});

test("controlled radio only changes when the caller echoes the value", async t => {
  const { mod, harness } = await mounted(t, "mountControlledRadio");
  const b = harness.byTestId("rb");
  harness.dispatch(b, "Click");
  assert.deepEqual(mod.api.log.radio, ["b"]);
  assert.equal(checked(harness, "rb"), false, "controlled value stays until echoed");
  assert.equal(checked(harness, "ra"), true);
  mod.controls.setEchoRadio(true);
  harness.flush();
  harness.dispatch(b, "Click");
  assert.deepEqual(mod.api.log.radio, ["b", "b"]);
  assert.equal(checked(harness, "rb"), true);
});

test("keyed reorder before first focus fixes the initial tab stop", async t => {
  const { mod, harness } = await mounted(t, "mountReorder");
  const ta = harness.byTestId("ta");
  assert.equal(prop(harness, "ta", "tabIndex"), 0);
  assert.equal(prop(harness, "tb", "tabIndex"), -1);

  mod.controls.setItemOrder(["tc", "ta", "tb"]);
  harness.flush();
  assert.equal(prop(harness, "tc", "tabIndex"), 0, "first tab stop follows live tree order");
  assert.equal(prop(harness, "ta", "tabIndex"), -1);
  assert.equal(harness.byTestId("ta"), ta, "keyed moves keep node identities");

  const tc = harness.byTestId("tc");
  sendKey(harness, tc, "ArrowRight");
  assert.equal(harness.focused.current, ta, "arrows after a reorder use the new order");

  harness.dispatch(tc, "FocusIn");
  mod.controls.setItemOrder(["tb", "tc", "ta"]);
  harness.flush();
  sendKey(harness, tc, "ArrowRight");
  assert.equal(harness.focused.current, ta);

  mod.controls.setItemOrder(["tb", "tc"]);
  harness.flush();
  assert.equal(harness.byTestId("ta"), undefined, "removed items unregister");
  assert.equal(prop(harness, "tb", "tabIndex"), 0, "the surviving item takes the tab stop");
  sendKey(harness, harness.byTestId("tb"), "ArrowRight");
  assert.equal(harness.focused.current, harness.byTestId("tc"));
  sendKey(harness, harness.byTestId("tc"), "ArrowRight");
  assert.equal(harness.focused.current, harness.byTestId("tb"), "wrapping skips the removed item");
});

test("toggle group single and multiple selection semantics", async t => {
  const { mod, harness } = await mounted(t, "mountToggle", "single");
  const clickItem = id => harness.dispatch(harness.byTestId(id), "Click");
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
  const click2 = id => second.dispatch(second.byTestId(id), "Click");
  assert.equal(prop(second, "ta", "role"), "button");
  click2("tb");
  click2("ta");
  assert.deepEqual(mod2.api.log.toggle, [["b"], ["a", "b"]], "multiple keeps a sorted unique set");
  click2("tb");
  assert.deepEqual(mod2.api.log.toggle, [["b"], ["a", "b"], ["a"]]);
  second.dispatch(second.byTestId("ta"), "FocusIn");
  const before = mod2.api.log.toggle.length;
  sendKey(second, second.byTestId("ta"), "ArrowRight");
  assert.equal(mod2.api.log.toggle.length, before, "arrow focus never toggles");
  assert.equal(second.focused.current, second.byTestId("tb"));
});

test("controlled toggle reports attempts while visual state stays until echoed", async t => {
  const { mod, harness } = await mounted(t, "mountControlledToggle");
  const a = harness.byTestId("ta");
  harness.dispatch(a, "Click");
  assert.deepEqual(mod.api.log.toggle, [["a"]]);
  assert.equal(prop(harness, "ta", "data-state"), "off");
  mod.controls.setEchoToggle(true);
  harness.flush();
  harness.dispatch(a, "Click");
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
  harness.flush();
  assert.equal(prop(harness, "ta", "data-variant"), "outline");
  assert.match(prop(harness, "ta", "class"), /zui-toggle--outline/);
  assert.equal(
    prop(harness, "ta", "data-size"),
    "sm",
    "group size inherits when the item does not override",
  );

  mod.controls.setGroupDisabled(true);
  harness.flush();
  assert.equal(
    prop(harness, "tb", "disabled"),
    true,
    "item disabled=false cannot override the group",
  );
  const length = mod.api.log.toggle.length;
  harness.dispatch(b, "Click");
  assert.equal(mod.api.log.toggle.length, length);
  const keyEvent = sendKey(harness, harness.byTestId("ta"), "ArrowRight");
  assert.equal(keyEvent.defaultPrevented, false, "a disabled group does not navigate");
});

test("nested groups only own their items and standalone items are inert", async t => {
  const { mod, harness } = await mounted(t, "mountNestedPair");
  const outer = harness.byTestId("outer");
  const i1 = harness.byTestId("i1");
  assert.equal(tabIndex(harness, "o1"), 0, "each group keeps its own tab stop");
  assert.equal(tabIndex(harness, "i1"), 0);
  sendKey(harness, i1, "ArrowDown");
  assert.equal(harness.focused.current, harness.byTestId("i2"));
  const foreign = harness.dispatch(outer, "KeyDown", {
    target: i1,
    key: "ArrowUp",
    keyWithoutModifiers: "ArrowUp",
  });
  assert.equal(foreign.defaultPrevented, false, "a group ignores keys targeting another group");
  harness.dispatch(i1, "Click");
  assert.deepEqual(mod.api.log.radio, ["i2", "i1"]);
  assert.equal(checked(harness, "o1"), false);

  const second = createHost();
  globalThis.__zgui = second.host;
  const mod2 = await moduleFor(t, fixture);
  const root2 = second.makeNode("root");
  const dispose2 = mod2.mountStandalone(root2);
  cleanup(t, second, dispose2);
  assert.equal(prop(second, "sr", "tabIndex"), 0, "standalone items stay focusable");
  second.dispatch(second.byTestId("sr"), "Click");
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
  assert.throws(
    () =>
      mod.mountRadioChildren(root, () => {
        ran = true;
        return "x";
      }),
    /does not accept children/,
  );
  assert.equal(ran, false, "children are rejected before the getter runs");
});

test("hide and show remount produces fresh item nodes", async t => {
  const { mod, harness } = await mounted(t, "mountShown");
  const first = harness.byTestId("ra");
  assert.ok(first);
  mod.controls.setShowGroup(false);
  harness.flush();
  assert.equal(harness.byTestId("ra"), undefined);
  mod.controls.setShowGroup(true);
  harness.flush();
  const second = harness.byTestId("ra");
  assert.ok(second);
  assert.notEqual(second, first, "remounting creates fresh item nodes");
});

test("RadioGroupPrimitive and RadioItemPrimitive provide unstyled headless radio semantics with arbitrary children", async t => {
  const { mod, harness } = await mounted(t, "mountPrimitiveRadio");
  assert.equal(
    harness.stylesheets.size,
    0,
    "headless radio primitives install zero built-in stylesheets",
  );
  const group = harness.byTestId("prg");
  const pa = harness.byTestId("pa");
  const pb = harness.byTestId("pb");
  const pc = harness.byTestId("pc");
  assert.equal(prop(harness, "prg", "role"), "radioGroup");
  assert.equal(prop(harness, "prg", "ariaLabel"), "Segments");
  assert.equal(prop(harness, "prg", "orientation"), "horizontal");
  assert.equal(prop(harness, "prg", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "prg", "class"), undefined, "no built-in class on group");
  assert.equal(prop(harness, "pa", "class"), undefined, "no built-in class on item");
  assert.equal(prop(harness, "pa", "role"), "radioButton");
  assert.equal(prop(harness, "pa", "ariaLabel"), "Alpha");
  assert.equal(prop(harness, "pa", "ariaRadioGroup"), group);
  assert.equal(prop(harness, "pb", "ariaRadioGroup"), group);
  assert.equal(prop(harness, "pc", "ariaRadioGroup"), group);

  // Arbitrary children rendered without built-in icon or extra tab stops
  assert.ok(harness.byTestId("pa-row"));
  assert.ok(harness.byTestId("pa-dot"));
  assert.ok(harness.byTestId("pa-text"));
  assert.equal(prop(harness, "pa-row", "tabIndex"), undefined);
  assert.equal(prop(harness, "pa-dot", "tabIndex"), undefined);
  assert.equal(
    pa.children.length,
    1,
    "only authored child tree is mounted (no built-in DISC icon)",
  );

  // Checked fallback wins initial tab stop
  assert.equal(tabIndex(harness, "pa"), -1);
  assert.equal(tabIndex(harness, "pb"), 0);
  assert.equal(tabIndex(harness, "pc"), -1);
  assert.equal(checked(harness, "pb"), true);
  assert.equal(prop(harness, "pb", "data-state"), "checked");
  assert.equal(prop(harness, "pa", "data-state"), "unchecked");

  // Horizontal arrow navigation focuses and selects via own-focus
  sendKey(harness, pb, "ArrowDown");
  assert.equal(harness.focused.current, null, "vertical arrow ignored in horizontal mode");
  const right = sendKey(harness, pb, "ArrowRight");
  assert.equal(right.defaultPrevented, true);
  assert.equal(harness.focused.current, pc);
  assert.equal(checked(harness, "pc"), true);
  assert.deepEqual(mod.api.log.radio, ["c"]);

  // Home / End and pointer click activation
  sendKey(harness, pc, "Home");
  assert.equal(harness.focused.current, pa);
  assert.equal(checked(harness, "pa"), true);
  sendKey(harness, pa, "End");
  assert.equal(harness.focused.current, pc);
  assert.equal(checked(harness, "pc"), true);
  harness.dispatch(pb, "Click");
  assert.equal(checked(harness, "pb"), true);

  // Focus bubbling from a descendant child must NOT trigger own-focus selection
  const countBeforeChildFocus = mod.api.log.radio.length;
  harness.dispatch(harness.byTestId("pa-row"), "FocusIn");
  assert.equal(mod.api.log.radio.length, countBeforeChildFocus);
  assert.equal(checked(harness, "pb"), true);

  // Disabling all items exposes zero tab stops
  mod.primControls.setPrimDisabledAll(true);
  harness.flush();
  assert.equal(tabIndex(harness, "pa"), -1);
  assert.equal(tabIndex(harness, "pb"), -1);
  assert.equal(tabIndex(harness, "pc"), -1);
  mod.primControls.setPrimDisabledAll(false);
  mod.primControls.setPrimDisableB(true);
  harness.flush();
  assert.equal(tabIndex(harness, "pb"), -1);
  assert.equal(tabIndex(harness, "pa"), 0);
});

test("RadioGroupPrimitive controlled value never steals focus and honors parent veto", async t => {
  const { mod, harness } = await mounted(t, "mountPrimitiveControlledRadio");
  assert.equal(harness.stylesheets.size, 0);
  const pa = harness.byTestId("pa");
  const pb = harness.byTestId("pb");
  const pc = harness.byTestId("pc");
  assert.equal(tabIndex(harness, "pa"), 0);
  assert.equal(checked(harness, "pa"), true);

  // Focus moves via arrow key, parent vetoes selection change
  sendKey(harness, pa, "ArrowDown");
  assert.equal(harness.focused.current, pb);
  assert.deepEqual(mod.api.log.radio, ["b"]);
  assert.equal(
    checked(harness, "pb"),
    false,
    "focus never implies checked without state agreement",
  );
  assert.equal(checked(harness, "pa"), true);
  assert.equal(tabIndex(harness, "pb"), 0, "enabled roving-active item retains tab stop");
  assert.equal(tabIndex(harness, "pa"), -1);

  // Controlled external value update does NOT steal focus or override active roving tab stop
  mod.primControls.setPrimControlled("c");
  harness.flush();
  assert.equal(checked(harness, "pc"), true);
  assert.equal(harness.focused.current, pb, "controlled value update never steals focus");
  assert.equal(
    tabIndex(harness, "pb"),
    0,
    "roving-active item keeps tab stop over checked fallback",
  );
  assert.equal(tabIndex(harness, "pc"), -1);

  // When parent echoes, click selects cleanly
  mod.primControls.setPrimEcho(true);
  harness.flush();
  harness.dispatch(pc, "Click");
  assert.equal(checked(harness, "pc"), true);
  assert.equal(tabIndex(harness, "pc"), 0);

  // Empty value still throws clearly
  const root2 = harness.makeNode("root2");
  assert.throws(() => mod.mountPrimitiveInvalid(root2, ""), /non-empty string/);
});
