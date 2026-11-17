import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { For, Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode, NativeStyle } from "@hewel/zgui";
import {
  Accordion, AccordionContent, AccordionItem, AccordionTrigger,
  Collapsible, CollapsibleContent, CollapsibleTrigger,
} from "@hewel/zgui/components";

export const log = { changes: [] as unknown[], constructions: [] as string[], cleans: [] as string[] };
export function reset() {
  log.changes.length = 0; log.constructions.length = 0; log.cleans.length = 0;
}

function Counter(props: { id: string }) {
  const [count, setCount] = createSignal(0);
  log.constructions.push(props.id);
  onCleanup(() => { log.cleans.push(props.id); });
  return <control testId={props.id} onClick={() => setCount(n => n + 1)}>{\`n\${count()}\`}</control>;
}

const [colDefault, setColDefault] = createSignal(true);
const [colDisabled, setColDisabled] = createSignal(false);
const [trigDisabled, setTrigDisabled] = createSignal(false);
const [cancelClick, setCancelClick] = createSignal(false);
export const collapsible = { setColDefault, setColDisabled, setTrigDisabled, setCancelClick };

export function mountCollapsible(root: NativeNode) {
  reset();
  return renderApp(() => <Collapsible testId="col" defaultOpen={colDefault()} disabled={colDisabled()}>
    <CollapsibleContent testId="col-content"><Counter id="col-count" /></CollapsibleContent>
    <CollapsibleTrigger testId="col-trigger" disabled={trigDisabled()}
      onClick={event => { if (cancelClick()) event.preventDefault(); }}>Toggle</CollapsibleTrigger>
  </Collapsible>, root);
}

const [controlledOpen, setControlledOpen] = createSignal(false);
export const controlled = { setControlledOpen };
export function mountControlledCollapsible(root: NativeNode) {
  reset();
  return renderApp(() => <Collapsible testId="col2" open={controlledOpen()} disabled={colDisabled()}
      onOpenChange={value => log.changes.push(value)}>
    <CollapsibleTrigger testId="col2-trigger" disabled={trigDisabled()}>Toggle</CollapsibleTrigger>
    <CollapsibleContent testId="col2-content"><Counter id="col2-count" /></CollapsibleContent>
  </Collapsible>, root);
}

export function mountNestedCollapsible(root: NativeNode) {
  reset();
  return renderApp(() => <Collapsible defaultOpen>
    <CollapsibleTrigger testId="outer-trig">Outer</CollapsibleTrigger>
    <CollapsibleContent testId="outer-content">
      <Collapsible testId="inner-col" defaultOpen>
        <CollapsibleTrigger testId="inner-trig">Inner</CollapsibleTrigger>
        <CollapsibleContent testId="inner-content"><Counter id="inner-count" /></CollapsibleContent>
      </Collapsible>
    </CollapsibleContent>
  </Collapsible>, root);
}

const [callerStyle, setCallerStyle] = createSignal<NativeStyle | undefined>({ color: "red" });
export const measured = { setCallerStyle };
export function mountMeasured(root: NativeNode) {
  reset();
  return renderApp(() => <Collapsible testId="m-col">
    <CollapsibleTrigger testId="m-trig">Toggle</CollapsibleTrigger>
    <CollapsibleContent testId="m-content" style={callerStyle()}><text>m</text></CollapsibleContent>
  </Collapsible>, root);
}

const [noTrigOpen, setNoTrigOpen] = createSignal(true);
export const focusless = { setNoTrigOpen };
export function mountTriggerless(root: NativeNode) {
  reset();
  return renderApp(() => <Collapsible testId="f-col" open={noTrigOpen()}>
    <CollapsibleContent testId="f-content"><text>f</text></CollapsibleContent>
  </Collapsible>, root);
}

const [blurOpen, setBlurOpen] = createSignal(true);
export function mountCloseOnBlur(root: NativeNode) {
  return renderApp(() => <column>
    <Collapsible testId="blur-root" open={blurOpen()}>
      <CollapsibleTrigger testId="blur-trigger">Details</CollapsibleTrigger>
      <CollapsibleContent testId="blur-content" onBlur={() => setBlurOpen(false)}>
        <Counter id="blur-child" />
      </CollapsibleContent>
    </Collapsible>
    <control testId="blur-outside">Outside</control>
  </column>, root);
}

const [colShown, setColShown] = createSignal(true);
export const shown = { setColShown };
export function mountShownCollapsible(root: NativeNode) {
  reset();
  return renderApp(() => <Show when={colShown()}>
    <Collapsible testId="s-col" defaultOpen>
      <CollapsibleTrigger testId="s-trig">Toggle</CollapsibleTrigger>
      <CollapsibleContent testId="s-content"><Counter id="s-count" /></CollapsibleContent>
    </Collapsible>
  </Show>, root);
}

export const inputB = ["b"];
const [collapsibleMode, setCollapsibleMode] = createSignal(true);
const [groupDisabled, setGroupDisabled] = createSignal(false);
const [bDisabled, setBDisabled] = createSignal(false);
const [cancelKeys, setCancelKeys] = createSignal(false);
const [cancelClicks, setCancelClicks] = createSignal(false);
const [accLevel, setAccLevel] = createSignal(3);
const [caption, setCaption] = createSignal("A");
export const accordion = {
  setCollapsibleMode, setGroupDisabled, setBDisabled,
  setCancelKeys, setCancelClicks, setAccLevel, setCaption,
};

export function mountAccordion(root: NativeNode) {
  reset();
  return renderApp(() => <Accordion testId="acc" collapsible={collapsibleMode()} disabled={groupDisabled()}
      defaultValue={inputB} onValueChange={v => log.changes.push([...v])}
      onKeyDown={event => { if (cancelKeys()) event.preventDefault(); }}>
    <AccordionItem value="a" testId="item-a">
      <AccordionTrigger testId="trig-a" level={accLevel()}
        onClick={event => { if (cancelClicks()) event.preventDefault(); }}>{caption()}</AccordionTrigger>
      <AccordionContent testId="cont-a"><Counter id="count-a" /></AccordionContent>
    </AccordionItem>
    <AccordionItem value="b" testId="item-b" disabled={bDisabled()}>
      <AccordionTrigger testId="trig-b">B</AccordionTrigger>
      <AccordionContent testId="cont-b"><Counter id="count-b" /></AccordionContent>
    </AccordionItem>
    <AccordionItem value="c" testId="item-c">
      <AccordionTrigger testId="trig-c">C</AccordionTrigger>
      <AccordionContent testId="cont-c"><Counter id="count-c" /></AccordionContent>
    </AccordionItem>
  </Accordion>, root);
}

export function mountAccordionMultiple(root: NativeNode) {
  reset();
  return renderApp(() => <Accordion testId="acc-m" selection="multiple" defaultValue={inputB}
      onValueChange={v => log.changes.push([...v])}>
    <AccordionItem value="a">
      <AccordionTrigger testId="m-trig-a">A</AccordionTrigger>
      <AccordionContent testId="m-cont-a"><text>a</text></AccordionContent>
    </AccordionItem>
    <AccordionItem value="b">
      <AccordionTrigger testId="m-trig-b">B</AccordionTrigger>
      <AccordionContent testId="m-cont-b"><text>b</text></AccordionContent>
    </AccordionItem>
  </Accordion>, root);
}

const [accValue, setAccValue] = createSignal<readonly string[]>(["b"]);
export const accControlled = { setAccValue };
export function mountAccordionControlled(root: NativeNode) {
  reset();
  return renderApp(() => <Accordion testId="acc-c" value={accValue()}
      onValueChange={v => log.changes.push([...v])}>
    <AccordionItem value="a">
      <AccordionTrigger testId="c-trig-a">A</AccordionTrigger>
      <AccordionContent testId="c-cont-a"><text>a</text></AccordionContent>
    </AccordionItem>
    <AccordionItem value="b">
      <AccordionTrigger testId="c-trig-b">B</AccordionTrigger>
      <AccordionContent testId="c-cont-b"><text>b</text></AccordionContent>
    </AccordionItem>
  </Accordion>, root);
}

export function mountNestedAccordion(root: NativeNode) {
  reset();
  return renderApp(() => <Accordion testId="outer-acc" defaultValue={["o1"]}
      onValueChange={v => log.changes.push(["outer", ...v])}>
    <AccordionItem value="o1">
      <AccordionTrigger testId="o-trig-1">O1</AccordionTrigger>
      <AccordionContent testId="o-cont-1">
        <Accordion testId="inner-acc" defaultValue={["i1"]}
            onValueChange={v => log.changes.push(["inner", ...v])}>
          <AccordionItem value="i1">
            <AccordionTrigger testId="i-trig-1">I1</AccordionTrigger>
            <AccordionContent testId="i-cont-1"><Counter id="i-count" /></AccordionContent>
          </AccordionItem>
          <AccordionItem value="i2">
            <AccordionTrigger testId="i-trig-2">I2</AccordionTrigger>
            <AccordionContent testId="i-cont-2"><text>i2</text></AccordionContent>
          </AccordionItem>
        </Accordion>
      </AccordionContent>
    </AccordionItem>
    <AccordionItem value="o2">
      <AccordionTrigger testId="o-trig-2">O2</AccordionTrigger>
      <AccordionContent testId="o-cont-2">
        <Collapsible testId="nc-col" defaultOpen>
          <CollapsibleTrigger testId="nc-trig">N</CollapsibleTrigger>
          <CollapsibleContent testId="nc-content"><text>n</text></CollapsibleContent>
        </Collapsible>
      </AccordionContent>
    </AccordionItem>
  </Accordion>, root);
}

export function mountStyledNestedTrigger(root: NativeNode) {
  reset();
  return renderApp(() => <Accordion testId="scoped-acc" defaultValue={["a"]}
      onValueChange={v => log.changes.push([...v])}>
    <AccordionItem value="a">
      <AccordionTrigger testId="scoped-a">A</AccordionTrigger>
      <AccordionContent>
        <Collapsible defaultOpen>
          <AccordionTrigger testId="styled-inner">Independent</AccordionTrigger>
          <CollapsibleContent testId="styled-body"><text>Body</text></CollapsibleContent>
        </Collapsible>
      </AccordionContent>
    </AccordionItem>
    <AccordionItem value="b"><AccordionTrigger testId="scoped-b">B</AccordionTrigger></AccordionItem>
  </Accordion>, root);
}

const [order, setOrder] = createSignal(["a", "b", "c"]);
const [showItemB, setShowItemB] = createSignal(true);
export const reorder = { setOrder, setShowItemB };
export function mountAccordionReorder(root: NativeNode) {
  reset();
  return renderApp(() => <Accordion testId="r-acc" defaultValue={["b"]}
      onValueChange={v => log.changes.push([...v])}>
    <For each={order()}>{v => <Show when={v !== "b" || showItemB()}>
      <AccordionItem value={v}>
        <AccordionTrigger testId={\`r-trig-\${v}\`}>{v.toUpperCase()}</AccordionTrigger>
        <AccordionContent testId={\`r-cont-\${v}\`}><Counter id={\`r-count-\${v}\`} /></AccordionContent>
      </AccordionItem>
    </Show>}</For>
  </Accordion>, root);
}

export function mountInvalid(root: NativeNode, kind: string) {
  if (kind === "selection") {
    return renderApp(() => <Accordion selection={"weird" as never} defaultValue={["a"]}>
      <AccordionItem value="a"><AccordionTrigger testId="bad-trig" /></AccordionItem>
    </Accordion>, root);
  }
  if (kind === "itemValue") {
    return renderApp(() => <Accordion><AccordionItem value="" /></Accordion>, root);
  }
  if (kind === "level") {
    return renderApp(() => <Accordion><AccordionItem value="a">
      <AccordionTrigger level={0} /></AccordionItem></Accordion>, root);
  }
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
  const registrations = [];
  harness.host.observeGeometry = (node, kind, callback) => {
    const record = { node, kind, callback, active: true, disposals: 0 };
    registrations.push(record);
    let disposed = false;
    return () => {
      if (disposed) return;
      disposed = true;
      record.disposals += 1;
      record.active = false;
    };
  };
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod[name](root, ...args);
  cleanup(t, harness, dispose);
  const emit = (record, value) => { if (record.active) record.callback(value); };
  const geometryOf = (node, kind) => registrations.find(r => r.node === node && r.kind === kind);
  return { mod, harness, registrations, emit, geometryOf };
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

test("collapsible keeps mounted children stable across close and open", async t => {
  const { mod, harness, registrations } = await mounted(t, "mountCollapsible");
  assert.equal(registrations.length, 2);
  const [innerObs, outerObs] = registrations;
  assert.equal(innerObs.kind, "borderSize");
  assert.equal(outerObs.kind, "scrollPosition");
  assert.deepEqual(mod.log.constructions, ["col-count"]);
  const trigger = node(harness, "col-trigger");
  const content = node(harness, "col-content");
  const counter = node(harness, "col-count");
  assert.equal(prop(harness, "col-trigger", "expanded"), true);
  assert.equal(prop(harness, "col-trigger", "data-state"), "open");
  assert.equal(prop(harness, "col", "data-state"), "open");
  assert.equal(prop(harness, "col-content", "ariaHidden"), false);
  assert.equal(prop(harness, "col-content", "data-state"), "open");
  assert.equal(prop(harness, "col-trigger", "ariaControls"), content);

  press(harness, "col-count");
  assert.equal(textOf(counter), "n1");
  press(harness, "col-trigger");
  assert.equal(prop(harness, "col-trigger", "expanded"), false);
  assert.equal(prop(harness, "col-trigger", "data-state"), "closed");
  assert.equal(prop(harness, "col-content", "ariaHidden"), true);
  assert.equal(prop(harness, "col-content", "data-state"), "closed");
  assert.equal(node(harness, "col-count"), counter, "counter survives closing");
  assert.equal(textOf(counter), "n1", "counter state survives closing");
  assert.equal(innerObs.active, true, "measurement stays live while closed");
  assert.equal(outerObs.active, true, "viewport observation stays live while closed");

  press(harness, "col-trigger");
  assert.equal(prop(harness, "col-trigger", "expanded"), true);
  assert.equal(node(harness, "col-count"), counter, "same counter node reopens");
  assert.equal(textOf(counter), "n1");
  assert.deepEqual(mod.log.constructions, ["col-count"], "no reconstruction");
  assert.deepEqual(mod.log.cleans, [], "no cleanup while mounted");
});

test("defaultOpen signal changes after mount do not reset state", async t => {
  const { mod, harness } = await mounted(t, "mountCollapsible");
  mod.collapsible.setColDefault(false);
  assert.equal(prop(harness, "col-trigger", "expanded"), true);
  press(harness, "col-trigger");
  assert.equal(prop(harness, "col-trigger", "expanded"), false);
  mod.collapsible.setColDefault(true);
  assert.equal(prop(harness, "col-trigger", "expanded"), false, "initial default is captured once");
});

test("controlled collapsible only changes after the value echoes", async t => {
  const { mod, harness } = await mounted(t, "mountControlledCollapsible");
  assert.equal(prop(harness, "col2-trigger", "expanded"), false);
  press(harness, "col2-trigger");
  assert.deepEqual(mod.log.changes, [true]);
  assert.equal(prop(harness, "col2-trigger", "expanded"), false, "request alone does not open");
  mod.controlled.setControlledOpen(true);
  assert.equal(prop(harness, "col2-trigger", "expanded"), true);
  press(harness, "col2-trigger");
  assert.deepEqual(mod.log.changes, [true, false]);
  assert.equal(prop(harness, "col2-trigger", "expanded"), true, "still open until echo");
  mod.controlled.setControlledOpen(false);
  assert.equal(prop(harness, "col2-trigger", "expanded"), false);
});

test("collapsible disabled and click cancellation block toggling", async t => {
  const { mod, harness } = await mounted(t, "mountCollapsible");
  mod.collapsible.setColDisabled(true);
  assert.equal(prop(harness, "col-trigger", "disabled"), true);
  press(harness, "col-trigger");
  assert.equal(prop(harness, "col-trigger", "expanded"), true, "root disabled blocks");
  mod.collapsible.setColDisabled(false);
  mod.collapsible.setTrigDisabled(true);
  assert.equal(prop(harness, "col-trigger", "disabled"), true);
  press(harness, "col-trigger");
  assert.equal(prop(harness, "col-trigger", "expanded"), true, "trigger disabled blocks");
  mod.collapsible.setTrigDisabled(false);
  mod.collapsible.setCancelClick(true);
  press(harness, "col-trigger");
  assert.equal(prop(harness, "col-trigger", "expanded"), true, "cancelled click blocks");
  mod.collapsible.setCancelClick(false);
  press(harness, "col-trigger");
  assert.equal(prop(harness, "col-trigger", "expanded"), false, "clearing restores toggle");
});

test("nested collapsible relates to its own body only", async t => {
  const { harness } = await mounted(t, "mountNestedCollapsible");
  const outerContent = node(harness, "outer-content");
  const innerContent = node(harness, "inner-content");
  assert.equal(prop(harness, "outer-trig", "ariaControls"), outerContent);
  assert.equal(prop(harness, "inner-trig", "ariaControls"), innerContent);
  assert.notEqual(prop(harness, "inner-trig", "ariaControls"), outerContent);
  press(harness, "inner-trig");
  assert.equal(prop(harness, "inner-content", "ariaHidden"), true);
  assert.equal(prop(harness, "outer-content", "ariaHidden"), false, "outer body untouched");
  press(harness, "outer-trig");
  assert.equal(prop(harness, "outer-content", "ariaHidden"), true);
  assert.equal(prop(harness, "inner-content", "ariaHidden"), true, "inner stays closed");
});

test("measured borderSize merges with caller style overrides", async t => {
  const { mod, harness, registrations } = await mounted(t, "mountMeasured");
  const innerObs = registrations.find(r => r.kind === "borderSize");
  const content = node(harness, "m-content");
  innerObs.callback({ width: 120, height: 40.25 });
  let style = prop(harness, "m-content", "style");
  assert.equal(style["--zui-collapsible-height"], "41px", "ceil of measured height");
  assert.equal(style.color, "red", "caller object style survives");

  mod.measured.setCallerStyle({ border: "2px" });
  style = prop(harness, "m-content", "style");
  assert.equal(style["--zui-collapsible-height"], "41px", "measurement retained across style change");
  assert.equal(style.border, "2px");

  mod.measured.setCallerStyle("color:blue;");
  style = prop(harness, "m-content", "style");
  assert.equal(style, "--zui-collapsible-height:41px;color:blue;", "string style keeps measurement");
  assert.equal(node(harness, "m-content"), content);
});

test("collapsed marker tracks the clipping viewport height", async t => {
  const { harness, registrations } = await mounted(t, "mountMeasured");
  assert.equal(prop(harness, "m-content", "ariaHidden"), true, "closed by default");
  const outerObs = registrations.find(r => r.kind === "scrollPosition");
  outerObs.callback({ scrollport: { width: 100, height: 12 } });
  assert.equal(prop(harness, "m-content", "data-collapsed"), undefined, "visible while clipping");
  outerObs.callback({ scrollport: { width: 100, height: 0 } });
  assert.equal(prop(harness, "m-content", "data-collapsed"), "", "hidden once viewport reaches zero");
  press(harness, "m-trig");
  assert.equal(prop(harness, "m-content", "data-collapsed"), undefined, "opening clears immediately");
  assert.equal(prop(harness, "m-content", "ariaHidden"), false);
});

test("closing restores focus to the trigger", async t => {
  const { harness } = await mounted(t, "mountCollapsible");
  focusIn(harness, "col-content");
  press(harness, "col-trigger");
  assert.equal(harness.focused.current, node(harness, "col-trigger"), "focus returns to trigger");
});

test("closing without a trigger restores focus to the root", async t => {
  const { mod, harness } = await mounted(t, "mountTriggerless");
  focusIn(harness, "f-content");
  mod.focusless.setNoTrigOpen(false);
  assert.equal(harness.focused.current, node(harness, "f-col"), "focus falls back to root");
  assert.equal(harness.focused.reveal, true);
});

test("closing in onBlur does not steal focus from the destination", async t => {
  const { harness } = await mounted(t, "mountCloseOnBlur");
  const content = node(harness, "blur-content");
  const child = node(harness, "blur-child");
  const outside = node(harness, "blur-outside");
  harness.focused.current = child;
  content.callbacks.onFocus(focusEvent(child));
  harness.focused.current = outside;
  content.callbacks.onBlur({ ...focusEvent(child), type: "focusout" });
  assert.equal(prop(harness, "blur-trigger", "expanded"), false);
  assert.equal(harness.focused.current, outside);
});

test("closing with a disabled trigger restores focus to the root", async t => {
  const { mod, harness } = await mounted(t, "mountControlledCollapsible");
  mod.controlled.setControlledOpen(true);
  const content = node(harness, "col2-content");
  const child = node(harness, "col2-count");
  content.callbacks.onFocus(focusEvent(child));
  mod.collapsible.setTrigDisabled(true);
  mod.controlled.setControlledOpen(false);
  assert.equal(harness.focused.current, node(harness, "col2"));
  assert.equal(prop(harness, "col2", "tabIndex"), -1);
});

test("closing with a disabled root restores focus to the root", async t => {
  const { mod, harness } = await mounted(t, "mountControlledCollapsible");
  mod.controlled.setControlledOpen(true);
  const content = node(harness, "col2-content");
  const child = node(harness, "col2-count");
  content.callbacks.onFocus(focusEvent(child));
  mod.collapsible.setColDisabled(true);
  mod.controlled.setControlledOpen(false);
  assert.equal(harness.focused.current, node(harness, "col2"));
  assert.equal(prop(harness, "col2", "tabIndex"), -1);
});

test("unmount cancels observations and remount registers fresh ones", async t => {
  const { mod, harness, registrations } = await mounted(t, "mountShownCollapsible");
  assert.equal(registrations.length, 2);
  const oldContent = node(harness, "s-content");
  mod.shown.setColShown(false);
  assert.equal(node(harness, "s-content"), undefined, "detached");
  assert.equal(registrations[0].disposals, 1);
  assert.equal(registrations[1].disposals, 1);
  assert.equal(registrations[0].active, false);
  assert.deepEqual(mod.log.cleans, ["s-count"]);

  mod.shown.setColShown(true);
  assert.equal(registrations.length, 4, "remount subscribes again");
  assert.equal(registrations[2].active, true);
  assert.equal(registrations[3].active, true);
  const newContent = node(harness, "s-content");
  assert.notEqual(newContent, oldContent, "fresh content node");
  assert.equal(prop(harness, "s-trig", "ariaControls"), newContent, "relation points at fresh content");
  registrations[0].callback({ width: 1, height: 999 });
  assert.ok(oldContent.props.get("style")["--zui-collapsible-height"] == null,
    "stale emission stays inert");
});

test("accordion arrows and Home/End move focus without changing expansion", async t => {
  const { mod, harness } = await mounted(t, "mountAccordion");
  const acc = node(harness, "acc");
  assert.equal(prop(harness, "acc", "role"), "group");
  assert.equal(prop(harness, "item-b", "data-state"), "open");
  assert.equal(prop(harness, "item-a", "data-state"), "closed");
  assert.equal(prop(harness, "trig-a", "tabIndex"), 0);
  assert.equal(prop(harness, "trig-b", "tabIndex"), -1);

  focusIn(harness, "trig-b");
  assert.equal(prop(harness, "trig-b", "tabIndex"), 0, "focus activates the roving item");
  assert.equal(prop(harness, "trig-a", "tabIndex"), -1);
  acc.callbacks.onKeyDown(key(node(harness, "trig-b"), "ArrowDown"));
  assert.equal(harness.focused.current, node(harness, "trig-c"));
  acc.callbacks.onKeyDown(key(node(harness, "trig-c"), "ArrowDown"));
  assert.equal(harness.focused.current, node(harness, "trig-a"), "wraps to first");
  acc.callbacks.onKeyDown(key(node(harness, "trig-a"), "End"));
  assert.equal(harness.focused.current, node(harness, "trig-c"));
  acc.callbacks.onKeyDown(key(node(harness, "trig-c"), "Home"));
  assert.equal(harness.focused.current, node(harness, "trig-a"));
  acc.callbacks.onKeyDown(key(node(harness, "trig-a"), "ArrowUp"));
  assert.equal(harness.focused.current, node(harness, "trig-c"), "up wraps to last");
  assert.equal(prop(harness, "item-b", "data-state"), "open", "navigation never expands");
  assert.equal(prop(harness, "item-a", "data-state"), "closed");
  assert.deepEqual(mod.log.changes, []);

  mod.accordion.setBDisabled(true);
  acc.callbacks.onKeyDown(key(node(harness, "trig-a"), "ArrowDown"));
  assert.equal(harness.focused.current, node(harness, "trig-c"), "skips disabled item");
});

test("accordion single clicks, collapsible policy, and disabled gating", async t => {
  const { mod, harness } = await mounted(t, "mountAccordion");
  press(harness, "trig-a");
  assert.deepEqual(mod.log.changes, [["a"]]);
  assert.equal(prop(harness, "item-a", "data-state"), "open");
  assert.equal(prop(harness, "item-b", "data-state"), "closed", "single selection replaces");

  mod.accordion.setCollapsibleMode(false);
  press(harness, "trig-a");
  assert.deepEqual(mod.log.changes, [["a"]], "collapsible=false refuses emptying");
  assert.equal(prop(harness, "item-a", "data-state"), "open");
  mod.accordion.setCollapsibleMode(true);
  press(harness, "trig-a");
  assert.deepEqual(mod.log.changes, [["a"], []]);
  assert.equal(prop(harness, "item-a", "data-state"), "closed");

  mod.accordion.setBDisabled(true);
  press(harness, "trig-b");
  assert.deepEqual(mod.log.changes, [["a"], []], "member disabled blocks");
  mod.accordion.setBDisabled(false);
  mod.accordion.setGroupDisabled(true);
  press(harness, "trig-c");
  assert.deepEqual(mod.log.changes, [["a"], []], "group disabled blocks");
  assert.equal(prop(harness, "trig-c", "disabled"), true, "group disabled reaches triggers");
  mod.accordion.setGroupDisabled(false);
  mod.accordion.setCancelClicks(true);
  press(harness, "trig-a");
  assert.deepEqual(mod.log.changes, [["a"], []], "cancelled click blocks");
  mod.accordion.setCancelClicks(false);
  press(harness, "trig-a");
  assert.deepEqual(mod.log.changes, [["a"], [], ["a"]]);
});

test("accordion cancelled keydown does not navigate", async t => {
  const { mod, harness } = await mounted(t, "mountAccordion");
  const acc = node(harness, "acc");
  mod.accordion.setCancelKeys(true);
  acc.callbacks.onKeyDown(key(node(harness, "trig-a"), "ArrowDown"));
  assert.notEqual(harness.focused.current, node(harness, "trig-b"), "cancelled key ignored");
});

test("controlled accordion waits for the echoed value", async t => {
  const { mod, harness } = await mounted(t, "mountAccordionControlled");
  assert.equal(prop(harness, "c-trig-b", "expanded"), true);
  press(harness, "c-trig-a");
  assert.deepEqual(mod.log.changes, [["a"]]);
  assert.equal(prop(harness, "c-trig-a", "expanded"), false, "request does not self-apply");
  assert.equal(prop(harness, "c-trig-b", "expanded"), true);
  mod.accControlled.setAccValue(["a"]);
  assert.equal(prop(harness, "c-trig-a", "expanded"), true);
  assert.equal(prop(harness, "c-trig-b", "expanded"), false);
});

test("accordion multiple unions and removes without mutating input", async t => {
  const { mod, harness } = await mounted(t, "mountAccordionMultiple");
  assert.equal(prop(harness, "m-trig-b", "expanded"), true);
  press(harness, "m-trig-a");
  assert.deepEqual(mod.log.changes, [["a", "b"]], "multiple unions sorted");
  press(harness, "m-trig-b");
  assert.deepEqual(mod.log.changes, [["a", "b"], ["a"]]);
  assert.deepEqual(mod.inputB, ["b"], "caller array untouched");
});

test("nested accordion state and focus stay isolated", async t => {
  const { mod, harness } = await mounted(t, "mountNestedAccordion");
  press(harness, "i-trig-2");
  assert.deepEqual(mod.log.changes, [["inner", "i2"]], "inner change only");
  assert.equal(prop(harness, "o-trig-1", "expanded"), true, "outer item untouched");
  press(harness, "o-trig-2");
  assert.deepEqual(mod.log.changes, [["inner", "i2"], ["outer", "o2"]],
    "outer toggles independently of inner");
  assert.equal(prop(harness, "o-trig-1", "expanded"), false, "single mode replaced outer");
  const outerAcc = node(harness, "outer-acc");
  const bodyDescendant = node(harness, "i-cont-1");
  outerAcc.callbacks.onKeyDown(key(bodyDescendant, "ArrowDown"));
  assert.notEqual(harness.focused.current, node(harness, "o-trig-2"),
    "body keydown does not navigate headings");
  const innerAcc = node(harness, "inner-acc");
  innerAcc.callbacks.onKeyDown(key(node(harness, "i-trig-1"), "ArrowDown"));
  assert.equal(harness.focused.current, node(harness, "i-trig-2"), "inner roving works alone");
});

test("keyed reorder preserves nodes and drives Home/End order", async t => {
  const { mod, harness } = await mounted(t, "mountAccordionReorder");
  const acc = node(harness, "r-acc");
  const trigA = node(harness, "r-trig-a");
  const trigB = node(harness, "r-trig-b");
  const countB = node(harness, "r-count-b");
  press(harness, "r-count-b");
  assert.equal(textOf(countB), "n1");
  focusIn(harness, "r-trig-b");

  mod.reorder.setOrder(["c", "a", "b"]);
  assert.equal(node(harness, "r-trig-a"), trigA, "trigger survives reorder");
  assert.equal(node(harness, "r-trig-b"), trigB);
  assert.equal(node(harness, "r-count-b"), countB, "counter survives reorder");
  assert.equal(textOf(countB), "n1", "counter state survives reorder");
  assert.deepEqual(mod.log.cleans, [], "nothing disposed on reorder");
  acc.callbacks.onKeyDown(key(node(harness, "r-trig-b"), "Home"));
  assert.equal(harness.focused.current, node(harness, "r-trig-c"), "Home follows native order");

  mod.reorder.setOrder(["a", "c"]);
  assert.equal(node(harness, "r-trig-b"), undefined, "removed item detaches");
  assert.equal(prop(harness, "r-trig-c", "tabIndex"), 0, "tab stop recovers to first enabled");
  assert.equal(prop(harness, "r-trig-a", "tabIndex"), -1);

  mod.reorder.setOrder(["a", "b", "c"]);
  const recreatedTrigB = node(harness, "r-trig-b");
  mod.reorder.setShowItemB(false);
  assert.equal(node(harness, "r-trig-b"), undefined);
  mod.reorder.setShowItemB(true);
  const newTrigB = node(harness, "r-trig-b");
  assert.ok(newTrigB);
  assert.notEqual(newTrigB, recreatedTrigB, "re-created item gets fresh handles");
  assert.equal(prop(harness, "r-trig-b", "ariaControls"), node(harness, "r-cont-b"),
    "fresh trigger relates to fresh content");
});

test("heading wrapper, region relation, chevron, and measure classes", async t => {
  const { mod, harness, registrations } = await mounted(t, "mountAccordion");
  const control = node(harness, "trig-a");
  const heading = harness.parents.get(control);
  assert.equal(heading.props.get("role"), "heading");
  assert.equal(heading.props.get("ariaLevel"), 3, "default heading level");
  assert.equal(heading.props.get("class"), "zui-accordion__header");
  mod.accordion.setAccLevel(2);
  assert.equal(heading.props.get("ariaLevel"), 2, "level prop is reactive");

  assert.equal(prop(harness, "trig-a", "role"), "button");
  assert.equal(prop(harness, "trig-a", "expanded"), false);
  assert.equal(prop(harness, "trig-a", "ariaControls"), node(harness, "cont-a"));
  assert.equal(prop(harness, "cont-a", "role"), "region");
  assert.equal(prop(harness, "cont-a", "ariaLabelledBy"), control, "region labelled by control");

  const chevron = control.children.find(child => child.tag === "vector");
  assert.ok(chevron, "trigger carries a chevron");
  assert.equal(chevron.props.get("ariaHidden"), true);
  assert.equal(chevron.props.get("data-state"), "closed");
  assert.ok(chevron.props.get("class").includes("zui-accordion__chevron"));
  press(harness, "trig-a");
  const chevronB = node(harness, "trig-b").children.find(child => child.tag === "vector");
  assert.equal(chevronB.props.get("data-state"), "closed", "b collapsed after a opens");
  assert.equal(chevron.props.get("data-state"), "open");
  mod.accordion.setCaption("A2");
  const after = node(harness, "trig-a").children.find(child => child.tag === "vector");
  assert.equal(after, chevron, "caption update keeps the same chevron node");
  assert.equal(textOf(node(harness, "trig-a")), "A2");

  const measure = node(harness, "cont-a").children[0];
  assert.equal(measure.props.get("class"), "zui-collapsible__measure zui-accordion__measure");
  const { harness: nested } = await mounted(t, "mountNestedAccordion");
  const outerMeasure = nested.byTestId("o-cont-1").children[0];
  const plainMeasure = nested.byTestId("nc-content").children[0];
  assert.equal(outerMeasure.props.get("class"), "zui-collapsible__measure zui-accordion__measure");
  assert.equal(plainMeasure.props.get("class"), "zui-collapsible__measure",
    "nested plain collapsible keeps its own measure class");
  assert.equal(registrations.length, 6, "one borderSize and one scrollPosition per content");
});

test("styled trigger in a nested collapsible stays outside the outer roving group", async t => {
  const { mod, harness } = await mounted(t, "mountStyledNestedTrigger");
  assert.equal(prop(harness, "styled-inner", "tabIndex"), 0);
  press(harness, "styled-inner");
  assert.equal(prop(harness, "styled-body", "ariaHidden"), true);
  assert.equal(prop(harness, "scoped-a", "expanded"), true);
  assert.deepEqual(mod.log.changes, []);
  node(harness, "scoped-acc").callbacks.onKeyDown(key(node(harness, "scoped-a"), "ArrowDown"));
  assert.equal(harness.focused.current, node(harness, "scoped-b"));
  assert.equal(prop(harness, "styled-inner", "tabIndex"), 0);
});

test("invalid selection, item value, and heading level throw", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  harness.host.observeGeometry = () => () => {};
  globalThis.__zgui = harness.host;
  t.after(() => { delete globalThis.__zgui; });
  assert.throws(() => mod.mountInvalid(harness.makeNode("root"), "selection"), TypeError);
  assert.throws(() => mod.mountInvalid(harness.makeNode("root"), "itemValue"), TypeError);
  assert.throws(() => mod.mountInvalid(harness.makeNode("root"), "level"), RangeError);
});
