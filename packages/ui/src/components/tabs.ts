import { Show, createContext, createMemo, createSignal, onCleanup, splitProps, useContext } from "#solid";
import { createElement, insert, mergeProps, spread } from "../renderer/index.js";
import type { Accessor, NativeElementProps, NativeEvent, NativeFocusEvent, NativeKeyEvent, NativeNode } from "../types.js";
import { callHandler, createControllable } from "./state.js";
import { classes, useStylesheet } from "./shared.js";
import { createRovingFocus, groupOrientation } from "./roving.js";
import type { GroupOrientation } from "./roving.js";
import { sheet as upstreamSheet } from "./generated/TabsStyle.js";

const sheet = {
  ...upstreamSheet,
  css: upstreamSheet.css
    .replaceAll('.zui-tabs__list[data-variant="default"] .zui-tabs__trigger', '.zui-tabs__trigger[data-variant="default"]')
    .replaceAll('.zui-tabs__list[data-variant="line"] .zui-tabs__trigger', '.zui-tabs__trigger[data-variant="line"]')
    .replaceAll(`.${upstreamSheet.className}[data-orientation="vertical"] .zui-tabs__list`, '.zui-tabs__list[data-orientation="vertical"]')
    .replaceAll(`.${upstreamSheet.className}[data-orientation="vertical"] .zui-tabs__trigger`, '.zui-tabs__trigger[data-orientation="vertical"]')
    .replaceAll(`.${upstreamSheet.className}[data-orientation="horizontal"] .zui-tabs__trigger`, '.zui-tabs__trigger[data-orientation="horizontal"]'),
};

export type TabsActivation = "automatic" | "manual";
export type TabsListVariant = "default" | "line";
export interface TabsProps extends Omit<NativeElementProps, "orientation"> {
  value?: string;
  defaultValue?: string;
  onValueChange?: (value: string) => void;
  orientation?: GroupOrientation;
  activation?: TabsActivation;
  label?: string | null;
}
export interface TabsListProps extends NativeElementProps {
  variant?: TabsListVariant;
  label?: string | null;
}
export interface TabsTriggerProps extends Omit<NativeElementProps, "selected" | "checked"> {
  value: string;
}
export interface TabsContentProps extends NativeElementProps {
  value: string;
  keepMounted?: boolean;
}

function partValue(value: string): string {
  if (typeof value !== "string" || value.length === 0) throw new TypeError("Tabs part value must be a non-empty string");
  return value;
}
function createParts() {
  const [parts, setParts] = createSignal<readonly { node: NativeNode; value: Accessor<string> }[]>([]);
  return {
    register(node: NativeNode, value: Accessor<string>) {
      const part = { node, value };
      setParts(items => [...items, part]);
      onCleanup(() => setParts(items => items.filter(item => item !== part)));
    },
    get(value: string): NativeNode | null {
      return parts().find(part => part.value() === value)?.node ?? null;
    },
  };
}
interface TabsContextValue {
  value: Accessor<string>;
  select: (value: string) => void;
  disabled: Accessor<boolean>;
  orientation: Accessor<GroupOrientation>;
  activation: Accessor<TabsActivation>;
  triggers: ReturnType<typeof createParts>;
  panels: ReturnType<typeof createParts>;
}
const TabsContext = createContext<TabsContextValue>();
const ListContext = createContext<{
  tabs: TabsContextValue | undefined;
  disabled: Accessor<boolean>;
  roving: ReturnType<typeof createRovingFocus>;
  variant: Accessor<TabsListVariant>;
}>();

export function Tabs(props: TabsProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["value", "defaultValue", "onValueChange", "orientation", "activation", "disabled", "label", "children"]);
  const [read, write] = createControllable<string>({
    value: () => own.value,
    defaultValue: () => own.defaultValue,
    onChange: () => own.onValueChange,
  }, "");
  const value = createMemo(() => {
    const current = read();
    if (typeof current !== "string") throw new TypeError("Tabs value must be a string");
    return current;
  });
  const orientation = createMemo(() => groupOrientation(own.orientation ?? "horizontal"));
  const activation = createMemo(() => {
    const current = own.activation ?? "automatic";
    if (current !== "automatic" && current !== "manual") throw new TypeError("Tabs activation must be automatic or manual");
    return current;
  });
  const disabled = () => !!own.disabled;
  const context: TabsContextValue = {
    value, disabled, orientation, activation,
    select: value => { if (!disabled()) write(value); },
    triggers: createParts(), panels: createParts(),
  };
  const node = createElement("column");
  spread(node, mergeProps({
    role: "group" as const,
    get disabled() { return disabled(); },
    get ariaLabel() { return own.label ?? null; },
    get "data-orientation"() { return orientation(); },
    get "data-activation"() { return activation(); },
  }, rest, {
    get class() { return classes(sheet.className, "zui-tabs", rest.class); },
  }));
  insert(node, TabsContext.Provider({ value: context, get children() { return own.children; } }));
  return node;
}

