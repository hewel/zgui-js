import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { createSignal, flush, renderApp } from "@hewel/zgui";
import type { NativeChild, NativeNode, NativeRole } from "@hewel/zgui";
import {
  AspectRatio, Badge, Button, ButtonGroupSeparator, Card, CardAction, CardContent, CardHeader,
  CardTitle, Separator, ThemeProvider,
} from "@hewel/zgui/components";
import type { ColorScheme, Theme } from "@hewel/zgui/components";
import { CHECK, INFO, Icon } from "@hewel/zgui/icons";
import type { IconData } from "@hewel/zgui/icons";

const [variant, setVariant] = createSignal<"default" | "secondary" | "ghost">("default");
const [size, setSize] = createSignal<"xs" | "md" | "lg">("md");
const [callerClass, setCallerClass] = createSignal("extra");
const [marker, setMarker] = createSignal("one");
const [badgeVariant, setBadgeVariant] = createSignal<"default" | "success">("default");
const [scheme, setScheme] = createSignal<ColorScheme>("light");
const [nestedTheme, setNestedTheme] = createSignal<Theme>({ preset: "ocean" });
const [nested, setNested] = createSignal(false);
const [sibling, setSibling] = createSignal(false);
const [actionA, setActionA] = createSignal(true);
const [actionB, setActionB] = createSignal(true);
const [icon, setIcon] = createSignal<IconData>(CHECK);
const [label, setLabel] = createSignal<string | null>(null);
const [buttonRole, setButtonRole] = createSignal<NativeRole | undefined>("button");
const [ratio, setRatio] = createSignal(2);
const [ratioOverride, setRatioOverride] = createSignal<string | undefined>("3");

let clicks = 0;

export const api = {
  clicks: () => clicks,
  icons: { CHECK, INFO },
  flush,
  setVariant, setSize, setCallerClass, setMarker, setBadgeVariant, setScheme,
  setNestedTheme, setNested, setSibling, setActionA, setActionB, setIcon, setLabel,
  setButtonRole, setRatio, setRatioOverride,
};

function App() {
  return <>
    <ThemeProvider scheme={scheme()}>
      <Button testId="button" variant={variant()} size={size()} class={callerClass()}
        data-marker={marker()} onClick={() => { clicks += 1; }}>Push</Button>
      <Button testId="button-copy">Copy</Button>
      <Badge testId="badge" variant={badgeVariant()}>Badge</Badge>
      {nested() && <ThemeProvider light={nestedTheme()} dark={nestedTheme()}>
        <Badge testId="nested-badge">Nested</Badge>
      </ThemeProvider>}
      <Card>
        <CardHeader testId="header">
          <CardTitle>Title</CardTitle>
          {actionA() && <CardAction testId="action-a"><text>A</text></CardAction>}
          {actionB() && <CardAction testId="action-b"><text>B</text></CardAction>}
        </CardHeader>
        <CardContent />
      </Card>
      <Icon testId="icon" icon={icon()} label={label()} />
      <Separator testId="separator" orientation="vertical" decorative={false} />
      <Button testId="role-button" role={buttonRole()}>Role</Button>
      <AspectRatio testId="ratio" ratio={ratio()}
        {...{ get "--zui-aspect-ratio"() { return ratioOverride(); } }} />
    </ThemeProvider>
    {sibling() && <ThemeProvider scheme="dark"><Badge testId="sibling-badge">Sibling</Badge></ThemeProvider>}
  </>;
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <App />, root);
}

export function mountIconWithChildren(root: NativeNode, child: () => NativeChild): () => void {
  return renderApp(() => Icon({ icon: CHECK, get children() { return child() as never; } }), root);
}

export function mountSeparatorWithChildren(root: NativeNode, child: () => NativeChild): () => void {
  return renderApp(() => Separator({ get children() { return child() as never; } }), root);
}

export function mountGroupSeparatorWithChildren(root: NativeNode, child: () => NativeChild): () => void {
  return renderApp(() => ButtonGroupSeparator({ get children() { return child() as never; } }), root);
}

