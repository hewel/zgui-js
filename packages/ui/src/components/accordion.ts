import { createContext, createMemo, splitProps, useContext } from "#solid";
import { createElement, insert, mergeProps, spread } from "../renderer/index.js";
import type { Accessor, NativeElementProps, NativeEvent, NativeFocusEvent, NativeKeyEvent, NativeNode } from "../types.js";
import { callHandler, createControllable } from "./state.js";
import { classes, element, useStylesheet } from "./shared.js";
import { createRovingFocus } from "./roving.js";
import { createDisclosure, DisclosureContext, disclosureContent } from "./collapsible.js";
import { Icon, CHEVRON_DOWN } from "./icons.js";
import { sheet as upstreamSheet } from "./generated/AccordionStyle.js";

const sheet = { ...upstreamSheet, css: upstreamSheet.css
  .replaceAll('.zui-accordion__content .zui-collapsible__measure', '.zui-accordion__measure')
  .replaceAll('.zui-accordion__trigger[data-state="open"] .zui-accordion__chevron', '.zui-accordion__chevron[data-state="open"]') };
export type AccordionSelection = "single" | "multiple";
export interface AccordionProps extends NativeElementProps {
  value?: readonly string[];
  defaultValue?: readonly string[];
  onValueChange?: (value: readonly string[]) => void;
  selection?: AccordionSelection;
  collapsible?: boolean;
}
export interface AccordionItemProps extends NativeElementProps { value: string; }
export interface AccordionTriggerProps extends NativeElementProps { level?: number; }
export type AccordionContentProps = NativeElementProps;
interface AccordionContextValue {
  values: Accessor<readonly string[]>;
  disabled: Accessor<boolean>;
  setOpen: (value: string, open: boolean) => void;
  roving: ReturnType<typeof createRovingFocus>;
}
const AccordionContext = createContext<AccordionContextValue>();
const accordionGroup = Symbol("AccordionGroup");
type ItemDisclosure = ReturnType<typeof createDisclosure> & { [accordionGroup]?: AccordionContextValue };

export function Accordion(props: AccordionProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["value", "defaultValue", "onValueChange", "selection", "collapsible", "disabled", "children", "onKeyDown"]);
  const [read, write] = createControllable<readonly string[]>({ value: () => own.value, defaultValue: () => own.defaultValue, onChange: () => own.onValueChange }, []);
  const values = createMemo(() => {
    const value = read();
    if (!Array.isArray(value) || !value.every(item => typeof item === "string")) throw new TypeError("Accordion value must be an array of strings");
    return value;
  });
  const selection = createMemo(() => {
    const value = own.selection ?? "single";
    if (value !== "single" && value !== "multiple") throw new TypeError("Accordion selection must be single or multiple");
    return value;
  });
  const disabled = () => !!own.disabled;
  const roving = createRovingFocus(() => "vertical");
  const context: AccordionContextValue = {
    values, disabled, roving,
    setOpen(value, open) {
      if (disabled()) return;
      const next = new Set(values());
      if (open) { if (selection() === "single") next.clear(); next.add(value); }
      else { if (selection() === "single" && !(own.collapsible ?? true)) return; next.delete(value); }
      const sorted = [...next].sort();
      const previous = [...new Set(values())].sort();
      if (sorted.length !== previous.length || sorted.some((value, index) => value !== previous[index])) write(sorted);
    },
  };
  const node = createElement("column");
  spread(node, mergeProps({ role: "group" as const }, rest, {
    get class() { return classes(sheet.className, "zui-accordion", rest.class); },
    onKeyDown(event: NativeKeyEvent) { if (callHandler(event, own.onKeyDown) && !disabled()) roving.onKeyDown(event); },
  }));
  insert(node, AccordionContext.Provider({ value: context, get children() { return own.children; } }));
  return node;
}

export function AccordionItem(props: AccordionItemProps): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["value", "disabled", "children"]);
  const group = useContext(AccordionContext);
  const value = createMemo(() => {
    if (typeof own.value !== "string" || own.value.length === 0) throw new TypeError("Accordion item value must be a non-empty string");
    return own.value;
  });
  const node = createElement("column");
  const disclosure: ItemDisclosure = {
    ...createDisclosure(node, () => group?.values().includes(value()) ?? false,
      () => !!own.disabled || !!group?.disabled(), next => group?.setOpen(value(), next)),
    [accordionGroup]: group,
  };
  spread(node, mergeProps({ tabIndex: -1,
    get "data-state"() { return disclosure.open() ? "open" : "closed"; },
    get "data-value"() { return value(); },
  }, rest, { get class() { return classes("zui-accordion__item", rest.class); } }));
  insert(node, DisclosureContext.Provider({ value: disclosure, get children() { return own.children; } }));
  return node;
}

export function AccordionTrigger(props: AccordionTriggerProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["level", "disabled", "children", "onClick", "onFocus"]);
  const context: ItemDisclosure | undefined = useContext(DisclosureContext);
  const group = context?.[accordionGroup];
  const disabled = () => !!own.disabled || !!context?.disabled();
  const level = createMemo(() => {
    const value = own.level ?? 3;
    if (!Number.isInteger(value) || value < 1 || value > 0xffffffff) throw new RangeError("Accordion heading level must be an integer between 1 and 4294967295");
    return value;
  });
  const heading = element("box", { class: "zui-accordion__header", role: "heading", get ariaLevel() { return level(); } });
  const node = createElement("control");
  context?.registerTrigger(node, disabled);
  const item = group?.roving.register(node, disabled);
  const chevron = Icon({ icon: CHEVRON_DOWN, class: "zui-accordion__chevron", get "data-state"() { return context?.open() ? "open" : "closed"; } });
  spread(node, mergeProps({ role: "button" as const,
    get tabIndex() { return item?.tabIndex() ?? 0; },
    get disabled() { return disabled(); },
    get expanded() { return context?.open() ?? false; },
    get ariaControls() { return context?.content() ?? null; },
    get "data-state"() { return context?.open() ? "open" : "closed"; },
  }, rest, {
    get class() { return classes("zui-accordion__trigger", rest.class); },
    get children() { return [own.children, chevron]; },
    onClick(event: NativeEvent) { if (callHandler(event, own.onClick) && !disabled()) { item?.activate(); context?.setOpen(!context.open()); } },
    onFocus(event: NativeFocusEvent) { if (callHandler(event, own.onFocus) && event.target === node && !disabled()) item?.activate(); },
  }));
  insert(heading, node);
  return heading;
}

export function AccordionContent(props: AccordionContentProps = {}): NativeNode {
  useStylesheet(sheet);
  const context = useContext(DisclosureContext);
  return disclosureContent(mergeProps({ role: "region" as const, get ariaLabelledBy() { return context?.trigger() ?? null; } }, props, {
    get class() { return classes("zui-accordion__content", props.class); },
  }), "zui-accordion__measure");
}
