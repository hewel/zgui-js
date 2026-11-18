import {
  createContext,
  createMemo,
  createSignal,
  onCleanup,
  omit,
  untrack,
  useContext,
} from "#solid";
import { createElement, effect, mergeProps, ref as applyRefProp, setProp, spread } from "../renderer/index.js";
import type { Accessor, NativeChild, NativeElementProps, NativeEvent, NativeFocusEvent, NativeKeyEvent, NativeNode } from "../types.js";
import { callHandler, createControllable } from "./state.js";
import { classes, isStaticProps, provide, slot, useStylesheet, withDefaults } from "./shared.js";
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
  const items: { node: NativeNode; value: Accessor<string> }[] = [];
  const [version, setVersion] = createSignal(0);
  return {
    register(node: NativeNode, value: Accessor<string>) {
      const part = { node, value };
      items.push(part);
      setVersion(v => v + 1);
      onCleanup(() => {
        const index = items.indexOf(part);
        if (index >= 0) {
          items.splice(index, 1);
          setVersion(v => v + 1);
        }
      });
    },
    get(value: string): NativeNode | null {
      version();
      return items.find(part => part.value() === value)?.node ?? null;
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
const TabsContext = createContext<TabsContextValue | null>(null);
const ListContext = createContext<{
  tabs: TabsContextValue | null;
  disabled: Accessor<boolean>;
  roving: ReturnType<typeof createRovingFocus>;
  variant: Accessor<TabsListVariant>;
} | null>(null);

export function Tabs(props: TabsProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "value", "defaultValue", "onValueChange", "orientation", "activation", "disabled", "label", "children");
  const [read, write] = createControllable<string>({
    value: () => props.value,
    defaultValue: () => props.defaultValue,
    onChange: () => props.onValueChange,
  }, "");
  const value = createMemo(() => {
    const current = read();
    if (typeof current !== "string") throw new TypeError("Tabs value must be a string");
    return current;
  });
  const orientation = createMemo(() => groupOrientation(props.orientation ?? "horizontal"));
  const activation = createMemo(() => {
    const current = props.activation ?? "automatic";
    if (current !== "automatic" && current !== "manual") throw new TypeError("Tabs activation must be automatic or manual");
    return current;
  });
  const disabled = () => !!props.disabled;
  const context: TabsContextValue = {
    value, disabled, orientation, activation,
    select: value => { if (!disabled()) write(value); },
    triggers: createParts(), panels: createParts(),
  };
  const node = createElement("column");
  spread(node, mergeProps(rest, withDefaults({
    role: "group" as const,
    get disabled() { return disabled(); },
    get ariaLabel() { return props.label ?? null; },
    get "data-orientation"() { return orientation(); },
    get "data-activation"() { return activation(); },
  }, rest), {
    get __zguiClass() { return classes(sheet.className, "zui-tabs"); },
  }));
  slot(node, provide(TabsContext, context, props));
  return node;
}

export function TabsList(props: TabsListProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "variant", "label", "disabled", "children", "onKeyDown");
  const tabs = useContext(TabsContext);
  const axis = () => tabs?.orientation() ?? "horizontal";
  const disabled = () => !!props.disabled || !!tabs?.disabled();
  const variant = createMemo(() => {
    const current = props.variant ?? "default";
    if (current !== "default" && current !== "line") throw new TypeError("TabsList variant must be default or line");
    return current;
  });
  const roving = createRovingFocus(axis);
  const node = createElement("box");
  spread(node, mergeProps(rest, withDefaults({
    role: "tabList" as const,
    get disabled() { return disabled(); },
    get ariaLabel() { return props.label ?? null; },
    get orientation() { return axis() === "vertical" ? "vertical" as const : "horizontal" as const; },
    get "data-orientation"() { return axis() === "vertical" ? "vertical" : "horizontal"; },
    get "data-variant"() { return variant(); },
  }, rest), {
    get __zguiClass() { return classes("zui-tabs__list", `zui-tabs__list--${variant()}`); },
    onKeyDown(event: NativeKeyEvent) {
      if (callHandler(event, props.onKeyDown) && !disabled()) roving.onKeyDown(event);
    },
  }));
  slot(node, provide(ListContext, { tabs, disabled, roving, variant }, props));
  return node;
}

