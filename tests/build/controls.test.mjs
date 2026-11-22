import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeChild, NativeNode } from "@hewel/zgui";
import {
  Alert, AlertDescription, AlertTitle, Checkbox, Input, Label, Progress,
  Skeleton, Spinner, Switch, Textarea, Toggle,
} from "@hewel/zgui/components";
import type { AlertVariant, CheckedState } from "@hewel/zgui/components";

const [checkDefault, setCheckDefault] = createSignal<CheckedState>("mixed");
const [checkDisabled, setCheckDisabled] = createSignal(false);
const [checkCancel, setCheckCancel] = createSignal(false);
const [controlledCheck, setControlledCheck] = createSignal<CheckedState | undefined>(false);
const [acceptCheck, setAcceptCheck] = createSignal(false);
const [switchValue, setSwitchValue] = createSignal(false);
const [switchDisabled, setSwitchDisabled] = createSignal(false);
const [toggleValue, setToggleValue] = createSignal(false);
const [secure, setSecure] = createSignal(false);
const [placeholder, setPlaceholder] = createSignal<string | null>(null);
const [inputValue, setInputValue] = createSignal("");
const [labelCancel, setLabelCancel] = createSignal(false);
const [progressValue, setProgressValue] = createSignal<number | null>(50);
const [progressMax, setProgressMax] = createSignal(100);
const [alertVariant, setAlertVariant] = createSignal<AlertVariant>("default");
const [alertLive, setAlertLive] = createSignal(true);
const [alertIcon, setAlertIcon] = createSignal(true);

let fieldNode: NativeNode | undefined;
let labelNode: NativeNode | undefined;
const log = {
  check: [] as CheckedState[],
  controlled: [] as CheckedState[],
  switch: [] as boolean[],
  toggle: [] as boolean[],
  inputHandlers: [] as string[],
  valueChanges: [] as string[],
  labelClicks: 0,
};

export const api = {
  log,
  nodes: () => ({ field: fieldNode, label: labelNode }),
  setCheckDefault, setCheckDisabled, setCheckCancel,
  setControlledCheck, setAcceptCheck,
  setSwitchValue, setSwitchDisabled, setToggleValue,
  setSecure, setPlaceholder, setInputValue, setLabelCancel,
  setProgressValue, setProgressMax,
  setAlertVariant, setAlertLive, setAlertIcon,
};

function App() {
  return <>
    <Label testId="name-label" ref={node => { labelNode = node; }}
      control={() => fieldNode}
      onClick={event => { log.labelClicks += 1; if (labelCancel()) event.preventDefault(); }}>
      Name
    </Label>
    <Input testId="name-input" ref={node => { fieldNode = node; }}
      label="Name" ariaLabelledBy={labelNode}
      value={inputValue()}
      onInput={event => { log.inputHandlers.push(event.value); }}
      onValueChange={value => { log.valueChanges.push(value); setInputValue(value); }} />
    <Input testId="secure-input" secure={secure()} placeholder={placeholder()} />
    <Textarea testId="readonly" defaultValue="Read only" readOnly />
    <Checkbox testId="check" defaultChecked={checkDefault()} disabled={checkDisabled()}
      onClick={event => { if (checkCancel()) event.preventDefault(); }}
      onCheckedChange={value => { log.check.push(value); }} />
    <Checkbox testId="check-controlled" checked={controlledCheck()}
      onCheckedChange={value => { log.controlled.push(value); if (acceptCheck()) setControlledCheck(value); }} />
    <Switch testId="switch" checked={switchValue()} disabled={switchDisabled()}
      onCheckedChange={value => { log.switch.push(value); setSwitchValue(value); }} />
    <Toggle testId="toggle" pressed={toggleValue()} label="Toggle"
      onPressedChange={value => { log.toggle.push(value); setToggleValue(value); }} />
    <Progress testId="progress" value={progressValue()} max={progressMax()} label="Loading" />
    <Alert testId="alert" variant={alertVariant()} live={alertLive()} icon={alertIcon()}>
      <AlertTitle>Title</AlertTitle>
      <AlertDescription>Body</AlertDescription>
    </Alert>
    <Spinner testId="spinner" />
    <Skeleton testId="skeleton" />
  </>;
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <App />, root);
}

export function mountWithChildren(root: NativeNode, name: string, child: () => NativeChild): () => void {
  const components = {
    Input, Textarea, Checkbox, Switch, Skeleton, Spinner, Progress,
  } as const;
  const component = components[name as keyof typeof components];
  return renderApp(() => component({ get children() { return child() as never; } }), root);
}

