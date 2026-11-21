import {
  createContext,
  createEffect,
  createMemo,
  createSignal,
  omit,
  untrack,
  useContext,
} from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import { focus } from "../nodes.js";
import { VirtualList, type VirtualListController } from "../virtual-list.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeKeyEvent,
  NativePointerDownEvent,
} from "../types.js";
import { Dialog, DialogContent, DialogTitle } from "./dialog.js";
import { Icon, SEARCH, type IconData } from "./icons.js";
import { Input } from "./input.js";
import { Kbd, KbdGroup } from "./kbd.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, element, provide, slot, useStylesheet } from "./shared.js";
import { sheet as commandSheet } from "./generated/CommandStyle.js";
import { sheet as selectSheet } from "./generated/SelectStyle.js";
import { sheet as menuSheet } from "./generated/MenuStyle.js";

export interface CommandItem {
  readonly id: string;
  readonly label: string;
  readonly group?: string | undefined;
  readonly description?: string | undefined;
  readonly keywords?: readonly string[] | undefined;
  readonly shortcut?: string | readonly string[] | undefined;
  readonly icon?: IconData | undefined;
  readonly disabled?: boolean | undefined;
  readonly onSelect?: (() => void) | undefined;
}

export type CommandFilter = (command: CommandItem, query: string) => boolean | number | null | undefined;

export interface CommandPaletteContextValue {
  readonly open: Accessor<boolean>;
  readonly setOpen: (open: boolean) => void;
  readonly toggle: () => void;
  readonly query: Accessor<string>;
  readonly setQuery: (query: string) => void;
  readonly commands: Accessor<readonly CommandItem[]>;
  readonly registerCommand: (command: CommandItem) => () => void;
  readonly registerCommands: (commands: readonly CommandItem[]) => () => void;
}

export interface CommandPaletteProviderProps {
  readonly open?: boolean | undefined;
  readonly defaultOpen?: boolean | undefined;
  readonly onOpenChange?: ((open: boolean) => void) | undefined;
  readonly query?: string | undefined;
  readonly defaultQuery?: string | undefined;
  readonly onQueryChange?: ((query: string) => void) | undefined;
  readonly commands?: readonly CommandItem[] | undefined;
  readonly children?: NativeChild;
}

export interface CommandPaletteProps extends Omit<NativeElementProps, "children" | "open"> {
  readonly children?: never;
  readonly commands?: readonly CommandItem[] | undefined;
  readonly open?: boolean | undefined;
  readonly defaultOpen?: boolean | undefined;
  readonly onOpenChange?: ((open: boolean) => void) | undefined;
  readonly query?: string | undefined;
  readonly defaultQuery?: string | undefined;
  readonly onQueryChange?: ((query: string) => void) | undefined;
  readonly onSelect?: ((command: CommandItem) => void) | undefined;
  readonly onActiveIdChange?: ((id: string | null) => void) | undefined;
  readonly filter?: CommandFilter | undefined;
  readonly placeholder?: string | undefined;
  readonly emptyText?: string | undefined;
  readonly title?: string | undefined;
  readonly label?: string | null | undefined;
  readonly itemHeight?: number | undefined;
  readonly listHeight?: number | undefined;
  readonly overscan?: number | undefined;
  readonly closeOnSelect?: boolean | undefined;
  readonly resetQueryOnClose?: boolean | undefined;
  readonly dismissOnOutsidePress?: boolean | undefined;
  readonly dismissOnEscape?: boolean | undefined;
  readonly dismissControl?: boolean | undefined;
  readonly inputTestId?: string | undefined;
  readonly listTestId?: string | undefined;
  readonly emptyTestId?: string | undefined;
}

interface RegistryBatch {
  readonly token: symbol;
  readonly commands: readonly CommandItem[];
}

type CommandRow =
  | { readonly kind: "group"; readonly key: string; readonly label: string }
  | { readonly kind: "item"; readonly key: string; readonly command: CommandItem };

const DEFAULT_ITEM_HEIGHT = 36;
const DEFAULT_LIST_HEIGHT = 300;
const DEFAULT_OVERSCAN = 4;

const CommandPaletteContext = createContext<CommandPaletteContextValue | null>(null);

