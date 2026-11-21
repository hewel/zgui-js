import { createMemo, createSignal, onCleanup, untrack } from "#solid";
import { focus, precedes } from "../nodes.js";
import { trackTreeOrder } from "../tree-order.js";
import type { Accessor, NativeKeyEvent, NativeNode } from "../types.js";

export type GroupOrientation = "horizontal" | "vertical" | "both";

interface Entry {
  node: NativeNode;
  disabled: Accessor<boolean>;
  preferred: Accessor<boolean>;
}

export interface RovingFocusItem {
  readonly tabIndex: () => 0 | -1;
  readonly active: () => boolean;
  activate(): void;
}

export interface RovingFocusController {
  readonly active: Accessor<NativeNode | undefined>;
  readonly register: (
    node: NativeNode,
    disabled: Accessor<boolean>,
    preferred?: Accessor<boolean>,
  ) => RovingFocusItem;
  readonly clearActive: (node?: NativeNode) => void;
  readonly firstEnabled: () => NativeNode | undefined;
  readonly focusFirst: () => boolean;
  readonly step: (direction: 1 | -1) => boolean;
  readonly focusNext: (matches: (node: NativeNode) => boolean) => boolean;
  readonly onKeyDown: (event: NativeKeyEvent) => void;
}

export function groupOrientation(value: GroupOrientation): GroupOrientation {
  if (value !== "horizontal" && value !== "vertical" && value !== "both") {
    throw new TypeError("Group orientation must be horizontal, vertical, or both");
  }
  return value;
}

export function createRovingFocus(
  orientation: Accessor<GroupOrientation>,
  navigation: "target" | "active" = "target",
): RovingFocusController {
  const items: Entry[] = [];
  const [version, setVersion] = createSignal(0, { ownedWrite: true });
  const [active, setActive] = createSignal<NativeNode | undefined>(undefined, { ownedWrite: true });
  const ordered = () => {
    trackTreeOrder();
    version();
    return items.length <= 1
      ? items
      : items
          .slice()
          .sort((a, b) => (precedes(a.node, b.node) ? -1 : precedes(b.node, a.node) ? 1 : 0));
  };
  const tabStop = createMemo(() => {
    version();
    let firstEnabled: Entry | undefined;
    let enabledCount = 0;
    const current = active();
    let activeMatch: Entry | undefined;
    let singlePreferred: Entry | undefined;
    let preferredCount = 0;
    for (const item of items) {
      if (item.disabled()) continue;
      if (enabledCount === 0) firstEnabled = item;
      enabledCount += 1;
      if (current && item.node === current) activeMatch = item;
      if (item.preferred()) {
        if (preferredCount === 0) singlePreferred = item;
        preferredCount += 1;
      }
    }
    if (enabledCount <= 1) return firstEnabled;
    if (activeMatch) return activeMatch;
    if (preferredCount === 1) return singlePreferred;
    const sorted = ordered();
    let firstSorted: Entry | undefined;
    for (const item of sorted) {
      if (item.disabled()) continue;
      if (!firstSorted) firstSorted = item;
      if (item.preferred()) return item;
    }
    return firstSorted;
  });
  const focusCandidate = (candidate: Entry | undefined): boolean => {
    if (!candidate) return false;
    setActive(candidate.node);
    focus(candidate.node, { reveal: true });
    return true;
  };
  const stepFrom = (from: number, direction: 1 | -1, orderedItems = ordered()): boolean => {
    const start = from < 0 ? (direction > 0 ? -1 : orderedItems.length) : from;
    for (let step = 1; step <= orderedItems.length; step++) {
      const candidate =
        orderedItems[(start + direction * step + orderedItems.length) % orderedItems.length];
      if (candidate && !candidate.disabled()) return focusCandidate(candidate);
    }
    return false;
  };
  return {
    active,
    register(
      node: NativeNode,
      disabled: Accessor<boolean>,
      preferred: Accessor<boolean> = () => false,
    ): RovingFocusItem {
      const entry = { node, disabled, preferred };
      items.push(entry);
      setVersion(v => v + 1);
      onCleanup(() => {
        const index = items.indexOf(entry);
        if (index >= 0) {
          items.splice(index, 1);
          setVersion(v => v + 1);
        }
        if (untrack(active) === node) setActive(undefined);
      });
      return {
        tabIndex: () => (tabStop()?.node === node ? 0 : -1),
        active: () => active() === node && !disabled(),
        activate: () => {
          if (!disabled()) setActive(node);
        },
      };
    },
    clearActive(node?: NativeNode) {
      if (node === undefined || untrack(active) === node) {
        setActive(undefined);
      }
    },
    firstEnabled(): NativeNode | undefined {
      return ordered().find(item => !item.disabled())?.node;
    },
    focusFirst(): boolean {
      return focusCandidate(ordered().find(item => !item.disabled()));
    },
    step(direction: 1 | -1): boolean {
      const sorted = ordered();
      const from = sorted.findIndex(item => item.node === active());
      if (from >= 0 && sorted.filter(item => !item.disabled()).length <= 1) return false;
      return stepFrom(from, direction, sorted);
    },
    focusNext(matches: (node: NativeNode) => boolean): boolean {
      const enabled = ordered().filter(item => !item.disabled());
      const from = enabled.findIndex(item => item.node === active());
      for (let step = 1; step <= enabled.length; step++) {
        const next = enabled[(from + step) % enabled.length];
        if (next && matches(next.node)) return focusCandidate(next);
      }
      return false;
    },
    onKeyDown(event: NativeKeyEvent) {
      if (event.character != null) return;
      const key = event.key;
      const axis = orientation();
      const direction =
        (key === "ArrowRight" && axis !== "vertical") ||
        (key === "ArrowDown" && axis !== "horizontal")
          ? 1
          : (key === "ArrowLeft" && axis !== "vertical") ||
              (key === "ArrowUp" && axis !== "horizontal")
            ? -1
            : 0;
      if (!direction && key !== "Home" && key !== "End") return;
      const sorted = ordered();
      const current =
        navigation === "active"
          ? (active() ?? sorted.find(item => !item.disabled())?.node)
          : event.target;
      const from = sorted.findIndex(item => item.node === current);
      if (from < 0 && navigation === "target") return;
      let moved = false;
      if (direction) moved = stepFrom(from, direction as 1 | -1, sorted);
      else {
        const enabled = sorted.filter(item => !item.disabled());
        moved = focusCandidate(key === "Home" ? enabled[0] : enabled[enabled.length - 1]);
      }
      if (!moved) return;
      event.preventDefault();
      event.stopPropagation();
    },
  };
}