export function mountAspectRatio(root: NativeNode, ratio: number): () => void {
  return renderApp(() => AspectRatio({ ratio }), root);
}
`;

test("component modules do not read __zgui before the host is installed", async t => {
  let touched = false;
  Object.defineProperty(globalThis, "__zgui", {
    configurable: true,
    get() { touched = true; return undefined; },
  });
  try {
    await moduleFor(t, fixture);
    assert.equal(touched, false);
  } finally {
    delete globalThis.__zgui;
  }
});

test("reactive props keep native identity and shared sheets deduplicate", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod.mount(root);
  t.after(() => { try { dispose(); } finally { delete globalThis.__zgui; } });

  const button = harness.byTestId("button");
  assert.ok(button);
  assert.equal(button.props.get("role"), "button");
  assert.equal(button.props.get("data-variant"), "default");
  assert.equal(button.props.get("data-size"), "md");
  assert.match(button.props.get("class"), /\bzui-button\b/);
  assert.match(button.props.get("class"), /\bzui-button--default\b/);
  assert.match(button.props.get("class"), /\bextra\b/);
  assert.equal(button.props.get("data-marker"), "one");
  assert.equal(button.props.has("variant"), false);
  assert.equal(button.props.has("size"), false);

  mod.api.setVariant("secondary");
  mod.api.setSize("lg");
  mod.api.setCallerClass("later");
  mod.api.setMarker("two");
  harness.flush();
  assert.equal(harness.byTestId("button"), button);
  assert.equal(button.props.get("data-variant"), "secondary");
  assert.equal(button.props.get("data-size"), "lg");
  assert.match(button.props.get("class"), /\bzui-button--secondary\b/);
  assert.match(button.props.get("class"), /\bzui-button--lg\b/);
  assert.match(button.props.get("class"), /\blater\b/);
  assert.equal(button.props.get("data-marker"), "two");

  const badge = harness.byTestId("badge");
  mod.api.setBadgeVariant("success");
  harness.flush();
  assert.equal(badge.props.get("data-variant"), "success");
  assert.match(badge.props.get("class"), /\bzui-badge--success\b/);

  assert.equal(typeof button.callbacks.onClick, "function");
  harness.dispatch(button, "Click");
  assert.equal(mod.api.clicks(), 1);

  const roleButton = harness.byTestId("role-button");
  assert.ok(roleButton);
  assert.equal(roleButton.props.get("role"), "button");
  mod.api.setButtonRole("tab");
  harness.flush();
  assert.equal(harness.byTestId("role-button"), roleButton);
  assert.equal(roleButton.props.get("role"), "tab");
  mod.api.setButtonRole(undefined);
  harness.flush();
  assert.equal(roleButton.props.get("role"), "button");

  const buttonSheets = harness.installs.filter(name => name === "ButtonStyle");
  assert.ok(harness.byTestId("button-copy"));
  assert.equal(buttonSheets.length, 1);
});

test("aspect ratio caller override wins and invalid ratios reject at mount", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod.mount(root);
  t.after(() => { try { dispose(); } finally { delete globalThis.__zgui; } });

  const ratioNode = harness.byTestId("ratio");
  assert.ok(ratioNode);
  assert.equal(ratioNode.props.get("--zui-aspect-ratio"), "3");
  mod.api.setRatioOverride("4");
  harness.flush();
  assert.equal(harness.byTestId("ratio"), ratioNode);
  assert.equal(ratioNode.props.get("--zui-aspect-ratio"), "4");
  mod.api.setRatioOverride(undefined);
  harness.flush();
  assert.equal(ratioNode.props.get("--zui-aspect-ratio"), "2");

  for (const bad of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.throws(() => mod.mountAspectRatio(harness.makeNode("root"), bad), TypeError);
  }
});

function rootSheet(harness) {
  return [...harness.stylesheets.entries()].find(([_name, css]) => css.includes(":root"));
}

test("theme provider replaces one sheet per provider and scopes nested tokens", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod.mount(root);
  t.after(() => { try { dispose(); } finally { delete globalThis.__zgui; } });

  const initial = rootSheet(harness);
  assert.ok(initial, "expected a :root theme sheet");
  const [key, lightCss] = initial;
  assert.match(key, /^zgui-ui-theme-/);
  assert.match(lightCss, /color-scheme: light/);
  assert.doesNotMatch(lightCss, /@media/);

  mod.api.setScheme("dark");
  harness.flush();
  const dark = rootSheet(harness);
  assert.equal(dark[0], key);
  assert.match(dark[1], /color-scheme: dark/);
  assert.doesNotMatch(dark[1], /@media/);

  mod.api.setScheme("system");
  harness.flush();
  const system = rootSheet(harness);
  assert.equal(system[0], key);
  assert.match(system[1], /@media \(prefers-color-scheme: dark\)/);
  assert.match(system[1], /color-scheme: light/);
  assert.match(system[1], /color-scheme: dark/);

  const wrapper = [...harness.nodes.values()].find(node => node.props.get("class")?.includes("zui-theme-"));
  assert.ok(wrapper);
  assert.match(JSON.stringify(wrapper.props.get("style")), /contents/);
});

test("nested theme override, invalid override, and sibling provider lifetimes", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod.mount(root);
  t.after(() => { try { dispose(); } finally { delete globalThis.__zgui; } });

  const [rootKey, rootCss] = rootSheet(harness);

  mod.api.setNested(true);
  harness.flush();
  const nestedEntries = [...harness.stylesheets.entries()].filter(([name, css]) =>
    name !== rootKey && css.includes(".zui-theme-"));
  assert.equal(nestedEntries.length, 1);
  const [[nestedKey, nestedCss]] = nestedEntries;
  assert.match(nestedKey, /^zgui-ui-theme-/);
  assert.notEqual(nestedKey, rootKey);
  assert.match(nestedCss, /\.zui-theme-/);
  assert.doesNotMatch(nestedCss, /:root \{/);
  assert.equal(harness.stylesheets.get(rootKey), rootCss);

  mod.api.setNestedTheme({ preset: "ocean", tokens: { "--zui-color-primary": "#102030" } });
  harness.flush();
  const overridden = harness.stylesheets.get(nestedKey);
  assert.match(overridden, /--zui-color-primary: #102030/);
  assert.equal(harness.stylesheets.get(rootKey), rootCss);

  assert.throws(
    () => {
      mod.api.setNestedTheme({ preset: "ocean", tokens: { "--zui-color-primary": "#fff;behavior" } });
      mod.api.flush();
      harness.flush();
    },
    TypeError,
  );
  assert.equal(harness.stylesheets.get(nestedKey), overridden);
  assert.throws(
    () => {
      mod.api.setNestedTheme({ tokens: { "--zui-bogus": "red" } });
      mod.api.flush();
      harness.flush();
    },
    TypeError,
  );
  assert.equal(harness.stylesheets.get(nestedKey), overridden);
  assert.throws(() => {
    mod.api.setNestedTheme({ preset: "neptune" });
    mod.api.flush();
    harness.flush();
  }, TypeError);
  assert.equal(harness.stylesheets.get(nestedKey), overridden);
  for (const preset of ["toString", "constructor", "__proto__"]) {
    assert.throws(() => {
      mod.api.setNestedTheme({ preset });
      mod.api.flush();
      harness.flush();
    }, TypeError);
    assert.equal(harness.stylesheets.get(nestedKey), overridden);
  }
  mod.api.setNestedTheme({ preset: "ember" });
  harness.flush();
  assert.notEqual(harness.stylesheets.get(nestedKey), overridden);
  mod.api.setNestedTheme({ preset: "ocean", tokens: { "--zui-color-primary": "#102030" } });
  harness.flush();
  assert.equal(harness.stylesheets.get(nestedKey), overridden);

  const firstNestedBadge = harness.byTestId("nested-badge");
  assert.ok(firstNestedBadge);
  mod.api.setNested(false);
  harness.flush();
  assert.equal(harness.stylesheets.has(nestedKey), false);
  assert.equal(harness.stylesheets.get(rootKey), rootCss);
  assert.equal(harness.byTestId("nested-badge"), undefined);
  mod.api.setNested(true);
  harness.flush();
  const recreatedNestedBadge = harness.byTestId("nested-badge");
  assert.ok(recreatedNestedBadge);
  assert.notEqual(recreatedNestedBadge, firstNestedBadge);
  assert.equal(harness.stylesheets.get(rootKey), rootCss);

  mod.api.setSibling(true);
  harness.flush();
  const siblingEntries = [...harness.stylesheets.entries()].filter(([_name, css]) => css.includes(":root"));
  assert.equal(siblingEntries.length, 2);
  const names = new Set(siblingEntries.map(([name]) => name));
  assert.equal(names.size, 2);
  mod.api.setSibling(false);
  harness.flush();
  assert.equal([...harness.stylesheets.values()].filter(css => css.includes(":root")).length, 1);
  assert.ok(harness.stylesheets.get(rootKey));
});

test("card header keeps the action layout until the last action unmounts", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod.mount(root);
  t.after(() => { try { dispose(); } finally { delete globalThis.__zgui; } });

  const header = harness.byTestId("header");
  assert.match(header.props.get("class"), /zui-card__header--action/);
  const firstActionA = harness.byTestId("action-a");

  mod.api.setActionB(false);
  harness.flush();
  assert.equal(harness.byTestId("action-b"), undefined);
  assert.match(header.props.get("class"), /zui-card__header--action/);

  mod.api.setActionA(false);
  harness.flush();
  assert.doesNotMatch(header.props.get("class"), /zui-card__header--action/);

  mod.api.setActionA(true);
  harness.flush();
  const recreated = harness.byTestId("action-a");
  assert.ok(recreated);
  assert.notEqual(recreated, firstActionA);
  assert.match(header.props.get("class"), /zui-card__header--action/);
});

test("icon stays one node across data and label changes and rejects children lazily", async t => {
  const mod = await moduleFor(t, fixture);
  const harness = createHost();
  globalThis.__zgui = harness.host;
  const root = harness.makeNode("root");
  const dispose = mod.mount(root);
  t.after(() => { try { dispose(); } finally { delete globalThis.__zgui; } });

  const icon = harness.byTestId("icon");
  assert.ok(icon);
  assert.equal(icon.tag, "vector");
  assert.equal(icon.props.get("data-icon"), "check");
  assert.equal(icon.props.get("d"), mod.api.icons.CHECK.d);
  assert.equal(icon.props.get("viewBox"), "0 0 24 24");
  assert.equal(icon.props.get("role"), "image");
  assert.equal(icon.props.get("ariaHidden"), true);

  mod.api.setIcon(mod.api.icons.INFO);
  harness.flush();
  assert.equal(harness.byTestId("icon"), icon);
  assert.equal(icon.props.get("data-icon"), "info");
  assert.equal(icon.props.get("d"), mod.api.icons.INFO.d);

  mod.api.setLabel("More information");
  harness.flush();
  assert.equal(icon.props.get("ariaHidden"), undefined);
  assert.equal(icon.props.get("ariaLabel"), "More information");
  mod.api.setLabel(null);
  harness.flush();
  assert.equal(icon.props.get("ariaHidden"), true);

  let evaluated = 0;
  const child = () => { evaluated += 1; return null; };
  const orphan = harness.makeNode("orphan");
  assert.throws(() => mod.mountIconWithChildren(orphan, child), TypeError);
  assert.equal(evaluated, 0);
  assert.throws(() => mod.mountSeparatorWithChildren(orphan, child), TypeError);
  assert.throws(() => mod.mountGroupSeparatorWithChildren(orphan, child), TypeError);
  assert.equal(evaluated, 0);
});
