import { For, createEffect, createMemo, createSignal, onCleanup, omit, untrack } from "#solid";
import { createElement, mergeProps, setProp, spread } from "../renderer/index.js";
import { focus, registerDismissLayer, scrollIntoView } from "../nodes.js";
import { createDelay } from "../timers.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeEvent,
  NativeKeyEvent,
  NativeNode,
  NativePointerDownEvent,
} from "../types.js";
import { CHECK, CHEVRON_DOWN, Icon } from "./icons.js";
import { Input } from "./input.js";
import { overlayPortal, raiseSurface } from "./overlay.js";
import { Popper } from "./popper.js";
import type { PopperPlacement } from "./popper.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, element, slot, useStylesheet, withDefaults } from "./shared.js";
import { sheet as selectSheet } from "./generated/SelectStyle.js";
import { sheet as comboboxSheet } from "./generated/ComboboxStyle.js";
import { sheet as overlaySheet } from "./generated/OverlayStyle.js";

export type SelectSize = "sm" | "md";

export interface SelectOption<T extends string = string> {
  readonly value: T;
  readonly label: string;
  readonly description?: string | undefined;
  readonly disabled?: boolean | undefined;
}

export interface SelectProps<T extends string = string> extends Omit<NativeElementProps, "children" | "open"> {
  children?: never;
  options: readonly SelectOption<T>[];
  value?: T | null | undefined;
  defaultValue?: T | null | undefined;
  onValueChange?: (value: T) => void;
  open?: boolean | undefined;
  defaultOpen?: boolean | undefined;
  onOpenChange?: (open: boolean) => void;
  placeholder?: string | undefined;
  emptyText?: string | undefined;
  size?: SelectSize | undefined;
  label?: string | null | undefined;
  placement?: PopperPlacement | undefined;
  offset?: number | undefined;
  listTestId?: string | undefined;
}

export interface ComboboxProps<T extends string = string> extends Omit<NativeElementProps, "children" | "open"> {
  children?: never;
  options: readonly SelectOption<T>[];
  value?: T | null | undefined;
  defaultValue?: T | null | undefined;
  onValueChange?: (value: T) => void;
  query?: string | undefined;
  defaultQuery?: string | undefined;
  onQueryChange?: (query: string) => void;
  filter?: ((option: SelectOption<T>, query: string) => boolean) | undefined;
  open?: boolean | undefined;
  defaultOpen?: boolean | undefined;
  onOpenChange?: (open: boolean) => void;
  placeholder?: string | undefined;
  emptyText?: string | undefined;
  label?: string | null | undefined;
  placement?: PopperPlacement | undefined;
  offset?: number | undefined;
  inputTestId?: string | undefined;
  listTestId?: string | undefined;
}

function firstEnabledIndex<T extends string>(options: readonly SelectOption<T>[]): number {
  for (let i = 0; i < options.length; i++) {
    if (!options[i]!.disabled) return i;
  }
  return -1;
}

function lastEnabledIndex<T extends string>(options: readonly SelectOption<T>[]): number {
  for (let i = options.length - 1; i >= 0; i--) {
    if (!options[i]!.disabled) return i;
  }
  return -1;
}

function nextEnabledIndex<T extends string>(
  options: readonly SelectOption<T>[],
  current: number,
  delta: 1 | -1,
): number {
  const len = options.length;
  if (len === 0) return -1;
  let index = current;
  for (let step = 0; step < len; step++) {
    index = (index + delta + len) % len;
    if (!options[index]!.disabled) return index;
  }
  return -1;
}

function indexForValue<T extends string>(options: readonly SelectOption<T>[], value: T | null | undefined): number {
  if (value == null) return -1;
  for (let i = 0; i < options.length; i++) {
    if (options[i]!.value === value && !options[i]!.disabled) return i;
  }
  return -1;
}

function defaultComboboxFilter<T extends string>(option: SelectOption<T>, rawQuery: string): boolean {
  const query = rawQuery.trim().toLowerCase();
  if (query.length === 0) return true;
  if (option.label.toLowerCase().includes(query)) return true;
  if (option.description && option.description.toLowerCase().includes(query)) return true;
  return option.value.toLowerCase().includes(query);
}

