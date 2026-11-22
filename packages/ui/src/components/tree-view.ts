import {
  For,
  createContext,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  omit,
  untrack,
  useContext,
} from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import { focus, precedes, scrollIntoView } from "../nodes.js";
import { VirtualList, type VirtualListController } from "../virtual-list.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeEvent,
  NativeKeyEvent,
  NativeNode,
} from "../types.js";
import { CHEVRON_DOWN, CHEVRON_RIGHT, Icon, type IconData } from "./icons.js";
import { sheet as treeSheet } from "./generated/TreeViewStyle.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, element, provide, slot, useStylesheet, withDefaults } from "./shared.js";

export interface TreeNodeData<T = unknown> {
  readonly id: string;
  readonly label: string;
  readonly children?: readonly TreeNodeData<T>[] | undefined;
  readonly disabled?: boolean | undefined;
  readonly icon?: IconData | undefined;
  readonly badge?: string | undefined;
  readonly meta?: T | undefined;
}

export interface TreeItemRenderState {
  readonly depth: number;
  readonly expanded: boolean;
  readonly selected: boolean;
  readonly focused: boolean;
  readonly hasChildren: boolean;
  readonly disabled: boolean;
  readonly toggleExpand: () => void;
}

export interface TreeVirtualizeOptions {
  readonly itemHeight?: number | undefined;
  readonly overscan?: number | undefined;
  readonly initialHeight?: number | undefined;
  readonly gap?: number | undefined;
}

export interface TreeViewProps<T = unknown> extends Omit<
  NativeElementProps,
  "children" | "onSelect"
> {
  readonly items?: readonly TreeNodeData<T>[] | undefined;
  readonly children?: NativeChild | undefined;
  readonly selectedId?: string | null | undefined;
  readonly defaultSelectedId?: string | null | undefined;
  readonly onSelect?: ((id: string, node?: TreeNodeData<T>) => void) | undefined;
  readonly onActivate?: ((id: string, node?: TreeNodeData<T>) => void) | undefined;
  readonly expandedIds?: readonly string[] | undefined;
  readonly defaultExpandedIds?: readonly string[] | undefined;
  readonly onExpandedChange?: ((ids: readonly string[]) => void) | undefined;
  readonly renderItem?:
    | ((node: TreeNodeData<T>, state: TreeItemRenderState) => NativeChild)
    | undefined;
  readonly virtualize?: boolean | TreeVirtualizeOptions | undefined;
  readonly emptyText?: string | undefined;
  readonly label?: string | null | undefined;
}

export interface TreeItemProps extends Omit<NativeElementProps, "children"> {
  readonly itemId: string;
  readonly label: string;
  readonly children?: NativeChild | undefined;
  readonly icon?: IconData | undefined;
  readonly badge?: string | undefined;
  readonly expandable?: boolean | undefined;
}

interface FlatTreeEntry<T = unknown> {
  readonly id: string;
  readonly node: TreeNodeData<T>;
  readonly depth: number;
  readonly parentId: string | null;
  readonly hasChildren: boolean;
  readonly expanded: boolean;
}

interface DeclarativeItemEntry {
  readonly id: string;
  readonly parentId: string | null;
  readonly depth: number;
  readonly node: NativeNode;
  readonly hasChildren: () => boolean;
  readonly disabled: () => boolean;
}

interface TreeContextValue {
  readonly selectedId: Accessor<string | null>;
  readonly focusedId: Accessor<string | null>;
  readonly isExpanded: (id: string) => boolean;
  readonly toggleExpand: (id: string) => void;
  readonly selectId: (id: string) => void;
  readonly activateId: (id: string) => void;
  readonly focusId: (id: string) => void;
  readonly registerDeclarative: (entry: DeclarativeItemEntry) => () => void;
  readonly handleKeyDown: (event: NativeKeyEvent) => void;
}

const TreeContext = createContext<TreeContextValue | null>(null);
const TreeBranchContext = createContext<{
  readonly depth: number;
  readonly parentId: string | null;
}>({
  depth: 0,
  parentId: null,
});

