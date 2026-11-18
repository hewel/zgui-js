import { createContext, createMemo, createSignal, onCleanup, splitProps, useContext } from "#solid";
import { createElement, effect, mergeProps, spread } from "../renderer/index.js";
import { focus, observeGeometry } from "../nodes.js";
import type { Accessor, NativeElementProps, NativeEvent, NativeFocusEvent, NativeNode, NativeStyle } from "../types.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, element, provide, slot, useStylesheet } from "./shared.js";
import { sheet as upstreamSheet } from "./generated/CollapsibleStyle.js";

const sheet = { ...upstreamSheet, css: upstreamSheet.css + '\n.zui-collapsible__content[data-collapsed] { visibility: hidden; }' };
export interface CollapsibleProps extends Omit<NativeElementProps, "open"> {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
}
export type CollapsibleTriggerProps = NativeElementProps;
export type CollapsibleContentProps = NativeElementProps;

export function createDisclosure(root: NativeNode, open: Accessor<boolean>, disabled: Accessor<boolean>, write: (open: boolean) => void) {
  const [trigger, setTrigger] = createSignal<{ node: NativeNode; disabled: Accessor<boolean> } | null>(null);
  const [content, setContent] = createSignal<NativeNode | null>(null);
  return {
    root, open, disabled, content,
    trigger: () => trigger()?.node ?? null,
    setOpen(next: boolean) { if (!disabled()) write(next); },
    restoreFocus() {
      const current = trigger();
      focus(current && !current.disabled() ? current.node : root, { reveal: true });
    },
    registerTrigger(node: NativeNode, disabled: Accessor<boolean>) {
      setTrigger({ node, disabled });
      onCleanup(() => setTrigger(current => current?.node === node ? null : current));
    },
    registerContent(node: NativeNode) {
      setContent(node);
      onCleanup(() => setContent(current => current === node ? null : current));
    },
  };
}
export const DisclosureContext = createContext<ReturnType<typeof createDisclosure>>();

export function Collapsible(props: CollapsibleProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["open", "defaultOpen", "onOpenChange", "disabled", "children"]);
  const [read, write] = createControllable<boolean>({
    value: () => own.open, defaultValue: () => own.defaultOpen, onChange: () => own.onOpenChange,
  }, false);
  const open = createMemo(() => {
    const value = read();
    if (typeof value !== "boolean") throw new TypeError("Collapsible open must be a boolean");
    return value;
  });
  const node = createElement("column");
  const context = createDisclosure(node, open, () => !!own.disabled, write);
  spread(node, mergeProps({ tabIndex: -1, get "data-state"() { return open() ? "open" : "closed"; } }, rest, {
    get __zguiClass() { return classes(sheet.className, "zui-collapsible"); },
  }));
  slot(node, provide(DisclosureContext, context, own));
  return node;
}

export function CollapsibleTrigger(props: CollapsibleTriggerProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["disabled", "onClick"]);
  const context = useContext(DisclosureContext);
  const disabled = () => !!own.disabled || !!context?.disabled();
  const node = createElement("control");
  context?.registerTrigger(node, disabled);
  spread(node, mergeProps({
    role: "button" as const, tabIndex: 0,
    get disabled() { return disabled(); },
    get expanded() { return context?.open() ?? false; },
    get ariaControls() { return context?.content() ?? null; },
    get "data-state"() { return context?.open() ? "open" : "closed"; },
  }, rest, {
    get __zguiClass() { return classes("zui-collapsible__trigger"); },
    onClick(event: NativeEvent) {
      if (callHandler(event, own.onClick) && !disabled()) context?.setOpen(!context.open());
    },
  }));
  return node;
}

export function disclosureContent(props: NativeElementProps, measureClass = ""): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["children", "--zui-collapsible-height", "onFocus", "onBlur"]);
  const context = useContext(DisclosureContext);
  const open = () => context?.open() ?? false;
  const [height, setHeight] = createSignal<number>();
  const [collapsed, setCollapsed] = createSignal(false);
  const [focused, setFocused] = createSignal(false);
  const node = createElement("box");
  context?.registerContent(node);
  const inner = element("box", { class: classes("zui-collapsible__measure", measureClass) });
  spread(node, mergeProps({
    get ariaHidden() { return !open(); },
    get "data-state"() { return open() ? "open" : "closed"; },
  }, rest, {
    get __zguiClass() { return classes("zui-collapsible__content"); },
    get "data-collapsed"() { return !open() && collapsed() ? "" : null; },
    get __zguiStyle(): Exclude<NativeStyle, string> {
      const measured = height();
      return { "--zui-collapsible-height": own["--zui-collapsible-height"] === undefined
        ? measured === undefined ? null : `${measured}px`
        : own["--zui-collapsible-height"] };
    },
    onFocus(event: NativeFocusEvent) { own.onFocus?.(event); setFocused(true); },
    onBlur(event: NativeFocusEvent) { setFocused(false); own.onBlur?.(event); },
  }));
  slot(inner, () => own.children);
  append(node, inner);
  observeGeometry(inner, "borderSize", size => setHeight(Math.ceil(size.height)));
  observeGeometry(node, "scrollPosition", value => setCollapsed(value.scrollport.height === 0));
  effect(() => {
    if (!open() && focused()) context?.restoreFocus();
  });
  return node;
}
export function CollapsibleContent(props: CollapsibleContentProps = {}): NativeNode {
  return disclosureContent(props);
}
