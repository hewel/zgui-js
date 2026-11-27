import { For, createEffect, createMemo, createSignal, omit, untrack } from "#solid";
import { focus, scrollIntoView } from "../nodes.js";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeKeyEvent,
  NativeNode,
  NativePointerEvent,
} from "../types.js";
import { VirtualList } from "../virtual-list.js";
import type { VirtualListController, VirtualListRowSizing } from "../virtual-list.js";
import { sheet as dataSheet } from "./generated/DataTableStyle.js";
import { sheet as tableSheet } from "./generated/TableStyle.js";
import { ARROW_DOWN, ARROW_UP, Icon } from "./icons.js";
import { element, useStylesheet, withDefaults } from "./shared.js";
import { callHandler, createControllable } from "./state.js";
import { TableCell, TableHead, TableRow } from "./table.js";
import type { TableCellProps } from "./table.js";

export type DataTableSortDirection = "asc" | "desc";

export interface DataTableSort {
  readonly columnId: string;
  readonly direction: DataTableSortDirection;
}

export interface DataTableColumn<T> {
  readonly id: string;
  readonly header: NativeChild | (() => NativeChild);
  readonly cell: (item: T, index: Accessor<number>) => NativeChild;
  readonly width?: number | undefined;
  readonly minWidth?: number | undefined;
  readonly maxWidth?: number | undefined;
  readonly sortable?: boolean | undefined;
  readonly sortAccessor?: ((item: T) => string | number) | undefined;
  readonly resizable?: boolean | undefined;
  readonly align?: TableCellProps["align"];
}

interface DataTableBaseProps<T> extends Omit<NativeElementProps, "children"> {
  readonly columns: readonly DataTableColumn<T>[];
  readonly data: readonly T[];
  readonly sort?: DataTableSort | null | undefined;
  readonly defaultSort?: DataTableSort | null | undefined;
  readonly onSortChange?: ((next: DataTableSort | null) => void) | undefined;
  /** Comparator for the requested direction. Without it or a sortAccessor, data order is unchanged. */
  readonly sortRows?: ((a: T, b: T, sort: DataTableSort) => number) | undefined;
  readonly columnWidths?: Readonly<Record<string, number>> | undefined;
  readonly defaultColumnWidths?: Readonly<Record<string, number>> | undefined;
  readonly onColumnWidthsChange?: ((widths: Record<string, number>) => void) | undefined;
  /** Maximum height of the scrolling body in logical pixels (320 when virtualized). */
  readonly height?: number | undefined;
  readonly empty?: NativeChild;
  readonly toolbar?: NativeChild;
  /** Called on Enter or Space with the active row's index in the displayed order. */
  readonly onRowActivate?: ((item: T, index: number) => void) | undefined;
}

export type DataTableProps<T> = DataTableBaseProps<T> &
  (VirtualListRowSizing | { readonly itemHeight?: never; readonly estimatedItemHeight?: never });

const RESIZE_STEP = 8;
const DEFAULT_BODY_HEIGHT = 320;

function widthBounds<T>(column: DataTableColumn<T>) {
  const min = column.minWidth ?? 72;
  return { min, max: Math.max(min, column.maxWidth ?? Infinity) };
}

function clampWidth<T>(column: DataTableColumn<T>, width: number): number {
  const { min, max } = widthBounds(column);
  return Math.min(max, Math.max(min, width));
}

function resizeGrip<T>(
  column: DataTableColumn<T>,
  width: Accessor<number>,
  resize: (value: number) => void,
): NativeNode {
  const grip = createElement("box");
  const { min, max } = widthBounds(column);
  let drag: { pointerId: string; x: number; width: number; last: number } | undefined;
  const move = (event: NativePointerEvent) => {
    if (!drag || drag.pointerId !== event.pointerId) return;
    const next = clampWidth(column, drag.width + event.position.x - drag.x);
    if (next === drag.last) return;
    drag.last = next;
    resize(next);
  };
  const release = (event: NativePointerEvent) => {
    drag = undefined;
    event.releasePointer();
  };
  spread(
    grip,
    {
      class: "zui-data-table__grip",
      role: "splitter",
      tabIndex: 0,
      orientation: "vertical",
      ariaLabel: `Resize ${column.id} column`,
      ariaValueMin: min,
      ariaValueMax: Number.isFinite(max) ? max : undefined,
      ariaValueStep: RESIZE_STEP,
      get ariaValueNow() {
        return width();
      },
      onKeyDown(event) {
        if (event.isComposing || (event.key !== "ArrowLeft" && event.key !== "ArrowRight")) return;
        event.preventDefault();
        resize(
          clampWidth(column, width() + (event.key === "ArrowRight" ? RESIZE_STEP : -RESIZE_STEP)),
        );
      },
      onPointerDown(event) {
        if (
          drag ||
          !event.primary ||
          (event.button !== "primary" && !(event.button === null && event.pointerKind !== "mouse"))
        )
          return;
        const initial = width();
        drag = { pointerId: event.pointerId, x: event.position.x, width: initial, last: initial };
        event.capturePointer();
        focus(grip);
        event.preventDefault();
      },
      onPointerMove: move,
      onPointerUp(event) {
        if (drag?.pointerId !== event.pointerId) return;
        move(event);
        release(event);
      },
      onPointerCancel(event) {
        if (drag?.pointerId === event.pointerId) release(event);
      },
    } satisfies NativeElementProps,
    true,
  );
  return grip;
}

