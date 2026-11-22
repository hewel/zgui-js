import {
  For,
  createContext,
  createEffect,
  createSignal,
  onCleanup,
  omit,
  untrack,
  useContext,
} from "#solid";
import { createElement, mergeProps, setProp, spread } from "../renderer/index.js";
import { tryBridgeFor } from "../host.js";
import { getReducedMotion, observeGeometry } from "../nodes.js";
import { createDelay } from "../timers.js";
import type { Accessor, NativeChild, NativeElementProps, NativeNode } from "../types.js";
import {
  ALERT_TRIANGLE,
  CHECK_CIRCLE,
  CROSS,
  CROSS_CIRCLE,
  INFO,
  Icon,
  SPINNER,
  type IconData,
} from "./icons.js";
import { overlayPortal } from "./overlay.js";
import { append, classes, element, provide, slot, useStylesheet } from "./shared.js";
import { sheet } from "./generated/ToastStyle.js";

export type ToastCorner = "bottom-right" | "bottom-left" | "top-right" | "top-left";
export type ToastKind = "default" | "success" | "info" | "warning" | "error" | "loading";

export interface ToastAction {
  readonly label: string;
  readonly tone?: "default" | "destructive" | undefined;
  readonly onClick: () => void;
  readonly dismiss?: boolean | undefined;
}

export interface ToastOptions {
  readonly id?: string | undefined;
  readonly title: string;
  readonly description?: string | undefined;
  readonly kind?: ToastKind | undefined;
  readonly duration?: number | undefined;
  readonly dismissible?: boolean | undefined;
  readonly action?: ToastAction | undefined;
  readonly cancel?: ToastAction | undefined;
  readonly onDismiss?: (() => void) | undefined;
}

export interface ToastPromiseMessages<T> {
  readonly loading: string;
  readonly success: string | ((value: T) => string | ToastOptions);
  readonly error: string | ((error: unknown) => string | ToastOptions);
}

export interface ToastRecord {
  readonly id: string;
  readonly title: string;
  readonly description?: string | undefined;
  readonly kind: ToastKind;
  readonly duration: number;
  readonly dismissible: boolean;
  readonly action?: ToastAction | undefined;
  readonly cancel?: ToastAction | undefined;
  readonly onDismiss?: (() => void) | undefined;
}

export interface ToastController {
  readonly show: (
    titleOrOptions: string | ToastOptions,
    options?: Omit<ToastOptions, "title">,
  ) => string;
  readonly success: (title: string, options?: Omit<ToastOptions, "title" | "kind">) => string;
  readonly error: (title: string, options?: Omit<ToastOptions, "title" | "kind">) => string;
  readonly info: (title: string, options?: Omit<ToastOptions, "title" | "kind">) => string;
  readonly warning: (title: string, options?: Omit<ToastOptions, "title" | "kind">) => string;
  readonly dismiss: (id?: string) => void;
  readonly promise: <T>(
    work: Promise<T> | (() => Promise<T>),
    messages: ToastPromiseMessages<T>,
  ) => Promise<T>;
  readonly items: Accessor<readonly ToastRecord[]>;
}

export interface ToastProviderProps extends Omit<NativeElementProps, "children"> {
  readonly children?: NativeChild;
  readonly corner?: ToastCorner | undefined;
  readonly maxVisible?: number | undefined;
  readonly defaultDuration?: number | undefined;
  readonly gap?: number | undefined;
  readonly regionTestId?: string | undefined;
}

const DEFAULT_DURATION_MS = 4000;
const DEFAULT_MAX_VISIBLE = 5;
const DEFAULT_TOAST_HEIGHT = 60;
const DEFAULT_TOAST_GAP = 14;

const ToastContext = createContext<ToastController | null>(null);
const controllersByBridge = new WeakMap<object, ToastController>();
let fallbackController: ToastController | null = null;
let toastSequence = 0;

function nextToastId(): string {
  toastSequence += 1;
  return `zui-toast-${toastSequence}`;
}

function normalizeOptions(
  titleOrOptions: string | ToastOptions,
  extra?: Omit<ToastOptions, "title">,
  forcedKind?: ToastKind,
): ToastOptions {
  if (typeof titleOrOptions === "string") {
    return { ...extra, title: titleOrOptions, kind: forcedKind ?? extra?.kind };
  }
  return { ...titleOrOptions, kind: forcedKind ?? titleOrOptions.kind };
}

