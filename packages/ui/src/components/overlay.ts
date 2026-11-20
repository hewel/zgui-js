import {
  createContext,
  createEffect,
  createMemo,
  createRoot,
  createSignal,
  getOwner,
  onCleanup,
  omit,
  runWithOwner,
  untrack,
  useContext,
} from "#solid";
import { createElement, mergeProps, setProp, spread } from "../renderer/index.js";
import { isAnimating, lockScroll, registerDismissLayer, trapFocus } from "../nodes.js";
import { allocatePortal } from "../portal.js";
import type {
  Accessor,
  NativeAnimationEvent,
  NativeChild,
  NativeDismissReason,
  NativeElementProps,
  NativeEvent,
  NativeNode,
  NativeOverlayLayer,
  NativeRole,
  NativeTransitionEvent,
} from "../types.js";
import { Button } from "./button.js";
import type { ButtonProps } from "./button.js";
import { Popper } from "./popper.js";
import type { PopperPlacement } from "./popper.js";
import { callHandler, createControllable } from "./state.js";
import { append, classes, element, provide, slot, type Stylesheet, useStylesheet, withDefaults } from "./shared.js";
import { portalThemeScope, useTheme } from "./theme.js";
import { sheet } from "./generated/OverlayStyle.js";

export interface OverlayRootProps {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  children?: NativeChild;
}

export interface AnchoredContentProps extends NativeElementProps {
  placement?: PopperPlacement;
  offset?: number;
  dismissOnOutsidePress?: boolean;
  dismissOnEscape?: boolean;
}

interface DismissalContentProps extends NativeElementProps {
  dismissOnOutsidePress?: boolean;
  dismissOnEscape?: boolean;
}

interface OverlaySurfaceConfig {
  layer: NativeOverlayLayer;
  class: string;
  role: NativeRole;
  trap?: boolean;
  lock?: boolean;
  surfaceClass?: boolean | undefined;
  onDismiss?: ((reason: NativeDismissReason) => void) | undefined;
  children?: (() => NativeChild) | undefined;
  wrap: (surface: NativeNode, presenceState: () => "open" | "closed") => NativeChild;
}

function createPresence(present: Accessor<boolean>) {
  const [mounted, setMounted] = createSignal(untrack(present));
  let surface: NativeNode | undefined;
  let frame: number | undefined;
  let pending: number | undefined;
  let deadline: number | undefined;
  let generation = 0;
  const cancel = () => {
    generation += 1;
    if (frame !== undefined) cancelAnimationFrame(frame);
    if (pending !== undefined) clearTimeout(pending);
    if (deadline !== undefined) clearTimeout(deadline);
    frame = pending = deadline = undefined;
  };
  const leave = () => { cancel(); setMounted(false); };
  const finish = () => {
    if (!untrack(present) && untrack(mounted) && (!surface || !isAnimating(surface))) leave();
  };
  createEffect(() => present(), open => {
    cancel();
    if (open) { setMounted(true); return; }
    if (!untrack(mounted)) return;
    const current = generation;
    frame = requestAnimationFrame(() => {
      if (current !== generation) return;
      frame = undefined;
      pending = setTimeout(() => {
        if (current !== generation) return;
        pending = undefined;
        finish();
      }, 0);
    });
    deadline = setTimeout(() => {
      if (current === generation && !untrack(present)) leave();
    }, 1000);
  });
  onCleanup(() => { cancel(); surface = undefined; });
  return {
    mounted,
    state: (): "open" | "closed" => present() ? "open" : "closed",
    ref(node: NativeNode) {
      surface = node;
      onCleanup(() => { if (surface === node) surface = undefined; });
    },
    onEnd(event: NativeAnimationEvent | NativeTransitionEvent) {
      if (frame === undefined && pending === undefined && event.target === surface) finish();
    },
  };
}

function reference() {
  const [node, setNode] = createSignal<NativeNode | null>(null);
  return {
    node,
    ref(value: NativeNode) {
      setNode(value);
      onCleanup(() => setNode(current => current === value ? null : current));
    },
  };
}