function flattenTree<T>(
  nodes: readonly TreeNodeData<T>[],
  expandedSet: ReadonlySet<string>,
  depth = 0,
  parentId: string | null = null,
  out: FlatTreeEntry<T>[] = [],
): FlatTreeEntry<T>[] {
  for (const node of nodes) {
    const hasChildren = (node.children?.length ?? 0) > 0;
    const expanded = hasChildren && expandedSet.has(node.id);
    out.push({ id: node.id, node, depth, parentId, hasChildren, expanded });
    if (expanded && node.children) {
      flattenTree(node.children, expandedSet, depth + 1, node.id, out);
    }
  }
  return out;
}

function findNodeById<T>(
  nodes: readonly TreeNodeData<T>[],
  id: string,
): TreeNodeData<T> | undefined {
  for (const node of nodes) {
    if (node.id === id) return node;
    if (node.children) {
      const found = findNodeById(node.children, id);
      if (found) return found;
    }
  }
  return undefined;
}

function sortEntries<T extends { readonly node: NativeNode }>(items: readonly T[]): T[] {
  if (items.length <= 1) return [...items];
  return [...items].sort((a, b) => {
    if (a.node === b.node) return 0;
    try {
      if (precedes(a.node, b.node)) return -1;
      if (precedes(b.node, a.node)) return 1;
      return 0;
    } catch {
      return 0;
    }
  });
}

