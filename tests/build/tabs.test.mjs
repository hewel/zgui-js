import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@hewel/zgui/components";
import type { GroupOrientation, TabsActivation, TabsListVariant } from "@hewel/zgui/components";

export const log = { changes: [] as string[], constructions: [] as string[], cleans: [] as string[] };
export function reset() {
  log.changes.length = 0; log.constructions.length = 0; log.cleans.length = 0;
}

function Counter(props: { id: string }) {
  const [count, setCount] = createSignal(0);
  log.constructions.push(props.id);
  onCleanup(() => { log.cleans.push(props.id); });
  return <control testId={props.id} onClick={() => setCount(n => n + 1)}>{\`n\${count()}\`}</control>;
}

export function mountPanelsFirst(root: NativeNode) {
  reset();
  return renderApp(() => <Tabs testId="tabs" defaultValue="b">
    <TabsContent testId="panel-a" value="a"><Counter id="count-a" /></TabsContent>
    <TabsContent testId="panel-b" value="b"><Counter id="count-b" /></TabsContent>
    <TabsList testId="list"><TabsTrigger testId="tab-a" value="a" /><TabsTrigger testId="tab-b" value="b" /></TabsList>
  </Tabs>, root);
}

export function mountTriggersFirst(root: NativeNode) {
  reset();
  return renderApp(() => <Tabs testId="tabs" defaultValue="b">
    <TabsList testId="list"><TabsTrigger testId="tab-a" value="a" /><TabsTrigger testId="tab-b" value="b" /></TabsList>
    <TabsContent testId="panel-a" value="a"><Counter id="count-a" /></TabsContent>
    <TabsContent testId="panel-b" value="b"><Counter id="count-b" /></TabsContent>
  </Tabs>, root);
}

export function mountKeepMounted(root: NativeNode) {
  reset();
  return renderApp(() => <Tabs testId="tabs" defaultValue="a">
    <TabsList testId="list"><TabsTrigger testId="tab-a" value="a" /><TabsTrigger testId="tab-b" value="b" /></TabsList>
    <TabsContent testId="panel-a" value="a" keepMounted><Counter id="count-a" /></TabsContent>
    <TabsContent testId="panel-b" value="b" keepMounted><Counter id="count-b" /></TabsContent>
  </Tabs>, root);
}

const [orientation, setOrientation] = createSignal<GroupOrientation>("horizontal");
const [activation, setActivation] = createSignal<TabsActivation>("automatic");
const [tabsDisabled, setTabsDisabled] = createSignal(false);
const [listDisabled, setListDisabled] = createSignal(false);
const [bDisabled, setBDisabled] = createSignal(false);
const [cancelKeys, setCancelKeys] = createSignal(false);
const [cancelClicks, setCancelClicks] = createSignal(false);
const [cancelFocus, setCancelFocus] = createSignal(false);

export const controls = {
  setOrientation, setActivation, setTabsDisabled, setListDisabled, setBDisabled,
  setCancelKeys, setCancelClicks, setCancelFocus,
};

export function mountInteractive(root: NativeNode) {
  reset();
  return renderApp(() => <Tabs testId="tabs" defaultValue="a" orientation={orientation()}
      activation={activation()} disabled={tabsDisabled()} onValueChange={v => log.changes.push(v)}>
    <TabsList testId="list" disabled={listDisabled()}
      onKeyDown={event => { if (cancelKeys()) event.preventDefault(); }}>
      <TabsTrigger testId="tab-a" value="a"
        onClick={event => { if (cancelClicks()) event.preventDefault(); }}
        onFocus={event => { if (cancelFocus()) event.preventDefault(); }} />
      <TabsTrigger testId="tab-b" value="b" disabled={bDisabled()}
        onClick={event => { if (cancelClicks()) event.preventDefault(); }}
        onFocus={event => { if (cancelFocus()) event.preventDefault(); }} />
      <TabsTrigger testId="tab-c" value="c" />
    </TabsList>
    <TabsContent testId="panel-a" value="a"><text>a</text></TabsContent>
    <TabsContent testId="panel-b" value="b"><text>b</text></TabsContent>
    <TabsContent testId="panel-c" value="c"><text>c</text></TabsContent>
  </Tabs>, root);
}

const [echo, setEcho] = createSignal("a");
const [ownedDefault, setOwnedDefault] = createSignal("a");
export const echoControls = { setEcho, setOwnedDefault };

export function mountControlled(root: NativeNode) {
  reset();
  return renderApp(() => <Tabs testId="tabs" value={echo()}
      onValueChange={v => log.changes.push(v)}>
    <TabsList testId="list">
      <TabsTrigger testId="tab-a" value="a" /><TabsTrigger testId="tab-b" value="b" />
      <TabsTrigger testId="tab-c" value="c" />
    </TabsList>
    <TabsContent testId="panel-a" value="a"><text>a</text></TabsContent>
    <TabsContent testId="panel-b" value="b"><text>b</text></TabsContent>
    <TabsContent testId="panel-c" value="c"><text>c</text></TabsContent>
  </Tabs>, root);
}

export function mountOwnedDefault(root: NativeNode) {
  reset();
  return renderApp(() => <Tabs testId="tabs" defaultValue={ownedDefault()}
      onValueChange={v => log.changes.push(v)}>
    <TabsList testId="list">
      <TabsTrigger testId="tab-a" value="a" /><TabsTrigger testId="tab-b" value="b" />
    </TabsList>
    <TabsContent testId="panel-a" value="a"><text>a</text></TabsContent>
    <TabsContent testId="panel-b" value="b"><text>b</text></TabsContent>
  </Tabs>, root);
}

function StatefulTrigger(props: { id: string; value: string }) {
  const [clicks, setClicks] = createSignal(0);
  return <TabsTrigger testId={\`tab-\${props.id}\`} value={props.value} onClick={() => setClicks(n => n + 1)}>
    <text testId={\`trigger-count-\${props.id}\`}>{clicks()}</text>
  </TabsTrigger>;
}

const [order, setOrder] = createSignal(["a", "b", "c"]);
const [showB, setShowB] = createSignal(true);
const [showPanelA, setShowPanelA] = createSignal(true);
const [renameA, setRenameA] = createSignal("a");
export const reorderControls = { setOrder, setShowB, setShowPanelA, setRenameA };

export function mountReorderable(root: NativeNode) {
  reset();
  return renderApp(() => <Tabs testId="tabs" defaultValue="b" onValueChange={v => log.changes.push(v)}>
    <TabsList testId="list"><For each={order()}>
      {v => <Show when={v !== "b" || showB()}>
        <StatefulTrigger id={v} value={v === "a" ? renameA() : v} />
      </Show>}
    </For></TabsList>
    <For each={order()}>{v => <Show when={v !== "a" || showPanelA()}>
      <TabsContent testId={\`panel-\${v}\`} value={v}><Counter id={\`count-\${v}\`} /></TabsContent>
    </Show>}</For>
    <TabsContent testId="panel-x" value="x"><text>x</text></TabsContent>
  </Tabs>, root);
}

export function mountNested(root: NativeNode) {
  reset();
  return renderApp(() => <Tabs testId="outer" defaultValue="a">
    <TabsList testId="outer-list">
      <TabsTrigger testId="outer-a" value="a" />
      <TabsTrigger testId="outer-b" value="ob" />
      <Tabs testId="inner-unlisted-root" defaultValue="a">
        <TabsTrigger testId="inner-unlisted" value="a" />
        <TabsContent testId="inner-unlisted-panel" value="a"><text>u</text></TabsContent>
      </Tabs>
    </TabsList>
    <TabsContent testId="outer-panel" value="a">
      <Tabs testId="inner" defaultValue="a">
        <TabsTrigger testId="inner-lonely" value="a" />
        <TabsContent testId="inner-panel" value="a"><text>i</text></TabsContent>
      </Tabs>
    </TabsContent>
    <TabsContent testId="outer-panel-b" value="ob"><text>ob</text></TabsContent>
  </Tabs>, root);
}

const [styleAxis, setStyleAxis] = createSignal<GroupOrientation>("vertical");
const [styleVariant, setStyleVariant] = createSignal<TabsListVariant>("line");
export const styleControls = { setStyleAxis, setStyleVariant };
export function mountStyledNested(root: NativeNode) {
  reset();
  return renderApp(() => <Tabs defaultValue="outer" orientation={styleAxis()}>
    <TabsList testId="styled-outer-list" variant={styleVariant()}>
      <box><TabsTrigger testId="styled-outer" value="outer">Outer</TabsTrigger></box>
      <Tabs defaultValue="inner" orientation="horizontal">
        <TabsList testId="styled-inner-list" variant="default">
          <box><TabsTrigger testId="styled-inner" value="inner">Inner</TabsTrigger></box>
        </TabsList>
        <TabsContent value="inner"><text>inner</text></TabsContent>
      </Tabs>
      <Tabs defaultValue="unlisted" orientation="both">
        <TabsTrigger testId="styled-unlisted" value="unlisted">Unlisted</TabsTrigger>
        <TabsContent value="unlisted"><text>unlisted</text></TabsContent>
      </Tabs>
    </TabsList>
    <TabsContent value="outer"><text>outer</text></TabsContent>
  </Tabs>, root);
}

export function mountInvalid(root: NativeNode, kind: string) {
  if (kind === "value") {
    return renderApp(() => <Tabs defaultValue={42 as never}><TabsList><TabsTrigger value="a" /></TabsList></Tabs>, root);
  }
  if (kind === "part") {
    return renderApp(() => <Tabs><TabsList><TabsTrigger value="" /></TabsList></Tabs>, root);
  }
  if (kind === "activation") {
    return renderApp(() => <Tabs activation={"weird" as never} />, root);
  }
  if (kind === "variant") {
    return renderApp(() => <Tabs><TabsList variant={"weird" as never} /></Tabs>, root);
  }
  return renderApp(() => <Tabs orientation={"weird" as never} />, root);
}
`;

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

async function mounted(t, name, ...args) {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod[name](root, ...args);
  cleanup(t, harness, dispose);
  return { mod, harness, dispose };
}

const prop = (harness, testId, name) => harness.byTestId(testId)?.props.get(name);
const node = (harness, testId) => harness.byTestId(testId);
const textOf = current =>
  current.tag === "#text" ? current.props.get("text") ?? "" : current.children.map(textOf).join("");
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
  preventDefault() { this.prevented = (this.prevented ?? 0) + 1; },
  stopPropagation() {}, stopImmediatePropagation() {},
});
const focusIn = (harness, testId) => {
  const current = node(harness, testId);
  current.callbacks.onFocus(focusEvent(current));
};
const press = (harness, testId) => {
  const current = node(harness, testId);
  current.callbacks.onClick(click(current));
};

