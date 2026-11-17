import { batch, createMemo, createSignal, onCleanup, untrack } from "#solid";
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
  const [members, setMembers] = createSignal<readonly Entry[]>([]);
  const [active, setActive] = createSignal<NativeNode>();
  const ordered = () => {
    trackTreeOrder();
    const list = members();
    return list.length <= 1 ? list : list.slice().sort((a, b) => precedes(a.node, b.node) ? -1 : precedes(b.node, a.node) ? 1 : 0);
  };
  const tabStop = createMemo(() => {
    const enabled = members().filter(item => !item.disabled());
    if (enabled.length <= 1) return enabled[0];
    const current = active();
    if (current) {
      const match = enabled.find(item => item.node === current);
      if (match) return match;
    }
    const preferred = enabled.filter(item => item.preferred());
    if (preferred.length === 1) return preferred[0];
    const sorted = ordered().filter(item => !item.disabled());
    return sorted.find(item => item.preferred()) ?? sorted[0];
  });
  const focusCandidate = (candidate: Entry | undefined): boolean => {
    if (!candidate) return false;
    setActive(candidate.node);
    focus(candidate.node, { reveal: true });
    return true;
  };
  const stepFrom = (from: number, direction: 1 | -1, items = ordered()): boolean => {
    const start = from < 0 ? (direction > 0 ? -1 : items.length) : from;
    for (let step = 1; step <= items.length; step++) {
      const candidate = items[(start + direction * step + items.length) % items.length];
      if (!candidate.disabled()) return focusCandidate(candidate);
    }
    return false;
  };
  return {
    register(node: NativeNode, disabled: Accessor<boolean>, preferred: Accessor<boolean> = () => false) {
      const entry = { node, disabled, preferred };
      setMembers(items => [...items, entry]);
      onCleanup(() => batch(() => {
        setMembers(items => items.filter(item => item !== entry));
        if (untrack(active) === node) setActive(undefined);
      }));
      return {
        tabIndex: () => tabStop()?.node === node ? 0 : -1,
        activate: () => { if (!disabled()) setActive(node); },
      };
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
