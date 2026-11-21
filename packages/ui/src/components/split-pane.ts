import { createContext, createEffect, createMemo, createSignal, onCleanup, omit, untrack, useContext } from "#solid";
import { createElement, mergeProps, spread } from "../renderer/index.js";
import { focus, getWindowRect, precedes } from "../nodes.js";
import type {
  Accessor,
  NativeChild,
  NativeElementProps,
  NativeKeyEvent,
  NativeNode,
  NativePointerDownEvent,
  NativePointerEvent,
} from "../types.js";
import { ELLIPSIS, Icon } from "./icons.js";
import { callHandler } from "./state.js";
import { append, classes, element, provide, slot, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/ResizableStyle.js";

export type SplitPaneDirection = "horizontal" | "vertical";

export interface SplitPaneProps extends Omit<NativeElementProps, "direction"> {
  direction?: SplitPaneDirection;
  sizes?: readonly number[];
  defaultSizes?: readonly number[];
  onSizesChange?: (sizes: readonly number[]) => void;
  keyboardStep?: number;
  label?: string | null;
}

export interface SplitPanePanelProps extends NativeElementProps {
  defaultSize?: number;
  minSize?: number;
  maxSize?: number;
  collapsible?: boolean;
  collapsedSize?: number;
  collapsed?: boolean;
  defaultCollapsed?: boolean;
  onCollapseChange?: (collapsed: boolean) => void;
}

export interface SplitPaneHandleProps extends Omit<NativeElementProps, "children"> {
  children?: NativeChild;
  withHandle?: boolean;
  disabled?: boolean;
  step?: number;
  label?: string | null;
}

interface PanelEntry {
  readonly node: NativeNode;
  readonly defaultSize: () => number | undefined;
  readonly minSize: () => number;
  readonly maxSize: () => number;
  readonly collapsible: () => boolean;
  readonly collapsedSize: () => number;
  readonly controlledCollapsed: () => boolean | undefined;
  readonly defaultCollapsed: () => boolean;
  readonly notifyCollapse: (collapsed: boolean) => void;
  lastExpanded: number;
  collapsedState: boolean;
}

interface HandleEntry {
  readonly node: NativeNode;
}

interface SplitPaneContextValue {
  readonly direction: Accessor<SplitPaneDirection>;
  readonly container: Accessor<NativeNode | null>;
  readonly keyboardStep: Accessor<number>;
  readonly registerPanel: (entry: PanelEntry) => () => void;
  readonly registerHandle: (entry: HandleEntry) => () => void;
  readonly panelIndex: (entry: PanelEntry) => number;
  readonly handleIndex: (entry: HandleEntry) => number;
  readonly sizeFor: (entry: PanelEntry) => number;
  readonly isCollapsed: (entry: PanelEntry) => boolean;
  readonly pairBounds: (handleIdx: number) => { min: number; max: number; now: number } | null;
  readonly snapshotSizes: () => readonly number[];
  readonly applyPairDelta: (handleIdx: number, baseSizes: readonly number[], deltaPct: number) => void;
  readonly nudgeHandle: (handleIdx: number, deltaPct: number) => void;
  readonly snapHandleEdge: (handleIdx: number, edge: "min" | "max") => void;
  readonly toggleCollapseAt: (handleIdx: number) => void;
}

const SplitPaneContext = createContext<SplitPaneContextValue | null>(null);

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function roundPct(value: number): number {
  return Math.round(value * 1000) / 1000;
}

function sortByTree<T extends { readonly node: NativeNode }>(items: readonly T[]): T[] {
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

function computeInitialSizes(
  panels: readonly PanelEntry[],
  explicit: readonly number[] | undefined,
): number[] {
  const count = panels.length;
  if (count === 0) return [];
  if (explicit && explicit.length === count) {
    const total = explicit.reduce((acc, n) => acc + (Number.isFinite(n) && n >= 0 ? n : 0), 0);
    if (total > 0) return explicit.map(n => roundPct((Math.max(0, n) / total) * 100));
  }
  const resolved: (number | undefined)[] = panels.map(panel => {
    const controlled = panel.controlledCollapsed();
    const collapsed = controlled ?? panel.defaultCollapsed();
    if (panel.collapsible() && collapsed) return clamp(panel.collapsedSize(), 0, 100);
    const def = panel.defaultSize();
    return def !== undefined && Number.isFinite(def) ? clamp(def, 0, 100) : undefined;
  });
  let assigned = 0;
  let missing = 0;
  for (const val of resolved) {
    if (val === undefined) missing += 1;
    else assigned += val;
  }
  const share = missing > 0 ? Math.max(0, 100 - assigned) / missing : 0;
  const raw = resolved.map(val => (val === undefined ? share : val));
  const sum = raw.reduce((acc, n) => acc + n, 0);
  if (sum <= 0) {
    const even = roundPct(100 / count);
    return raw.map(() => even);
  }
  return raw.map(val => roundPct((val / sum) * 100));
}

function panelLimits(panel: PanelEntry): { min: number; max: number; collapsed: number; collapsible: boolean } {
  const min = clamp(Number.isFinite(panel.minSize()) ? panel.minSize() : 5, 0, 100);
  const max = clamp(Number.isFinite(panel.maxSize()) ? panel.maxSize() : 100, min, 100);
  const collapsible = panel.collapsible();
  const collapsed = clamp(Number.isFinite(panel.collapsedSize()) ? panel.collapsedSize() : 0, 0, min);
  return { min, max, collapsed, collapsible };
}

function computePair(
  leftPanel: PanelEntry,
  rightPanel: PanelEntry,
  pairTotal: number,
  rawLeft: number,
): { left: number; right: number; leftCollapsed: boolean; rightCollapsed: boolean } {
  const l = panelLimits(leftPanel);
  const r = panelLimits(rightPanel);
  if (l.collapsible && rawLeft <= (l.collapsed + l.min) / 2) {
    const left = l.collapsed;
    const right = clamp(pairTotal - left, r.collapsible ? r.collapsed : r.min, r.max);
    return { left: roundPct(pairTotal - right), right: roundPct(right), leftCollapsed: true, rightCollapsed: false };
  }
  const rawRight = pairTotal - rawLeft;
  if (r.collapsible && rawRight <= (r.collapsed + r.min) / 2) {
    const right = r.collapsed;
    const left = clamp(pairTotal - right, l.collapsible ? l.collapsed : l.min, l.max);
    return { left: roundPct(left), right: roundPct(pairTotal - left), leftCollapsed: false, rightCollapsed: true };
  }
  const minLeft = Math.max(l.min, pairTotal - r.max);
  const maxLeft = Math.min(l.max, pairTotal - r.min);
  const left = minLeft <= maxLeft ? clamp(rawLeft, minLeft, maxLeft) : clamp(rawLeft, 0, pairTotal);
  const right = Math.max(0, pairTotal - left);
  return {
    left: roundPct(left),
    right: roundPct(right),
    leftCollapsed: l.collapsible && left <= l.collapsed + 0.01,
    rightCollapsed: r.collapsible && right <= r.collapsed + 0.01,
  };
}

export function SplitPane(props: SplitPaneProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "direction", "sizes", "defaultSizes", "onSizesChange", "keyboardStep", "label", "children");
  const direction = createMemo<SplitPaneDirection>(() => {
    const dir = props.direction ?? "horizontal";
    if (dir !== "horizontal" && dir !== "vertical") {
      throw new TypeError(`SplitPane direction must be "horizontal" or "vertical"`);
    }
    return dir;
  });
  const keyboardStep = () => {
    const value = props.keyboardStep ?? 2;
    return Number.isFinite(value) && value > 0 ? value : 2;
  };
  const [panels, setPanels] = createSignal<readonly PanelEntry[]>([], { ownedWrite: true });
  const [handles, setHandles] = createSignal<readonly HandleEntry[]>([], { ownedWrite: true });
  const [innerSizes, setInnerSizes] = createSignal<readonly number[]>([], { ownedWrite: true });
  const [container, setContainer] = createSignal<NativeNode | null>(null, { ownedWrite: true });

  const resolvedSizes = createMemo<readonly number[]>(() => {
    const list = panels();
    if (list.length === 0) return [];
    const controlled = props.sizes;
    if (controlled && controlled.length === list.length) {
      return controlled;
    }
    const inner = innerSizes();
    if (inner.length === list.length) return inner;
    return computeInitialSizes(list, props.defaultSizes);
  });

  const commitSizes = (next: readonly number[]) => {
    const list = panels();
    const prev = untrack(resolvedSizes);
    const same = prev.length === next.length && prev.every((v, i) => Math.abs(v - next[i]!) < 0.001);
    if (same) return;
    if (untrack(() => props.sizes) === undefined) {
      setInnerSizes(next);
    }
    for (let i = 0; i < list.length; i++) {
      const panel = list[i]!;
      const limits = panelLimits(panel);
      const size = next[i] ?? 0;
      const collapsedNow = limits.collapsible && size <= limits.collapsed + 0.01;
      if (!collapsedNow && size >= limits.min) {
        panel.lastExpanded = size;
      }
      if (collapsedNow !== panel.collapsedState) {
        panel.collapsedState = collapsedNow;
        untrack(() => panel.notifyCollapse(collapsedNow));
      }
    }
    untrack(() => props.onSizesChange?.(next));
  };

  createEffect(
    () => panels().length,
    count => {
      if (count === 0) return;
      const list = untrack(panels);
      const current = untrack(innerSizes);
      if (current.length !== count) {
        const init = computeInitialSizes(list, untrack(() => props.sizes ?? props.defaultSizes));
        setInnerSizes(init);
        for (let i = 0; i < list.length; i++) {
          const panel = list[i]!;
          const limits = panelLimits(panel);
          const sz = init[i] ?? 0;
          panel.collapsedState = limits.collapsible && sz <= limits.collapsed + 0.01;
          if (!panel.collapsedState) panel.lastExpanded = Math.max(limits.min, sz);
        }
      }
    },
  );

  const registerPanel = (entry: PanelEntry): (() => void) => {
    setPanels(prev => sortByTree([...prev, entry]));
    return () => setPanels(prev => prev.filter(item => item !== entry));
  };

  const registerHandle = (entry: HandleEntry): (() => void) => {
    setHandles(prev => sortByTree([...prev, entry]));
    return () => setHandles(prev => prev.filter(item => item !== entry));
  };

  const applyPairTarget = (handleIdx: number, baseSizes: readonly number[], rawLeft: number) => {
    const list = untrack(panels);
    if (handleIdx < 0 || handleIdx + 1 >= list.length) return;
    if (baseSizes.length !== list.length) return;
    const leftPanel = list[handleIdx]!;
    const rightPanel = list[handleIdx + 1]!;
    const pairTotal = baseSizes[handleIdx]! + baseSizes[handleIdx + 1]!;
    const { left, right } = computePair(leftPanel, rightPanel, pairTotal, rawLeft);
    const next = [...baseSizes];
    next[handleIdx] = left;
    next[handleIdx + 1] = right;
    commitSizes(next);
  };

  const context: SplitPaneContextValue = {
    direction,
    container,
    keyboardStep,
    registerPanel,
    registerHandle,
    panelIndex: entry => panels().indexOf(entry),
    handleIndex: entry => handles().indexOf(entry),
    sizeFor(entry) {
      const idx = panels().indexOf(entry);
      if (idx < 0) return 0;
      return resolvedSizes()[idx] ?? 0;
    },
    isCollapsed(entry) {
      const controlled = entry.controlledCollapsed();
      if (controlled !== undefined) return controlled;
      const limits = panelLimits(entry);
      return limits.collapsible && context.sizeFor(entry) <= limits.collapsed + 0.01;
    },
    pairBounds(handleIdx) {
      const list = panels();
      const sizes = resolvedSizes();
      if (handleIdx < 0 || handleIdx + 1 >= list.length || sizes.length !== list.length) return null;
      const l = panelLimits(list[handleIdx]!);
      const r = panelLimits(list[handleIdx + 1]!);
      const pairTotal = sizes[handleIdx]! + sizes[handleIdx + 1]!;
      const min = l.collapsible ? l.collapsed : Math.max(l.min, pairTotal - r.max);
      const max = Math.min(l.max, pairTotal - (r.collapsible ? r.collapsed : r.min));
      return { min: Math.round(min), max: Math.round(Math.max(min, max)), now: Math.round(sizes[handleIdx]!) };
    },
    snapshotSizes: () => [...untrack(resolvedSizes)],
    applyPairDelta(handleIdx, baseSizes, deltaPct) {
      if (handleIdx < 0 || handleIdx + 1 >= baseSizes.length) return;
      applyPairTarget(handleIdx, baseSizes, baseSizes[handleIdx]! + deltaPct);
    },
    nudgeHandle(handleIdx, deltaPct) {
      const base = [...untrack(resolvedSizes)];
      if (handleIdx < 0 || handleIdx + 1 >= base.length) return;
      const list = untrack(panels);
      const leftPanel = list[handleIdx]!;
      const l = panelLimits(leftPanel);
      let target = base[handleIdx]! + deltaPct;
      if (l.collapsible && deltaPct > 0 && base[handleIdx]! <= l.collapsed + 0.01) {
        target = Math.max(l.min, target);
      }
      applyPairTarget(handleIdx, base, target);
    },
    snapHandleEdge(handleIdx, edge) {
      const base = [...untrack(resolvedSizes)];
      const list = untrack(panels);
      if (handleIdx < 0 || handleIdx + 1 >= list.length || base.length !== list.length) return;
      const l = panelLimits(list[handleIdx]!);
      const r = panelLimits(list[handleIdx + 1]!);
      const pairTotal = base[handleIdx]! + base[handleIdx + 1]!;
      const target = edge === "min"
        ? (l.collapsible ? l.collapsed : l.min)
        : Math.min(l.max, pairTotal - (r.collapsible ? r.collapsed : r.min));
      applyPairTarget(handleIdx, base, target);
    },
    toggleCollapseAt(handleIdx) {
      const base = [...untrack(resolvedSizes)];
      const list = untrack(panels);
      if (handleIdx < 0 || handleIdx + 1 >= list.length || base.length !== list.length) return;
      const leftPanel = list[handleIdx]!;
      const rightPanel = list[handleIdx + 1]!;
      const l = panelLimits(leftPanel);
      const r = panelLimits(rightPanel);
      const pairTotal = base[handleIdx]! + base[handleIdx + 1]!;
      if (l.collapsible) {
        const isCollapsedNow = base[handleIdx]! <= l.collapsed + 0.01;
        const target = isCollapsedNow ? Math.max(l.min, leftPanel.lastExpanded || l.min) : l.collapsed;
        applyPairTarget(handleIdx, base, target);
        return;
      }
      if (r.collapsible) {
        const isCollapsedNow = base[handleIdx + 1]! <= r.collapsed + 0.01;
        const desiredRight = isCollapsedNow ? Math.max(r.min, rightPanel.lastExpanded || r.min) : r.collapsed;
        applyPairTarget(handleIdx, base, pairTotal - desiredRight);
      }
    },
  };

  const root = createElement("box");
  setContainer(root);
  spread(root, mergeProps(
    rest,
    withDefaults({
      role: "group" as const,
      get ariaLabel() { return props.label ?? null; },
      get "data-direction"() { return direction(); },
    }, rest),
    {
      get __zguiClass() { return classes(sheet.className, "zui-split-pane"); },
    },
  ), true);
  slot(root, provide(SplitPaneContext, context, props));
  return root;
}

export function SplitPanePanel(props: SplitPanePanelProps = {}): NativeNode {
  useStylesheet(sheet);
  const context = useContext(SplitPaneContext);
  const rest = omit(
    props,
    "defaultSize",
    "minSize",
    "maxSize",
    "collapsible",
    "collapsedSize",
    "collapsed",
    "defaultCollapsed",
    "onCollapseChange",
    "children",
  );
  const node = createElement("box");
  const entry: PanelEntry = {
    node,
    defaultSize: () => props.defaultSize,
    minSize: () => props.minSize ?? 5,
    maxSize: () => props.maxSize ?? 100,
    collapsible: () => !!props.collapsible,
    collapsedSize: () => props.collapsedSize ?? 0,
    controlledCollapsed: () => props.collapsed,
    defaultCollapsed: () => !!props.defaultCollapsed,
    notifyCollapse: collapsed => props.onCollapseChange?.(collapsed),
    lastExpanded: props.defaultSize ?? Math.max(props.minSize ?? 5, 25),
    collapsedState: !!props.defaultCollapsed,
  };

  if (context) {
    const unregister = context.registerPanel(entry);
    onCleanup(unregister);
  }

  spread(node, mergeProps(
    rest,
    withDefaults({
      get "data-collapsed"() { return context?.isCollapsed(entry) ? "true" : "false"; },
      get __zguiStyle() {
        if (!context) return {};
        const size = context.sizeFor(entry);
        const horizontal = context.direction() === "horizontal";
        return {
          flexBasis: `${size}%`,
          width: horizontal ? `${size}%` : "100%",
          height: horizontal ? "100%" : `${size}%`,
          minWidth: "0px",
          minHeight: "0px",
        };
      },
    }, rest),
    {
      get __zguiClass() { return classes("zui-resizable__panel"); },
    },
  ), true);
  slot(node, () => props.children);
  return node;
}

export function SplitPaneHandle(props: SplitPaneHandleProps = {}): NativeNode {
  useStylesheet(sheet);
  const context = useContext(SplitPaneContext);
  const rest = omit(
    props,
    "children",
    "withHandle",
    "disabled",
    "step",
    "label",
    "onKeyDown",
    "onPointerDown",
    "onPointerMove",
    "onPointerUp",
    "onPointerCancel",
  );
  const root = createElement("control");
  const entry: HandleEntry = { node: root };
  if (context) {
    const unregister = context.registerHandle(entry);
    onCleanup(unregister);
  }

  const [dragging, setDragging] = createSignal(false, { ownedWrite: true });
  let captured: string | undefined;
  let startCoord = 0;
  let startSpan = 0;
  let startSizes: readonly number[] = [];

  const disabled = () => !!props.disabled;
  onCleanup(() => { captured = undefined; });

  const endDrag = (event: NativePointerEvent) => {
    captured = undefined;
    setDragging(false);
    event.releasePointer();
  };

  const bounds = createMemo(() => {
    if (!context) return null;
    return context.pairBounds(context.handleIndex(entry));
  });

  spread(root, mergeProps(
    rest,
    withDefaults({
      role: "splitter" as const,
      tabIndex: 0,
      get disabled() { return disabled(); },
      get orientation() {
        return context?.direction() === "vertical" ? ("horizontal" as const) : ("vertical" as const);
      },
      get ariaLabel() { return props.label ?? "Resize panels"; },
      get ariaValueMin() { return bounds()?.min ?? 0; },
      get ariaValueMax() { return bounds()?.max ?? 100; },
      get ariaValueNow() { return bounds()?.now ?? 50; },
      get "data-direction"() { return context?.direction() ?? "horizontal"; },
      get "data-dragging"() { return dragging() ? "true" : "false"; },
    }, rest),
    {
      get __zguiClass() { return classes("zui-resizable__handle"); },
      onKeyDown(event: NativeKeyEvent) {
        if (!callHandler(event, props.onKeyDown) || disabled() || !context) return;
        const idx = context.handleIndex(entry);
        if (idx < 0) return;
        const horizontal = context.direction() === "horizontal";
        const baseStep = props.step !== undefined && Number.isFinite(props.step) && props.step > 0
          ? props.step
          : context.keyboardStep();
        const step = event.modifiers.shift ? baseStep * 5 : baseStep;
        if ((horizontal && event.key === "ArrowLeft") || (!horizontal && event.key === "ArrowUp")) {
          context.nudgeHandle(idx, -step);
          event.preventDefault();
          event.stopPropagation();
        } else if ((horizontal && event.key === "ArrowRight") || (!horizontal && event.key === "ArrowDown")) {
          context.nudgeHandle(idx, step);
          event.preventDefault();
          event.stopPropagation();
        } else if (event.key === "Home") {
          context.snapHandleEdge(idx, "min");
          event.preventDefault();
          event.stopPropagation();
        } else if (event.key === "End") {
          context.snapHandleEdge(idx, "max");
          event.preventDefault();
          event.stopPropagation();
        } else if (event.key === "Enter" || event.key === "Space" || event.key === " ") {
          context.toggleCollapseAt(idx);
          event.preventDefault();
          event.stopPropagation();
        }
      },
      onPointerDown(event: NativePointerDownEvent) {
        if (!callHandler(event, props.onPointerDown) || disabled() || !context || !event.primary) return;
        if (!(event.button === "primary" || (event.button === null && event.pointerKind !== "mouse"))) return;
        const containerNode = context.container();
        const rect = containerNode ? getWindowRect(containerNode) : null;
        if (!rect) return;
        const horizontal = context.direction() === "horizontal";
        const span = horizontal ? rect.width : rect.height;
        if (span <= 0) return;
        captured = event.pointerId;
        startCoord = horizontal ? event.position.x : event.position.y;
        startSpan = span;
        startSizes = context.snapshotSizes();
        setDragging(true);
        event.capturePointer();
        focus(root);
        event.preventDefault();
      },
      onPointerMove(event: NativePointerEvent) {
        const allowed = callHandler(event, props.onPointerMove);
        if (event.pointerId !== captured) return;
        if (!dragging() || disabled() || !context) { endDrag(event); return; }
        if (!allowed || startSpan <= 0) return;
        const horizontal = context.direction() === "horizontal";
        const coord = horizontal ? event.position.x : event.position.y;
        const deltaPct = ((coord - startCoord) / startSpan) * 100;
        context.applyPairDelta(context.handleIndex(entry), startSizes, deltaPct);
      },
      onPointerUp(event: NativePointerEvent) {
        const allowed = callHandler(event, props.onPointerUp);
        if (event.pointerId !== captured) return;
        if (allowed && dragging() && !disabled() && context && startSpan > 0) {
          const horizontal = context.direction() === "horizontal";
          const coord = horizontal ? event.position.x : event.position.y;
          const deltaPct = ((coord - startCoord) / startSpan) * 100;
          context.applyPairDelta(context.handleIndex(entry), startSizes, deltaPct);
        }
        endDrag(event);
      },
      onPointerCancel(event: NativePointerEvent) {
        callHandler(event, props.onPointerCancel);
        if (event.pointerId === captured) endDrag(event);
      },
    },
  ), true);

  append(root, element("box", { class: "zui-resizable__line", ariaHidden: true }));
  slot(root, () => {
    if ("children" in props) return props.children;
    if (!props.withHandle) return null;
    const grip = element("box", { class: "zui-resizable__grip", ariaHidden: true });
    append(grip, Icon({ icon: ELLIPSIS, size: "sm" }));
    return grip;
  });
  return root;
}
