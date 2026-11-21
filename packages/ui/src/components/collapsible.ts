import { $PROXY, createContext, createMemo, createSignal, onCleanup, omit, useContext } from "#solid";
import { createElement, effect, mergeProps, setProp, spread } from "../renderer/index.js";
import { focus, observeGeometry } from "../nodes.js";
import type { Accessor, NativeElementProps, NativeEvent, NativeFocusEvent, NativeNode, NativeRole, NativeStyle } from "../types.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, provide, slot, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/CollapsibleStyle.js";
export interface CollapsibleProps extends Omit<NativeElementProps, "open"> {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
}
export type CollapsibleTriggerProps = NativeElementProps;
export type CollapsibleContentProps = NativeElementProps;

export function createDisclosure(root: NativeNode, open: Accessor<boolean>, disabled: Accessor<boolean>, write: (open: boolean) => void) {
  const [trigger, setTrigger] = createSignal<{ node: NativeNode; disabled: Accessor<boolean> } | null>(null, { ownedWrite: true });
  const [content, setContent] = createSignal<NativeNode | null>(null, { ownedWrite: true });
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
export const DisclosureContext = createContext<ReturnType<typeof createDisclosure> | null>(null);

export function Collapsible(props: CollapsibleProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "open", "defaultOpen", "onOpenChange", "disabled", "children");
  const [read, write] = createControllable<boolean>({
    value: () => props.open, defaultValue: () => props.defaultOpen, onChange: () => props.onOpenChange,
  }, false);
  const open = createMemo(() => {
    const value = read();
    if (typeof value !== "boolean") throw new TypeError("Collapsible open must be a boolean");
    return value;
  });
  const node = createElement("column");
  const context = createDisclosure(node, open, () => !!props.disabled, write);
  spread(node, mergeProps(rest, withDefaults({
    tabIndex: -1,
    get "data-state"() { return open() ? "open" : "closed"; },
  }, rest), {
    get __zguiClass() { return classes(sheet.className, "zui-collapsible"); },
  }));
  slot(node, provide(DisclosureContext, context, props));
  return node;
}

export function CollapsibleTrigger(props: CollapsibleTriggerProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "disabled", "onClick");
  const context = useContext(DisclosureContext);
  const disabled = () => !!props.disabled || !!context?.disabled();
  const node = createElement("control");
  context?.registerTrigger(node, disabled);
  spread(node, mergeProps(rest, withDefaults({
    role: "button" as const, tabIndex: 0,
    get disabled() { return disabled(); },
    get expanded() { return context?.open() ?? false; },
    get ariaControls() { return context?.content() ?? null; },
    get "data-state"() { return context?.open() ? "open" : "closed"; },
  }, rest), {
    get __zguiClass() { return classes("zui-collapsible__trigger"); },
    onClick(event: NativeEvent) {
      if (callHandler(event, props.onClick) && !disabled()) context?.setOpen(!context.open());
    },
  }));
  return node;
}

export function disclosureContent(
  props: NativeElementProps,
  options: {
    measureClass?: string;
    contentClass?: string;
    role?: NativeRole;
    ariaLabelledBy?: () => NativeNode | null;
  } = {},
): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "children", "--zui-collapsible-height", "onFocus", "onBlur");
  const context = useContext(DisclosureContext);
  const open = () => context?.open() ?? false;
  const [height, setHeight] = createSignal<number>();
  const [collapsed, setCollapsed] = createSignal(false);
  const [focused, setFocused] = createSignal(false);
  const node = createElement("box");
  context?.registerContent(node);
  const inner = createElement("box");
  setProp(inner, "class", classes("zui-collapsible__measure", options.measureClass));
  spread(node, mergeProps(rest, withDefaults({
    ...(options.role !== undefined ? { role: options.role } : {}),
    ...(options.ariaLabelledBy ? { get ariaLabelledBy() { return options.ariaLabelledBy!(); } } : {}),
    get ariaHidden() { return !open(); },
    get "data-state"() { return open() ? "open" : "closed"; },
  }, rest), {
    get __zguiClass() { return classes("zui-collapsible__content", options.contentClass); },
    get "data-collapsed"() { return !open() && collapsed() ? "" : null; },
    get __zguiStyle(): Exclude<NativeStyle, string> {
      const measured = height();
      return { "--zui-collapsible-height": props["--zui-collapsible-height"] === undefined
        ? measured === undefined ? null : `${measured}px`
        : props["--zui-collapsible-height"] };
    },
    onFocus(event: NativeFocusEvent) { props.onFocus?.(event); setFocused(true); },
    onBlur(event: NativeFocusEvent) { setFocused(false); props.onBlur?.(event); },
  }));
  if (!($PROXY in props) && Object.getOwnPropertyDescriptor(props, "children")?.get === undefined) {
    slot(inner, props.children);
  } else {
    slot(inner, () => props.children);
  }
  append(node, inner);
  observeGeometry(inner, "borderSize", size => setHeight(Math.ceil(size.height)));
  observeGeometry(node, "scrollPosition", value => setCollapsed(value.scrollport.height === 0));
  effect(() => !open() && focused(), shouldRestore => {
    if (shouldRestore) context?.restoreFocus();
  });
  return node;
}
export function CollapsibleContent(props: CollapsibleContentProps = {}): NativeNode {
  return disclosureContent(props);
}