function isWordBoundary(text: string, index: number): boolean {
  if (index <= 0) return true;
  const prev = text.charCodeAt(index - 1);
  return prev === 32 || prev === 45 || prev === 95 || prev === 47 || prev === 46 || prev === 58;
}

function fuzzySubsequenceScore(haystack: string, needle: string): number | null {
  if (needle.length === 0) return 0;
  if (needle.length > haystack.length) return null;
  let score = 0;
  let hayIndex = 0;
  let prevMatch = -2;
  for (let i = 0; i < needle.length; i++) {
    const ch = needle.charCodeAt(i);
    let found = -1;
    for (let j = hayIndex; j < haystack.length; j++) {
      if (haystack.charCodeAt(j) === ch) {
        found = j;
        break;
      }
    }
    if (found < 0) return null;
    score += 10;
    if (found === prevMatch + 1) {
      score += 15;
    } else if (prevMatch >= 0) {
      score -= Math.min(8, found - prevMatch - 1);
    }
    if (isWordBoundary(haystack, found)) {
      score += 12;
    }
    prevMatch = found;
    hayIndex = found + 1;
  }
  return score;
}

function scoreSingleToken(command: CommandItem, token: string): number | null {
  const label = command.label.toLowerCase();
  if (label === token) return 1000;
  if (label.startsWith(token)) return 800 - Math.min(100, label.length - token.length);
  const labelIndex = label.indexOf(token);
  if (labelIndex >= 0) {
    return 600 - Math.min(100, labelIndex) + (isWordBoundary(label, labelIndex) ? 40 : 0);
  }

  const secondaryParts: string[] = [];
  if (command.keywords && command.keywords.length > 0) secondaryParts.push(command.keywords.join(" "));
  if (command.group) secondaryParts.push(command.group);
  if (command.description) secondaryParts.push(command.description);
  const secondary = secondaryParts.join(" ").toLowerCase();

  if (secondary.length > 0) {
    const secondaryIndex = secondary.indexOf(token);
    if (secondaryIndex >= 0) {
      return 400 - Math.min(100, secondaryIndex);
    }
  }

  const labelFuzzy = fuzzySubsequenceScore(label, token);
  if (labelFuzzy !== null) return 200 + labelFuzzy;

  if (secondary.length > 0) {
    const combined = `${label} ${secondary}`;
    const combinedFuzzy = fuzzySubsequenceScore(combined, token);
    if (combinedFuzzy !== null) return 100 + combinedFuzzy;
  }

  return null;
}

/**
 * Default fuzzy + substring matcher for `CommandPalette`.
 * Returns a numeric relevance score (higher is better), or `null` when the command does not match.
 */
export function scoreCommandMatch(command: CommandItem, rawQuery: string): number | null {
  const normalized = rawQuery.trim().toLowerCase();
  if (normalized.length === 0) return 0;
  const tokens = normalized.split(/\s+/);
  let total = 0;
  for (const token of tokens) {
    const score = scoreSingleToken(command, token);
    if (score === null) return null;
    total += score;
  }
  return total;
}

function mergeCommands(...sources: readonly (readonly CommandItem[] | undefined)[]): readonly CommandItem[] {
  const byId = new Map<string, CommandItem>();
  const order: string[] = [];
  for (const source of sources) {
    if (!source) continue;
    for (const item of source) {
      if (!byId.has(item.id)) order.push(item.id);
      byId.set(item.id, item);
    }
  }
  const result: CommandItem[] = [];
  for (const id of order) {
    const item = byId.get(id);
    if (item) result.push(item);
  }
  return result;
}

function parseShortcutTokens(shortcut: string | readonly string[] | undefined): readonly string[] {
  if (!shortcut) return [];
  if (Array.isArray(shortcut)) {
    return shortcut.filter((part): part is string => typeof part === "string" && part.length > 0);
  }
  const text = (shortcut as string).trim();
  if (text.length === 0) return [];
  if (text === "+") return ["+"];
  if (!text.includes("+")) return [text];
  const tokens: string[] = [];
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "+") {
      const part = text.slice(start, i).trim();
      if (part.length > 0) tokens.push(part);
      else if (i === text.length - 1 || text[i + 1] === "+") {
        tokens.push("+");
        if (text[i + 1] === "+") i += 1;
      }
      start = i + 1;
    }
  }
  const tail = text.slice(start).trim();
  if (tail.length > 0) tokens.push(tail);
  return tokens;
}

