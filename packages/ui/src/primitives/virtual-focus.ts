import { createEffect, createMemo, createSignal, getOwner, onCleanup, untrack } from "#solid";
import type { Accessor, NativeKeyEvent, NativeNode } from "../types.js";

export type VirtualFocusHorizontalKeys = "always" | "whenEmpty" | "never";

export interface VirtualFocusOptions<T> {
  readonly items: Accessor<readonly T[]>;
  readonly getId: (item: T) => string;
  readonly isDisabled?: ((item: T) => boolean) | undefined;
  /** Number of columns in row-major order. Values <= 1 (or non-finite) use 1D linear navigation. Default: `1`. */
  readonly columns?: number | Accessor<number> | undefined;
  /** Wrap at ends in 1D linear lists (default `true`). 2D grids never wrap horizontally across rows. */
  readonly wrap?: boolean | undefined;
  /**
   * Controls whether plain ArrowLeft / ArrowRight navigate a 2D grid.
   * Defaults to `"whenEmpty"` when `textValue` is provided, and `"always"` otherwise.
   */
  readonly horizontalKeys?: VirtualFocusHorizontalKeys | undefined;
  /**
   * Raw text accessor for single-line editable focus owners (e.g., search `<TextField>`).
   * Presence enables single-line editable key-routing rules; never install on multiline editors.
   */
  readonly textValue?: Accessor<string> | undefined;
  /** Called immediately after `activeId` updates on keyboard navigation with the item's ID and index in `items()`. */
  readonly onReveal?: ((id: string, itemIndex: number) => void) | undefined;
  /** Called once on plain non-repeat Enter (and named Space when non-editable) for the active enabled item. */
  readonly onActivate?: ((item: T) => void) | undefined;
}

export interface VirtualFocusController {
  readonly activeId: Accessor<string | null>;
  readonly activeNode: Accessor<NativeNode | null>;
  readonly setActive: (id: string | null) => void;
  /**
   * Registers a mounted node for `id` within the current Solid owner.
   * Owner cleanup removes only the matching `(id, node)` binding (never clobbering a newer mount or clearing `activeId`).
   */
  readonly register: (id: string, node: NativeNode) => () => void;
  readonly onKeyDown: (event: NativeKeyEvent) => void;
}

interface IndexedEntry<T> {
  readonly item: T;
  readonly id: string;
  readonly index: number;
  readonly disabled: boolean;
}

interface IndexedModel<T> {
  readonly list: readonly IndexedEntry<T>[];
  readonly byId: ReadonlyMap<string, IndexedEntry<T>>;
  readonly firstEnabled: IndexedEntry<T> | null;
  readonly lastEnabled: IndexedEntry<T> | null;
}

type SelectionState =
  | { readonly kind: "auto" }
  | { readonly kind: "id"; readonly id: string }
  | { readonly kind: "none" };

function findFirstEnabled<T>(
  list: readonly IndexedEntry<T>[],
  start: number,
  end: number,
): IndexedEntry<T> | null {
  for (let i = start; i <= end; i++) {
    const entry = list[i]!;
    if (!entry.disabled) return entry;
  }
  return null;
}

function findLastEnabled<T>(
  list: readonly IndexedEntry<T>[],
  start: number,
  end: number,
): IndexedEntry<T> | null {
  for (let i = end; i >= start; i--) {
    const entry = list[i]!;
    if (!entry.disabled) return entry;
  }
  return null;
}