export function TabsList(props: TabsListProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["variant", "label", "disabled", "children", "onKeyDown"]);
  const tabs = useContext(TabsContext);
  const axis = () => tabs?.orientation() ?? "horizontal";
  const disabled = () => !!own.disabled || !!tabs?.disabled();
  const variant = createMemo(() => {
    const current = own.variant ?? "default";
    if (current !== "default" && current !== "line") throw new TypeError("TabsList variant must be default or line");
    return current;
  });
  const roving = createRovingFocus(axis);
  const node = createElement("box");
  spread(node, mergeProps({
    role: "tabList" as const,
    get disabled() { return disabled(); },
    get ariaLabel() { return own.label ?? null; },
    get orientation() { return axis() === "vertical" ? "vertical" as const : "horizontal" as const; },
    get "data-orientation"() { return axis() === "vertical" ? "vertical" : "horizontal"; },
    get "data-variant"() { return variant(); },
  }, rest, {
    get class() { return classes("zui-tabs__list", `zui-tabs__list--${variant()}`, rest.class); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, own.onKeyDown) && !disabled()) roving.onKeyDown(event);
    },
  }));
  insert(node, ListContext.Provider({ value: { tabs, disabled, roving, variant }, get children() { return own.children; } }));
  return node;
}

export function TabsTrigger(props: TabsTriggerProps): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["value", "disabled", "onClick", "onFocus"]);
  const tabs = useContext(TabsContext);
  const inherited = useContext(ListContext);
  const list = inherited?.tabs === tabs ? inherited : undefined;
  const value = createMemo(() => partValue(own.value));
  const disabled = () => !!own.disabled || !!tabs?.disabled() || !!list?.disabled();
  const selected = () => tabs?.value() === value();
  const node = createElement("control");
  tabs?.triggers.register(node, value);
  const panel = createMemo(() => tabs?.panels.get(value()) ?? null);
  const item = list?.roving.register(node, disabled, selected);
  const select = () => { if (!disabled()) { item?.activate(); tabs?.select(value()); } };
  spread(node, mergeProps({
    role: "tab" as const,
    get tabIndex() { return item?.tabIndex() ?? 0; },
    get disabled() { return disabled(); },
    get selected() { return selected(); },
    get ariaControls() { return panel(); },
    get "data-state"() { return selected() ? "active" : "inactive"; },
    get "data-value"() { return value(); },
    get "data-orientation"() { return tabs?.orientation() === "vertical" ? "vertical" : "horizontal"; },
    get "data-variant"() { return list?.variant() ?? null; },
  }, rest, {
    get class() { return classes("zui-tabs__trigger", rest.class); },
    onClick(event: NativeEvent) { if (callHandler(event, own.onClick)) select(); },
    onFocus(event: NativeFocusEvent) {
      if (!callHandler(event, own.onFocus) || event.target !== node || disabled()) return;
      item?.activate();
      if (tabs?.activation() === "automatic") select();
    },
  }));
  return node;
}

export function TabsContent(props: TabsContentProps): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["value", "keepMounted", "children"]);
  const tabs = useContext(TabsContext);
  const value = createMemo(() => partValue(own.value));
  const selected = () => tabs?.value() === value();
  const node = createElement("box");
  tabs?.panels.register(node, value);
  const trigger = createMemo(() => tabs?.triggers.get(value()) ?? null);
  spread(node, mergeProps({
    role: "tabPanel" as const,
    tabIndex: 0,
    get ariaHidden() { return !selected(); },
    get ariaLabelledBy() { return trigger(); },
    get "data-state"() { return selected() ? "active" : "inactive"; },
    get "data-value"() { return value(); },
  }, rest, {
    get class() { return classes("zui-tabs__content", rest.class); },
  }));
  insert(node, Show({ get when() { return !!own.keepMounted || selected(); }, get children() { return own.children; } }));
  return node;
}