interface ListPopupConfig<T extends string> {
  readonly open: Accessor<boolean>;
  readonly setOpen: (next: boolean) => void;
  readonly anchor: Accessor<NativeNode | null>;
  readonly exclude: Accessor<NativeNode | null>;
  readonly options: Accessor<readonly SelectOption<T>[]>;
  readonly selectedValue: Accessor<T | null>;
  readonly activeIndex: Accessor<number>;
  readonly setActiveIndex: (index: number) => void;
  readonly onSelect: (option: SelectOption<T>) => void;
  readonly listClass: string;
  readonly emptyText: Accessor<string>;
  readonly placement: Accessor<PopperPlacement>;
  readonly offset: Accessor<number>;
  readonly listTestId: Accessor<string | undefined>;
}

function mountOptionPopup<T extends string>(config: ListPopupConfig<T>): NativeNode {
  const elevation = raiseSurface("popover");
  const holder = createElement("box");
  setProp(holder, "__zguiInvariantStyle", { display: "none" });
  const itemNodes = new Map<T, NativeNode>();

  createEffect(
    () => ({ open: config.open(), idx: config.activeIndex(), list: config.options() }),
    ({ open, idx, list }) => {
      if (!open || idx < 0 || idx >= list.length) return;
      const target = itemNodes.get(list[idx]!.value);
      if (target) scrollIntoView(target, { block: "nearest", behavior: "instant" });
    },
  );

  slot(holder, () => (config.open() ? untrack(() => overlayPortal(elevation, () => {
    const surface = createElement("box");
    const dismiss = registerDismissLayer(
      surface,
      () => config.setOpen(false),
      {
        layer: elevation.layer,
        dismissOnOutsidePress: true,
        dismissOnEscape: true,
        exclude: untrack(config.exclude),
        leaving: false,
      },
    );
    createEffect(
      () => ({ exclude: config.exclude(), leaving: !config.open() }),
      next => dismiss.update({
        layer: elevation.layer,
        dismissOnOutsidePress: true,
        dismissOnEscape: true,
        exclude: next.exclude,
        leaving: next.leaving,
      }),
    );

    spread(surface, {
      __zguiClass: classes(overlaySheet.className, "zui-surface"),
      "data-state": "open",
    }, true);

    const listBox = createElement("scroll");
    spread(listBox, {
      role: "listBox",
      get testId() { return config.listTestId() ?? null; },
      __zguiClass: classes(selectSheet.className, comboboxSheet.className, config.listClass),
      __zguiStyle: { minWidth: "var(--zui-popper-anchor-width, 160px)" },
    }, true);

    slot(listBox, For({
      get each() { return config.options(); },
      get fallback() {
        return element("box", {
          class: "zui-combobox__empty",
          role: "status",
          get children() { return config.emptyText(); },
        });
      },
      children(option: SelectOption<T>, index: Accessor<number>): NativeChild {
          const row = createElement("control");
          itemNodes.set(option.value, row);
          onCleanup(() => {
            if (itemNodes.get(option.value) === row) itemNodes.delete(option.value);
          });
          spread(row, {
            role: "listBoxOption",
            tabIndex: -1,
            get disabled() { return !!option.disabled; },
            get selected() { return config.selectedValue() === option.value; },
            get "data-active"() { return config.activeIndex() === index() ? "true" : "false"; },
            get "data-disabled"() { return option.disabled ? "true" : "false"; },
            get "data-value"() { return option.value; },
            __zguiClass: "zui-select__item",
            onPointerEnter() {
              if (!option.disabled) config.setActiveIndex(index());
            },
            onPointerDown(event: NativePointerDownEvent) {
              event.preventDefault();
            },
            onClick() {
              if (!option.disabled) config.onSelect(option);
            },
          }, true);
          const labelNode = element("box", {
            __zguiStyle: { display: "flex", flexDirection: "column", flexGrow: 1, minWidth: "0px" },
          });
          append(labelNode, element("label", { children: option.label }));
          if (option.description) {
            append(labelNode, element("label", {
              class: "zui-select__placeholder",
              __zguiStyle: { fontSize: "var(--zui-type-size-xs)" },
              children: option.description,
            }));
          }
          append(row, labelNode);
          slot(row, () => {
            if (config.selectedValue() !== option.value) return null;
            const mark = element("box", { class: "zui-select__indicator", ariaHidden: true });
            append(mark, Icon({ icon: CHECK, size: "sm" }));
            return mark;
          });
          return row;
        },
    }));

    append(surface, listBox);
    return Popper({
      get anchor() { return config.anchor(); },
      get placement() { return config.placement(); },
      get offset() { return config.offset(); },
      class: "zui-overlay-positioner",
      children: surface,
    });
  })) : null));
  return holder;
}

