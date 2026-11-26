import { $PROXY, createContext, createMemo, omit, untrack, useContext } from "#solid";
import {
  createElement,
  effect,
  mergeProps,
  ref as applyRefProp,
  setProp,
  spread,
} from "../renderer/index.js";
import type {
  Accessor,
  NativeElementProps,
  NativeEvent,
  NativeFocusEvent,
  NativeKeyEvent,
  NativeNode,
} from "../types.js";
import { callHandler, createControllable } from "./state.js";
import {
  append,
  classes,
  element,
  isStaticProps,
  provide,
  slot,
  useStylesheet,
  withDefaults,
} from "./shared.js";
import { createRovingFocus } from "./roving.js";
import { createDisclosure, DisclosureContext, disclosureContent } from "./collapsible.js";
import { Icon, CHEVRON_DOWN } from "./icons.js";
import { sheet } from "./generated/AccordionStyle.js";
export type AccordionSelection = "single" | "multiple";
export interface AccordionProps extends NativeElementProps {
  value?: readonly string[];
  defaultValue?: readonly string[];
  onValueChange?: (value: readonly string[]) => void;
  selection?: AccordionSelection;
  collapsible?: boolean;
}
export interface AccordionItemProps extends NativeElementProps {
  value: string;
}
export interface AccordionTriggerProps extends NativeElementProps {
  level?: number;
}
export type AccordionContentProps = NativeElementProps;
interface AccordionContextValue {
  values: Accessor<readonly string[]>;
  disabled: Accessor<boolean>;
  setOpen: (value: string, open: boolean) => void;
  roving: ReturnType<typeof createRovingFocus>;
}
const AccordionContext = createContext<AccordionContextValue | null>(null);
const accordionGroup = Symbol("AccordionGroup");
type ItemDisclosure = ReturnType<typeof createDisclosure> & {
  [accordionGroup]?: AccordionContextValue | null;
};

function accordionValues(value: readonly string[]): readonly string[] {
  if (!Array.isArray(value) || !value.every(item => typeof item === "string")) {
    throw new TypeError("Accordion value must be an array of strings");
  }
  return value;
}
function accordionSelection(value: AccordionSelection | undefined): AccordionSelection {
  const resolved = value ?? "single";
  if (resolved !== "single" && resolved !== "multiple") {
    throw new TypeError("Accordion selection must be single or multiple");
  }
  return resolved;
}
function accordionItemValue(value: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new TypeError("Accordion item value must be a non-empty string");
  }
  return value;
}
function accordionHeadingLevel(level: number | undefined): number {
  const value = level ?? 3;
  if (!Number.isInteger(value) || value < 1 || value > 0xffffffff) {
    throw new RangeError("Accordion heading level must be an integer between 1 and 4294967295");
  }
  return value;
}

export function Accordion(props: AccordionProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(
    props,
    "value",
    "defaultValue",
    "onValueChange",
    "selection",
    "collapsible",
    "disabled",
    "children",
    "onKeyDown",
  );
  const [read, write] = createControllable<readonly string[]>(
    {
      value: () => props.value,
      defaultValue: () => props.defaultValue,
      onChange: () => props.onValueChange,
    },
    [],
  );
  untrack(() => {
    accordionValues(read());
    accordionSelection(props.selection);
  });
  const values = createMemo(() => accordionValues(read()));
  const selection = createMemo(() => accordionSelection(props.selection));
  const disabled = () => !!props.disabled;
  const roving = createRovingFocus(() => "vertical");
  const context: AccordionContextValue = {
    values,
    disabled,
    roving,
    setOpen(value, open) {
      if (disabled()) return;
      const next = new Set(values());
      if (open) {
        if (selection() === "single") next.clear();
        next.add(value);
      } else {
        if (selection() === "single" && !(props.collapsible ?? true)) return;
        next.delete(value);
      }
      const sorted = [...next].sort();
      const previous = [...new Set(values())].sort();
      if (
        sorted.length !== previous.length ||
        sorted.some((value, index) => value !== previous[index])
      )
        write(sorted);
    },
  };
  const node = createElement("column");
  spread(
    node,
    mergeProps(rest, withDefaults({ role: "group" as const }, rest), {
      get __zguiClass() {
        return classes(sheet.className, "zui-accordion");
      },
      onKeyDown(event: NativeKeyEvent) {
        if (callHandler(event, props.onKeyDown) && !disabled()) roving.onKeyDown(event);
      },
    }),
  );
  slot(node, provide(AccordionContext, context, props));
  return node;
}