function renderShortcut(shortcut: string | readonly string[] | undefined): NativeChild {
  const tokens = parseShortcutTokens(shortcut);
  if (tokens.length === 0) return null;
  if (tokens.length === 1) {
    return Kbd({
      style: { marginLeft: "auto", flexShrink: 0 },
      children: tokens[0]!,
    });
  }
  const group = KbdGroup({
    style: { marginLeft: "auto", flexShrink: 0 },
  });
  for (const token of tokens) {
    append(group, Kbd({ children: token }));
  }
  return group;
}

export function CommandPaletteProvider(props: CommandPaletteProviderProps = {}): NativeChild {
  const [open, setOpen] = createControllable<boolean>({
    value: () => props.open,
    defaultValue: () => props.defaultOpen,
    onChange: () => props.onOpenChange,
  }, false);

  const [query, setQuery] = createControllable<string>({
    value: () => props.query,
    defaultValue: () => props.defaultQuery,
    onChange: () => props.onQueryChange,
  }, "");

  const [batches, setBatches] = createSignal<readonly RegistryBatch[]>([], { ownedWrite: true });

  const registerCommands = (commands: readonly CommandItem[]): (() => void) => {
    const token = Symbol("command-batch");
    const snapshot = [...commands];
    setBatches(current => [...current, { token, commands: snapshot }]);
    let active = true;
    return () => {
      if (!active) return;
      active = false;
      setBatches(current => current.filter(entry => entry.token !== token));
    };
  };

  const registerCommand = (command: CommandItem): (() => void) => registerCommands([command]);

  const commands = createMemo(() => {
    const lists: (readonly CommandItem[] | undefined)[] = [props.commands];
    for (const batch of batches()) lists.push(batch.commands);
    return mergeCommands(...lists);
  });

  const value: CommandPaletteContextValue = {
    open,
    setOpen,
    toggle() {
      setOpen(!untrack(open));
    },
    query,
    setQuery,
    commands,
    registerCommand,
    registerCommands,
  };

  return provide(CommandPaletteContext, value, props);
}

export function useCommandPalette(): CommandPaletteContextValue {
  const ctx = useContext(CommandPaletteContext);
  if (!ctx) {
    throw new Error("useCommandPalette must be used inside a CommandPaletteProvider");
  }
  return ctx;
}

interface FilteredCommandModel {
  readonly rows: readonly CommandRow[];
  readonly enabledIds: readonly string[];
  readonly enabledById: ReadonlyMap<string, CommandItem>;
  readonly rowIndexById: ReadonlyMap<string, number>;
}

function buildFilteredModel(
  commands: readonly CommandItem[],
  query: string,
  filter?: CommandFilter,
): FilteredCommandModel {
  const hasQuery = query.trim().length > 0;
  const matcher = filter ?? scoreCommandMatch;

  interface ScoredEntry {
    readonly command: CommandItem;
    readonly score: number;
    readonly order: number;
  }

  interface GroupBucket {
    readonly name: string;
    readonly order: number;
    readonly items: ScoredEntry[];
    bestScore: number;
  }

  const groups = new Map<string, GroupBucket>();
  let order = 0;

  for (const command of commands) {
    const raw = matcher(command, query);
    let score: number | null;
    if (raw === false || raw === null || raw === undefined) {
      score = null;
    } else if (raw === true) {
      score = 1;
    } else if (typeof raw === "number" && Number.isFinite(raw)) {
      score = raw;
    } else {
      score = null;
    }
    if (score === null) continue;

    const groupName = command.group ?? "";
    let bucket = groups.get(groupName);
    if (!bucket) {
      bucket = { name: groupName, order: groups.size, items: [], bestScore: score };
      groups.set(groupName, bucket);
    } else if (score > bucket.bestScore) {
      bucket.bestScore = score;
    }
    bucket.items.push({ command, score, order: order++ });
  }

  const orderedGroups = [...groups.values()];
  if (hasQuery) {
    orderedGroups.sort((a, b) => (b.bestScore - a.bestScore) || (a.order - b.order));
    for (const bucket of orderedGroups) {
      bucket.items.sort((a, b) => (b.score - a.score) || (a.order - b.order));
    }
  }

  const rows: CommandRow[] = [];
  const enabledIds: string[] = [];
  const enabledById = new Map<string, CommandItem>();
  const rowIndexById = new Map<string, number>();

  for (const bucket of orderedGroups) {
    if (bucket.name.length > 0) {
      rows.push({ kind: "group", key: `group:${bucket.name}`, label: bucket.name });
    }
    for (const entry of bucket.items) {
      const cmd = entry.command;
      const rowIndex = rows.length;
      rows.push({ kind: "item", key: `item:${cmd.id}`, command: cmd });
      rowIndexById.set(cmd.id, rowIndex);
      if (!cmd.disabled) {
        enabledIds.push(cmd.id);
        enabledById.set(cmd.id, cmd);
      }
    }
  }

  return { rows, enabledIds, enabledById, rowIndexById };
}