export function Select<T extends string = string>(props: SelectProps<T>): NativeNode {
  useStylesheet(selectSheet);
  useStylesheet(comboboxSheet);
  useStylesheet(overlaySheet);

  const rest = omit(
    props,
    "options",
    "value",
    "defaultValue",
    "onValueChange",
    "open",
    "defaultOpen",
    "onOpenChange",
    "placeholder",
    "emptyText",
    "size",
    "disabled",
    "invalid",
    "label",
    "placement",
    "offset",
    "listTestId",
    "onClick",
    "onKeyDown",
  );

  const [value, setValue] = createControllable<T | null>({
    value: () => props.value,
    defaultValue: () => props.defaultValue ?? null,
    onChange: () => next => {
      if (next !== null) props.onValueChange?.(next);
    },
  }, null);

  const [open, setOpen] = createControllable<boolean>({
    value: () => props.open,
    defaultValue: () => props.defaultOpen,
    onChange: () => props.onOpenChange,
  }, false);

  const [activeIndex, setActiveIndex] = createSignal(-1, { ownedWrite: true });
  const [triggerNode, setTriggerNode] = createSignal<NativeNode | null>(null, { ownedWrite: true });
  const typeaheadTimer = createDelay();
  let typeaheadBuffer = "";

  const selectedOption = createMemo(() => {
    const current = value();
    if (current == null) return undefined;
    return props.options.find(item => item.value === current);
  });

  const openList = (initialIndex?: number) => {
    if (props.disabled) return;
    const opts = props.options;
    const chosen = initialIndex ?? indexForValue(opts, untrack(value));
    setActiveIndex(chosen >= 0 ? chosen : firstEnabledIndex(opts));
    setOpen(true);
  };

  const commitOption = (option: SelectOption<T>) => {
    setValue(option.value);
    setOpen(false);
    const node = untrack(triggerNode);
    if (node) focus(node);
  };

  const handleTypeahead = (ch: string) => {
    typeaheadBuffer += ch.toLowerCase();
    typeaheadTimer.schedule(500, () => { typeaheadBuffer = ""; });
    const opts = props.options;
    const matchIndex = opts.findIndex(opt => !opt.disabled && opt.label.toLowerCase().startsWith(typeaheadBuffer));
    if (matchIndex < 0) return;
    setActiveIndex(matchIndex);
    if (!untrack(open)) {
      setValue(opts[matchIndex]!.value);
    }
  };

  const trigger = createElement("control");
  setTriggerNode(trigger);

  spread(trigger, mergeProps(
    rest,
    withDefaults({
      role: "comboBox" as const,
      tabIndex: 0,
      ariaHasPopup: "listbox" as const,
      get expanded() { return open(); },
      get disabled() { return !!props.disabled; },
      get invalid() { return !!props.invalid; },
      get ariaLabel() { return props.label ?? null; },
      get "data-size"() { return props.size ?? "md"; },
      get "data-state"() { return open() ? "open" : "closed"; },
    }, rest),
    {
      get __zguiClass() { return classes(selectSheet.className, "zui-select"); },
      onClick(event: NativeEvent) {
        if (!callHandler(event, props.onClick) || props.disabled) return;
        if (untrack(open)) setOpen(false);
        else openList();
      },
      onKeyDown(event: NativeKeyEvent) {
        if (!callHandler(event, props.onKeyDown) || props.disabled) return;
        const opts = props.options;
        const isOpen = untrack(open);
        if (event.key === "ArrowDown") {
          event.preventDefault();
          event.stopPropagation();
          if (!isOpen) openList();
          else setActiveIndex(nextEnabledIndex(opts, untrack(activeIndex), 1));
          return;
        }
        if (event.key === "ArrowUp") {
          event.preventDefault();
          event.stopPropagation();
          if (!isOpen) openList(lastEnabledIndex(opts));
          else setActiveIndex(nextEnabledIndex(opts, untrack(activeIndex), -1));
          return;
        }
        if (event.key === "Home" && isOpen) {
          event.preventDefault();
          event.stopPropagation();
          setActiveIndex(firstEnabledIndex(opts));
          return;
        }
        if (event.key === "End" && isOpen) {
          event.preventDefault();
          event.stopPropagation();
          setActiveIndex(lastEnabledIndex(opts));
          return;
        }
        if (event.key === "Enter" || event.key === "Space" || event.key === " ") {
          event.preventDefault();
          event.stopPropagation();
          if (!isOpen) {
            openList();
          } else {
            const idx = untrack(activeIndex);
            if (idx >= 0 && idx < opts.length && !opts[idx]!.disabled) {
              commitOption(opts[idx]!);
            } else {
              setOpen(false);
            }
          }
          return;
        }
        if (event.key === "Escape" && isOpen) {
          event.preventDefault();
          event.stopPropagation();
          setOpen(false);
          return;
        }
        if (event.character && event.character.length === 1 && !event.modifiers.control && !event.modifiers.meta && !event.modifiers.alt) {
          handleTypeahead(event.character);
        }
      },
    },
  ), true);

  slot(trigger, () => {
    const selected = selectedOption();
    if (selected) return element("label", { children: selected.label });
    return element("label", { class: "zui-select__placeholder", children: props.placeholder ?? "Select..." });
  });
  append(trigger, Icon({ icon: CHEVRON_DOWN, size: "sm", class: "zui-select__chevron" }));

  const popupHolder = mountOptionPopup({
    open,
    setOpen,
    anchor: triggerNode,
    exclude: triggerNode,
    options: () => props.options,
    selectedValue: value,
    activeIndex,
    setActiveIndex,
    onSelect: commitOption,
    listClass: "zui-select__list",
    emptyText: () => props.emptyText ?? "No options",
    placement: () => props.placement ?? { side: "bottom", align: "start" },
    offset: () => props.offset ?? 4,
    listTestId: () => props.listTestId,
  });
  append(trigger, popupHolder);
  return trigger;
}

