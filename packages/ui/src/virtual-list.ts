import { For, createMemo, createSignal, omit, untrack } from "#solid";
import { createElement, mergeProps, setProp, spread } from "./renderer/index.js";
import { append, slot, withDefaults } from "./components/shared.js";
import { callHandler } from "./components/state.js";
import { getReducedMotion, observeGeometry, scrollTo } from "./nodes.js";
import { createDelay } from "./timers.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeNode,
  NativePointerDownEvent,
  NativeWheelEvent,
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

/** Use a fixed stride, or measure mounted rows starting from an estimated height. */
export type VirtualListRowSizing =
  | { readonly itemHeight: number; readonly estimatedItemHeight?: never }
  | { readonly itemHeight?: never; readonly estimatedItemHeight: number };

export interface VirtualListBaseProps<T> extends Omit<NativeElementProps, "children"> {
  /** Data array to virtualize. */
  readonly each: readonly T[];
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

interface VirtualListIdentity<T> extends VirtualListBaseProps<T> {
  readonly keyed?: true | undefined;
  /** Row renderer receiving the item and its 0-based index in `each`. */
  readonly children: (item: T, index: Accessor<number>) => NativeChild;
}

interface VirtualListKeyed<T> extends VirtualListBaseProps<T> {
  /** Stable key extractor; matches Solid `<For keyed={...}>` and passes an item accessor to `children`. */
  readonly keyed: (item: T) => unknown;
  /** Row renderer receiving a reactive item accessor and its 0-based index in `each`. */
  readonly children: (item: Accessor<T>, index: Accessor<number>) => NativeChild;
}

export type VirtualListIdentityProps<T> = VirtualListIdentity<T> & VirtualListRowSizing;
export type VirtualListKeyedProps<T> = VirtualListKeyed<T> & VirtualListRowSizing;

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
  return alignScrollTop(
    rowTop,
    rowTop + rowHeight,
    currentScrollTop,
    viewportHeight,
    maxScroll,
    align,
  );
}