export function mountProgress(root: NativeNode, value: number | null, max: number): () => void {
  return renderApp(() => Progress({ value, max }), root);
}

let iconlessSet: (value: boolean) => void = () => {};
export function mountIconless(root: NativeNode): () => void {
  return renderApp(() => {
    const [show, setShow] = createSignal(false);
    iconlessSet = setShow;
    return <Alert testId="iconless" icon={show()}><AlertTitle>Hidden</AlertTitle></Alert>;
  }, root);
}
export function setIconlessIcon(value: boolean): void {
  iconlessSet(value);
}
`;

async function mounted(t) {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod.mount(root);
  t.after(() => {
    try {
      dispose();
    } finally {
      delete globalThis.__zgui;
    }
  });
  return { mod, harness };
}

test("checkbox toggles through mixed, true, false with one callback per click", async t => {
  const { mod, harness } = await mounted(t);
  const check = harness.byTestId("check");
  assert.ok(check);
  assert.equal(check.tag, "control");
  assert.equal(check.props.get("role"), "checkBox");
  assert.equal(check.props.get("checked"), "mixed");
  assert.equal(check.props.get("data-state"), "indeterminate");
  assert.equal(check.children.length, 2);
  assert.match(check.children[0].props.get("class"), /zui-checkbox__tick/);
  assert.match(check.children[1].props.get("class"), /zui-checkbox__dash/);

  harness.dispatch(check, "Click");
  assert.equal(check.props.get("checked"), true);
  assert.equal(check.props.get("data-state"), "checked");
  assert.deepEqual(mod.api.log.check, [true]);

  harness.dispatch(check, "Click");
  assert.equal(check.props.get("checked"), false);
  assert.equal(check.props.get("data-state"), "unchecked");
  assert.deepEqual(mod.api.log.check, [true, false]);

  mod.api.setCheckDefault(false);
  harness.flush();
  assert.equal(check.props.get("checked"), false);
  harness.dispatch(check, "Click");
  assert.equal(check.props.get("checked"), true);
  mod.api.setCheckDefault("mixed");
  harness.flush();
  assert.equal(check.props.get("checked"), true);
});

test("controlled checkbox proposes state and a disabled or cancelled click does nothing", async t => {
  const { mod, harness } = await mounted(t);
  const controlled = harness.byTestId("check-controlled");
  assert.ok(controlled);

  harness.dispatch(controlled, "Click");
  assert.deepEqual(mod.api.log.controlled, [true]);
  assert.equal(controlled.props.get("checked"), false);
  assert.equal(controlled.props.get("data-state"), "unchecked");

  mod.api.setAcceptCheck(true);
  harness.flush();
  harness.dispatch(controlled, "Click");
  assert.deepEqual(mod.api.log.controlled, [true, true]);
  assert.equal(controlled.props.get("checked"), true);

  mod.api.setControlledCheck("mixed");
  harness.flush();
  assert.equal(controlled.props.get("checked"), "mixed");
  assert.equal(mod.api.log.controlled.length, 2);

  mod.api.setControlledCheck(undefined);
  harness.flush();
  harness.dispatch(controlled, "Click");
  assert.equal(controlled.props.get("checked"), true);
  assert.equal(mod.api.log.controlled.length, 3);

  mod.api.setCheckDisabled(true);
  harness.flush();
  const check = harness.byTestId("check");
  const before = mod.api.log.check.length;
  harness.dispatch(check, "Click");
  assert.equal(mod.api.log.check.length, before);

  mod.api.setCheckDisabled(false);
  mod.api.setCheckCancel(true);
  harness.flush();
  harness.dispatch(check, "Click");
  assert.equal(mod.api.log.check.length, before);
  mod.api.setCheckCancel(false);
  harness.flush();
  harness.dispatch(check, "Click");
  assert.equal(mod.api.log.check.length, before + 1);
});

test("switch and toggle share the controllable primitive and press requests activation once", async t => {
  const { mod, harness } = await mounted(t);
  const sw = harness.byTestId("switch");
  const toggle = harness.byTestId("toggle");
  assert.equal(sw.props.get("role"), "switch");
  assert.equal(sw.props.get("data-state"), "unchecked");
  assert.match(sw.children[0].props.get("class"), /zui-switch__thumb/);
  assert.equal(toggle.props.get("role"), "button");
  assert.equal(toggle.props.get("data-state"), "off");
  assert.equal(toggle.props.get("ariaLabel"), "Toggle");

  harness.dispatch(sw, "Click");
  assert.deepEqual(mod.api.log.switch, [true]);
  assert.equal(sw.props.get("checked"), true);
  assert.equal(sw.props.get("data-state"), "checked");

  harness.dispatch(toggle, "Click");
  assert.deepEqual(mod.api.log.toggle, [true]);
  assert.equal(toggle.props.get("checked"), true);
  assert.equal(toggle.props.get("data-state"), "on");

  const activations = { count: 0 };
  const check = harness.byTestId("check");
  harness.dispatch(check, "PointerDown", {
    button: "primary",
    activateNow: () => {
      activations.count += 1;
    },
  });
  assert.equal(activations.count, 1);
  harness.dispatch(check, "PointerDown", {
    button: "secondary",
    activateNow: () => {
      activations.count += 1;
    },
  });
  assert.equal(activations.count, 1);
  mod.api.setSwitchDisabled(true);
  harness.flush();
  harness.dispatch(sw, "PointerDown", {
    button: "primary",
    activateNow: () => {
      activations.count += 1;
    },
  });
  assert.equal(activations.count, 1);
  mod.api.setSwitchDisabled(false);
  harness.flush();

  assert.equal(toggle.props.get("data-state"), "on");
  assert.equal(typeof toggle.callbacks.onPointerDown, "undefined");
});

test("input and textarea use native editing, compose input handlers, and forward secure state", async t => {
  const { mod, harness } = await mounted(t);
  const input = harness.byTestId("name-input");
  const secure = harness.byTestId("secure-input");
  const readonly = harness.byTestId("readonly");
  assert.equal(input.tag, "field");
  assert.equal(readonly.tag, "editor");
  assert.equal(input.props.get("role"), "textInput");
  assert.equal(input.props.get("ariaLabel"), "Name");
  assert.equal(readonly.props.get("role"), "multilineTextInput");
  assert.equal(readonly.props.get("readOnly"), true);
  assert.equal(readonly.props.get("defaultValue"), "Read only");
  assert.match(input.props.get("class"), /\bzui-field\b/);
  assert.match(readonly.props.get("class"), /\bzui-field\b.*\bzui-textarea\b/);

  harness.dispatch(input, "Input", { value: "é🙂" });
  assert.deepEqual(mod.api.log.inputHandlers, ["é🙂"]);
  assert.deepEqual(mod.api.log.valueChanges, ["é🙂"]);
  assert.equal(input.props.get("value"), "é🙂");

  mod.api.setInputValue("programmatic");
  harness.flush();
  assert.equal(input.props.get("value"), "programmatic");
  assert.equal(mod.api.log.valueChanges.length, 1);

  assert.equal(secure.props.get("role"), "textInput");
  assert.equal(secure.props.get("secure"), false);
  mod.api.setSecure(true);
  harness.flush();
  assert.equal(secure.props.get("role"), "passwordInput");
  assert.equal(secure.props.get("secure"), true);

  mod.api.setPlaceholder('say "hi"\nbye');
  harness.flush();
  assert.equal(secure.props.get("placeholder"), 'say "hi"\nbye');
  assert.equal(secure.props.get("--zui-field-placeholder"), '"say \\"hi\\"\\A bye"');
  mod.api.setPlaceholder(null);
  harness.flush();
  assert.equal(secure.props.has("--zui-field-placeholder"), false);
  assert.equal(secure.props.has("placeholder"), false);
});

test("label resolves its control accessor at click time and forwards relations", async t => {
  const { mod, harness } = await mounted(t);
  const label = harness.byTestId("name-label");
  const input = harness.byTestId("name-input");
  assert.ok(label);
  assert.equal(label.tag, "label");
  assert.equal(label.props.get("role"), "label");
  assert.equal(input.props.get("ariaLabelledBy"), mod.api.nodes().label);

  harness.dispatch(label, "Click");
  assert.equal(harness.focused.current, input);
  assert.equal(mod.api.log.labelClicks, 1);

  harness.host.focus(label);
  mod.api.setLabelCancel(true);
  harness.flush();
  harness.dispatch(label, "Click");
  assert.equal(harness.focused.current, label);
  assert.equal(mod.api.log.labelClicks, 2);
});

test("progress clamps and reports determinate and indeterminate states", async t => {
  const { mod, harness } = await mounted(t);
  const progress = harness.byTestId("progress");
  assert.ok(progress);
  assert.equal(progress.props.get("role"), "progressIndicator");
  assert.equal(progress.props.get("ariaValueMin"), 0);
  assert.equal(progress.props.get("ariaValueMax"), 100);
  assert.equal(progress.props.get("ariaValueNow"), 50);
  assert.equal(progress.props.get("ariaLabel"), "Loading");
  assert.equal(progress.props.get("data-state"), "determinate");
  assert.equal(progress.props.get("--zui-progress-fraction"), "50.0000%");
  assert.match(progress.children[0].props.get("class"), /zui-progress__fill/);

  mod.api.setProgressValue(-5);
  harness.flush();
  assert.equal(progress.props.get("ariaValueNow"), 0);
  assert.equal(progress.props.get("--zui-progress-fraction"), "0.0000%");
  mod.api.setProgressValue(150);
  harness.flush();
  assert.equal(progress.props.get("ariaValueNow"), 100);
  assert.equal(progress.props.get("--zui-progress-fraction"), "100.0000%");
  mod.api.setProgressValue(null);
  harness.flush();
  assert.equal(progress.props.has("ariaValueNow"), false);
  assert.equal(progress.props.has("--zui-progress-fraction"), false);
  assert.equal(progress.props.get("data-state"), "indeterminate");
  assert.equal(harness.byTestId("progress"), progress);

  for (const [value, max] of [
    [10, 0],
    [10, -2],
    [Number.NaN, 100],
    [Number.POSITIVE_INFINITY, 100],
  ]) {
    assert.throws(() => mod.mountProgress(harness.makeNode("root"), value, max), TypeError);
  }
});

test("alert, spinner, and skeleton carry busy and live semantics", async t => {
  const { mod, harness } = await mounted(t);
  const alert = harness.byTestId("alert");
  const spinner = harness.byTestId("spinner");
  const skeleton = harness.byTestId("skeleton");
  assert.equal(alert.props.get("role"), "alert");
  assert.equal(alert.props.get("ariaLive"), "polite");
  assert.equal(alert.props.get("data-variant"), "default");
  assert.equal(alert.props.get("data-icon"), "true");
  assert.match(alert.props.get("class"), /\bzui-alert--default\b/);
  assert.match(alert.children[0].props.get("class"), /zui-alert__icon/);
  assert.equal(alert.children[0].tag, "vector");
  assert.equal(alert.children[0].props.get("data-icon"), "info");
  const mark = alert.children[0];
  const title = alert.children[1];
  const body = alert.children[2];

  assert.equal(spinner.props.get("role"), "status");
  assert.equal(spinner.props.get("ariaBusy"), true);
  assert.equal(spinner.props.get("ariaLabel"), "Loading");
  assert.match(spinner.children[0].props.get("class"), /zui-spinner__mark/);
  assert.equal(skeleton.props.get("role"), "genericContainer");
  assert.equal(skeleton.props.get("ariaBusy"), true);

  mod.api.setAlertVariant("destructive");
  harness.flush();
  assert.equal(alert.children[0].props.get("data-icon"), "alert-triangle");
  assert.match(alert.props.get("class"), /\bzui-alert--destructive\b/);
  mod.api.setAlertLive(false);
  harness.flush();
  assert.equal(alert.props.get("role"), "group");
  assert.equal(alert.props.has("ariaLive"), false);
  mod.api.setAlertVariant("default");

  mod.api.setAlertIcon(false);
  harness.flush();
  assert.equal(alert.children[0], title);
  assert.equal(alert.children[1], body);
  assert.equal(alert.children.includes(mark), false);
  assert.equal(alert.props.get("data-icon"), "false");

  mod.api.setAlertVariant("destructive");
  mod.api.setAlertIcon(true);
  harness.flush();
  const revealed = alert.children[0];
  assert.notEqual(revealed, mark);
  assert.equal(revealed.tag, "vector");
  assert.equal(revealed.props.get("data-icon"), "alert-triangle");
  assert.equal(alert.children[1], title);
  assert.equal(alert.children[2], body);

  const disposeIconless = mod.mountIconless(harness.makeNode("root"));
  const iconless = harness.byTestId("iconless");
  assert.notEqual(iconless.children[0].tag, "vector");
  mod.setIconlessIcon(true);
  harness.flush();
  assert.equal(iconless.children[0].tag, "vector");
  disposeIconless();
});

test("childless controls reject lazy children without evaluating them", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  t.after(() => {
    delete globalThis.__zgui;
  });

  let evaluated = 0;
  const child = () => {
    evaluated += 1;
    return null;
  };
  for (const name of [
    "Input",
    "Textarea",
    "Checkbox",
    "Switch",
    "Skeleton",
    "Spinner",
    "Progress",
  ]) {
    assert.throws(
      () => mod.mountWithChildren(harness.makeNode("root"), name, child),
      TypeError,
      name,
    );
  }
  assert.equal(evaluated, 0);
});
