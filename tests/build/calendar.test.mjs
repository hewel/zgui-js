import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

const source = `
import { renderApp, createSignal } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Calendar } from "@hewel/zgui/components";

export function validate(field: string, value: string) {
  Calendar({ [field]: value });
}
export function mount(root: NativeNode) {
  const [value, setValue] = createSignal<string | null>(null);
  return renderApp(() => <column>
    <Calendar testId="calendar" value={value()} onValueChange={setValue} defaultMonth="2024-02"
      minDate="2024-02-27" maxDate="2024-03-05" isDateDisabled={day => day === "2024-02-28"}/>
    <text testId="chosen">{value() ?? "none"}</text>
  </column>, root);
}
`;

const day = (h, iso) =>
  [...h.nodes.values()].find(
    node => node.props.get("ariaLabel") === iso && node.props.get("data-outside") !== "true",
  );

test("standalone Calendar validates civil input and cannot select disabled dates", async t => {
  const mod = await moduleFor(t, source);
  const h = createHost();
  globalThis.__zgui = h.host;
  t.after(() => delete globalThis.__zgui);
  assert.equal(globalThis.__zguiServices, undefined);
  for (const field of ["value", "defaultValue", "minDate", "maxDate"])
    assert.throws(() => mod.validate(field, "2026-02-29"), RangeError);
  for (const field of ["month", "defaultMonth"])
    assert.throws(() => mod.validate(field, "2024-13"), RangeError);

  const dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  t.after(() => h.runTurn(dispose));
  assert.throws(() => mod.validate("value", "1900-02-29"), RangeError);
  const leap = day(h, "2024-02-29");
  assert.ok(leap);
  for (const date of ["2024-02-26", "2024-02-28"]) {
    const disabled = day(h, date);
    assert.equal(disabled.props.get("disabled"), true);
    h.dispatch(disabled, "Click");
    h.dispatch(disabled, "KeyDown", { key: "Enter" });
    h.dispatch(disabled, "KeyDown", { key: " " });
  }
  assert.notEqual(leap.props.get("selected"), true);
  h.dispatch(leap, "KeyDown", { key: "Enter", isComposing: true });
  assert.notEqual(leap.props.get("selected"), true);
  h.dispatch(leap, "KeyDown", { key: "Enter" });
  assert.equal(leap.props.get("selected"), true);
  h.dispatch(leap, "KeyDown", { key: "ArrowRight" });
  assert.ok(day(h, "2024-03-01"));
  assert.equal(h.focused.current, day(h, "2024-03-01"));
});

test("standalone DatePicker displays ISO text, selects a leap day and restores trigger focus on close and clear", async t => {
  const mod = await moduleFor(
    t,
    `
    import { renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { DatePicker } from "@hewel/zgui/components";
    export function mount(root: NativeNode) {
      return renderApp(() => <DatePicker clearable defaultValue="2024-02-28" testId="picker"
        triggerTestId="trigger" contentTestId="content" calendarTestId="popup-calendar" />, root);
    }
  `,
  );
  const h = createHost();
  const clock = createClock(t, h);
  globalThis.__zgui = h.host;
  const dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  t.after(() => {
    try {
      h.runTurn(dispose);
    } finally {
      delete globalThis.__zgui;
    }
  });
  const trigger = h.byTestId("trigger");
  const text = node =>
    node.tag === "#text" ? node.props.get("text") : node.children.map(text).join("");
  assert.match(text(trigger), /2024-02-28/);
  h.dispatch(trigger, "Click");
  assert.ok(h.byTestId("popup-calendar"));
  h.dispatch(day(h, "2024-02-29"), "Click");
  assert.match(text(trigger), /2024-02-29/);
  assert.equal(trigger.props.get("expanded"), false);
  assert.equal(h.focused.current, trigger);
  clock.frame();
  clock.advance(0);
  assert.equal(h.byTestId("content"), undefined);
  const clear = [...h.nodes.values()].find(node => node.props.get("ariaLabel") === "Clear date");
  h.dispatch(clear, "Click");
  assert.equal(trigger.props.get("data-empty"), "true");
  assert.equal(h.focused.current, trigger);
});

test("controlled range DatePicker stays open after the first selection and closes on the sorted second selection", async t => {
  const mod = await moduleFor(
    t,
    `
    import { createSignal, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { DatePicker } from "@hewel/zgui/components";
    import type { DateRangeValue } from "@hewel/zgui/components";
    export function mount(root: NativeNode) {
      const [value, setValue] = createSignal<DateRangeValue | null>({ start: "2024-02-02", end: "2024-02-05" });
      const [open, setOpen] = createSignal(true);
      return renderApp(() => <DatePicker mode="range" value={value()} onValueChange={setValue}
        open={open()} onOpenChange={setOpen} triggerTestId="trigger" />, root);
    }
  `,
  );
  const h = createHost();
  const clock = createClock(t, h);
  globalThis.__zgui = h.host;
  const dispose = h.runTurn(() => mod.mount(h.makeNode("root")));
  t.after(() => {
    try {
      h.runTurn(dispose);
    } finally {
      delete globalThis.__zgui;
    }
  });
  const trigger = h.byTestId("trigger");
  const last = day(h, "2024-02-29");
  const first = day(h, "2024-02-25");
  h.dispatch(last, "Click");
  assert.equal(trigger.props.get("expanded"), true);
  h.dispatch(first, "PointerEnter");
  assert.equal(first.props.get("data-range"), "start");
  assert.equal(day(h, "2024-02-27").props.get("data-range"), "middle");
  assert.equal(last.props.get("data-range"), "end");
  h.dispatch(first, "Click");
  assert.equal(trigger.props.get("expanded"), false);
  assert.equal(h.focused.current, trigger);
  const text = node =>
    node.tag === "#text" ? node.props.get("text") : node.children.map(text).join("");
  assert.equal(text(trigger), "2024-02-25 – 2024-02-29");
  clock.frame();
  clock.advance(0);
});
