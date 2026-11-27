import { createMemo, createSignal, getOwner, onCleanup, untrack } from "#solid";
import { getWindowRect, observeGeometry, scrollBy } from "../nodes.js";
import { animateFrames } from "../spring.js";
import type {
  Accessor,
  NativeKeyEvent,
  NativeNodeTarget,
  NativePoint,
  NativePointerDownEvent,
  NativePointerEvent,
  NativeScrollPosition,
  NativeWindowRect,
} from "../types.js";

export interface SelectionOptions<K> {
  readonly items: Accessor<readonly K[]>;
  readonly selectedKeys?: Accessor<ReadonlySet<K>> | undefined;
  readonly defaultSelectedKeys?: Iterable<K> | undefined;
  readonly onSelectionChange?: ((next: Set<K>) => void) | undefined;
  readonly selectionMode?: "single" | "multiple" | undefined;
  readonly disallowEmpty?: boolean | undefined;
}

export interface SelectionClickModifiers {
  readonly ctrl?: boolean | undefined;
  readonly meta?: boolean | undefined;
  readonly shift?: boolean | undefined;
}

export interface SelectionModel<K> {
  readonly selectedKeys: Accessor<ReadonlySet<K>>;
  readonly isSelected: (key: K) => boolean;
  readonly anchorKey: Accessor<K | null>;
  readonly focusedKey: Accessor<K | null>;
  readonly setFocusedKey: (key: K | null) => void;
  readonly selectSingle: (key: K) => void;
  readonly toggle: (key: K) => void;
  readonly selectRange: (toKey: K, additive?: boolean) => void;
  readonly selectByClick: (key: K, modifiers: SelectionClickModifiers) => void;
  readonly selectAll: () => void;
  readonly clear: () => void;
  readonly replaceSelection: (keys: Iterable<K>) => void;
  readonly handleKeyDown: (event: NativeKeyEvent) => void;
}

/** Selection over unique keys in display order. Arrow ranges follow that order, not grid columns. */
export function createSelectionModel<K>(options: SelectionOptions<K>): SelectionModel<K> {
  const multiple = options.selectionMode !== "single";
  const index = createMemo(() => new Map(options.items().map((key, i) => [key, i])));
  const normalize = (keys: Iterable<K>) => {
    const next = new Set<K>();
    const known = index();
    for (const key of keys) {
      if (!known.has(key)) continue;
      next.add(key);
      if (!multiple) break;
    }
    return next;
  };
  const [local, setLocal] = createSignal<ReadonlySet<K>>(
    untrack(() => normalize(options.defaultSelectedKeys ?? [])),
  );
  const [anchor, setAnchor] = createSignal<K | null>(null);
  const [focused, setFocused] = createSignal<K | null>(null);
  const selectedKeys = options.selectedKeys ?? local;
  const valid = (key: K | null) => (key !== null && index().has(key) ? key : null);
  const anchorKey = () => valid(anchor());
  const focusedKey = () => valid(focused());
  const setFocusedKey = (key: K | null) => setFocused(() => valid(key));

  function commit(next: Set<K>) {
    const previous = selectedKeys();
    if (next.size === 0 && options.disallowEmpty) return;
    if (next.size === previous.size && [...next].every(key => previous.has(key))) return;
    if (!options.selectedKeys) setLocal(next);
    options.onSelectionChange?.(next);
  }

  function selectSingle(key: K) {
    if (!index().has(key)) return;
    setAnchor(() => key);
    setFocusedKey(key);
    commit(new Set([key]));
  }

  function toggle(key: K) {
    if (!index().has(key)) return;
    setAnchor(() => key);
    setFocusedKey(key);
    const next = multiple ? normalize(selectedKeys()) : new Set<K>();
    if (selectedKeys().has(key)) next.delete(key);
    else next.add(key);
    commit(next);
  }

  function selectRange(toKey: K, additive = false) {
    if (!index().has(toKey)) return;
    const start = anchorKey() ?? focusedKey();
    if (!multiple || start === null) {
      selectSingle(toKey);
      return;
    }
    const from = index().get(start)!;
    const to = index().get(toKey)!;
    const items = options.items();
    const next = additive ? normalize(selectedKeys()) : new Set<K>();
    for (let i = Math.min(from, to); i <= Math.max(from, to); i += 1) next.add(items[i]!);
    setAnchor(() => start);
    setFocusedKey(toKey);
    commit(next);
  }

  function clear() {
    commit(new Set());
  }

  function selectAll() {
    if (multiple) commit(new Set(options.items()));
  }

  function handleKeyDown(event: NativeKeyEvent) {
    if (event.isComposing) return;
    const { control, meta, shift, alt } = event.modifiers;
    const primary = control || meta;
    const key = event.key;
    if (alt) return;
    if (primary && !shift && (event.keyWithoutModifiers ?? key)?.toLowerCase() === "a") {
      if (!multiple) return;
      selectAll();
    } else if (key === "Escape") {
      clear();
    } else if (!shift && !primary && (key === "Space" || key === " ")) {
      const current = focusedKey();
      if (current === null) return;
      if (!event.repeat) toggle(current);
    } else if (shift && multiple) {
      const current = focusedKey();
      const items = options.items();
      if (items.length === 0) return;
      const at = current === null ? -1 : (index().get(current) ?? -1);
      let next: number;
      switch (key) {
        case "ArrowUp":
        case "ArrowLeft":
          next = Math.max(0, at - 1);
          break;
        case "ArrowDown":
        case "ArrowRight":
          next = Math.min(items.length - 1, at + 1);
          break;
        case "Home":
          next = 0;
          break;
        case "End":
          next = items.length - 1;
          break;
        default:
          return;
      }
      selectRange(items[next]!, primary);
    } else {
      return;
    }
    event.preventDefault();
  }

  return {
    selectedKeys,
    isSelected: key => selectedKeys().has(key),
    anchorKey,
    focusedKey,
    setFocusedKey,
    selectSingle,
    toggle,
    selectRange,
    selectByClick: (key, { ctrl, meta, shift }) => {
      if (shift) selectRange(key, ctrl || meta);
      else if (ctrl || meta) toggle(key);
      else selectSingle(key);
    },
    selectAll,
    clear,
    replaceSelection: keys => commit(normalize(keys)),
    handleKeyDown,
  };
}