function alignScrollTop(
  rowTop: number,
  rowBottom: number,
  currentScrollTop: number,
  viewportHeight: number,
  maxScroll: number,
  align: VirtualScrollAlign,
): number | null {
  let target: number;
  if (align === "start") target = rowTop;
  else if (align === "end") target = rowBottom - viewportHeight;
  else if (align === "center") target = (rowTop + rowBottom - viewportHeight) / 2;
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

/** Prefix sums of row strides (including a gap after each row). */
class RowOffsets {
  private readonly sums: Float64Array;
  private readonly firstBit: number;

  constructor(length: number, stride: number, strideAt?: (index: number) => number) {
    this.sums = new Float64Array(length + 1);
    this.firstBit = length > 0 ? 2 ** Math.floor(Math.log2(length)) : 0;
    if (!strideAt) {
      for (let i = 1; i <= length; i++) this.sums[i] = (i & -i) * stride;
      return;
    }
    // Each completed subtotal contributes once to its parent: rebuild in linear time.
    for (let i = 1; i <= length; i++) {
      this.sums[i]! += strideAt(i - 1);
      const parent = i + (i & -i);
      if (parent <= length) this.sums[parent]! += this.sums[i]!;
    }
  }

  add(index: number, delta: number): void {
    for (let i = index + 1; i < this.sums.length; i += i & -i) this.sums[i]! += delta;
  }

  before(count: number): number {
    let sum = 0;
    for (let i = count; i > 0; i -= i & -i) sum += this.sums[i]!;
    return sum;
  }

  at(offset: number): number {
    let count = 0;
    // Find the longest prefix at or below offset by descending the Fenwick tree once.
    for (let bit = this.firstBit; bit > 0; bit >>= 1) {
      const next = count + bit;
      if (next < this.sums.length && this.sums[next]! <= offset) {
        offset -= this.sums[next]!;
        count = next;
      }
    }
    return Math.min(count, this.sums.length - 2);
  }
}

interface MeasuredListModel<T> {
  readonly items: readonly T[];
  readonly gap: number;
  readonly estimate: number;
  readonly offsets: RowOffsets;
}

interface MeasuredScrollRequest {
  readonly offsets: RowOffsets;
  readonly index: number;
  readonly align: VirtualScrollAlign;
  readonly smoothTop?: number | undefined;
}

function mountMeasuredList<T>(
  scroller: NativeNode,
  props: VirtualListProps<T>,
  nativeProps: NativeElementProps,
  initialHeight: number,
): NativeNode {
  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewportHeight, setViewportHeight] = createSignal(initialHeight);
  const [version, invalidate] = createSignal(0);
  const realign = createDelay();
  let heights = new Map<unknown, number>();
  const keyedProp = untrack(() => props.keyed);
  const keyed = typeof keyedProp === "function" ? keyedProp : undefined;
  const keyAt = (items: readonly T[], index: number) => (keyed ? keyed(items[index]!) : index);
  const model = createMemo<MeasuredListModel<T>>(() => {
    const items = props.each;
    const gap = validateNonNegative(props.gap ?? 0, "VirtualList gap");
    const estimate = validatePositive(
      props.estimatedItemHeight!,
      "VirtualList estimatedItemHeight",
    );
    const stride = estimate + gap;
    if (heights.size === 0) {
      return { items, gap, estimate, offsets: new RowOffsets(items.length, stride) };
    }
    const retained = new Map<unknown, number>();
    const offsets = new RowOffsets(items.length, stride, index => {
      const key = keyAt(items, index);
      const height = heights.get(key);
      if (height === undefined) return stride;
      retained.set(key, height);
      return height + gap;
    });
    heights = retained;
    return { items, gap, estimate, offsets };
  });
  let pending: MeasuredScrollRequest | undefined;
  const cancelJump = () => {
    if (!pending) return;
    realign.cancel();
    pending = undefined;
  };
  const maxScroll = ({ items, gap, offsets }: MeasuredListModel<T>, viewport = viewportHeight()) =>
    Math.max(0, offsets.before(items.length) - gap - viewport);
  const jump = (top: number, behavior: ScrollBehavior) => {
    if (behavior === "instant" || getReducedMotion()) setScrollTop(top);
    scrollTo(scroller, { top, behavior });
  };
  const target = (
    current: MeasuredListModel<T>,
    index: number,
    align: VirtualScrollAlign,
  ): number | null => {
    const top = current.offsets.before(index);
    const bottom = current.offsets.before(index + 1) - current.gap;
    const viewport = viewportHeight();
    return alignScrollTop(top, bottom, scrollTop(), viewport, maxScroll(current, viewport), align);
  };
  const scheduleRealignment = (request: MeasuredScrollRequest) => {
    // Other rows in this layout can report after the target. Align once after those samples.
    realign.schedule(0, () => {
      if (pending !== request) return;
      pending = undefined;
      const current = model();
      if (current.offsets !== request.offsets) return;
      if (request.smoothTop !== undefined && needsSample(current, request.index)) {
        // The live prefix may have moved the target outside the native glide's final window.
        pending = { offsets: request.offsets, index: request.index, align: request.align };
      }
      const next = target(current, request.index, request.align);
      if (next !== null && next !== scrollTop()) jump(next, "instant");
    });
  };

  spread(
    scroller,
    mergeProps(nativeProps, {
      onWheel(event: NativeWheelEvent) {
        if (callHandler(event, props.onWheel)) cancelJump();
      },
      onPointerDown(event: NativePointerDownEvent) {
        if (callHandler(event, props.onPointerDown)) cancelJump();
      },
    }),
    true,
  );

  observeGeometry(scroller, "scrollPosition", pos => {
    const offset = Math.max(0, pos.offset.y);
    setScrollTop(offset);
    if (pos.scrollport.height > 0) setViewportHeight(pos.scrollport.height);
    const request = pending;
    if (request?.smoothTop !== undefined) {
      // Measurements can change total height before the native animation reaches its destination.
      const limit = Math.max(0, pos.contentSize.height - pos.scrollport.height);
      if (Math.abs(offset - Math.min(request.smoothTop, limit)) <= 1) scheduleRealignment(request);
    }
  });

  const range = createMemo(() => {
    const current = model();
    const { items, offsets } = current;
    if (items.length === 0) return { start: 0, end: 0, top: 0, bottom: 0 };
    version();
    const overscan = validateOverscan(props.overscan ?? 3);
    const top = Math.min(scrollTop(), maxScroll(current));
    const start = Math.max(0, offsets.at(top) - overscan);
    const end = Math.min(items.length, offsets.at(top + viewportHeight()) + 1 + overscan);
    return {
      start,
      end,
      top: offsets.before(start),
      bottom: offsets.before(items.length) - offsets.before(end),
    };
  });
  const start = createMemo(() => range().start);
  const end = createMemo(() => range().end);
  const top = createMemo(() => range().top);
  const bottom = createMemo(() => range().bottom);
  const visible = createMemo(() => model().items.slice(start(), end()));
  const hasItems = createMemo(() => model().items.length > 0);

  const needsSample = (current: MeasuredListModel<T>, row: number) =>
    row < start() || row >= end() || !heights.has(keyAt(current.items, row));
  const onController = untrack(() => props.controller);
  onController?.({
    scrollToIndex(index, options) {
      if (!Number.isFinite(index)) {
        throw new RangeError("VirtualList scrollToIndex index must be a finite number");
      }
      const current = model();
      cancelJump();
      if (current.items.length === 0) return;
      const row = Math.max(0, Math.min(current.items.length - 1, Math.trunc(index)));
      const align = options?.align ?? "auto";
      const top = target(current, row, align);
      const behavior = options?.behavior ?? "instant";
      const smoothTop =
        top !== null && top !== scrollTop() && behavior === "smooth" && !getReducedMotion()
          ? top
          : undefined;
      if (smoothTop !== undefined || needsSample(current, row)) {
        pending = { offsets: current.offsets, index: row, align, smoothTop };
      }
      if (top !== null) jump(top, behavior);
    },
  });

  const measure = (index: Accessor<number>, children: NativeChild) => {
    const row = createElement("column");
    setProp(row, "style", { width: "100%", flexShrink: 0 });
    observeGeometry(row, "borderBox", ({ height }) => {
      const current = model();
      const position = index();
      if (position < 0 || position >= current.items.length) return;
      const key = keyAt(current.items, position);
      const previous = heights.get(key);
      const delta = height - (previous ?? current.estimate);
      heights.set(key, height);
      const request = pending?.offsets === current.offsets ? pending : undefined;
      if (delta !== 0) {
        const offset = scrollTop();
        const aboveAnchor =
          request?.smoothTop === undefined && position < current.offsets.at(offset);
        current.offsets.add(position, delta);
        invalidate(value => value + 1);
        if (aboveAnchor) jump(Math.max(0, Math.min(maxScroll(current), offset + delta)), "instant");
      }
      if (request?.index === position && request.smoothTop === undefined && !realign.pending()) {
        if (delta !== 0) scheduleRealignment(request);
        else cancelJump();
      }
    });
    slot(row, children);
    return row;
  };
  const content = createElement("column");
  setProp(content, "style", { width: "100%" });
  const body = createElement("column");
  const gap = createMemo(() => model().gap);
  spread(
    body,
    {
      get style() {
        return { width: "100%", gap: `${gap()}px` };
      },
    },
    true,
  );
  slot(body, () => {
    if (!hasItems()) return props.fallback ?? null;
    if (typeof props.keyed === "function") {
      const { keyed: getKey, children } = props;
      return renderMappedItems(
        visible,
        start,
        getKey,
        (item: Accessor<T>, index: Accessor<number>) => measure(index, children(item, index)),
      );
    }
    const children = props.children;
    return renderMappedItems(visible, start, props.keyed, (item: T, index: Accessor<number>) =>
      measure(index, children(item, index)),
    );
  });
  append(content, spacer(top));
  append(content, body);
  append(content, spacer(bottom));
  append(scroller, content);
  return scroller;
}