export function TreeView<T = unknown>(props: TreeViewProps<T> = {}): NativeNode {
  useStylesheet(treeSheet);
  const rest = omit(
    props,
    "items",
    "children",
    "selectedId",
    "defaultSelectedId",
    "onSelect",
    "onActivate",
    "expandedIds",
    "defaultExpandedIds",
    "onExpandedChange",
    "renderItem",
    "virtualize",
    "emptyText",
    "label",
    "onKeyDown",
  );

  const [selectedId, setSelectedId] = createControllable<string | null>(
    {
      value: () => props.selectedId,
      defaultValue: () => props.defaultSelectedId ?? null,
      onChange: () => next => {
        if (next !== null) {
          const node = props.items ? findNodeById(props.items, next) : undefined;
          props.onSelect?.(next, node);
        }
      },
    },
    null,
  );

  const [expandedIds, setExpandedIds] = createControllable<readonly string[]>(
    {
      value: () => props.expandedIds,
      defaultValue: () => props.defaultExpandedIds ?? [],
      onChange: () => props.onExpandedChange,
    },
    [],
  );

  const expandedSet = createMemo(() => new Set(expandedIds()));
  const [focusedId, setFocusedId] = createSignal<string | null>(untrack(selectedId), {
    ownedWrite: true,
  });
  const [declarativeItems, setDeclarativeItems] = createSignal<readonly DeclarativeItemEntry[]>(
    [],
    { ownedWrite: true },
  );

  const rowNodes = new Map<string, NativeNode>();
  let virtualController: VirtualListController | null = null;
  let pendingFocusId: string | null = null;

  const flatData = createMemo<readonly FlatTreeEntry<T>[]>(() => {
    if (!props.items) return [];
    return flattenTree(props.items, expandedSet());
  });

  const visibleEntries = createMemo<
    readonly {
      id: string;
      parentId: string | null;
      hasChildren: boolean;
      disabled: boolean;
      node?: TreeNodeData<T> | undefined;
    }[]
  >(() => {
    if (props.items) {
      return flatData().map(item => ({
        id: item.id,
        parentId: item.parentId,
        hasChildren: item.hasChildren,
        disabled: !!item.node.disabled,
        node: item.node,
      }));
    }
    return declarativeItems().map(item => ({
      id: item.id,
      parentId: item.parentId,
      hasChildren: item.hasChildren(),
      disabled: item.disabled(),
    }));
  });

  const isExpanded = (id: string) => expandedSet().has(id);

  const setExpandedState = (id: string, nextOpen: boolean) => {
    const current = untrack(expandedIds);
    const has = current.includes(id);
    if (nextOpen && !has) setExpandedIds([...current, id]);
    else if (!nextOpen && has) setExpandedIds(current.filter(item => item !== id));
  };

  const toggleExpand = (id: string) => {
    setExpandedState(id, !isExpanded(id));
  };

  const revealById = (id: string) => {
    const list = untrack(visibleEntries);
    const idx = list.findIndex(item => item.id === id);
    if (idx < 0) return;
    if (props.virtualize && virtualController) {
      virtualController.scrollToIndex(idx, { align: "auto" });
    }
    const target = rowNodes.get(id);
    if (target) {
      pendingFocusId = null;
      scrollIntoView(target, { block: "nearest", behavior: "instant" });
      focus(target);
    } else {
      pendingFocusId = id;
    }
  };

  const moveFocusTo = (id: string) => {
    setFocusedId(id);
    revealById(id);
  };

  const selectId = (id: string) => {
    setFocusedId(id);
    setSelectedId(id);
  };

  const activateId = (id: string) => {
    setFocusedId(id);
    setSelectedId(id);
    const node = props.items ? findNodeById(props.items, id) : undefined;
    props.onActivate?.(id, node);
  };

  const findNextEnabled = (startIdx: number, delta: 1 | -1): string | null => {
    const list = untrack(visibleEntries);
    for (let i = startIdx + delta; i >= 0 && i < list.length; i += delta) {
      if (!list[i]!.disabled) return list[i]!.id;
    }
    return null;
  };

  const handleKeyDown = (event: NativeKeyEvent) => {
    const list = untrack(visibleEntries);
    if (list.length === 0) return;
    const currentId = untrack(focusedId) ?? untrack(selectedId);
    let currentIdx = currentId ? list.findIndex(item => item.id === currentId) : -1;
    if (currentIdx < 0) {
      currentIdx = list.findIndex(item => !item.disabled);
      if (currentIdx < 0) return;
    }
    const current = list[currentIdx]!;

    if (event.key === "ArrowDown") {
      event.preventDefault();
      event.stopPropagation();
      const next = findNextEnabled(currentIdx, 1);
      if (next) moveFocusTo(next);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      event.stopPropagation();
      const prev = findNextEnabled(currentIdx, -1);
      if (prev) moveFocusTo(prev);
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      event.stopPropagation();
      const first = list.find(item => !item.disabled);
      if (first) moveFocusTo(first.id);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      event.stopPropagation();
      for (let i = list.length - 1; i >= 0; i--) {
        if (!list[i]!.disabled) {
          moveFocusTo(list[i]!.id);
          break;
        }
      }
      return;
    }
    if (event.key === "ArrowRight") {
      event.preventDefault();
      event.stopPropagation();
      if (!current.hasChildren) return;
      if (!isExpanded(current.id)) {
        setExpandedState(current.id, true);
      } else {
        const next = findNextEnabled(currentIdx, 1);
        if (next) moveFocusTo(next);
      }
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      event.stopPropagation();
      if (current.hasChildren && isExpanded(current.id)) {
        setExpandedState(current.id, false);
      } else if (current.parentId) {
        moveFocusTo(current.parentId);
      }
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
      if (!current.disabled) activateId(current.id);
      return;
    }
    if (event.key === "Space" || event.key === " ") {
      event.preventDefault();
      event.stopPropagation();
      if (!current.disabled) selectId(current.id);
    }
  };

  createEffect(selectedId, sel => {
    if (sel && untrack(focusedId) === null) setFocusedId(sel);
  });

  const registerDeclarative = (entry: DeclarativeItemEntry): (() => void) => {
    rowNodes.set(entry.id, entry.node);
    setDeclarativeItems(prev => sortEntries([...prev, entry]));
    return () => {
      if (rowNodes.get(entry.id) === entry.node) rowNodes.delete(entry.id);
      setDeclarativeItems(prev => prev.filter(item => item !== entry));
    };
  };

  const context: TreeContextValue = {
    selectedId,
    focusedId,
    isExpanded,
    toggleExpand,
    selectId,
    activateId,
    focusId: setFocusedId,
    registerDeclarative,
    handleKeyDown,
  };

  const renderDataRow = (entryAccessor: Accessor<FlatTreeEntry<T>>): NativeNode => {
    const initial = untrack(entryAccessor);
    const itemId = initial.id;
    const row = createElement("control");
    rowNodes.set(itemId, row);
    onCleanup(() => {
      if (rowNodes.get(itemId) === row) rowNodes.delete(itemId);
    });

    const isSelected = () => selectedId() === itemId;
    const isFocused = () => focusedId() === itemId;
    const isOpen = () => isExpanded(itemId);

    spread(
      row,
      {
        role: "listItem",
        tabIndex: 0,
        get ariaLevel() {
          return entryAccessor().depth + 1;
        },
        testId: `tree-item-${itemId}`,
        get disabled() {
          return !!entryAccessor().node.disabled;
        },
        get selected() {
          return isSelected();
        },
        get expanded() {
          return entryAccessor().hasChildren ? isOpen() : null;
        },
        get "data-selected"() {
          return isSelected() ? "true" : "false";
        },
        get "data-focused"() {
          return isFocused() ? "true" : "false";
        },
        get "data-expanded"() {
          return entryAccessor().hasChildren ? (isOpen() ? "true" : "false") : null;
        },
        get "data-disabled"() {
          return entryAccessor().node.disabled ? "true" : "false";
        },
        "data-item-id": itemId,
        __zguiClass: "zui-tree__row",
        get __zguiStyle() {
          return { "--zui-tree-depth": String(entryAccessor().depth) };
        },
        onFocus() {
          setFocusedId(itemId);
        },
        onKeyDown: handleKeyDown,
        onClick() {
          const cur = untrack(entryAccessor);
          if (cur.node.disabled) return;
          focus(row);
          if (cur.hasChildren) toggleExpand(itemId);
          activateId(itemId);
        },
      },
      true,
    );

    slot(row, () => {
      const cur = entryAccessor();
      if (!cur.hasChildren) {
        return element("box", { class: "zui-tree__spacer", ariaHidden: true });
      }
      const chevron = element("control", {
        role: "button",
        tabIndex: -1,
        get ariaLabel() {
          return isOpen()
            ? `Collapse ${entryAccessor().node.label}`
            : `Expand ${entryAccessor().node.label}`;
        },
        testId: `tree-toggle-${itemId}`,
        class: "zui-tree__chevron",
        onClick(event: NativeEvent) {
          event.stopPropagation();
          toggleExpand(itemId);
        },
      });
      slot(chevron, () => Icon({ icon: isOpen() ? CHEVRON_DOWN : CHEVRON_RIGHT, size: "sm" }));
      return chevron;
    });

    if (props.renderItem) {
      const custom = props.renderItem(initial.node, {
        get depth() {
          return entryAccessor().depth;
        },
        get expanded() {
          return isOpen();
        },
        get selected() {
          return isSelected();
        },
        get focused() {
          return isFocused();
        },
        get hasChildren() {
          return entryAccessor().hasChildren;
        },
        get disabled() {
          return !!entryAccessor().node.disabled;
        },
        toggleExpand: () => toggleExpand(itemId),
      });
      slot(row, custom);
    } else {
      slot(row, () => {
        const iconData = entryAccessor().node.icon;
        if (!iconData) return null;
        const iconWrap = element("box", { class: "zui-tree__icon", ariaHidden: true });
        append(iconWrap, Icon({ icon: iconData, size: "sm" }));
        return iconWrap;
      });
      append(
        row,
        element("label", {
          class: "zui-tree__label",
          get children() {
            return entryAccessor().node.label;
          },
        }),
      );
      slot(row, () => {
        const badgeText = entryAccessor().node.badge;
        return badgeText
          ? element("label", { class: "zui-tree__badge", children: badgeText })
          : null;
      });
    }

    if (pendingFocusId === itemId) {
      createEffect(
        () => true,
        () => {
          if (pendingFocusId === itemId) {
            pendingFocusId = null;
            focus(row);
          }
        },
      );
    }
    return row;
  };

  const root = createElement("box");
  spread(
    root,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "list" as const,
          tabIndex: 0,
          get ariaLabel() {
            return props.label ?? null;
          },
          get "data-virtualized"() {
            return props.virtualize ? "true" : "false";
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(treeSheet.className);
        },
        onKeyDown(event: NativeKeyEvent) {
          if (!callHandler(event, props.onKeyDown)) return;
          handleKeyDown(event);
        },
      },
    ),
    true,
  );

  if (props.items !== undefined) {
    const emptyFallback = () =>
      element("box", {
        class: "zui-tree__empty",
        role: "status",
        get children() {
          return props.emptyText ?? "No items";
        },
      });
    if (props.virtualize) {
      const opts = typeof props.virtualize === "object" ? props.virtualize : {};
      append(
        root,
        VirtualList({
          get each() {
            return flatData();
          },
          keyed: entry => entry.id,
          itemHeight: opts.itemHeight ?? 28,
          overscan: opts.overscan ?? 4,
          initialHeight: opts.initialHeight ?? 320,
          gap: opts.gap ?? 0,
          style: { flexGrow: 1, height: "100%", minHeight: "0px" },
          controller: handle => {
            virtualController = handle;
          },
          get fallback() {
            return emptyFallback();
          },
          children: entry => renderDataRow(entry),
        }),
      );
    } else {
      slot(
        root,
        For({
          get each() {
            return flatData();
          },
          keyed: entry => entry.id,
          get fallback() {
            return emptyFallback();
          },
          children: entry => renderDataRow(entry),
        }),
      );
    }
  } else {
    slot(
      root,
      provide(TreeContext, context, () =>
        provide(TreeBranchContext, { depth: 0, parentId: null }, props),
      ),
    );
  }

  return root;
}

