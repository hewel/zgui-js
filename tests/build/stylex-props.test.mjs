import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

async function mount(t, source, options) {
  const mod = await moduleFor(t, source);
  const h = createHost(options);
  globalThis.__zgui = h.host;
  const dispose = mod.mount(h.makeNode("root"));
  t.after(() => { try { dispose(); } finally { delete globalThis.__zgui; } });
  return { h, api: mod.api };
}

test("aliases switch, ignore empty contributions, and retain application classes on conflict", async t => {
  const { h, api } = await mount(t, `
    import { renderApp as render, createSignal } from '@hewel/zgui';
    import { Button, InputGroupButton, Card, AspectRatio } from '@hewel/zgui/components';
    const [props, set] = createSignal({ class: 'first', className: '  ', 'data-marker': 'one' });
    export const api = { set };
    export function mount(root) { return render(() => <column>
      <box testId="native" {...props()} />
      <Button testId="button" {...props()} />
      <InputGroupButton testId="wrapped" {...props()} />
      <Card testId="card" {...props()} />
      <AspectRatio testId="ratio" {...props()} />
    </column>, root); }
  `);
  const native = h.byTestId("native");
  assert.equal(native.props.get("class"), "first");
  api.set({ className: "next", "data-marker": "two" });
  h.flush();
  assert.equal(native.props.get("class"), "next");
  for (const id of ["button", "wrapped", "card", "ratio"]) {
    assert.match(h.byTestId(id).props.get("class"), /\bnext\b/);
    assert.match(h.byTestId(id).props.get("class"), /\b(?:zui-|zs-)/);
  }
  assert.match(h.byTestId("wrapped").props.get("class"), /zui-input-group__button/);
  assert.match(h.byTestId("wrapped").props.get("class"), /zui-button/);
  api.set({ class: "conflict-a", className: "conflict-b", "data-marker": "three" });
  h.flush();
  assert.equal(native.props.get("class"), "next");
  assert.equal(native.props.get("data-marker"), "three");
  assert.ok(h.styleDiagnostics.some(d => d.code === "CLASS_ALIAS_CONFLICT"));
  api.set({ class: "\t ", className: " " });
  h.flush();
  assert.equal(native.props.has("class"), false);
  assert.match(h.byTestId("button").props.get("class"), /zui-button/);
});

test("StyleX props reactively toggle classes, dynamic zero values, and source metadata", async t => {
  const { h, api } = await mount(t, `
    import { renderApp as render, createSignal } from '@hewel/zgui';
    import * as stylex from '@stylexjs/stylex';
    const styles = stylex.create({
      base: { color: 'red' },
      active: { backgroundColor: 'blue' },
      dynamic: value => ({ opacity: value, width: value }),
    });
    const [active, toggle] = createSignal(true);
    const [value, setValue] = createSignal(0);
    const [source, setSource] = createSignal('first.tsx:1');
    export const api = { toggle, setValue, setSource };
    export function mount(root) { return render(() => <box testId="stylex"
      {...stylex.props(styles.base, active() && styles.active, active() && styles.dynamic(value()))}
      data-style-src={source()} />, root); }
  `);
  const node = h.byTestId("stylex");
  const initial = node.props.get("class");
  assert.match(initial, /\bx/);
  assert.ok(Object.values(node.props.get("style")).includes("0"));
  api.setValue(0.5);
  h.flush();
  assert.ok(Object.values(node.props.get("style")).includes("0.5"));
  const accepted = node.props.get("style");
  api.setValue(NaN);
  h.flush();
  assert.deepEqual(node.props.get("style"), accepted);
  assert.ok(h.styleDiagnostics.some(diagnostic => diagnostic.code === "STYLE_VALUE_INVALID"));
  api.setValue(null);
  h.flush();
  assert.equal(node.props.has("style"), false);
  api.setValue(0.5);
  api.toggle(false);
  h.flush();
  assert.notEqual(node.props.get("class"), initial);
  assert.equal(node.props.has("style"), false);
  api.setSource("second.tsx:4");
  h.flush();
  assert.equal(node.props.get("data-style-src"), "second.tsx:4");
  api.setSource(undefined);
  h.flush();
  assert.equal(node.props.has("data-style-src"), false);
  api.toggle(true);
  h.flush();
  assert.equal(h.byTestId("stylex"), node);
  assert.equal(node.props.get("class"), initial);
});