test("lazy panels relate both ways regardless of creation order", async t => {
  for (const name of ["mountPanelsFirst", "mountTriggersFirst"]) {
    const { mod, harness } = await mounted(t, name);
    assert.equal(prop(harness, "list", "role"), "tabList", name);
    assert.equal(prop(harness, "tabs", "role"), "group", name);
    for (const id of ["tab-a", "tab-b"]) assert.equal(prop(harness, id, "role"), "tab", `${name} ${id}`);
    for (const id of ["panel-a", "panel-b"]) assert.equal(prop(harness, id, "role"), "tabPanel", `${name} ${id}`);
    assert.equal(prop(harness, "tab-a", "selected"), false, name);
    assert.equal(prop(harness, "tab-b", "selected"), true, name);
    assert.equal(prop(harness, "tab-a", "tabIndex"), -1, name);
    assert.equal(prop(harness, "tab-b", "tabIndex"), 0, name);
    assert.equal(prop(harness, "panel-a", "ariaHidden"), true, name);
    const tabA = node(harness, "tab-a");
    const panelA = node(harness, "panel-a");
    const tabB = node(harness, "tab-b");
    const panelB = node(harness, "panel-b");
    assert.equal(prop(harness, "tab-a", "ariaControls"), panelA, name);
    assert.equal(prop(harness, "panel-a", "ariaLabelledBy"), tabA, name);
    assert.equal(prop(harness, "tab-b", "ariaControls"), panelB, name);
    assert.equal(prop(harness, "panel-b", "ariaLabelledBy"), tabB, name);
    assert.deepEqual(mod.log.constructions, ["count-b"], `${name} only active panel builds children`);
    const firstB = node(harness, "count-b");
    press(harness, "tab-a");
    assert.equal(prop(harness, "tab-a", "selected"), true, name);
    assert.equal(node(harness, "panel-a"), panelA, `${name} panel shells keep identity`);
    assert.equal(node(harness, "panel-b"), panelB, name);
    assert.equal(prop(harness, "panel-b", "ariaHidden"), true, name);
    assert.deepEqual(mod.log.cleans, ["count-b"], `${name} leaving panel disposes its children once`);
    assert.ok(node(harness, "count-a"), `${name} entering panel builds children`);
    press(harness, "count-a");
    press(harness, "tab-b");
    const rebornB = node(harness, "count-b");
    assert.ok(rebornB, name);
    assert.notEqual(rebornB, firstB, `${name} returning panel gets fresh children`);
    assert.equal(textOf(rebornB), "n0", `${name} fresh children reset state`);
  }
});