export function createOverlayState(props: OverlayRootProps) {
  const [read, setOpen] = createControllable<boolean>({
    value: () => props.open, defaultValue: () => props.defaultOpen, onChange: () => props.onOpenChange,
  }, false);
  const open = createMemo(() => {
    const value = read();
    if (typeof value !== "boolean") throw new TypeError("Overlay open must be a boolean");
    return value;
  });
  return { open, setOpen, trigger: reference(), content: reference(), title: reference(), description: reference() };
}

export type OverlayState = ReturnType<typeof createOverlayState>;
export const OverlayContext = createContext<OverlayState | null>(null);
export function OverlayRoot(props: OverlayRootProps): NativeChild {
  return provide(OverlayContext, createOverlayState(props), props);
}

interface Elevation { layer: NativeOverlayLayer; depth: number; }
const ElevationContext = createContext<Elevation | null>(null);
const layers: readonly NativeOverlayLayer[] = ["content", "popover", "modal", "toast"];
export function raiseSurface(wanted: NativeOverlayLayer): Elevation {
  const parent = useContext(ElevationContext);
  return {
    layer: parent && layers.indexOf(parent.layer) > layers.indexOf(wanted) ? parent.layer : wanted,
    depth: parent ? parent.depth + 1 : 0,
  };
}

export function overlayPortal(elevation: Elevation, build: () => NativeChild): NativeNode {
  useStylesheet(sheet);
  const theme = useTheme();
  const body = (): NativeNode => {
    const box = createElement("box");
    setProp(box, "class", "zui-overlay-depth");
    setProp(box, "__zguiInvariantStyle", { "--zui-overlay-depth": String(elevation.depth) });
    const built = untrack(build);
    if (Array.isArray(built)) {
      for (const item of built) if (item != null) append(box, item as NativeNode);
    } else if (built != null) {
      append(box, built as NativeNode);
    }
    return box;
  };
  const root = allocatePortal(elevation.layer);
  const parentOwner = getOwner();
  let cancelled = false;
  onCleanup(() => { cancelled = true; });
  createEffect(
    () => true,
    () => {
      if (cancelled) return;
      let disposeBranch: (() => void) | undefined;
      runWithOwner(parentOwner, () => {
        createRoot(dispose => {
          disposeBranch = dispose;
          slot(root, provide(ElevationContext, elevation, () => {
            const content = body();
            return theme ? portalThemeScope(theme, content) : content;
          }));
        });
      });
      return () => disposeBranch?.();
    },
  );
  return root;
}

function mountOverlay(state: OverlayState | null | undefined, props: DismissalContentProps, config: OverlaySurfaceConfig): NativeChild {
  useStylesheet(sheet);
  const presence = createPresence(() => state?.open() ?? false);
  const elevation = raiseSurface(config.layer);
  const holder = createElement("box");
  setProp(holder, "__zguiInvariantStyle", { display: "none" });
  slot(holder, () => presence.mounted() ? untrack(() => overlayPortal(elevation, () => {
    const rest = omit(props as AnchoredContentProps, "children", "placement", "offset", "dismissOnOutsidePress", "dismissOnEscape", "onAnimationEnd", "onAnimationCancel", "onTransitionEnd", "onTransitionCancel");
    const context = state!;
    const surface = createElement("box");
    context.content.ref(surface);
    presence.ref(surface);
    if (config.trap) trapFocus(surface);
    const options = () => ({
      layer: elevation.layer,
      dismissOnOutsidePress: props.dismissOnOutsidePress ?? true,
      dismissOnEscape: props.dismissOnEscape ?? true,
      exclude: context.trigger.node(), leaving: !context.open(),
    });
    const dismiss = registerDismissLayer(surface, reason => {
      if (config.onDismiss) config.onDismiss(reason); else context.setOpen(false);
    }, untrack(options));
    createEffect(options, next => { dismiss.update(next); });
    if (config.lock) createEffect(() => context.open(), open => (open ? lockScroll(surface) : undefined));
    spread(surface, mergeProps(rest, withDefaults({
      ...(config.trap ? { tabIndex: -1 } : {}),
      role: config.role,
      ariaModal: config.trap ?? false,
      get ariaPopupFor() { return context.trigger.node(); },
      get "data-state"() { return presence.state(); },
    }, rest), {
      get __zguiClass() { return classes(config.surfaceClass === false ? null : "zui-surface", config.class); },
      onAnimationEnd(event: NativeAnimationEvent) { props.onAnimationEnd?.(event); presence.onEnd(event); },
      onAnimationCancel(event: NativeAnimationEvent) { props.onAnimationCancel?.(event); presence.onEnd(event); },
      onTransitionEnd(event: NativeTransitionEvent) { props.onTransitionEnd?.(event); presence.onEnd(event); },
      onTransitionCancel(event: NativeTransitionEvent) { props.onTransitionCancel?.(event); presence.onEnd(event); },
    }), true);
    if (config.children) {
      const built = config.children();
      if (built != null && typeof built === "object" && !Array.isArray(built)) {
        append(surface, built as NativeNode);
      } else {
        slot(surface, built);
      }
    } else {
      slot(surface, () => props.children);
    }
    return config.wrap(surface, presence.state);
  })) : null);
  return holder;
}