export function TabsTrigger(props: TabsTriggerProps): NativeNode {
  useStylesheet(sheet);
  const tabs = useContext(TabsContext);
  const inherited = useContext(ListContext);
  const list = inherited?.tabs === tabs ? inherited : undefined;
  if (isStaticProps(props) && !("__zguiClass" in props)) {
    const val = partValue(props.value);
    const value = () => val;
    const disabled = () => !!props.disabled || !!tabs?.disabled() || !!list?.disabled();
    const selected = () => tabs?.value() === val;
    const node = createElement("control");
    tabs?.triggers.register(node, value);
    const item = list?.roving.register(node, disabled, selected);
    const select = () => { if (!disabled()) { item?.activate(); tabs?.select(val); } };
    const record = props as unknown as Record<string, unknown>;
    for (const key of Object.keys(record)) {
      if (key === "value" || key === "disabled" || key === "onClick" || key === "onFocus" || key === "children" || key === "ref") continue;
      const v = record[key];
      if (v !== undefined) setProp(node, key, v);
    }
    if (typeof props.ref === "function" || Array.isArray(props.ref)) {
      const refProp = props.ref;
      applyRefProp(() => refProp, node);
    }
    if (props.role === undefined) setProp(node, "role", "tab");
    if (record["data-value"] === undefined) setProp(node, "data-value", val);
    setProp(node, "__zguiClass", "zui-tabs__trigger");
    setProp(node, "onClick", (event: NativeEvent) => { if (callHandler(event, props.onClick)) select(); });
    setProp(node, "onFocus", (event: NativeFocusEvent) => {
      if (!callHandler(event, props.onFocus) || event.target !== node || disabled()) return;
      item?.activate();
      if (tabs?.activation() === "automatic") select();
    });
    const childrenDesc = Object.getOwnPropertyDescriptor(record, "children");
    if (childrenDesc?.get !== undefined) slot(node, () => props.children);
    else if (childrenDesc?.value !== undefined) slot(node, childrenDesc.value as NativeChild);
    const hasTabIndex = props.tabIndex !== undefined;
    const hasSelected = record.selected !== undefined;
    const hasAriaControls = props.ariaControls !== undefined;
    const hasDataState = record["data-state"] !== undefined;
    const hasDataOrient = record["data-orientation"] !== undefined;
    const hasDataVariant = record["data-variant"] !== undefined;
    effect(
      () => [
        item?.tabIndex() ?? 0,
        disabled(),
        selected(),
        tabs?.panels.get(val) ?? null,
        tabs?.orientation() === "vertical" ? "vertical" : "horizontal",
        list?.variant() ?? null,
      ] as const,
      ([tabIndex, isDisabled, isSelected, ariaControls, orientation, variant], prev) => {
        if (!hasTabIndex && (!prev || prev[0] !== tabIndex)) setProp(node, "tabIndex", tabIndex);
        if (!prev || prev[1] !== isDisabled) setProp(node, "disabled", isDisabled);
        if (!hasSelected && (!prev || prev[2] !== isSelected)) setProp(node, "selected", isSelected);
        if (!hasDataState && (!prev || prev[2] !== isSelected)) setProp(node, "data-state", isSelected ? "active" : "inactive");
        if (!hasAriaControls && (!prev || prev[3] !== ariaControls)) setProp(node, "ariaControls", ariaControls);
        if (!hasDataOrient && (!prev || prev[4] !== orientation)) setProp(node, "data-orientation", orientation);
        if (!hasDataVariant && (!prev || prev[5] !== variant)) setProp(node, "data-variant", variant);
      },
    );
    return node;
  }
  const rest = omit(props, "value", "disabled", "onClick", "onFocus");
  const value = createMemo(() => partValue(props.value));
  const disabled = () => !!props.disabled || !!tabs?.disabled() || !!list?.disabled();
  const selected = () => tabs?.value() === value();
  const node = createElement("control");
  tabs?.triggers.register(node, value);
  const item = list?.roving.register(node, disabled, selected);
  const select = () => { if (!disabled()) { item?.activate(); tabs?.select(value()); } };
  spread(node, mergeProps(rest, withDefaults({
    role: "tab" as const,
    get tabIndex() { return item?.tabIndex() ?? 0; },
    get disabled() { return disabled(); },
    get selected() { return selected(); },
    get ariaControls() { return tabs?.panels.get(value()) ?? null; },
    get "data-state"() { return selected() ? "active" : "inactive"; },
    get "data-value"() { return value(); },
    get "data-orientation"() { return tabs?.orientation() === "vertical" ? "vertical" : "horizontal"; },
    get "data-variant"() { return list?.variant() ?? null; },
  }, rest), {
    get __zguiClass() { return classes("zui-tabs__trigger"); },
    onClick(event: NativeEvent) { if (callHandler(event, props.onClick)) select(); },
    onFocus(event: NativeFocusEvent) {
      if (!callHandler(event, props.onFocus) || event.target !== node || disabled()) return;
      item?.activate();
      if (tabs?.activation() === "automatic") select();
    },
  }));
  return node;
}