export interface MarqueeRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface MarqueeSelectionOptions<K> {
  /** Attach handlers to this container; only forward presses that may start a marquee. */
  readonly container: NativeNodeTarget;
  readonly selection: Pick<SelectionModel<K>, "selectedKeys" | "replaceSelection">;
  readonly resolveIntersectingKeys: (contentRect: MarqueeRect) => readonly K[];
  /** Pointer travel in logical pixels before selection starts. Default: 4. */
  readonly threshold?: number | undefined;
  /** Auto-scroll zone in logical pixels. Default: 24; 0 disables auto-scroll. */
  readonly edgeSize?: number | undefined;
}

export interface MarqueeSelection {
  readonly active: Accessor<boolean>;
  /** Scroll-content coordinates, or container-local when not scrolling. */
  readonly rect: Accessor<MarqueeRect | null>;
  readonly onPointerDown: (event: NativePointerDownEvent) => void;
  readonly onPointerMove: (event: NativePointerEvent) => void;
  readonly onPointerUp: (event: NativePointerEvent) => void;
  readonly onPointerCancel: (event: NativePointerEvent) => void;
  /** Restore the pre-drag selection and stop scrolling. */
  readonly cancel: () => void;
}

interface MarqueeDrag<K> {
  readonly pointerId: string;
  readonly press: NativePoint;
  readonly origin: NativePoint;
  readonly initial: ReadonlySet<K>;
  readonly additive: boolean;
  point: NativePoint;
  bounds: NativeWindowRect;
  rect: MarqueeRect | null;
}