test("keepMounted builds once and preserves child state across selection", async t => {
  const { mod, harness } = await mounted(t, "mountKeepMounted");
  assert.deepEqual(mod.log.constructions, ["count-a", "count-b"]);
  const countA = node(harness, "count-a");
  press(harness, "count-a");
  assert.equal(textOf(countA), "n1");
  press(harness, "tab-b");
  assert.equal(prop(harness, "tab-b", "selected"), true);
  assert.equal(prop(harness, "panel-a", "ariaHidden"), true);
  assert.equal(prop(harness, "panel-a", "data-state"), "inactive");
  assert.equal(prop(harness, "panel-b", "ariaHidden"), false);
  press(harness, "tab-a");
  assert.equal(node(harness, "count-a"), countA, "same counter node returns");
  assert.equal(textOf(countA), "n1", "counter state preserved");
  assert.deepEqual(mod.log.constructions, ["count-a", "count-b"], "no reconstruction");
  assert.deepEqual(mod.log.cleans, [], "no child cleanup while mounted");
});

test("automatic focus activation, roving navigation and axis switching", async t => {
  const { mod, harness } = await mounted(t, "mountInteractive");
  const list = node(harness, "list");
  focusIn(harness, "tab-b");
  assert.equal(prop(harness, "tab-b", "selected"), true, "automatic focus selects");
  focusIn(harness, "tab-a");
  assert.equal(prop(harness, "tab-a", "selected"), true);
  mod.controls.setBDisabled(true);
  const right = key(node(harness, "tab-a"), "ArrowRight");
  list.callbacks.onKeyDown(right);
  assert.equal(harness.focused.current, node(harness, "tab-c"), "ArrowRight skips disabled middle and lands on c");
  assert.equal(right.prevented, 1);
  const wrap = key(node(harness, "tab-c"), "ArrowRight");
  list.callbacks.onKeyDown(wrap);
  assert.equal(harness.focused.current, node(harness, "tab-a"), "wrap skips disabled item both directions");
  const end = key(node(harness, "tab-a"), "End");
  list.callbacks.onKeyDown(end);
  assert.equal(harness.focused.current, node(harness, "tab-c"));
  const home = key(node(harness, "tab-c"), "Home");
  list.callbacks.onKeyDown(home);
  assert.equal(harness.focused.current, node(harness, "tab-a"));
  mod.controls.setOrientation("vertical");
  const offAxis = key(node(harness, "tab-a"), "ArrowRight");
  list.callbacks.onKeyDown(offAxis);
  assert.equal(offAxis.prevented, 0, "ArrowRight is not consumed on a vertical list");
  assert.equal(harness.focused.current, node(harness, "tab-a"));
  const down = key(node(harness, "tab-a"), "ArrowDown");
  list.callbacks.onKeyDown(down);
  assert.equal(harness.focused.current, node(harness, "tab-c"), "ArrowDown works on vertical axis");
  mod.controls.setOrientation("horizontal");
  mod.controls.setActivation("manual");
  focusIn(harness, "tab-c");
  assert.equal(prop(harness, "tab-c", "selected"), false, "manual focus does not select");
  assert.equal(prop(harness, "tab-a", "selected"), true);
  press(harness, "tab-c");
  assert.equal(prop(harness, "tab-c", "selected"), true, "manual click selects");
});