export function AccordionItem(props: AccordionItemProps): NativeNode {
  useStylesheet(sheet);
  const group = useContext(AccordionContext);
  if (isStaticProps(props) && !("__zguiClass" in props)) {
    const val = accordionItemValue(props.value);
    const node = createElement("column");
    const disclosure: ItemDisclosure = {
      ...createDisclosure(
        node,
        () => group?.values().includes(val) ?? false,
        () => !!props.disabled || !!group?.disabled(),
        next => group?.setOpen(val, next),
      ),
      [accordionGroup]: group,
    };
    const record = props as unknown as Record<string, unknown>;
    for (const key of Object.keys(record)) {
      if (key === "value" || key === "disabled" || key === "children" || key === "ref") continue;
      const v = record[key];
      if (v !== undefined) setProp(node, key, v);
    }
    if (props.tabIndex === undefined) setProp(node, "tabIndex", -1);
    if (record["data-value"] === undefined) setProp(node, "data-value", val);
    setProp(node, "__zguiClass", "zui-accordion__item");
    if (typeof props.ref === "function" || Array.isArray(props.ref)) {
      const refProp = props.ref;
      applyRefProp(() => refProp, node);
    }
    effect(
      () => (disclosure.open() ? "open" : "closed"),
      state => {
        if (record["data-state"] === undefined) setProp(node, "data-state", state);
      },
    );
    slot(node, provide(DisclosureContext, disclosure, props));
    return node;
  }
  const rest = omit(props, "value", "disabled", "children");
  accordionItemValue(untrack(() => props.value));
  const value = createMemo(() => accordionItemValue(props.value));
  const node = createElement("column");
  const disclosure: ItemDisclosure = {
    ...createDisclosure(
      node,
      () => group?.values().includes(value()) ?? false,
      () => !!props.disabled || !!group?.disabled(),
      next => group?.setOpen(value(), next),
    ),
    [accordionGroup]: group,
  };
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          tabIndex: -1,
          get "data-state"() {
            return disclosure.open() ? "open" : "closed";
          },
          get "data-value"() {
            return value();
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes("zui-accordion__item");
        },
      },
    ),
  );
  slot(node, provide(DisclosureContext, disclosure, props));
  return node;
}

export function AccordionTrigger(props: AccordionTriggerProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "level", "disabled", "children", "onClick", "onFocus");
  const context: ItemDisclosure | null = useContext(DisclosureContext);
  const group = context?.[accordionGroup];
  const disabled = () => !!props.disabled || !!context?.disabled();
  const heading =
    !($PROXY in props) && Object.getOwnPropertyDescriptor(props, "level")?.get === undefined
      ? (() => {
          const box = createElement("box");
          setProp(box, "class", "zui-accordion__header");
          setProp(box, "role", "heading");
          setProp(box, "ariaLevel", accordionHeadingLevel(props.level));
          return box;
        })()
      : (() => {
          accordionHeadingLevel(untrack(() => props.level));
          const level = createMemo(() => accordionHeadingLevel(props.level));
          return element("box", {
            class: "zui-accordion__header",
            role: "heading",
            get ariaLevel() {
              return level();
            },
          });
        })();
  const node = createElement("control");
  context?.registerTrigger(node, disabled);
  const item = group?.roving.register(node, disabled);
  const chevron = Icon({
    icon: CHEVRON_DOWN,
    class: "zui-accordion__chevron",
    get "data-state"() {
      return context?.open() ? "open" : "closed";
    },
  });
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "button" as const,
          get tabIndex() {
            return item?.tabIndex() ?? 0;
          },
          get disabled() {
            return disabled();
          },
          get expanded() {
            return context?.open() ?? false;
          },
          get ariaControls() {
            return context?.content() ?? null;
          },
          get "data-state"() {
            return context?.open() ? "open" : "closed";
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes("zui-accordion__trigger");
        },
        get children() {
          return [props.children, chevron];
        },
        onClick(event: NativeEvent) {
          if (callHandler(event, props.onClick) && !disabled()) {
            item?.activate();
            context?.setOpen(!context.open());
          }
        },
        onFocus(event: NativeFocusEvent) {
          if (callHandler(event, props.onFocus) && event.target === node && !disabled())
            item?.activate();
        },
      },
    ),
  );
  append(heading, node);
  return heading;
}

export function AccordionContent(props: AccordionContentProps = {}): NativeNode {
  useStylesheet(sheet);
  const context = useContext(DisclosureContext);
  return disclosureContent(props, {
    measureClass: "zui-accordion__measure",
    contentClass: "zui-accordion__content",
    role: "region",
    ariaLabelledBy: () => context?.trigger() ?? null,
  });
}