/**
 * Windowed vertical list with fixed-stride or measured rows. Only visible rows plus overscan mount.
 */
export function VirtualList<T>(props: VirtualListIdentityProps<T>): NativeNode;
export function VirtualList<T>(props: VirtualListKeyedProps<T>): NativeNode;
export function VirtualList<T>(props: VirtualListProps<T>): NativeNode {
  const [fixed, estimated, initial] = untrack(
    () => [props.itemHeight, props.estimatedItemHeight, props.initialHeight] as const,
  );
  if ((fixed === undefined) === (estimated === undefined)) {
    throw new TypeError("VirtualList requires exactly one of itemHeight or estimatedItemHeight");
  }
  if (estimated !== undefined) validatePositive(estimated, "VirtualList estimatedItemHeight");
  const initialHeight = validatePositive(initial ?? 480, "VirtualList initialHeight");
  const rest = omit(
    props,
    "each",
    "itemHeight",
    "estimatedItemHeight",
    "gap",
    "overscan",
    "initialHeight",
    "controller",
    "keyed",
    "fallback",
    "children",
  );
  const scroller = createElement("scroll");
  const nativeProps = mergeProps(rest, withDefaults({ role: "list" as const }, rest));
  if (estimated !== undefined) {
    return mountMeasuredList(scroller, props, nativeProps, initialHeight);
  }
  spread(scroller, nativeProps, true);

  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewportHeight, setViewportHeight] = createSignal(initialHeight);

  observeGeometry(scroller, "scrollPosition", pos => {
    const nextTop = Math.max(0, pos.offset.y);
    if (nextTop !== scrollTop()) setScrollTop(nextTop);
    if (pos.scrollport.height > 0 && pos.scrollport.height !== viewportHeight()) {
      setViewportHeight(pos.scrollport.height);
    }
  });

  const metrics = createMemo(() => {
    const itemHeight = validatePositive(props.itemHeight!, "VirtualList itemHeight");
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

  const onController = untrack(() => props.controller);
  onController?.({
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