export function modalSurface(state: OverlayState | null | undefined, props: DismissalContentProps,
  config: Pick<OverlaySurfaceConfig, "class" | "role" | "children">,
): NativeChild {
  return mountOverlay(state, props, {
    layer: "modal",
    class: config.class,
    role: config.role,
    trap: true,
    lock: true,
    children: config.children,
    wrap: (surface, presenceState) => {
      const scrim = element("box", {
        class: classes(sheet.className, "zui-overlay-scrim"), ariaHidden: true,
        get "data-state"() { return presenceState(); },
      });
      return [scrim, surface];
    },
  });
}

export function anchoredSurface(state: OverlayState | null | undefined, props: AnchoredContentProps,
  defaults: Pick<OverlaySurfaceConfig, "class" | "role" | "trap" | "surfaceClass" | "onDismiss" | "children">,
): NativeChild {
  return mountOverlay(state, props, {
    layer: "popover",
    ...defaults,
    wrap: surface => Popper({
      get anchor() { return state!.trigger.node(); },
      get placement() { return props.placement; },
      get offset() { return props.offset; },
      class: "zui-overlay-positioner",
      children: surface,
    }),
  });
}

export function overlayTrigger(popup: "dialog" | "menu", props: ButtonProps = {}): NativeNode {
  const context = useContext(OverlayContext);
  const rest = omit(props, "onClick", "ref");
  return Button(mergeProps(rest, withDefaults({
    get expanded() { return context?.open(); },
    get ariaHasPopup() { return context ? popup : null; },
    get ariaControls() { return context?.content.node() ?? null; },
    get "data-state"() { return context ? context.open() ? "open" : "closed" : null; },
  }, rest), {
    ref(node: NativeNode) { context?.trigger.ref(node); if (typeof props.ref === "function") props.ref(node); },
    onClick(event: NativeEvent) {
      if (callHandler(event, props.onClick) && !props.disabled) context?.setOpen(!context.open());
    },
  }));
}

export function overlayClose(props: ButtonProps = {}): NativeNode {
  const context = useContext(OverlayContext);
  const rest = omit(props, "onClick");
  return Button(mergeProps(rest, withDefaults({ variant: "ghost" as const }, rest), {
    onClick(event: NativeEvent) {
      if (callHandler(event, props.onClick) && !props.disabled) context?.setOpen(false);
    },
  }));
}

function overlayLabel(
  tag: "label" | "box",
  kind: "title" | "description",
  cssSheet: Stylesheet,
  className: string,
  props: NativeElementProps = {},
): NativeNode {
  useStylesheet(cssSheet);
  const context = useContext(OverlayContext);
  const rest = omit(props, "ref");
  return element(tag, mergeProps(rest, {
    ref(node: NativeNode) { context?.[kind].ref(node); if (typeof props.ref === "function") props.ref(node); },
    get __zguiClass() { return classes(className); },
  }));
}

export function overlayTitle(tag: "label" | "box", cssSheet: Stylesheet, className: string, props: NativeElementProps = {}): NativeNode {
  return overlayLabel(tag, "title", cssSheet, className, props);
}

export function overlayDescription(cssSheet: Stylesheet, className: string, props: NativeElementProps = {}): NativeNode {
  return overlayLabel("box", "description", cssSheet, className, props);
}
