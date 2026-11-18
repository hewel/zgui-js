import { createMemo, createSignal, onCleanup, untrack } from "#solid";
import { focus, precedes } from "../nodes.js";
import { trackTreeOrder } from "../tree-order.js";
import type { Accessor, NativeKeyEvent, NativeNode } from "../types.js";
export type GroupOrientation = "horizontal" | "vertical" | "both";
interface Entry { node: NativeNode; disabled: Accessor<boolean>; preferred: Accessor<boolean>; }
export function groupOrientation(value: GroupOrientation): GroupOrientation {
  if (value !== "horizontal" && value !== "vertical" && value !== "both") throw new TypeError("Group orientation must be horizontal, vertical, or both");
  return value;
}
export function createRovingFocus(orientation: Accessor<GroupOrientation>, navigation: "target" | "active" = "target") {
  const items: Entry[] = [];
  const [version, setVersion] = createSignal(0);
  const [active, setActive] = createSignal<NativeNode>();
  const ordered = () => {
    trackTreeOrder();
    version();
    return items.length <= 1 ? items : items.slice().sort((a, b) => precedes(a.node, b.node) ? -1 : precedes(b.node, a.node) ? 1 : 0);
  };
  const tabStop = createMemo(() => {
    version();
    let firstEnabled: Entry | undefined;
    let enabledCount = 0;
    const current = active();
    let activeMatch: Entry | undefined;
    let singlePreferred: Entry | undefined;
    let preferredCount = 0;
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
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
    for (let i = 0; i < sorted.length; i++) {
      const item = sorted[i];
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
      const candidate = orderedItems[(start + direction * step + orderedItems.length) % orderedItems.length];
      if (!candidate.disabled()) return focusCandidate(candidate);
    }
    return false;
  };
  return {
    register(node: NativeNode, disabled: Accessor<boolean>, preferred: Accessor<boolean> = () => false) {
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
        tabIndex: () => tabStop()?.node === node ? 0 : -1,
        activate: () => { if (!disabled()) setActive(node); },
      };
    },
    firstEnabled(): NativeNode | undefined {
      return ordered().find(item => !item.disabled())?.node;
    },
    focusFirst(): boolean {
      return focusCandidate(ordered().find(item => !item.disabled()));
    },
    step(direction: 1 | -1): boolean {
      const items = ordered();
      const from = items.findIndex(item => item.node === active());
      if (from >= 0 && items.filter(item => !item.disabled()).length <= 1) return false;
      return stepFrom(from, direction, items);
    },
    focusNext(matches: (node: NativeNode) => boolean): boolean {
      const items = ordered().filter(item => !item.disabled());
      const from = items.findIndex(item => item.node === active());
      for (let step = 1; step <= items.length; step++) {
        const next = items[(from + step) % items.length];
        if (matches(next.node)) return focusCandidate(next);
      }
      return false;
    },
    onKeyDown(event: NativeKeyEvent) {
      if (event.character != null) return;
      const key = event.key;
      const axis = orientation();
      const direction = ((key === "ArrowRight" && axis !== "vertical") || (key === "ArrowDown" && axis !== "horizontal")) ? 1
        : ((key === "ArrowLeft" && axis !== "vertical") || (key === "ArrowUp" && axis !== "horizontal")) ? -1 : 0;
      if (!direction && key !== "Home" && key !== "End") return;
      const items = ordered();
      const current = navigation === "active" ? active() ?? items.find(item => !item.disabled())?.node : event.target;
      const from = items.findIndex(item => item.node === current);
      if (from < 0 && navigation === "target") return;
      let moved = false;
      if (direction) moved = stepFrom(from, direction as 1 | -1, items);
      else {
        const enabled = items.filter(item => !item.disabled());
        moved = focusCandidate(key === "Home" ? enabled[0] : enabled[enabled.length - 1]);
      }
      if (!moved) return;
      event.preventDefault();
      event.stopPropagation();
    },
  };
}