test("controlled value waits for an echo and owned default survives prop changes", async t => {
  const { mod, harness } = await mounted(t, "mountControlled");
  assert.equal(prop(harness, "tab-a", "selected"), true);
  press(harness, "tab-b");
  assert.deepEqual(mod.log.changes, ["b"], "attempt reported");
  assert.equal(prop(harness, "tab-a", "selected"), true, "unecached attempt does not move selection");
  press(harness, "tab-b");
  assert.deepEqual(mod.log.changes, ["b", "b"], "repeated attempt reported again");
  mod.echoControls.setEcho("b");
  assert.equal(prop(harness, "tab-b", "selected"), true, "echo drives selection");
  assert.equal(prop(harness, "panel-b", "data-state"), "active");
  assert.deepEqual(mod.log.changes, ["b", "b"], "prop echo emits no callback");
  mod.echoControls.setEcho("c");
  assert.equal(prop(harness, "tab-c", "selected"), true, "later parent update drives panels");
  assert.deepEqual(mod.log.changes, ["b", "b"]);

  const owned = await mounted(t, "mountOwnedDefault");
  press(owned.harness, "tab-b");
  assert.equal(prop(owned.harness, "tab-b", "selected"), true);
  const beforeDefaultChange = owned.mod.log.changes.slice();
  owned.mod.echoControls.setOwnedDefault("unmatched");
  assert.equal(prop(owned.harness, "tab-b", "selected"), true, "defaultValue prop changes never reset owned state");
  assert.deepEqual(owned.mod.log.changes, beforeDefaultChange, "ignored default update emits no callback");
});

