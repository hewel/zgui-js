import { For, createMemo, createSignal, omit } from "#solid";
import { createElement, mergeProps, setProp, spread } from "./renderer/index.js";
import { append, slot, withDefaults } from "./components/shared.js";
import { getReducedMotion, observeGeometry, scrollTo } from "./nodes.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeNode,
  ScrollBehavior,
} from "./types.js";

export type VirtualScrollAlign = "auto" | "start" | "center" | "end";

export interface VirtualScrollToIndexOptions {
  readonly align?: VirtualScrollAlign | undefined;
  readonly behavior?: ScrollBehavior | undefined;
}

export interface VirtualListController {
  /** Scroll the list/grid so the item at `index` (clamped to valid range) is aligned in view. */
  scrollToIndex(index: number, options?: VirtualScrollToIndexOptions): void;
}

export interface VirtualListBaseProps<T> extends Omit<NativeElementProps, "children"> {
  /** Data array to virtualize. */
  readonly each: readonly T[];
  /** Fixed row height in logical pixels (must be finite and positive). */
  readonly itemHeight: number;
  /** Vertical gap between rows in logical pixels (default 0). */
  readonly gap?: number | undefined;
  /** Extra rows mounted above and below the visible scrollport (default 3). */
  readonly overscan?: number | undefined;
  /** Fallback scrollport height before first layout observation (default 480). */
  readonly initialHeight?: number | undefined;
  /** Optional imperative handle callback receiving `{ scrollToIndex }`. */
  readonly controller?: ((handle: VirtualListController) => void) | undefined;
  /** Optional fallback rendered when `each` is empty. */
  readonly fallback?: NativeChild | undefined;
}

export interface VirtualListIdentityProps<T> extends VirtualListBaseProps<T> {
  readonly keyed?: true | undefined;
  /** Row renderer receiving the item and its 0-based index in `each`. */
  readonly children: (item: T, index: Accessor<number>) => NativeChild;
}

export interface VirtualListKeyedProps<T> extends VirtualListBaseProps<T> {
  /** Stable key extractor; matches Solid `<For keyed={...}>` and passes an item accessor to `children`. */
  readonly keyed: (item: T) => unknown;
  /** Row renderer receiving a reactive item accessor and its 0-based index in `each`. */
  readonly children: (item: Accessor<T>, index: Accessor<number>) => NativeChild;
}

export type VirtualListProps<T> = VirtualListIdentityProps<T> | VirtualListKeyedProps<T>;

export interface VirtualGridBaseProps<T> extends Omit<NativeElementProps, "children"> {
  /** Data array to virtualize across rows of columns. */
  readonly each: readonly T[];
  /** Fixed row height in logical pixels (must be finite and positive). */
  readonly itemHeight: number;
  /** Fixed number of columns per row (takes precedence over `minColumnWidth`). */
  readonly columns?: number | undefined;
  /** Minimum column width in logical pixels used to auto-fit columns to scrollport width (default 140). */
  readonly minColumnWidth?: number | undefined;
  /** Horizontal and vertical gap between cells in logical pixels (default 0). */
  readonly gap?: number | undefined;
  /** Extra rows mounted above and below the visible scrollport (default 2). */
  readonly overscan?: number | undefined;
  /** Fallback scrollport width before first layout observation (default 640). */
  readonly initialWidth?: number | undefined;
  /** Fallback scrollport height before first layout observation (default 480). */
  readonly initialHeight?: number | undefined;
  /** Optional imperative handle callback receiving `{ scrollToIndex }`. */
  readonly controller?: ((handle: VirtualListController) => void) | undefined;
  /** Optional fallback rendered when `each` is empty. */
  readonly fallback?: NativeChild | undefined;
}

export interface VirtualGridIdentityProps<T> extends VirtualGridBaseProps<T> {
  readonly keyed?: true | undefined;
  /** Cell renderer receiving the item and its 0-based index in `each`. */
  readonly children: (item: T, index: Accessor<number>) => NativeChild;
}

export interface VirtualGridKeyedProps<T> extends VirtualGridBaseProps<T> {
  /** Stable key extractor; matches Solid `<For keyed={...}>` and passes an item accessor to `children`. */
  readonly keyed: (item: T) => unknown;
  /** Cell renderer receiving a reactive item accessor and its 0-based index in `each`. */
  readonly children: (item: Accessor<T>, index: Accessor<number>) => NativeChild;
}