export function CommandPalette(props: CommandPaletteProps = {}): NativeChild {
  useStylesheet(commandSheet);
  useStylesheet(selectSheet);
  useStylesheet(menuSheet);

  const provider = useContext(CommandPaletteContext);

  const rest = omit(
    props,
    "commands",
    "open",
    "defaultOpen",
    "onOpenChange",
    "query",
    "defaultQuery",
    "onQueryChange",
    "onSelect",
    "onActiveIdChange",
    "filter",
    "placeholder",
    "emptyText",
    "title",
    "label",
    "itemHeight",
    "listHeight",
    "overscan",
    "closeOnSelect",
    "resetQueryOnClose",
    "dismissOnOutsidePress",
    "dismissOnEscape",
    "dismissControl",
    "inputTestId",
    "listTestId",
    "emptyTestId",
    "class",
    "onKeyDown",
  );

  const useProviderOpen = provider !== null && props.open === undefined && props.defaultOpen === undefined;
  const [localOpen, setLocalOpen] = createControllable<boolean>({
    value: () => props.open,
    defaultValue: () => props.defaultOpen,
    onChange: () => props.onOpenChange,
  }, false);

  const open: Accessor<boolean> = useProviderOpen ? () => provider.open() : localOpen;
  const setOpen = (next: boolean) => {
    if (useProviderOpen) {
      provider.setOpen(next);
      props.onOpenChange?.(next);
    } else {
      setLocalOpen(next);
    }
  };

  const useProviderQuery = provider !== null && props.query === undefined && props.defaultQuery === undefined;
  const [localQuery, setLocalQuery] = createControllable<string>({
    value: () => props.query,
    defaultValue: () => props.defaultQuery,
    onChange: () => props.onQueryChange,
  }, "");

  const query: Accessor<string> = useProviderQuery ? () => provider.query() : localQuery;
  const setQuery = (next: string) => {
    if (useProviderQuery) {
      provider.setQuery(next);
      props.onQueryChange?.(next);
    } else {
      setLocalQuery(next);
    }
  };

  const allCommands = createMemo(() => {
    if (provider && props.commands) return mergeCommands(provider.commands(), props.commands);
    if (provider) return provider.commands();
    return props.commands ? mergeCommands(props.commands) : [];
  });

  const model = createMemo(() => buildFilteredModel(allCommands(), query(), props.filter));
  const [activeId, setActiveId] = createSignal<string | null>(null, { ownedWrite: true });

  let listController: VirtualListController | null = null;
  let invokedInOpenCycle = false;
  let wasOpen = untrack(open);

  const scrollActiveIntoView = (commandId: string | null, revealTopHeader = false) => {
    if (!commandId || !listController) return;
    const currentModel = untrack(model);
    const rowIndex = currentModel.rowIndexById.get(commandId);
    if (rowIndex === undefined) return;
    const targetRow = revealTopHeader && rowIndex === 1 && currentModel.rows[0]?.kind === "group"
      ? 0
      : rowIndex;
    listController.scrollToIndex(targetRow, { align: "auto", behavior: "instant" });
  };

  createEffect(
    () => open(),
    isOpen => {
      if (isOpen && !wasOpen) {
        invokedInOpenCycle = false;
      } else if (!isOpen && wasOpen) {
        if (props.resetQueryOnClose ?? true) {
          setQuery("");
        }
        listController = null;
      }
      wasOpen = isOpen;
    },
  );

  // Keep activeId pinned to a valid enabled command ID across filter/list changes.
  createEffect(
    () => ({ isOpen: open(), current: model() }),
    ({ isOpen, current }) => {
      if (!isOpen) return;
      const prev = untrack(activeId);
      if (prev !== null && current.enabledById.has(prev)) return;
      const next = current.enabledIds[0] ?? null;
      setActiveId(next);
      if (next !== null) scrollActiveIntoView(next, true);
    },
  );

  createEffect(
    () => ({ isOpen: open(), id: activeId() }),
    ({ isOpen, id }) => {
      props.onActiveIdChange?.(isOpen ? id : null);
    },
  );

  const executeCommand = (command: CommandItem) => {
    if (command.disabled) return;
    const closeOnSelect = props.closeOnSelect ?? true;
    if (closeOnSelect && invokedInOpenCycle) return;
    if (closeOnSelect) {
      invokedInOpenCycle = true;
      setOpen(false);
    }
    command.onSelect?.();
    props.onSelect?.(command);
  };

  const moveActive = (direction: "next" | "prev" | "first" | "last") => {
    const currentModel = untrack(model);
    const ids = currentModel.enabledIds;
    if (ids.length === 0) {
      setActiveId(null);
      return;
    }
    const currentId = untrack(activeId);
    const currentIndex = currentId === null ? -1 : ids.indexOf(currentId);
    let nextIndex: number;
    if (direction === "first") {
      nextIndex = 0;
    } else if (direction === "last") {
      nextIndex = ids.length - 1;
    } else if (direction === "next") {
      nextIndex = currentIndex < 0 ? 0 : (currentIndex + 1) % ids.length;
    } else {
      nextIndex = currentIndex < 0 ? ids.length - 1 : (currentIndex - 1 + ids.length) % ids.length;
    }
    const nextId = ids[nextIndex]!;
    setActiveId(nextId);
    scrollActiveIntoView(nextId, nextIndex === 0);
  };

  const buildBody = (): NativeChild => {
    const itemHeight = props.itemHeight ?? DEFAULT_ITEM_HEIGHT;
    const listHeight = props.listHeight ?? DEFAULT_LIST_HEIGHT;

    const root = element("column", {
      __zguiClass: classes(commandSheet.className, "zui-command"),
    });

    const fieldRow = element("row", {
      __zguiClass: "zui-command__field",
    });

    const inputNode = Input({
      role: "comboBox",
      ariaHasPopup: "listbox",
      expanded: true,
      class: "zui-command__input",
      get testId() { return props.inputTestId ?? null; },
      get value() { return query(); },
      get placeholder() { return props.placeholder ?? "Type a command or search..."; },
      get label() { return props.label ?? props.title ?? "Command Palette"; },
      onValueChange(nextValue: string) {
        setQuery(nextValue);
      },
      onKeyDown(event: NativeKeyEvent) {
        if (!callHandler(event, props.onKeyDown)) return;
        if (event.key === "ArrowDown") {
          event.preventDefault();
          event.stopPropagation();
          moveActive("next");
          return;
        }
        if (event.key === "ArrowUp") {
          event.preventDefault();
          event.stopPropagation();
          moveActive("prev");
          return;
        }
        if (event.key === "Home") {
          event.preventDefault();
          event.stopPropagation();
          moveActive("first");
          return;
        }
        if (event.key === "End") {
          event.preventDefault();
          event.stopPropagation();
          moveActive("last");
          return;
        }
        if (event.key === "Enter") {
          event.preventDefault();
          event.stopPropagation();
          if (event.repeat) return;
          const id = untrack(activeId);
          if (id === null) return;
          const cmd = untrack(model).enabledById.get(id);
          if (cmd) executeCommand(cmd);
        }
      },
    });

    createEffect(
      () => open(),
      isOpen => {
        if (isOpen) focus(inputNode);
      },
    );

    append(fieldRow, Icon({ icon: SEARCH, size: "sm" }));
    append(fieldRow, inputNode);
    append(root, fieldRow);

    const listNode = VirtualList<CommandRow>({
      get each() { return model().rows; },
      keyed: row => row.key,
      itemHeight,
      initialHeight: listHeight,
      get overscan() { return props.overscan ?? DEFAULT_OVERSCAN; },
      role: "listBox",
      get testId() { return props.listTestId ?? null; },
      class: "zui-command__list",
      style: { maxHeight: `${listHeight}px` },
      controller(ctrl) {
        listController = ctrl;
      },
      get fallback() {
        return element("box", {
          __zguiClass: "zui-combobox__empty",
          role: "status",
          get testId() { return props.emptyTestId ?? null; },
          get children() { return props.emptyText ?? "No commands found."; },
        });
      },
      children(rowAccessor: Accessor<CommandRow>) {
        const initial = untrack(rowAccessor);
        if (initial.kind === "group") {
          const groupNode = element("box", {
            role: "group",
            __zguiClass: "zui-command__group",
            __zguiStyle: {
              height: `${itemHeight}px`,
              flexShrink: 0,
              justifyContent: "center",
              paddingTop: "0px",
              paddingBottom: "0px",
            },
          });
          append(groupNode, element("label", {
            __zguiClass: "zui-menu__label",
            __zguiStyle: { paddingTop: "0px", paddingBottom: "0px" },
            get children() {
              const r = rowAccessor();
              return r.kind === "group" ? r.label : "";
            },
          }));
          return groupNode;
        }

        const command = (): CommandItem => {
          const r = rowAccessor();
          return r.kind === "item" ? r.command : initial.command;
        };

        const itemNode = createElement("control");
        spread(itemNode, {
          role: "listBoxOption",
          tabIndex: -1,
          __zguiClass: classes("zui-select__item", "zui-command__item"),
          __zguiStyle: {
            height: `${itemHeight}px`,
            flexShrink: 0,
            boxSizing: "border-box",
          },
          get disabled() { return !!command().disabled; },
          get selected() { return activeId() === command().id; },
          get "data-active"() { return activeId() === command().id ? "true" : "false"; },
          get "data-disabled"() { return command().disabled ? "true" : "false"; },
          get "data-command-id"() { return command().id; },
          onPointerEnter() {
            const cmd = command();
            if (!cmd.disabled) setActiveId(cmd.id);
          },
          onPointerDown(event: NativePointerDownEvent) {
            // Keep keyboard focus and IME composition inside the search input field.
            event.preventDefault();
          },
          onClick() {
            executeCommand(command());
          },
        }, true);

        slot(itemNode, () => {
          const iconData = command().icon;
          return iconData ? Icon({ icon: iconData, size: "sm" }) : null;
        });

        const textWrap = element("row", {
          __zguiStyle: {
            alignItems: "center",
            gap: "var(--zui-space-sm)",
            flexGrow: 1,
            minWidth: "0px",
            overflow: "hidden",
          },
        });
        append(textWrap, element("label", {
          __zguiStyle: { whiteSpace: "nowrap" },
          get children() { return command().label; },
        }));
        slot(textWrap, () => {
          const desc = command().description;
          if (!desc) return null;
          return element("label", {
            __zguiClass: "zui-select__placeholder",
            __zguiStyle: {
              fontSize: "var(--zui-type-size-xs)",
              whiteSpace: "nowrap",
              overflow: "hidden",
            },
            children: desc,
          });
        });
        append(itemNode, textWrap);

        slot(itemNode, () => renderShortcut(command().shortcut));
        return itemNode;
      },
    });

    append(root, listNode);
    return root;
  };

  return Dialog({
    get open() { return open(); },
    onOpenChange: setOpen,
    get children() {
      return DialogContent(mergeProps(rest, {
        get class() { return classes("zui-command__dialog", props.class); },
        get dismissControl() { return props.dismissControl ?? false; },
        get dismissOnOutsidePress() { return props.dismissOnOutsidePress ?? true; },
        get dismissOnEscape() { return props.dismissOnEscape ?? true; },
        get children() {
          return [
            DialogTitle({
              get children() { return props.title ?? "Command Palette"; },
            }),
            buildBody(),
          ];
        },
      }));
    },
  });
}