test("caller prevention and disabled state gate activation", async t => {
  const { mod, harness } = await mounted(t, "mountInteractive");
  const list = node(harness, "list");
  mod.controls.setCancelKeys(true);
  const cancelled = key(node(harness, "tab-a"), "ArrowRight");
  list.callbacks.onKeyDown(cancelled);
  assert.equal(harness.focused.current, null, "cancelled list key runs no internal navigation");
  mod.controls.setCancelKeys(false);
  mod.controls.setActivation("manual");
  focusIn(harness, "tab-b");
  mod.controls.setCancelFocus(true);
  focusIn(harness, "tab-c");
  assert.equal(prop(harness, "tab-c", "selected"), false);
  mod.controls.setActivation("automatic");
  focusIn(harness, "tab-b");
  assert.equal(prop(harness, "tab-b", "selected"), false, "cancelled focus runs no automatic selection");
  mod.controls.setCancelFocus(false);
  mod.controls.setCancelClicks(true);
  press(harness, "tab-b");
  assert.equal(prop(harness, "tab-b", "selected"), false, "cancelled click runs no selection");
  mod.controls.setCancelClicks(false);
  mod.controls.setBDisabled(true);
  press(harness, "tab-b");
  assert.equal(prop(harness, "tab-b", "selected"), false, "disabled item cannot be selected");
  mod.controls.setListDisabled(true);
  const beforeListDisabled = mod.log.changes.slice();
  press(harness, "tab-c");
  assert.equal(prop(harness, "tab-c", "selected"), false, "list-disabled item cannot be selected");
  assert.equal(prop(harness, "tab-a", "selected"), true);
  assert.deepEqual(mod.log.changes, beforeListDisabled, "list disable emits no callback");
  mod.controls.setBDisabled(false);
  mod.controls.setTabsDisabled(true);
  mod.controls.setListDisabled(false);
  const beforeTabsDisabled = mod.log.changes.slice();
  press(harness, "tab-b");
  assert.equal(prop(harness, "tab-b", "selected"), false, "root-disabled item cannot be selected");
  assert.deepEqual(mod.log.changes, beforeTabsDisabled, "root disable emits no callback");
  const blocked = key(node(harness, "tab-a"), "ArrowRight");
  list.callbacks.onKeyDown(blocked);
  assert.equal(blocked.prevented, 0, "root-disabled list key is not consumed");
  assert.equal(harness.focused.current, null);
  mod.controls.setTabsDisabled(false);
});

