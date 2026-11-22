import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

const fixture = `
import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeChild, NativeNode } from "@hewel/zgui";
import {
  Avatar, AvatarBadge, AvatarFallback, AvatarImage, Empty, EmptyContent,
  EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle,
  InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput, InputGroupTextarea,
  Item, ItemContent, ItemDescription, ItemMedia, ItemSeparator, ItemTitle,
} from "@hewel/zgui/components";
import type { ItemSize, ItemVariant } from "@hewel/zgui/components";

const [avatarSrc, setAvatarSrc] = createSignal<string | null>("assets/a.png");
const [avatarLabel, setAvatarLabel] = createSignal<string | null>("Ada");
const [itemVariant, setItemVariant] = createSignal<ItemVariant>("default");
const [itemSize, setItemSize] = createSignal<ItemSize>("md");
const [descA, setDescA] = createSignal(true);
const [descB, setDescB] = createSignal(true);
const [showItem, setShowItem] = createSignal(true);
const [groupDisabled, setGroupDisabled] = createSignal(false);
const [fieldDisabled, setFieldDisabled] = createSignal(false);
const [query, setQuery] = createSignal("");

let childRuns = 0;
const log = {
  inputs: [] as string[],
  values: [] as string[],
  buttons: 0,
};
let fieldNode: NativeNode | undefined;

export const api = {
  log,
  childRuns: () => childRuns,
  field: () => fieldNode,
  setAvatarSrc, setAvatarLabel, setItemVariant, setItemSize,
  setDescA, setDescB, setShowItem, setGroupDisabled, setFieldDisabled, setQuery,
};

function App() {
  return <>
    <Avatar testId="avatar" src={avatarSrc()} label={avatarLabel()}
      {...{ get children() { childRuns += 1; return [
        <AvatarFallback testId="avatar-fallback">AL</AvatarFallback>,
        <AvatarBadge testId="avatar-badge" />,
      ]; } }} />
    <Avatar testId="avatar-override" label={null}
      ariaLabel="Decor" ariaHidden={false} role="button" />
    <Item testId="item" variant={itemVariant()} size={itemSize()} class="caller">
      <ItemMedia variant="icon" />
      <ItemContent>
        <ItemTitle>Title</ItemTitle>
        {descA() ? <ItemDescription testId="description-a">A</ItemDescription> : null}
        {descB() ? <ItemDescription testId="description-b">B</ItemDescription> : null}
        <Item testId="nested-item" size="sm">
          <ItemContent><ItemDescription testId="nested-description">N</ItemDescription></ItemContent>
        </Item>
      </ItemContent>
    </Item>
    {showItem() ? <Item testId="remounted" /> : null}
    <InputGroup testId="group" disabled={groupDisabled()}>
      <InputGroupAddon align="inline-end" />
      <InputGroupInput testId="grouped-input" ref={node => { fieldNode = node; }}
        label="Search" disabled={fieldDisabled()}
        value={query()}
        onInput={event => { log.inputs.push(event.value); }}
        onValueChange={value => { log.values.push(value); setQuery(value); }} />
      <InputGroupInput testId="grouped-secure" secure label="Secret" />
      <InputGroupTextarea testId="grouped-area" defaultValue="Notes" readOnly />
      <InputGroupButton testId="grouped-button" onClick={() => { log.buttons += 1; }}>Go</InputGroupButton>
      <InputGroupButton testId="grouped-button-alt" size="sm" variant="outline" />
    </InputGroup>
    <Empty testId="empty" ariaLabel="Empty">
      <EmptyHeader><EmptyMedia variant="icon" /><EmptyTitle>None</EmptyTitle></EmptyHeader>
      <EmptyContent><EmptyDescription>Try again</EmptyDescription></EmptyContent>
    </Empty>
  </>;
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <App />, root);
}

export function mountWithChildren(root: NativeNode, name: string, child: () => NativeChild): () => void {
  const components = {
    AvatarImage, ItemSeparator, InputGroupInput, InputGroupTextarea,
  } as const;
  const component = components[name as keyof typeof components];
  return renderApp(() => component({ get children() { return child() as never; } }), root);
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

test("module imports before the native host exists", async t => {
  const mod = await moduleFor(t, fixture);
  assert.equal(typeof mod.mount, "function");
});

test("avatar keeps its image node across source changes and allocates fresh on restore", async t => {
  const { mod, harness } = await mounted(t);
  const avatar = harness.byTestId("avatar");
  const fallback = harness.byTestId("avatar-fallback");
  const badge = harness.byTestId("avatar-badge");
  assert.equal(avatar.props.get("role"), "image");
  assert.equal(avatar.props.get("ariaLabel"), "Ada");
  assert.equal(avatar.props.get("ariaHidden"), false);
  assert.equal(avatar.props.get("data-size"), "md");
  const image = avatar.children.find(node => node.tag === "image");
  assert.ok(image);
  assert.equal(image.props.get("src"), "assets/a.png");
  assert.equal(image.props.get("role"), "image");
  assert.equal(image.props.get("ariaHidden"), true);
  assert.match(image.props.get("class"), /zui-avatar__image/);
  assert.equal(avatar.children.indexOf(fallback) >= 0, true);
  assert.equal(badge.props.get("role"), "genericContainer");
  assert.equal(badge.props.get("ariaHidden"), true);

  mod.api.setAvatarSrc("assets/b.png");
  harness.flush();
  const updated = avatar.children.find(node => node.tag === "image");
  assert.equal(updated, image);
  assert.equal(updated.props.get("src"), "assets/b.png");
  assert.equal(harness.byTestId("avatar-fallback"), fallback);
  assert.equal(harness.byTestId("avatar-badge"), badge);

  mod.api.setAvatarSrc(null);
  harness.flush();
  assert.equal(
    avatar.children.some(node => node.tag === "image"),
    false,
  );
  assert.equal(harness.byTestId("avatar-fallback"), fallback);
  assert.equal(harness.byTestId("avatar-badge"), badge);

  mod.api.setAvatarSrc("assets/a.png");
  harness.flush();
  const restored = avatar.children.find(node => node.tag === "image");
  assert.ok(restored);
  assert.notEqual(restored, image);
  assert.equal(harness.byTestId("avatar-fallback"), fallback);
  assert.equal(harness.byTestId("avatar-badge"), badge);
  assert.equal(mod.api.childRuns(), 1);

  const override = harness.byTestId("avatar-override");
  assert.equal(override.props.get("role"), "button");
  assert.equal(override.props.get("ariaHidden"), false);
  assert.equal(override.props.get("ariaLabel"), "Decor");

  assert.equal(harness.installs.filter(name => name === "AvatarStyle").length, 1);
});

test("item tracks mounted descriptions, isolates nested context, and preserves reactive class", async t => {
  const { mod, harness } = await mounted(t);
  const item = harness.byTestId("item");
  const nested = harness.byTestId("nested-item");
  assert.match(item.props.get("class"), /\bzui-item--described\b/);
  assert.match(nested.props.get("class"), /\bzui-item--described\b/);
  assert.equal(item.props.get("data-variant"), "default");
  assert.equal(item.props.get("data-size"), "md");
  assert.match(item.props.get("class"), /caller$/);

  mod.api.setDescB(false);
  harness.flush();
  assert.equal(harness.byTestId("description-b"), undefined);
  assert.match(item.props.get("class"), /\bzui-item--described\b/);
  mod.api.setDescA(false);
  harness.flush();
  assert.equal(harness.byTestId("description-a"), undefined);
  assert.doesNotMatch(item.props.get("class"), /\bzui-item--described\b/);
  assert.match(nested.props.get("class"), /\bzui-item--described\b/);
  mod.api.setDescA(true);
  harness.flush();
  assert.match(item.props.get("class"), /\bzui-item--described\b/);

  mod.api.setItemVariant("muted");
  mod.api.setItemSize("sm");
  harness.flush();
  assert.match(item.props.get("class"), /\bzui-item--muted\b/);
  assert.match(item.props.get("class"), /\bzui-item--sm\b/);
  assert.equal(harness.byTestId("item"), item);
  assert.match(item.props.get("class"), /\bzui-item--described\b/);

  const first = harness.byTestId("remounted");
  mod.api.setShowItem(false);
  harness.flush();
  assert.equal(harness.byTestId("remounted"), undefined);
  mod.api.setShowItem(true);
  harness.flush();
  const second = harness.byTestId("remounted");
  assert.ok(second);
  assert.notEqual(second, first);
});

test("input group forwards editing and keeps frame state independent of the field", async t => {
  const { mod, harness } = await mounted(t);
  const group = harness.byTestId("group");
  const input = harness.byTestId("grouped-input");
  const secure = harness.byTestId("grouped-secure");
  const area = harness.byTestId("grouped-area");
  const button = harness.byTestId("grouped-button");
  const buttonAlt = harness.byTestId("grouped-button-alt");
  const addon = group.children.find(node => node.props.get("data-align") === "inline-end");

  assert.equal(group.props.get("role"), "group");
  assert.equal(group.props.get("data-disabled"), "false");
  assert.match(group.props.get("class"), /\bzui-input-group\b/);
  assert.ok(addon);
  assert.match(addon.props.get("class"), /zui-input-group__addon--inline-end/);

  assert.equal(input.tag, "field");
  assert.equal(input.props.get("role"), "textInput");
  assert.match(input.props.get("class"), /zui-input-group__field/);
  assert.equal(mod.api.field(), input);
  assert.equal(secure.props.get("role"), "passwordInput");
  assert.equal(area.tag, "editor");
  assert.equal(area.props.get("readOnly"), true);
  assert.match(area.props.get("class"), /zui-input-group__area/);

  harness.dispatch(input, "Input", { value: "gro" });
  assert.deepEqual(mod.api.log.inputs, ["gro"]);
  assert.deepEqual(mod.api.log.values, ["gro"]);
  assert.equal(input.props.get("value"), "gro");
  mod.api.setQuery("set");
  harness.flush();
  assert.equal(input.props.get("value"), "set");
  assert.equal(harness.byTestId("grouped-input"), input);
  assert.equal(mod.api.log.values.length, 1);

  harness.dispatch(button, "Click");
  assert.equal(mod.api.log.buttons, 1);
  assert.equal(button.props.get("role"), "button");
  assert.match(button.props.get("class"), /zui-input-group__button/);
  assert.equal(buttonAlt.props.get("data-size"), "sm");
  assert.match(buttonAlt.props.get("class"), /\bzui-button--outline\b/);

  mod.api.setGroupDisabled(true);
  harness.flush();
  assert.equal(group.props.get("data-disabled"), "true");
  assert.equal(input.props.get("disabled"), false);
  mod.api.setFieldDisabled(true);
  harness.flush();
  assert.equal(input.props.get("disabled"), true);
});

test("childless composition parts reject lazy children without evaluating them", async t => {
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
  for (const name of ["AvatarImage", "ItemSeparator", "InputGroupInput", "InputGroupTextarea"]) {
    assert.throws(
      () => mod.mountWithChildren(harness.makeNode("root"), name, child),
      TypeError,
      name,
    );
  }
  assert.equal(evaluated, 0);
});