test("application inline maps replace atomically, normalize names, and restore SDK defaults", async t => {
  const { h, api } = await mount(t, `
    import { renderApp as render, createSignal } from '@hewel/zgui';
    import { AspectRatio, Popper } from '@hewel/zgui/components';
    const [style, setStyle] = createSignal({ backgroundColor: 'red', opacity: 0, '--Accent': 0, '--zui-aspect-ratio': 4 });
    const [marker, setMarker] = createSignal('one');
    export const api = { setStyle, setMarker };
    export function mount(root) { return render(() => <column>
      <AspectRatio testId="owned" ratio={2} style={style()} data-marker={marker()} />
      <Popper testId="popper" anchor={null} className="application" style={style()} />
    </column>, root); }
  `);
  const node = h.byTestId("owned");
  assert.equal(node.props.get("style")["background-color"], "red");
  assert.equal(node.props.get("style").opacity, "0");
  assert.equal(node.props.get("style")["--Accent"], "0");
  assert.equal(node.props.get("--zui-aspect-ratio"), "4");
  const previous = node.props.get("style");
  for (const style of [
    { height: "30px", width: 0 }, { height: "30px", opacity: NaN },
    { height: "30px", "--dynamic": Infinity }, { height: "30px", width: "NaNpx" },
    { height: "30px", width: false }, { height: "30px", width: {} },
  ]) {
    api.setStyle(style);
    api.setMarker("still applies");
    h.flush();
    assert.deepEqual(node.props.get("style"), previous);
    assert.equal(node.props.get("data-marker"), "still applies");
  }
  api.setStyle("width: 40px; --Text: 7; position: relative");
  h.flush();
  assert.equal(node.props.get("style").width, "40px");
  assert.equal(node.props.get("style").opacity, undefined);
  assert.equal(h.byTestId("popper").props.get("style").position, "fixed");
  assert.equal(h.byTestId("popper").props.get("style").visibility, "hidden");
  assert.equal(h.byTestId("popper").props.get("class"), "application");
  api.setStyle({ padding: "1px", paddingLeft: "8px" });
  h.flush();
  api.setStyle({ paddingLeft: "8px" });
  h.flush();
  assert.equal(node.props.get("style").padding, undefined);
  assert.equal(node.props.get("style")["padding-left"], "8px");
  assert.equal(node.props.get("--zui-aspect-ratio"), "2");
  assert.equal(node.props.get("style")["--Text"], undefined);
  api.setStyle(undefined);
  h.flush();
  assert.deepEqual(node.props.get("style"), { "--zui-aspect-ratio": "2" });
  assert.ok(h.styleDiagnostics.length >= 6);
});

test("Preview style failures throw instead of silently retaining a partial map", async t => {
  const { h, api } = await mount(t, `
    import { renderApp as render, createSignal } from '@hewel/zgui';
    const [style, setStyle] = createSignal({ width: '10px' });
    export const api = { setStyle };
    export function mount(root) { return render(() => <box testId="strict" style={style()} />, root); }
  `, { strictStyles: true });
  assert.throws(() => {
    api.setStyle({ height: "20px", width: "NaNpx" });
    h.flush();
  }, /STYLE_VALUE_INVALID/);
  assert.deepEqual(h.byTestId("strict").props.get("style"), { width: "10px" });
});

test("ThemeProvider alone emits reduced-motion SDK duration overrides", async t => {
  const { h } = await mount(t, `
    import { renderApp as render } from '@hewel/zgui';
    import { Button, ThemeProvider } from '@hewel/zgui/components';
    export const api = {};
    export function mount(root) { return render(() => <ThemeProvider scheme="light"
      light={{ tokens: { '--zui-motion-duration-fast': '900ms' } }}>
      <Button testId="motion">Motion</Button>
    </ThemeProvider>, root); }
  `, { reducedMotion: true });
  const theme = [...h.stylesheets.values()].find(css => css.includes(":root"));
  for (const duration of ["none", "fast", "normal", "slow", "slower", "slowest"]) {
    const declarations = [...theme.matchAll(new RegExp(`--zui-motion-duration-${duration}:\\s*([^;]+);`, "g"))];
    assert.equal(declarations.at(-1)[1], "0ms");
  }
  assert.equal(h.byTestId("motion").props.has("style"), false);
});