export type VirtualGridProps<T> = VirtualGridIdentityProps<T> | VirtualGridKeyedProps<T>;

function validatePositive(value: number, name: string): number {
  if (!Number.isFinite(value) || value <= 0) {
    throw new RangeError(`${name} must be a finite positive number`);
  }
  return value;
}

function validateNonNegative(value: number, name: string): number {
  if (!Number.isFinite(value) || value < 0) {
    throw new RangeError(`${name} must be a finite non-negative number`);
  }
  return value;
}

function validateOverscan(value: number): number {
  if (!Number.isInteger(value) || value < 0) {
    throw new RangeError("VirtualList overscan must be a non-negative integer");
  }
  return value;
}

function computeWindow(
  totalRows: number,
  rowHeight: number,
  gap: number,
  overscan: number,
  scrollTop: number,
  viewportHeight: number,
): { start: number; end: number; topSpacer: number; bottomSpacer: number } {
  if (totalRows <= 0) return { start: 0, end: 0, topSpacer: 0, bottomSpacer: 0 };
  const stride = rowHeight + gap;
  const totalHeight = Math.max(0, totalRows * stride - gap);
  const maxScroll = Math.max(0, totalHeight - viewportHeight);
  const clampedScroll = Math.min(Math.max(0, scrollTop), maxScroll);
  const start = Math.max(0, Math.min(totalRows - 1, Math.floor(clampedScroll / stride) - overscan));
  const visibleRows = Math.max(1, Math.ceil(viewportHeight / stride) + 1);
  const end = Math.min(totalRows, start + visibleRows + overscan * 2);
  const topSpacer = start * stride;
  const bottomSpacer = (totalRows - end) * stride;
  return { start, end, topSpacer, bottomSpacer };
}

function computeTargetScrollTop(
  rowIndex: number,
  totalRows: number,
  rowHeight: number,
  gap: number,
  currentScrollTop: number,
  viewportHeight: number,
  align: VirtualScrollAlign,
): number | null {
  if (totalRows <= 0) return null;
  const stride = rowHeight + gap;
  const totalHeight = Math.max(0, totalRows * stride - gap);
  const maxScroll = Math.max(0, totalHeight - viewportHeight);
  const rowTop = rowIndex * stride;
  const rowBottom = rowTop + rowHeight;
  let target: number;
  if (align === "start") target = rowTop;
  else if (align === "end") target = rowBottom - viewportHeight;
  else if (align === "center") target = rowTop - (viewportHeight - rowHeight) / 2;
  else if (align === "auto") {
    if (rowTop < currentScrollTop) target = rowTop;
    else if (rowBottom > currentScrollTop + viewportHeight) target = rowBottom - viewportHeight;
    else return null;
  } else {
    throw new TypeError(`unsupported VirtualList scrollToIndex align: ${String(align)}`);
  }
  return Math.max(0, Math.min(maxScroll, target));
}

function spacer(height: Accessor<number>): NativeNode {
  const node = createElement("box");
  spread(
    node,
    {
      get style() {
        const h = height();
        return { height: `${h}px`, flexShrink: 0, display: h > 0 ? "flex" : "none" } as const;
      },
    },
    true,
  );
  return node;
}

function renderMappedItems<T>(
  items: Accessor<readonly T[]>,
  startIndex: Accessor<number>,
  keyed: true | ((item: T) => unknown) | undefined,
  renderItem:
    | ((item: T, index: Accessor<number>) => NativeChild)
    | ((item: Accessor<T>, index: Accessor<number>) => NativeChild),
): NativeChild {
  if (typeof keyed === "function") {
    const renderKeyed = renderItem as (item: Accessor<T>, index: Accessor<number>) => NativeChild;
    return For<T>({
      get each() {
        return items();
      },
      keyed,
      children: (item, localIndex) => renderKeyed(item, () => startIndex() + localIndex()),
    });
  }
  const renderIdentity = renderItem as (item: T, index: Accessor<number>) => NativeChild;
  return For<T>({
    get each() {
      return items();
    },
    children: (item, localIndex) => renderIdentity(item, () => startIndex() + localIndex()),
  });
}

/**
 * Windowed vertical list backed by a native `<scroll>` container and `observeGeometry("scrollPosition")`.
 * Mounts only visible rows plus `overscan`, keeping QuickJS turn times constant for large lists.
 */