export function Combobox<T extends string = string>(props: ComboboxProps<T>): NativeNode {
  useStylesheet(selectSheet);
  useStylesheet(comboboxSheet);
  useStylesheet(overlaySheet);

  const rest = omit(
    props,
    "options",
    "value",
    "defaultValue",
    "onValueChange",
    "query",
    "defaultQuery",
    "onQueryChange",
    "filter",
    "open",
    "defaultOpen",
    "onOpenChange",
    "placeholder",
    "emptyText",
    "disabled",
    "invalid",
    "label",
    "placement",
    "offset",
    "inputTestId",
    "listTestId",
    "onKeyDown",
  );

  const [value, setValue] = createControllable<T | null>({
    value: () => props.value,
    defaultValue: () => props.defaultValue ?? null,
    onChange: () => next => {
      if (next !== null) props.onValueChange?.(next);
    },
  }, null);

  const initialQuery = () => {
    if (props.defaultQuery !== undefined) return props.defaultQuery;
    const initVal = props.value ?? props.defaultValue;
    const match = initVal == null ? undefined : props.options.find(opt => opt.value === initVal);
    return match?.label ?? "";
  };

  const [query, setQuery] = createControllable<string>({
    value: () => props.query,
    defaultValue: initialQuery,
    onChange: () => props.onQueryChange,
  }, "");

  const [open, setOpen] = createControllable<boolean>({
    value: () => props.open,
    defaultValue: () => props.defaultOpen,
    onChange: () => props.onOpenChange,
  }, false);

  const filteredOptions = createMemo(() => {
    const predicate = props.filter ?? defaultComboboxFilter;
    const text = query();
    return props.options.filter(option => predicate(option, text));
  });

  const [activeIndex, setActiveIndex] = createSignal(-1, { ownedWrite: true });
  const [rootNode, setRootNode] = createSignal<NativeNode | null>(null, { ownedWrite: true });

  createEffect(
    () => ({ open: open(), list: filteredOptions() }),
    ({ open: isOpen, list }) => {
      if (!isOpen) return;
      const current = untrack(activeIndex);
      if (current >= 0 && current < list.length && !list[current]!.disabled) return;
      const selectedIdx = indexForValue(list, untrack(value));
      setActiveIndex(selectedIdx >= 0 ? selectedIdx : firstEnabledIndex(list));
    },
  );

  const commitOption = (option: SelectOption<T>) => {
    setValue(option.value);
    setQuery(option.label);
    setOpen(false);
  };

  const root = createElement("box");
  setRootNode(root);
  spread(root, mergeProps(
    rest,
    {
      get __zguiClass() { return classes(comboboxSheet.className, "zui-combobox"); },
      get "data-state"() { return open() ? "open" : "closed"; },
    },
  ), true);

  const inputNode = Input({
    role: "comboBox",
    ariaHasPopup: "listbox",
    get testId() { return props.inputTestId ?? null; },
    get value() { return query(); },
    get placeholder() { return props.placeholder ?? "Search..."; },
    get disabled() { return !!props.disabled; },
    get invalid() { return !!props.invalid; },
    get label() { return props.label ?? null; },
    get expanded() { return open(); },
    onFocus() {
      if (!props.disabled && !untrack(open)) setOpen(true);
    },
    onClick() {
      if (!props.disabled && !untrack(open)) setOpen(true);
    },
    onValueChange(nextQuery: string) {
      setQuery(nextQuery);
      if (!props.disabled) {
        setOpen(true);
        const list = filteredOptions();
        setActiveIndex(firstEnabledIndex(list));
      }
    },
    onKeyDown(event: NativeKeyEvent) {
      if (!callHandler(event, props.onKeyDown) || props.disabled) return;
      const list = filteredOptions();
      const isOpen = untrack(open);
      if (event.key === "ArrowDown") {
        event.preventDefault();
        event.stopPropagation();
        if (!isOpen) {
          setOpen(true);
          const sel = indexForValue(list, untrack(value));
          setActiveIndex(sel >= 0 ? sel : firstEnabledIndex(list));
        } else {
          setActiveIndex(nextEnabledIndex(list, untrack(activeIndex), 1));
        }
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        event.stopPropagation();
        if (!isOpen) {
          setOpen(true);
          setActiveIndex(lastEnabledIndex(list));
        } else {
          setActiveIndex(nextEnabledIndex(list, untrack(activeIndex), -1));
        }
        return;
      }
      if (event.key === "Home" && isOpen) {
        event.preventDefault();
        event.stopPropagation();
        setActiveIndex(firstEnabledIndex(list));
        return;
      }
      if (event.key === "End" && isOpen) {
        event.preventDefault();
        event.stopPropagation();
        setActiveIndex(lastEnabledIndex(list));
        return;
      }
      if (event.key === "Enter" && isOpen) {
        const idx = untrack(activeIndex);
        if (idx >= 0 && idx < list.length && !list[idx]!.disabled) {
          event.preventDefault();
          event.stopPropagation();
          commitOption(list[idx]!);
        }
        return;
      }
      if (event.key === "Escape" && isOpen) {
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
        const sel = untrack(value);
        if (props.query === undefined && sel != null) {
          const match = props.options.find(opt => opt.value === sel);
          if (match) setQuery(match.label);
        }
      }
    },
  });

  append(root, inputNode);
  const popupHolder = mountOptionPopup({
    open,
    setOpen,
    anchor: rootNode,
    exclude: rootNode,
    options: filteredOptions,
    selectedValue: value,
    activeIndex,
    setActiveIndex,
    onSelect: commitOption,
    listClass: "zui-combobox__list",
    emptyText: () => props.emptyText ?? "No results found",
    placement: () => props.placement ?? { side: "bottom", align: "start" },
    offset: () => props.offset ?? 4,
    listTestId: () => props.listTestId,
  });
  append(root, popupHolder);
  return root;
}