function resolveController(): ToastController | null {
  const bridge = tryBridgeFor();
  if (bridge) {
    const bound = controllersByBridge.get(bridge);
    if (bound) return bound;
  }
  return fallbackController;
}

function iconForKind(kind: ToastKind): IconData | null {
  switch (kind) {
    case "success":
      return CHECK_CIRCLE;
    case "info":
      return INFO;
    case "warning":
      return ALERT_TRIANGLE;
    case "error":
      return CROSS_CIRCLE;
    case "loading":
      return SPINNER;
    default:
      return null;
  }
}

export const toast: Omit<ToastController, "items"> = {
  show(titleOrOptions, options) {
    const active = resolveController();
    if (!active) return "";
    return active.show(titleOrOptions, options);
  },
  success(title, options) {
    const active = resolveController();
    if (!active) return "";
    return active.success(title, options);
  },
  error(title, options) {
    const active = resolveController();
    if (!active) return "";
    return active.error(title, options);
  },
  info(title, options) {
    const active = resolveController();
    if (!active) return "";
    return active.info(title, options);
  },
  warning(title, options) {
    const active = resolveController();
    if (!active) return "";
    return active.warning(title, options);
  },
  dismiss(id) {
    resolveController()?.dismiss(id);
  },
  promise(work, messages) {
    const active = resolveController();
    if (!active) return typeof work === "function" ? work() : work;
    return active.promise(work, messages);
  },
};

export function useToast(): ToastController {
  const ctx = useContext(ToastContext) ?? resolveController();
  if (!ctx) {
    throw new Error("useToast() must be called inside a <ToastProvider>");
  }
  return ctx;
}

interface ToastRowProps {
  readonly record: ToastRecord;
  readonly index: Accessor<number>;
  readonly paused: Accessor<boolean>;
  readonly offsetFor: (id: string) => number;
  readonly reportHeight: (id: string, height: number) => void;
  readonly dismiss: (id: string) => void;
}

function nowMs(): number {
  const perf = (globalThis as { performance?: { now?: () => number } }).performance;
  return typeof perf?.now === "function" ? perf.now() : Date.now();
}

function ToastRow(props: ToastRowProps): NativeNode {
  const timer = createDelay();
  let remainingMs = props.record.duration;
  let startedAt = 0;

  const armTimer = () => {
    timer.cancel();
    if (!Number.isFinite(remainingMs) || remainingMs <= 0) return;
    startedAt = nowMs();
    timer.schedule(remainingMs, () => props.dismiss(props.record.id));
  };

  createEffect(
    () => ({ duration: props.record.duration, kind: props.record.kind }),
    ({ duration }) => {
      remainingMs = duration;
      if (!untrack(props.paused)) armTimer();
    },
  );

  createEffect(props.paused, isPaused => {
    if (!Number.isFinite(remainingMs) || remainingMs <= 0) {
      timer.cancel();
      return;
    }
    if (isPaused) {
      if (startedAt > 0) {
        remainingMs = Math.max(0, remainingMs - (nowMs() - startedAt));
        startedAt = 0;
      }
      timer.cancel();
    } else {
      armTimer();
    }
  });

  const slotNode = createElement("box");
  spread(
    slotNode,
    {
      __zguiClass: "zui-toast__slot",
      get "data-front"() {
        return props.index() === 0 ? "true" : "false";
      },
      get __zguiStyle() {
        const idx = props.index();
        return {
          "--zui-toast-offset": `${props.offsetFor(props.record.id)}px`,
          "--zui-toast-depth": String(idx),
          "--zui-toast-layer": String(100 - idx),
        };
      },
    },
    true,
  );

  const card = createElement("box");
  const isUrgent = props.record.kind === "error" || props.record.kind === "warning";
  spread(
    card,
    {
      role: isUrgent ? "alert" : "status",
      ariaLive: isUrgent ? "assertive" : "polite",
      testId: `toast-item-${props.record.id}`,
      "data-kind": props.record.kind,
      "data-state": "open",
      __zguiClass: "zui-toast",
    },
    true,
  );

  observeGeometry(card, "contentSize", size => {
    if (size.height > 0) props.reportHeight(props.record.id, size.height);
  });

  const icon = iconForKind(props.record.kind);
  if (icon) {
    const iconBox = element("box", { class: "zui-toast__icon", ariaHidden: true });
    append(iconBox, Icon({ icon, size: "sm" }));
    append(card, iconBox);
  }

  const textBox = element("box", { class: "zui-toast__text" });
  append(textBox, element("label", { class: "zui-toast__title", children: props.record.title }));
  if (props.record.description) {
    append(
      textBox,
      element("label", { class: "zui-toast__description", children: props.record.description }),
    );
  }
  append(card, textBox);

  if (props.record.cancel) {
    const cancelSpec = props.record.cancel;
    const cancelBtn = element("control", {
      role: "button",
      tabIndex: 0,
      class: "zui-toast__cancel",
      children: cancelSpec.label,
      onClick() {
        cancelSpec.onClick();
        if (cancelSpec.dismiss !== false) props.dismiss(props.record.id);
      },
    });
    append(card, cancelBtn);
  }

  if (props.record.action) {
    const actionSpec = props.record.action;
    const actionBtn = element("control", {
      role: "button",
      tabIndex: 0,
      class: "zui-toast__action",
      "data-tone": actionSpec.tone ?? "default",
      testId: `toast-action-${props.record.id}`,
      children: actionSpec.label,
      onClick() {
        actionSpec.onClick();
        if (actionSpec.dismiss !== false) props.dismiss(props.record.id);
      },
    });
    append(card, actionBtn);
  }

  if (props.record.dismissible) {
    const closeBtn = element("control", {
      role: "button",
      tabIndex: 0,
      ariaLabel: "Dismiss notification",
      class: "zui-toast__close",
      testId: `toast-close-${props.record.id}`,
      onClick() {
        props.dismiss(props.record.id);
      },
    });
    append(closeBtn, Icon({ icon: CROSS, size: "sm" }));
    append(card, closeBtn);
  }

  append(slotNode, card);
  return slotNode;
}