export function VirtualList<T>(props: VirtualListIdentityProps<T>): NativeNode;
export function VirtualList<T>(props: VirtualListKeyedProps<T>): NativeNode;
export function VirtualList<T>(props: VirtualListProps<T>): NativeNode {
  const rest = omit(
    props,
    "each",
    "itemHeight",
    "gap",
    "overscan",
    "initialHeight",
    "controller",
    "keyed",
    "fallback",
    "children",
  );
  const scroller = createElement("scroll");
  spread(scroller, mergeProps(rest, withDefaults({ role: "list" as const }, rest)), true);

  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewportHeight, setViewportHeight] = createSignal(
    validatePositive(props.initialHeight ?? 480, "VirtualList initialHeight"),
  );

  observeGeometry(scroller, "scrollPosition", pos => {
    const nextTop = Math.max(0, pos.offset.y);
    if (nextTop !== scrollTop()) setScrollTop(nextTop);
    if (pos.scrollport.height > 0 && pos.scrollport.height !== viewportHeight()) {
      setViewportHeight(pos.scrollport.height);
    }
  });

  const metrics = createMemo(() => {
    const itemHeight = validatePositive(props.itemHeight, "VirtualList itemHeight");
    const gap = validateNonNegative(props.gap ?? 0, "VirtualList gap");
    const overscan = validateOverscan(props.overscan ?? 3);
    const items = props.each;
    return {
      itemHeight,
      gap,
      items,
      ...computeWindow(items.length, itemHeight, gap, overscan, scrollTop(), viewportHeight()),
    };
  });

  props.controller?.({
    scrollToIndex(index, options) {
      if (!Number.isFinite(index)) {
        throw new RangeError("VirtualList scrollToIndex index must be a finite number");
      }
      const { items, itemHeight, gap } = metrics();
      if (items.length === 0) return;
      const clamped = Math.max(0, Math.min(items.length - 1, Math.trunc(index)));
      const targetTop = computeTargetScrollTop(
        clamped,
        items.length,
        itemHeight,
        gap,
        scrollTop(),
        viewportHeight(),
        options?.align ?? "auto",
      );
      if (targetTop === null) return;
      const behavior = options?.behavior ?? "instant";
      if (behavior === "instant" || getReducedMotion()) setScrollTop(targetTop);
      scrollTo(scroller, { top: targetTop, behavior });
    },
  });

  const visibleItems = createMemo<readonly T[]>(() => {
    const { items, start, end } = metrics();
    return items.slice(start, end);
  });

  const content = createElement("column");
  setProp(content, "style", { width: "100%" });
  const topBox = spacer(() => metrics().topSpacer);
  const bottomBox = spacer(() => metrics().bottomSpacer);
  const body = createElement("column");
  spread(
    body,
    {
      get style() {
        const gap = metrics().gap;
        return gap > 0 ? { width: "100%", gap: `${gap}px` } : { width: "100%" };
      },
    },
    true,
  );

  slot(body, () =>
    metrics().items.length === 0
      ? (props.fallback ?? null)
      : renderMappedItems(visibleItems, () => metrics().start, props.keyed, props.children),
  );
  append(content, topBox);
  append(content, body);
  append(content, bottomBox);
  append(scroller, content);
  return scroller;
}

interface GridRow<T> {
  readonly rowIndex: number;
  readonly rowStart: number;
  readonly items: readonly T[];
}

/**
 * Windowed 2D grid backed by a native `<scroll>` container and `observeGeometry("scrollPosition")`.
 * Virtualizes by row so grids with hundreds or thousands of tiles stay within QuickJS turn budgets.
 */