export function createVirtualFocus<T>(options: VirtualFocusOptions<T>): VirtualFocusController {
  const columnCount = createMemo(() => {
    const raw = typeof options.columns === "function" ? options.columns() : (options.columns ?? 1);
    if (!Number.isFinite(raw) || raw <= 1) return 1;
    return Math.floor(raw);
  });

  const model = createMemo<IndexedModel<T>>(() => {
    const source = options.items();
    const list: IndexedEntry<T>[] = new Array(source.length);
    const byId = new Map<string, IndexedEntry<T>>();
    let firstEnabled: IndexedEntry<T> | null = null;
    let lastEnabled: IndexedEntry<T> | null = null;
    for (let i = 0; i < source.length; i++) {
      const item = source[i]!;
      const id = options.getId(item);
      const disabled = options.isDisabled ? Boolean(options.isDisabled(item)) : false;
      const entry: IndexedEntry<T> = { item, id, index: i, disabled };
      list[i] = entry;
      if (!byId.has(id)) byId.set(id, entry);
      if (!disabled) {
        if (firstEnabled === null) firstEnabled = entry;
        lastEnabled = entry;
      }
    }
    return { list, byId, firstEnabled, lastEnabled };
  });

  const [selection, setSelectionSignal] = createSignal<SelectionState>(
    { kind: "auto" },
    { ownedWrite: true },
  );
  // Mirror latest selection synchronously so same-turn reads (e.g. inside `onReveal`
  // or back-to-back `setActive` + `onKeyDown` calls before Solid 2.0's flush) stay consistent.
  let currentSelection: SelectionState = { kind: "auto" };
  const writeSelection = (next: SelectionState): void => {
    currentSelection = next;
    setSelectionSignal(next);
  };

  const mountedNodes = new Map<string, NativeNode>();
  const [mountVersion, setMountVersion] = createSignal(0, { ownedWrite: true });

  let desiredColumn: number | null = null;
  let lastColumns = untrack(columnCount);

  if (getOwner()) {
    onCleanup(() => {
      mountedNodes.clear();
    });
  }

  const activeId: Accessor<string | null> = () => {
    const m = model();
    selection();
    const current = currentSelection;
    if (current.kind === "none") return null;
    if (current.kind === "id") {
      const found = m.byId.get(current.id);
      if (found && !found.disabled) return current.id;
    }
    return m.firstEnabled?.id ?? null;
  };

  // When an explicitly selected ID disappears or becomes disabled, drop the stale ID
  // so re-introducing that ID later does not unexpectedly jump highlight back to it.
  createEffect(
    () => ({ m: model(), current: selection() }),
    ({ m }) => {
      if (currentSelection.kind !== "id") return;
      const found = m.byId.get(currentSelection.id);
      if (!found || found.disabled) {
        desiredColumn = null;
        writeSelection({ kind: "auto" });
      }
    },
  );

  // Reset retained vertical column whenever the grid column count changes.
  createEffect(
    () => columnCount(),
    cols => {
      if (cols !== lastColumns) {
        desiredColumn = null;
        lastColumns = cols;
      }
    },
  );

  const activeNode: Accessor<NativeNode | null> = () => {
    mountVersion();
    const id = activeId();
    if (id === null) return null;
    return mountedNodes.get(id) ?? null;
  };

  const setActive = (id: string | null): void => {
    desiredColumn = null;
    if (id === null) {
      writeSelection({ kind: "none" });
      return;
    }
    const found = untrack(model).byId.get(id);
    if (!found || found.disabled) return;
    writeSelection({ kind: "id", id });
  };

  const register = (id: string, node: NativeNode): (() => void) => {
    if (mountedNodes.get(id) !== node) {
      mountedNodes.set(id, node);
      setMountVersion(v => v + 1);
    }
    let disposed = false;
    const unregister = () => {
      if (disposed) return;
      disposed = true;
      if (mountedNodes.get(id) === node) {
        mountedNodes.delete(id);
        setMountVersion(v => v + 1);
      }
    };
    if (getOwner()) {
      onCleanup(unregister);
    }
    return unregister;
  };

  const commitMove = (entry: IndexedEntry<T>, nextDesiredCol: number | null): void => {
    desiredColumn = nextDesiredCol;
    writeSelection({ kind: "id", id: entry.id });
    options.onReveal?.(entry.id, entry.index);
  };

  const onKeyDown = (event: NativeKeyEvent): void => {
    if (event.isComposing) return;

    const cols = untrack(columnCount);
    if (cols !== lastColumns) {
      desiredColumn = null;
      lastColumns = cols;
    }

    const isEditable = options.textValue !== undefined;
    const isEmpty = isEditable ? untrack(options.textValue!).length === 0 : true;
    const horizPolicy: VirtualFocusHorizontalKeys =
      options.horizontalKeys ?? (isEditable ? "whenEmpty" : "always");

    const mods = event.modifiers;
    const hasAnyModifier = mods.shift || mods.control || mods.alt || mods.meta;
    const isCtrlOnly = mods.control && !mods.shift && !mods.alt && !mods.meta;
    const key = event.key;

    const consume = () => {
      event.preventDefault();
      event.stopPropagation();
    };

    if (key === "ArrowDown" || key === "ArrowUp") {
      if (hasAnyModifier) return;
      consume();
      const m = untrack(model);
      const n = m.list.length;
      if (n === 0 || m.firstEnabled === null) return;

      const currentId = untrack(activeId);
      const currentEntry = currentId !== null ? m.byId.get(currentId) : undefined;
      const currentIndex = currentEntry ? currentEntry.index : -1;
      const step = key === "ArrowDown" ? 1 : -1;

      if (cols === 1) {
        if (currentIndex < 0) {
          const target = step > 0 ? m.firstEnabled : m.lastEnabled!;
          commitMove(target, null);
          return;
        }
        const wrap = options.wrap ?? true;
        if (wrap) {
          for (let offset = 1; offset <= n; offset++) {
            const cand = m.list[(currentIndex + step * offset + n) % n]!;
            if (!cand.disabled) {
              commitMove(cand, null);
              return;
            }
          }
        } else {
          for (let i = currentIndex + step; i >= 0 && i < n; i += step) {
            const cand = m.list[i]!;
            if (!cand.disabled) {
              commitMove(cand, null);
              return;
            }
          }
        }
        return;
      }

      // 2D grid vertical navigation.
      if (currentIndex < 0) {
        const target = step > 0 ? m.firstEnabled : m.lastEnabled!;
        commitMove(target, target.index % cols);
        return;
      }
      const row = Math.floor(currentIndex / cols);
      const col = currentIndex % cols;
      const totalRows = Math.ceil(n / cols);
      const targetCol = desiredColumn ?? col;
      desiredColumn = targetCol;

      for (let r = row + step; r >= 0 && r < totalRows; r += step) {
        const rStart = r * cols;
        const rEnd = Math.min(n - 1, rStart + cols - 1);
        const clampedCol = Math.min(targetCol, rEnd - rStart);
        const cand = m.list[rStart + clampedCol]!;
        if (!cand.disabled) {
          commitMove(cand, targetCol);
          return;
        }
      }
      return;
    }

    if (key === "ArrowLeft" || key === "ArrowRight") {
      if (cols <= 1 || hasAnyModifier) return;
      if (horizPolicy === "never") return;
      if (horizPolicy === "whenEmpty" && !isEmpty) return;
      consume();
      const m = untrack(model);
      const n = m.list.length;
      if (n === 0 || m.firstEnabled === null) return;

      desiredColumn = null;
      const currentId = untrack(activeId);
      const currentEntry = currentId !== null ? m.byId.get(currentId) : undefined;
      const currentIndex = currentEntry ? currentEntry.index : -1;
      const step = key === "ArrowRight" ? 1 : -1;

      if (currentIndex < 0) {
        const target = step > 0 ? m.firstEnabled : m.lastEnabled!;
        commitMove(target, null);
        return;
      }
      const row = Math.floor(currentIndex / cols);
      const col = currentIndex % cols;
      const rowStart = row * cols;
      const rowEnd = Math.min(n - 1, rowStart + cols - 1);
      const rowLen = rowEnd - rowStart + 1;

      for (let c = col + step; c >= 0 && c < rowLen; c += step) {
        const cand = m.list[rowStart + c]!;
        if (!cand.disabled) {
          commitMove(cand, null);
          return;
        }
      }
      return;
    }

    if (key === "Home" || key === "End") {
      if (isEditable && !isEmpty) return;
      const toStart = key === "Home";
      if (cols === 1) {
        if (hasAnyModifier) return;
        consume();
        const m = untrack(model);
        const target = toStart ? m.firstEnabled : m.lastEnabled;
        if (target) commitMove(target, null);
        return;
      }

      if (isCtrlOnly) {
        consume();
        const m = untrack(model);
        const target = toStart ? m.firstEnabled : m.lastEnabled;
        if (target) commitMove(target, null);
        return;
      }
      if (hasAnyModifier) return;
      consume();
      const m = untrack(model);
      const n = m.list.length;
      if (n === 0 || m.firstEnabled === null) return;

      desiredColumn = null;
      const currentId = untrack(activeId);
      const currentEntry = currentId !== null ? m.byId.get(currentId) : undefined;
      if (!currentEntry) {
        const target = toStart ? m.firstEnabled : m.lastEnabled!;
        commitMove(target, null);
        return;
      }
      const row = Math.floor(currentEntry.index / cols);
      const rowStart = row * cols;
      const rowEnd = Math.min(n - 1, rowStart + cols - 1);
      const target = toStart
        ? findFirstEnabled(m.list, rowStart, rowEnd)
        : findLastEnabled(m.list, rowStart, rowEnd);
      if (target) commitMove(target, null);
      return;
    }

    const isSpace = key === "Space" || key === " ";
    if ((key === "Enter" || (!isEditable && isSpace)) && !hasAnyModifier) {
      if (!options.onActivate) return;
      consume();
      if (event.repeat) return;
      const m = untrack(model);
      const currentId = untrack(activeId);
      const currentEntry = currentId !== null ? m.byId.get(currentId) : undefined;
      if (currentEntry && !currentEntry.disabled) {
        options.onActivate(currentEntry.item);
      }
    }
  };

  return {
    activeId,
    activeNode,
    setActive,
    register,
    onKeyDown,
  };
}