export function ToastProvider(props: ToastProviderProps = {}): NativeChild {
  useStylesheet(sheet);
  const rest = omit(
    props,
    "children",
    "corner",
    "maxVisible",
    "defaultDuration",
    "gap",
    "regionTestId",
  );
  const [items, setItems] = createSignal<readonly ToastRecord[]>([], { ownedWrite: true });
  const [paused, setPaused] = createSignal(false, { ownedWrite: true });
  const [heights, setHeights] = createSignal<Readonly<Record<string, number>>>(
    {},
    { ownedWrite: true },
  );

  const maxVisible = () => {
    const n = props.maxVisible ?? DEFAULT_MAX_VISIBLE;
    return Number.isInteger(n) && n > 0 ? n : DEFAULT_MAX_VISIBLE;
  };
  const defaultDuration = () => {
    const ms = props.defaultDuration ?? DEFAULT_DURATION_MS;
    return Number.isFinite(ms) && ms >= 0 ? ms : DEFAULT_DURATION_MS;
  };
  const gapPx = () => {
    const g = props.gap ?? DEFAULT_TOAST_GAP;
    return Number.isFinite(g) && g >= 0 ? g : DEFAULT_TOAST_GAP;
  };

  const dismiss = (id?: string) => {
    if (id === undefined) {
      const removed = untrack(items);
      setItems([]);
      setHeights({});
      for (const item of removed) item.onDismiss?.();
      return;
    }
    let dismissedItem: ToastRecord | undefined;
    setItems(prev =>
      prev.filter(item => {
        if (item.id === id) {
          dismissedItem = item;
          return false;
        }
        return true;
      }),
    );
    if (dismissedItem) {
      setHeights(prev => {
        if (!(id in prev)) return prev;
        const next = { ...prev };
        delete next[id];
        return next;
      });
      dismissedItem.onDismiss?.();
    }
  };

  const upsert = (opts: ToastOptions): string => {
    const id = opts.id ?? nextToastId();
    const kind: ToastKind = opts.kind ?? "default";
    const rawDuration = opts.duration;
    const duration =
      rawDuration !== undefined
        ? rawDuration === 0
          ? Infinity
          : rawDuration
        : kind === "loading"
          ? Infinity
          : defaultDuration();
    const record: ToastRecord = {
      id,
      title: opts.title,
      description: opts.description,
      kind,
      duration,
      dismissible: opts.dismissible ?? true,
      action: opts.action,
      cancel: opts.cancel,
      onDismiss: opts.onDismiss,
    };
    const evicted: ToastRecord[] = [];
    setItems(prev => {
      const existing = prev.findIndex(item => item.id === id);
      if (existing >= 0) {
        const next = [...prev];
        next[existing] = record;
        return next;
      }
      const combined = [record, ...prev];
      const limit = maxVisible();
      if (combined.length > limit) {
        evicted.push(...combined.slice(limit));
        return combined.slice(0, limit);
      }
      return combined;
    });
    for (const item of evicted) item.onDismiss?.();
    return id;
  };

  const controller: ToastController = {
    show: (titleOrOptions, options) => upsert(normalizeOptions(titleOrOptions, options)),
    success: (title, options) => upsert(normalizeOptions(title, options, "success")),
    error: (title, options) => upsert(normalizeOptions(title, options, "error")),
    info: (title, options) => upsert(normalizeOptions(title, options, "info")),
    warning: (title, options) => upsert(normalizeOptions(title, options, "warning")),
    dismiss,
    async promise<T>(
      work: Promise<T> | (() => Promise<T>),
      messages: ToastPromiseMessages<T>,
    ): Promise<T> {
      const id = upsert({ title: messages.loading, kind: "loading", duration: Infinity });
      try {
        const result = await (typeof work === "function" ? work() : work);
        const resolved =
          typeof messages.success === "function" ? messages.success(result) : messages.success;
        upsert({ ...normalizeOptions(resolved, undefined, "success"), id });
        return result;
      } catch (err) {
        const resolved =
          typeof messages.error === "function" ? messages.error(err) : messages.error;
        upsert({ ...normalizeOptions(resolved, undefined, "error"), id });
        throw err;
      }
    },
    items,
  };

  const hostBridge = tryBridgeFor();
  if (hostBridge) controllersByBridge.set(hostBridge, controller);
  fallbackController = controller;
  onCleanup(() => {
    if (hostBridge && controllersByBridge.get(hostBridge) === controller) {
      controllersByBridge.delete(hostBridge);
    }
    if (fallbackController === controller) fallbackController = null;
    setItems([]);
  });

  const reportHeight = (id: string, height: number) => {
    setHeights(prev => (prev[id] === height ? prev : { ...prev, [id]: height }));
  };

  const offsetFor = (id: string): number => {
    const list = items();
    const map = heights();
    const gap = gapPx();
    let offset = 0;
    for (const item of list) {
      if (item.id === id) return offset;
      offset += (map[item.id] ?? DEFAULT_TOAST_HEIGHT) + gap;
    }
    return offset;
  };

  const totalExtent = (): number => {
    const list = items();
    if (list.length === 0) return 0;
    const map = heights();
    const gap = gapPx();
    let sum = 0;
    for (let i = 0; i < list.length; i++) {
      sum += map[list[i]!.id] ?? DEFAULT_TOAST_HEIGHT;
      if (i > 0) sum += gap;
    }
    return sum;
  };

  const mountRegion = () =>
    overlayPortal({ layer: "toast", depth: 0 }, () => {
      const region = createElement("box");
      spread(
        region,
        mergeProps(rest, {
          role: "group" as const,
          ariaLabel: "Notifications",
          get testId() {
            return props.regionTestId ?? "toast-region";
          },
          get "data-corner"() {
            return props.corner ?? "bottom-right";
          },
          "data-expanded": "true",
          get "data-paused"() {
            return paused() ? "true" : "false";
          },
          get __zguiClass() {
            return classes(sheet.className, "zui-toast__region");
          },
          get __zguiStyle() {
            return {
              "--zui-toast-extent": `${totalExtent()}px`,
              "--zui-toast-gap": `${gapPx()}px`,
              ...(getReducedMotion() ? { "--zui-toast-move": "0ms" } : {}),
            };
          },
          onPointerEnter() {
            setPaused(true);
          },
          onPointerLeave() {
            setPaused(false);
          },
        }),
        true,
      );

      slot(
        region,
        For({
          get each() {
            return items();
          },
          children(record: ToastRecord, index: Accessor<number>) {
            return ToastRow({
              record,
              index,
              paused,
              offsetFor,
              reportHeight,
              dismiss,
            });
          },
        }),
      );
      return region;
    });

  const holder = createElement("box");
  setProp(holder, "__zguiInvariantStyle", { display: "none" });
  slot(holder, () => (items().length > 0 ? untrack(mountRegion) : null));
  onCleanup(() => {
    setItems([]);
  });

  if (!("children" in props)) return holder;
  return [provide(ToastContext, controller, props), holder];
}