export function VirtualGrid<T>(props: VirtualGridIdentityProps<T>): NativeNode;
export function VirtualGrid<T>(props: VirtualGridKeyedProps<T>): NativeNode;
export function VirtualGrid<T>(props: VirtualGridProps<T>): NativeNode {
  const rest = omit(
    props,
    "each",
    "itemHeight",
    "columns",
    "minColumnWidth",
    "gap",
    "overscan",
    "initialWidth",
    "initialHeight",
    "controller",
    "keyed",
    "fallback",
    "children",
  );
  const scroller = createElement("scroll");
  spread(scroller, mergeProps(rest, withDefaults({ role: "grid" as const }, rest)), true);

  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewportWidth, setViewportWidth] = createSignal(
    validatePositive(props.initialWidth ?? 640, "VirtualGrid initialWidth"),
  );
  const [viewportHeight, setViewportHeight] = createSignal(
    validatePositive(props.initialHeight ?? 480, "VirtualGrid initialHeight"),
  );

  observeGeometry(scroller, "scrollPosition", pos => {
    const nextTop = Math.max(0, pos.offset.y);
    if (nextTop !== scrollTop()) setScrollTop(nextTop);
    if (pos.scrollport.width > 0 && pos.scrollport.width !== viewportWidth()) {
      setViewportWidth(pos.scrollport.width);
    }
    if (pos.scrollport.height > 0 && pos.scrollport.height !== viewportHeight()) {
      setViewportHeight(pos.scrollport.height);
    }
  });

  const metrics = createMemo(() => {
    const itemHeight = validatePositive(props.itemHeight, "VirtualGrid itemHeight");
    const gap = validateNonNegative(props.gap ?? 0, "VirtualGrid gap");
    const overscan = validateOverscan(props.overscan ?? 2);
    let cols: number;
    if (props.columns !== undefined) {
      if (!Number.isInteger(props.columns) || props.columns < 1) {
        throw new RangeError("VirtualGrid columns must be a positive integer");
      }
      cols = props.columns;
    } else {
      const minWidth = validatePositive(props.minColumnWidth ?? 140, "VirtualGrid minColumnWidth");
      cols = Math.max(1, Math.floor((viewportWidth() + gap) / (minWidth + gap)));
    }
    const items = props.each;
    const totalRows = Math.ceil(items.length / cols);
    return {
      itemHeight,
      gap,
      cols,
      totalRows,
      items,
      ...computeWindow(totalRows, itemHeight, gap, overscan, scrollTop(), viewportHeight()),
    };
  });

  props.controller?.({
    scrollToIndex(index, options) {
      if (!Number.isFinite(index)) {
        throw new RangeError("VirtualGrid scrollToIndex index must be a finite number");
      }
      const { items, itemHeight, gap, cols, totalRows } = metrics();
      if (items.length === 0) return;
      const clamped = Math.max(0, Math.min(items.length - 1, Math.trunc(index)));
      const rowIndex = Math.floor(clamped / cols);
      const targetTop = computeTargetScrollTop(
        rowIndex,
        totalRows,
        itemHeight,
        gap,
        scrollTop(),
        viewportHeight(),
        options?.align ?? "auto",
      );
      if (targetTop === null) return;
      const behavior = options?.behavior ?? "instant";
      if (behavior === "instant" || getReducedMotion()) setScrollTop(targetTop);
      scrollTo(scroller, { top: targetTop, behavior });
    },
  });

  const visibleRows = createMemo<readonly GridRow<T>[]>(() => {
    const { items, cols, start, end } = metrics();
    const rows: GridRow<T>[] = [];
    for (let rowIndex = start; rowIndex < end; rowIndex += 1) {
      const rowStart = rowIndex * cols;
      const rowEnd = Math.min(items.length, rowStart + cols);
      rows.push({ rowIndex, rowStart, items: items.slice(rowStart, rowEnd) });
    }
    return rows;
  });

  const content = createElement("column");
  setProp(content, "style", { width: "100%" });
  const topBox = spacer(() => metrics().topSpacer);
  const bottomBox = spacer(() => metrics().bottomSpacer);
  const body = createElement("column");
  spread(
    body,
    {
      get style() {
        const gap = metrics().gap;
        return gap > 0 ? { width: "100%", gap: `${gap}px` } : { width: "100%" };
      },
    },
    true,
  );

  slot(body, () =>
    metrics().items.length === 0
      ? (props.fallback ?? null)
      : For<GridRow<T>>({
          get each() {
            return visibleRows();
          },
          keyed: row => row.rowIndex,
          children: row => {
            const rowNode = createElement("row");
            spread(
              rowNode,
              {
                get style() {
                  const { gap, itemHeight } = metrics();
                  return {
                    width: "100%",
                    height: `${itemHeight}px`,
                    flexShrink: 0,
                    ...(gap > 0 ? { gap: `${gap}px` } : {}),
                  };
                },
              },
              true,
            );
            slot(rowNode, () =>
              renderMappedItems(
                () => row().items,
                () => row().rowStart,
                props.keyed,
                props.children,
              ),
            );
            return rowNode;
          },
        }),
  );
  append(content, topBox);
  append(content, body);
  append(content, bottomBox);
  append(scroller, content);
  return scroller;
}