export function TreeItem(props: TreeItemProps): NativeNode {
  useStylesheet(treeSheet);
  const context = useContext(TreeContext);
  const branch = useContext(TreeBranchContext);
  const rest = omit(
    props,
    "itemId",
    "label",
    "children",
    "icon",
    "badge",
    "disabled",
    "expandable",
    "onClick",
    "onKeyDown",
  );

  const hasChildren = () => props.expandable ?? "children" in props;
  const isExpanded = () => context?.isExpanded(props.itemId) ?? false;
  const isSelected = () => context?.selectedId() === props.itemId;
  const isFocused = () => context?.focusedId() === props.itemId;

  const container = element("box", { class: "zui-tree__children" });
  const row = createElement("control");

  if (context) {
    const unregister = context.registerDeclarative({
      id: props.itemId,
      parentId: branch.parentId,
      depth: branch.depth,
      node: row,
      hasChildren,
      disabled: () => !!props.disabled,
    });
    onCleanup(unregister);
  }

  spread(
    row,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "listItem" as const,
          tabIndex: 0,
          ariaLevel: branch.depth + 1,
          get disabled() {
            return !!props.disabled;
          },
          get selected() {
            return isSelected();
          },
          get expanded() {
            return hasChildren() ? isExpanded() : null;
          },
          get "data-selected"() {
            return isSelected() ? "true" : "false";
          },
          get "data-focused"() {
            return isFocused() ? "true" : "false";
          },
          get "data-expanded"() {
            return hasChildren() ? (isExpanded() ? "true" : "false") : null;
          },
          get "data-disabled"() {
            return props.disabled ? "true" : "false";
          },
          get "data-item-id"() {
            return props.itemId;
          },
          __zguiStyle: { "--zui-tree-depth": String(branch.depth) },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes("zui-tree__row");
        },
        onFocus() {
          context?.focusId(props.itemId);
        },
        onKeyDown(event: NativeKeyEvent) {
          if (!callHandler(event, props.onKeyDown)) return;
          context?.handleKeyDown(event);
        },
        onClick(event: NativeEvent) {
          if (!callHandler(event, props.onClick) || props.disabled) return;
          focus(row);
          if (hasChildren()) context?.toggleExpand(props.itemId);
          context?.activateId(props.itemId);
        },
      },
    ),
    true,
  );

  slot(row, () => {
    if (!hasChildren()) return element("box", { class: "zui-tree__spacer", ariaHidden: true });
    const chevron = element("control", {
      role: "button",
      tabIndex: -1,
      ariaLabel: isExpanded() ? `Collapse ${props.label}` : `Expand ${props.label}`,
      class: "zui-tree__chevron",
      onClick(event: NativeEvent) {
        event.stopPropagation();
        context?.toggleExpand(props.itemId);
      },
    });
    slot(chevron, () => Icon({ icon: isExpanded() ? CHEVRON_DOWN : CHEVRON_RIGHT, size: "sm" }));
    return chevron;
  });

  if (props.icon) {
    const iconWrap = element("box", { class: "zui-tree__icon", ariaHidden: true });
    append(iconWrap, Icon({ icon: props.icon, size: "sm" }));
    append(row, iconWrap);
  }
  append(row, element("label", { class: "zui-tree__label", children: props.label }));
  if (props.badge) {
    append(row, element("label", { class: "zui-tree__badge", children: props.badge }));
  }

  append(container, row);
  slot(container, () => {
    if (!hasChildren() || !isExpanded()) return null;
    return provide(TreeBranchContext, { depth: branch.depth + 1, parentId: props.itemId }, () =>
      untrack(() => props.children),
    );
  });

  return container;
}