export function DataTable<T>(props: DataTableProps<T>): NativeNode {
  const fixed = untrack(() => props.itemHeight !== undefined);
  const measured = untrack(() => props.estimatedItemHeight !== undefined);
  if (fixed && measured) throw new TypeError("DataTable accepts only one row height mode");
  useStylesheet(dataSheet);
  useStylesheet(tableSheet);
  const [sort, setSort] = createControllable(
    {
      value: () => props.sort,
      defaultValue: () => props.defaultSort,
      onChange: () => props.onSortChange,
    },
    null,
  );
  const [widths, setWidths] = createControllable<Readonly<Record<string, number>>>(
    {
      value: () => props.columnWidths,
      defaultValue: () => props.defaultColumnWidths,
      onChange: () => props.onColumnWidthsChange,
    },
    {},
  );
  const columnWidth = (column: DataTableColumn<T>) => {
    const value = widths()[column.id];
    return clampWidth(column, typeof value === "number" ? value : (column.width ?? 160));
  };
  const layout = createMemo(() => {
    let total = 0;
    const tracks = props.columns.map(column => {
      const width = columnWidth(column);
      total += width;
      return `${width}px`;
    });
    return { minWidth: `${total}px`, "--zui-table-columns": tracks.join(" ") || undefined };
  });
  const rows = createMemo(() => {
    const data = props.data;
    const current = sort();
    if (!current) return data;
    const column = props.columns.find(candidate => candidate.id === current.columnId);
    if (!column?.sortable) return data;
    const compare = props.sortRows;
    if (compare) return [...data].sort((a, b) => compare(a, b, current));
    const key = column.sortAccessor;
    if (!key) return data;
    const direction = current.direction === "asc" ? 1 : -1;
    return [...data].sort((a, b) => {
      const av = key(a);
      const bv = key(b);
      if (typeof av === "number" && typeof bv === "number") return (av - bv) * direction;
      const left = String(av);
      const right = String(bv);
      return (left < right ? -1 : left > right ? 1 : 0) * direction;
    });
  });
  const hasRows = createMemo(() => rows().length > 0);
  const [active, setActive] = createSignal(0);
  const activeIndex = createMemo(() => Math.min(active(), rows().length - 1));
  const [activeRow, setActiveRow] = createSignal<{
    readonly semantic: NativeNode;
    readonly layout: NativeNode;
  } | null>(null);
  let controller: VirtualListController | undefined;
  let reveal: number | undefined;
  const root = createElement("column");
  const rest = omit(
    props,
    "columns",
    "data",
    "sort",
    "defaultSort",
    "onSortChange",
    "sortRows",
    "columnWidths",
    "defaultColumnWidths",
    "onColumnWidthsChange",
    "itemHeight",
    "estimatedItemHeight",
    "height",
    "empty",
    "toolbar",
    "onRowActivate",
    "onKeyDown",
  );
  const rootProps = mergeProps(rest, withDefaults({ role: "grid" as const, tabIndex: 0 }, rest), {
    __zguiClass: dataSheet.className,
    get ariaActiveDescendant() {
      return activeRow()?.semantic ?? null;
    },
    onKeyDown(event: NativeKeyEvent) {
      if (!callHandler(event, props.onKeyDown) || event.target !== root || event.isComposing)
        return;
      const { alt, control, meta, shift } = event.modifiers;
      if (alt || control || meta || shift) return;
      const data = rows();
      const index = activeIndex();
      if (index < 0) return;
      let next: number;
      if (event.key === "ArrowDown") next = Math.min(data.length - 1, index + 1);
      else if (event.key === "ArrowUp") next = Math.max(0, index - 1);
      else if (event.key === "Home") next = 0;
      else if (event.key === "End") next = data.length - 1;
      else if (event.key === "Enter" || event.key === "Space" || event.key === " ") {
        if (!props.onRowActivate) return;
        event.preventDefault();
        if (!event.repeat) props.onRowActivate(data[index]!, index);
        return;
      } else return;
      event.preventDefault();
      setActive(next);
      if (controller) controller.scrollToIndex(next);
      else if (next === index) {
        const layoutRow = activeRow()?.layout;
        if (layoutRow) scrollIntoView(layoutRow, { block: "nearest" });
      } else reveal = next;
    },
    get children() {
      return [toolbar(), frame];
    },
  });

  const header = element("box", {
    __zguiClass: tableSheet.className,
    children: TableRow({
      children: For({
        get each() {
          return props.columns;
        },
        children: column => {
          let sortButton = column.header;
          if (column.sortable) {
            const direction = createMemo(() => {
              const current = sort();
              return current?.columnId === column.id ? current.direction : undefined;
            });
            const arrow = createMemo(() => {
              const state = direction();
              return state
                ? Icon({
                    icon: state === "asc" ? ARROW_UP : ARROW_DOWN,
                    size: "sm",
                    class: "zui-data-table__arrow",
                  })
                : null;
            });
            sortButton = element("control", {
              class: "zui-data-table__sort",
              role: "button",
              tabIndex: 0,
              get ariaDescription() {
                const state = direction();
                return state === "asc"
                  ? "Sorted ascending"
                  : state === "desc"
                    ? "Sorted descending"
                    : "Not sorted";
              },
              onClick() {
                const state = direction();
                setSort(
                  state === "desc"
                    ? null
                    : { columnId: column.id, direction: state === "asc" ? "desc" : "asc" },
                );
              },
              children: [column.header, arrow],
            });
          }
          const grip = column.resizable
            ? resizeGrip(
                column,
                () => columnWidth(column),
                next => {
                  if (next !== columnWidth(column)) setWidths({ ...widths(), [column.id]: next });
                },
              )
            : null;
          return TableHead({ align: column.align, children: [sortButton, grip] });
        },
      }),
    }),
  });

  const renderRow = (item: T, index: Accessor<number>) => {
    const selected = createMemo(() => index() === activeIndex());
    const row = TableRow({
      get selected() {
        return selected();
      },
      children: For({
        get each() {
          return props.columns;
        },
        children: column =>
          TableCell({ role: "gridCell", align: column.align, children: column.cell(item, index) }),
      }),
    });
    const layoutRow = element("box", {
      __zguiClass: `${tableSheet.className} zui-table__body`,
      __zguiStyle: { flexShrink: 0 },
      children: row,
    });
    createEffect(selected, isActive => {
      if (!isActive) return;
      setActiveRow({ semantic: row, layout: layoutRow });
      if (reveal !== undefined && reveal === untrack(index)) {
        reveal = undefined;
        scrollIntoView(layoutRow, { block: "nearest" });
      }
      return () => setActiveRow(current => (current?.semantic === row ? null : current));
    });
    return layoutRow;
  };
  const empty = () =>
    element("box", {
      class: "zui-data-table__empty",
      role: "status",
      get children() {
        return props.empty ?? "No results.";
      },
    });
  const bodyProps = {
    class: "zui-data-table__grid",
    role: "rowGroup" as const,
    style: { border: "none", "--zgui-scrollbar-mode": "overlay" },
    get "--zui-data-table-height"() {
      const height = props.height ?? (fixed || measured ? DEFAULT_BODY_HEIGHT : undefined);
      return height === undefined ? undefined : `${height}px`;
    },
    get "--zui-data-table-row"() {
      return fixed ? `${props.itemHeight}px` : undefined;
    },
    "data-virtualized": fixed ? "true" : undefined,
  } as const;
  const listProps = mergeProps(bodyProps, {
    get each() {
      return rows();
    },
    get fallback() {
      return empty();
    },
    initialHeight: untrack(() => props.height ?? DEFAULT_BODY_HEIGHT),
    controller: (handle: VirtualListController) => {
      controller = handle;
    },
    children: renderRow,
  });
  const scroller = fixed
    ? VirtualList<T>(
        mergeProps(listProps, {
          get itemHeight() {
            return props.itemHeight!;
          },
        }),
      )
    : measured
      ? VirtualList<T>(
          mergeProps(listProps, {
            get estimatedItemHeight() {
              return props.estimatedItemHeight!;
            },
          }),
        )
      : element(
          "scroll",
          mergeProps(bodyProps, {
            get children() {
              return hasRows()
                ? For({
                    get each() {
                      return rows();
                    },
                    children: renderRow,
                  })
                : empty();
            },
          }),
        );

  const frame = element("scroll", {
    __zguiStyle: {
      width: "100%",
      minWidth: "0px",
      overflowX: "auto",
      overflowY: "hidden",
      border: "1px solid var(--zui-color-border)",
      borderRadius: "var(--zui-radius-md)",
    },
    children: element("column", {
      get __zguiStyle() {
        return layout();
      },
      children: [header, scroller],
    }),
  });
  const toolbar = createMemo(() => {
    const children = props.toolbar;
    return children == null ? null : element("row", { class: "zui-data-table__toolbar", children });
  });
  spread(root, rootProps);
  return root;
}