export function TabsContent(props: TabsContentProps): NativeNode {
  useStylesheet(sheet);
  const tabs = useContext(TabsContext);
  if (isStaticProps(props) && !("__zguiClass" in props)) {
    const val = partValue(props.value);
    const value = () => val;
    const selected = () => tabs?.value() === val;
    const node = createElement("box");
    tabs?.panels.register(node, value);
    const record = props as unknown as Record<string, unknown>;
    for (const key of Object.keys(record)) {
      if (key === "value" || key === "keepMounted" || key === "children" || key === "ref") continue;
      const v = record[key];
      if (v !== undefined) setProp(node, key, v);
    }
    if (typeof props.ref === "function" || Array.isArray(props.ref)) {
      const refProp = props.ref;
      applyRefProp(() => refProp, node);
    }
    if (props.role === undefined) setProp(node, "role", "tabPanel");
    if (props.tabIndex === undefined) setProp(node, "tabIndex", 0);
    if (record["data-value"] === undefined) setProp(node, "data-value", val);
    setProp(node, "__zguiClass", "zui-tabs__content");
    const hasAriaHidden = props.ariaHidden !== undefined;
    const hasAriaLabelledBy = props.ariaLabelledBy !== undefined;
    const hasDataState = record["data-state"] !== undefined;
    effect(
      () => [selected(), tabs?.triggers.get(val) ?? null] as const,
      ([isSelected, ariaLabelledBy], prev) => {
        if (!hasAriaHidden && (!prev || prev[0] !== isSelected)) setProp(node, "ariaHidden", !isSelected);
        if (!hasDataState && (!prev || prev[0] !== isSelected)) setProp(node, "data-state", isSelected ? "active" : "inactive");
        if (!hasAriaLabelledBy && (!prev || prev[1] !== ariaLabelledBy)) setProp(node, "ariaLabelledBy", ariaLabelledBy);
      },
    );
    const visible = createMemo(() => !!props.keepMounted || selected());
    slot(node, () => visible() ? untrack(() => props.children) : null);
    return node;
  }
  const rest = omit(props, "value", "keepMounted", "children");
  const value = createMemo(() => partValue(props.value));
  const selected = () => tabs?.value() === value();
  const node = createElement("box");
  tabs?.panels.register(node, value);
  spread(node, mergeProps(rest, withDefaults({
    role: "tabPanel" as const,
    tabIndex: 0,
    get ariaHidden() { return !selected(); },
    get ariaLabelledBy() { return tabs?.triggers.get(value()) ?? null; },
    get "data-state"() { return selected() ? "active" : "inactive"; },
    get "data-value"() { return value(); },
  }, rest), {
    get __zguiClass() { return classes("zui-tabs__content"); },
  }));
  const visible = createMemo(() => !!props.keepMounted || selected());
  slot(node, () => visible() ? untrack(() => props.children) : null);
  return node;
}