/** Pointer-captured rubberband selection over caller-supplied content geometry. */
export function createMarqueeSelection<K>(options: MarqueeSelectionOptions<K>): MarqueeSelection {
  const [rect, setRect] = createSignal<MarqueeRect | null>(null);
  const threshold = options.threshold ?? 4;
  const edgeSize = options.edgeSize ?? 24;
  const container = () =>
    typeof options.container === "function" ? options.container() : options.container;
  let scroll: NativeScrollPosition | undefined;
  let drag: MarqueeDrag<K> | undefined;
  let released: string | undefined;
  let stopFrames: (() => void) | undefined;

  function update() {
    if (
      !drag ||
      (!drag.rect &&
        Math.hypot(drag.point.x - drag.press.x, drag.point.y - drag.press.y) < threshold)
    )
      return;
    const x =
      Math.max(
        0,
        Math.min(drag.point.x - drag.bounds.x, scroll?.scrollport.width ?? drag.bounds.width),
      ) + (scroll?.offset.x ?? 0);
    const y =
      Math.max(
        0,
        Math.min(drag.point.y - drag.bounds.y, scroll?.scrollport.height ?? drag.bounds.height),
      ) + (scroll?.offset.y ?? 0);
    const next = {
      x: Math.min(x, drag.origin.x),
      y: Math.min(y, drag.origin.y),
      width: Math.abs(x - drag.origin.x),
      height: Math.abs(y - drag.origin.y),
    };
    const prev = drag.rect;
    if (
      prev &&
      prev.x === next.x &&
      prev.y === next.y &&
      prev.width === next.width &&
      prev.height === next.height
    )
      return;
    const current = drag;
    current.rect = next;
    setRect(next);
    const keys = options.resolveIntersectingKeys(next);
    options.selection.replaceSelection(
      current.additive ? new Set([...current.initial, ...keys]) : keys,
    );
  }

  function speed(point: number, extent: number, offset: number, content: number): number {
    const edge = Math.min(edgeSize, extent / 2);
    // At the edge, scroll at 960 logical pixels/second regardless of refresh rate.
    if (point < edge && offset > 0) return -Math.min(1, (edge - point) / edge) * 0.96;
    if (point > extent - edge && offset < content - extent)
      return Math.min(1, (point - extent + edge) / edge) * 0.96;
    return 0;
  }

  function edgeDelta() {
    if (!drag?.rect || !scroll || edgeSize <= 0) return undefined;
    const left = speed(
      drag.point.x - drag.bounds.x,
      scroll.scrollport.width,
      scroll.offset.x,
      scroll.contentSize.width,
    );
    const top = speed(
      drag.point.y - drag.bounds.y,
      scroll.scrollport.height,
      scroll.offset.y,
      scroll.contentSize.height,
    );
    return left || top ? { left, top } : undefined;
  }

  function scheduleScroll() {
    if (stopFrames || !edgeDelta()) return;
    const node = container();
    if (!node) return;
    let lastTime: number | undefined;
    stopFrames = animateFrames(node, time => {
      const delta = container() === node && edgeDelta();
      if (delta) {
        // Virtual previews can render several frames at the same instant.
        const elapsed = lastTime === undefined ? 0 : Math.min(32, time - lastTime);
        lastTime = time;
        if (elapsed > 0) {
          scrollBy(node, {
            left: delta.left * elapsed,
            top: delta.top * elapsed,
            behavior: "instant",
          });
        }
        return true;
      }
      stopFrames = undefined;
      return false;
    });
  }

  const stopObserving = observeGeometry(container, "scrollPosition", value => {
    scroll = value;
    update();
    scheduleScroll();
  });
  function finish(restore: boolean) {
    const previous = drag;
    drag = undefined;
    stopFrames?.();
    stopFrames = undefined;
    setRect(null);
    if (restore && previous?.rect) options.selection.replaceSelection(previous.initial);
  }
  if (getOwner()) {
    onCleanup(() => {
      stopObserving();
      finish(false);
    });
  }

  function move(event: NativePointerEvent) {
    if (event.pointerId === released) {
      released = undefined;
      event.releasePointer();
    }
    if (event.pointerId !== drag?.pointerId) return;
    const node = container();
    const bounds = node && getWindowRect(node);
    if (!bounds) return;
    drag.bounds = bounds;
    drag.point = event.position;
    update();
    if (drag?.rect) event.preventDefault();
    scheduleScroll();
  }

  return {
    active: () => rect() !== null,
    rect,
    onPointerDown: event => {
      if (
        drag ||
        !event.primary ||
        (event.button !== "primary" && (event.button !== null || event.pointerKind === "mouse"))
      )
        return;
      const node = container();
      if (!node || node !== event.currentTarget) return;
      const bounds = getWindowRect(node);
      if (!bounds) return;
      const { x, y } = event.position;
      released = undefined;
      drag = {
        pointerId: event.pointerId,
        press: event.position,
        point: event.position,
        origin: {
          x: x - bounds.x + (scroll?.offset.x ?? 0),
          y: y - bounds.y + (scroll?.offset.y ?? 0),
        },
        bounds,
        initial: new Set(options.selection.selectedKeys()),
        additive: event.modifiers.control || event.modifiers.meta || event.modifiers.shift,
        rect: null,
      };
      event.capturePointer();
    },
    onPointerMove: move,
    onPointerUp: event => {
      if (
        event.pointerId === drag?.pointerId &&
        (event.button === "primary" || event.button === null)
      ) {
        move(event);
        event.releasePointer();
        finish(false);
      } else if (event.pointerId === released) {
        move(event);
      }
    },
    onPointerCancel: event => {
      if (event.pointerId === drag?.pointerId) {
        event.releasePointer();
        finish(true);
      } else if (event.pointerId === released) {
        move(event);
      }
    },
    cancel: () => {
      if (!drag) return;
      // Native release is only available during a pointer dispatch.
      released = drag.pointerId;
      finish(true);
    },
  };
}