test("keyed reorder, part removal and value rename keep relations live", async t => {
  const { mod, harness } = await mounted(t, "mountReorderable");
  const list = node(harness, "list");
  const countB = node(harness, "count-b");
  press(harness, "count-b");
  assert.equal(textOf(countB), "n1");
  const parentOf = id => harness.parents.get(node(harness, id));
  const orderIds = () => parentOf("tab-a").children.map(child => child.props.get("testId"));
  assert.deepEqual(orderIds(), ["tab-a", "tab-b", "tab-c"]);
  const tabs = Object.fromEntries(["tab-a", "tab-b", "tab-c"].map(id => [id, node(harness, id)]));
  press(harness, "tab-b");
  assert.equal(textOf(node(harness, "trigger-count-b")), "1", "trigger-local counter increments");
  mod.reorderControls.setOrder(["c", "a", "b"]);
  assert.deepEqual(orderIds(), ["tab-c", "tab-a", "tab-b"], "native order follows keyed reorder");
  for (const id of ["tab-a", "tab-b", "tab-c"]) {
    assert.equal(node(harness, id), tabs[id], `${id} keeps identity through reorder`);
  }
  assert.equal(textOf(node(harness, "trigger-count-b")), "1", "trigger-local state survives reorder");
  assert.equal(node(harness, "count-b"), countB, "reorder keeps panel child identity");
  assert.equal(textOf(countB), "n1", "reorder keeps panel child state");
  focusIn(harness, "tab-a");
  list.callbacks.onKeyDown(key(node(harness, "tab-a"), "ArrowRight"));
  assert.equal(harness.focused.current, node(harness, "tab-b"), "arrows follow new tree order");
  const firstB = node(harness, "tab-b");
  mod.reorderControls.setShowB(false);
  assert.equal(prop(harness, "panel-b", "ariaLabelledBy") ?? null, null, "missing trigger clears the relation");
  mod.reorderControls.setShowB(true);
  const rebornB = node(harness, "tab-b");
  assert.notEqual(rebornB, firstB, "reinserted trigger is a fresh node");
  assert.equal(prop(harness, "panel-b", "ariaLabelledBy"), rebornB, "relation follows the fresh handle");
  const firstPanelA = node(harness, "panel-a");
  mod.reorderControls.setShowPanelA(false);
  assert.equal(prop(harness, "tab-a", "ariaControls") ?? null, null, "missing panel clears the relation");
  mod.reorderControls.setShowPanelA(true);
  const rebornPanelA = node(harness, "panel-a");
  assert.notEqual(rebornPanelA, firstPanelA, "reinserted panel is a fresh node");
  assert.equal(prop(harness, "tab-a", "ariaControls"), rebornPanelA);
  mod.reorderControls.setRenameA("x");
  assert.equal(prop(harness, "tab-a", "data-value"), "x", "renamed trigger reports the current value");
  assert.equal(prop(harness, "tab-a", "ariaControls"), node(harness, "panel-x"), "relation follows the current value");
  press(harness, "tab-a");
  assert.deepEqual(mod.log.changes, ["a", "x"], "selection uses the current value");
  assert.equal(prop(harness, "panel-x", "data-state"), "active");
});

test("nested tabs and same-named values stay in their own context", async t => {
  const { mod, harness } = await mounted(t, "mountNested");
  const outerList = node(harness, "outer-list");
  assert.equal(prop(harness, "outer-a", "ariaControls"), node(harness, "outer-panel"));
  assert.equal(prop(harness, "inner-lonely", "ariaControls"), node(harness, "inner-panel"),
    "inner trigger relates to its own panel");
  assert.notEqual(prop(harness, "inner-lonely", "ariaControls"), node(harness, "outer-panel"));
  assert.equal(prop(harness, "inner-lonely", "selected"), true, "inner selection independent of outer");
  assert.equal(prop(harness, "inner-panel", "ariaHidden"), false);
  focusIn(harness, "outer-a");
  outerList.callbacks.onKeyDown(key(node(harness, "outer-a"), "ArrowRight"));
  assert.equal(harness.focused.current, node(harness, "outer-b"), "outer arrows stay in the outer group");
  assert.equal(prop(harness, "inner-unlisted", "ariaControls"), node(harness, "inner-unlisted-panel"),
    "unlisted inner trigger relates to its own panel");
  assert.equal(prop(harness, "inner-unlisted", "selected"), true);
  outerList.callbacks.onKeyDown(key(node(harness, "outer-b"), "ArrowRight"));
  assert.equal(harness.focused.current, node(harness, "outer-a"), "outer wrap ignores the inner trigger");
  focusIn(harness, "outer-b");
  assert.equal(prop(harness, "outer-b", "selected"), true);
  assert.equal(node(harness, "inner-lonely"), undefined, "leaving the outer panel unmounts inner children");
});

test("disposal under a stopping host is quiet and invalid props fail clearly", async t => {
  const { mod, harness, dispose } = await mounted(t, "mountReorderable");
  const registrationsAfterStop = [];
  let stopping = false;
  const original = harness.host.setProperty;
  harness.host.setProperty = (n, name, value, previous) => {
    if (stopping && name.startsWith("on") && value != null) {
      registrationsAfterStop.push(name);
      throw new ReferenceError("native session is stopping");
    }
    return original(n, name, value, previous);
  };
  press(harness, "tab-a");
  press(harness, "tab-c");
  assert.deepEqual(mod.log.constructions.sort(), ["count-a", "count-b", "count-c"], "all panels built before teardown");
  stopping = true;
  assert.doesNotThrow(() => dispose());
  assert.deepEqual(registrationsAfterStop, []);
  assert.deepEqual(mod.log.cleans.sort(), ["count-a", "count-b", "count-c"], "every child cleanup ran");
  const invalidMod = await moduleFor(t, fixture);
  for (const kind of ["value", "part", "activation", "variant", "orientation"]) {
    const invalidHarness = createHost();
    globalThis.__zgui = invalidHarness.host;
    const root = invalidHarness.makeNode("root");
    assert.throws(() => invalidMod.mountInvalid(root, kind), TypeError, `${kind} fails clearly`);
  }
  delete globalThis.__zgui;
});

test("nested tabs keep their own orientation and variant attributes", async t => {
  const { mod, harness } = await mounted(t, "mountStyledNested");
  const inner = node(harness, "styled-inner");
  const unlisted = node(harness, "styled-unlisted");
  assert.ok(inner && unlisted);
  assert.equal(prop(harness, "styled-outer-list", "data-orientation"), "vertical");
  assert.equal(prop(harness, "styled-outer-list", "data-variant"), "line");
  assert.equal(prop(harness, "styled-outer", "data-orientation"), "vertical");
  assert.equal(prop(harness, "styled-outer", "data-variant"), "line");
  assert.equal(prop(harness, "styled-inner-list", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "styled-inner", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "styled-inner", "data-variant"), "default");
  assert.equal(prop(harness, "styled-unlisted", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "styled-unlisted", "data-variant") ?? null, null);
  mod.styleControls.setStyleAxis("horizontal");
  mod.styleControls.setStyleVariant("default");
  assert.equal(prop(harness, "styled-outer-list", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "styled-outer-list", "data-variant"), "default");
  assert.equal(prop(harness, "styled-outer", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "styled-outer", "data-variant"), "default");
  assert.equal(node(harness, "styled-inner"), inner, "inner trigger keeps identity across outer restyle");
  assert.equal(node(harness, "styled-unlisted"), unlisted, "unlisted trigger keeps identity across outer restyle");
  assert.equal(prop(harness, "styled-inner", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "styled-inner", "data-variant"), "default");
  assert.equal(prop(harness, "styled-unlisted", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "styled-unlisted", "data-variant") ?? null, null);
  mod.styleControls.setStyleAxis("vertical");
  mod.styleControls.setStyleVariant("line");
  assert.equal(prop(harness, "styled-outer", "data-orientation"), "vertical");
  assert.equal(prop(harness, "styled-outer", "data-variant"), "line");
  assert.equal(prop(harness, "styled-inner", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "styled-inner", "data-variant"), "default");
  assert.equal(prop(harness, "styled-unlisted", "data-orientation"), "horizontal");
  assert.equal(prop(harness, "styled-unlisted", "data-variant") ?? null, null);
});
