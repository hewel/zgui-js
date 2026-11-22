import {
  $PROXY,
  createContext,
  createEffect,
  createSignal,
  onCleanup,
  omit,
  untrack,
  useContext,
} from "#solid";
import { createElement, flush, memo, mergeProps, spread } from "../renderer/index.js";
import { sameDocument } from "../host.js";
import { focus, getTextContent, getWindowRect, observeGeometry } from "../nodes.js";
import type {
  Accessor,
  NativeChild,
  NativeDismissReason,
  NativeElementProps,
  NativeEvent,
  NativeFocusEvent,
  NativeKeyEvent,
  NativeNode,
  NativeOverlayLayer,
  NativePoint,
  NativePointerBoundaryEvent,
  NativePointerDownEvent,
  NativePointerEvent,
  NativeRole,
  NativeWindowRect,
} from "../types.js";
import { callHandler, createControllable } from "../components/state.js";
import { provide, slot, withDefaults } from "../components/shared.js";
import { createDelayed, type DelayedController } from "./delay.js";
import {
  createDismissLayer,
  createOverlayState,
  OverlayContext,
  type OverlayRootProps,
  type OverlayState,
} from "./overlay.js";
import { createRovingFocus, type GroupOrientation, type RovingFocusController } from "./roving.js";

export interface MenuHierarchy {
  readonly state: OverlayState;
  dismissCascade(): void;
  readonly sub?: { readonly state: OverlayState; readonly closing: DelayedController };
}

export interface MenuSurfaceContextValue {
  readonly node: Accessor<NativeNode | null>;
  readonly roving: RovingFocusController;
}

export interface MenuRadioContextValue {
  readonly value: Accessor<string>;
  choose(value: string): void;
}

export const MenuHierarchyContext = createContext<MenuHierarchy | null>(null);
export const MenuSurfaceContext = createContext<MenuSurfaceContextValue | null>(null);
export const MenuRadioContext = createContext<MenuRadioContextValue | null>(null);

export interface MenuRootPrimitiveProps extends OverlayRootProps {
  onDismiss?: (() => void) | undefined;
}

export interface MenuListPrimitiveProps extends Omit<NativeElementProps, "orientation"> {
  orientation?: GroupOrientation | undefined;
  autoFocus?: boolean | undefined;
  typeahead?: boolean | undefined;
  dismissOnEscape?: boolean | undefined;
  dismissOnOutsidePress?: boolean | undefined;
  dismissLayer?: NativeOverlayLayer | false | undefined;
  onDismiss?: ((reason: NativeDismissReason) => void) | undefined;
  label?: string | null | undefined;
}

export interface MenuItemPrimitiveProps extends NativeElementProps {
  closeOnSelect?: boolean | undefined;
  onSelect?: (() => void) | undefined;
}

export interface MenuCheckboxItemPrimitiveProps extends Omit<NativeElementProps, "checked"> {
  checked?: boolean | undefined;
  defaultChecked?: boolean | undefined;
  onCheckedChange?: ((checked: boolean) => void) | undefined;
  closeOnSelect?: boolean | undefined;
}

export interface MenuRadioGroupPrimitiveProps extends NativeElementProps {
  label?: string | null | undefined;
  value?: string | undefined;
  defaultValue?: string | undefined;
  onValueChange?: ((value: string) => void) | undefined;
}

export interface MenuRadioItemPrimitiveProps extends NativeElementProps {
  value: string;
  closeOnSelect?: boolean | undefined;
}

export type MenuSubPrimitiveProps = OverlayRootProps;
export type MenuSubTriggerPrimitiveProps = NativeElementProps;
export type MenuSubContentPrimitiveProps = MenuListPrimitiveProps;

type MenuSubOpenMode = "keyboard" | "hover" | "pointer" | "programmatic";
type MenuSubCloseReason = NativeDismissReason | "keyboard" | "programmatic";

interface MenuSubBranch {
  readonly state: OverlayState;
  readonly closing: DelayedController;
  readonly hierarchy: MenuHierarchyInternal;
  readonly order: () => number;
  openFrom(mode: MenuSubOpenMode): void;
  shouldAutoFocus(): boolean;
  cancelOpening(): void;
  cancelBranchClosing(): void;
  cancelSiblingOpening(): void;
  closeSelf(reason: MenuSubCloseReason): void;
  closeSubtree(): void;
  registerOpening(timer: DelayedController): void;
  unregisterOpening(timer: DelayedController): void;
  registerFocusFirst(callback: () => boolean): void;
  unregisterFocusFirst(callback: () => boolean): void;
}

interface MenuHierarchyInternal {
  readonly ownState: OverlayState;
  readonly parent: MenuHierarchyInternal | null;
  readonly sub?: MenuSubBranch;
  readonly children: Set<MenuSubBranch>;
  isClosing(): boolean;
  findDeepestOpenSub(): MenuSubBranch | null;
  closeDeepestSub(reason: MenuSubCloseReason): boolean;
  closeSiblingSubs(except: MenuSubBranch): void;
  cancelSiblingOpening(except: MenuSubBranch): void;
}

const hierarchyInternals = new WeakMap<MenuHierarchy, MenuHierarchyInternal>();
let subOpenSequence = 0;

function internalHierarchy(
  hierarchy: MenuHierarchy | null | undefined,
): MenuHierarchyInternal | null {
  return hierarchy ? (hierarchyInternals.get(hierarchy) ?? null) : null;
}

function findDeepestIn(children: ReadonlySet<MenuSubBranch>): MenuSubBranch | null {
  let latest: MenuSubBranch | null = null;
  for (const child of children) {
    if (!untrack(child.state.open)) continue;
    if (!latest || child.order() >= latest.order()) {
      latest = child;
    }
  }
  if (!latest) return null;
  return latest.hierarchy.findDeepestOpenSub() ?? latest;
}

/** Internal helper for styled menu surfaces sharing the same Escape branch policy. */
export function closeDeepestMenuSub(
  hierarchy: MenuHierarchy | null | undefined,
  reason: MenuSubCloseReason = "escape",
): boolean {
  return internalHierarchy(hierarchy)?.closeDeepestSub(reason) ?? false;
}

/** Internal helper for styled submenu surfaces honoring hover vs keyboard open focus policy. */
export function shouldAutoFocusMenuSub(hierarchy: MenuHierarchy | null | undefined): boolean {
  return internalHierarchy(hierarchy)?.sub?.shouldAutoFocus() ?? true;
}

/**
 * Pure hover-corridor test checking whether moving from `from` to `to` stays inside the inclusive
 * triangle pointing at `target`'s near vertical edge. All three inputs must share one coordinate frame.
 */
export function headingToward(
  from: NativePoint,
  to: NativePoint,
  target: NativeWindowRect,
): boolean {
  const near = from.x <= target.x ? target.x : target.x + target.width;
  const top = { x: near, y: target.y };
  const bottom = { x: near, y: target.y + target.height };
  const cross = (a: NativePoint, b: NativePoint) =>
    (b.x - a.x) * (to.y - a.y) - (b.y - a.y) * (to.x - a.x);
  const first = cross(from, top);
  const second = cross(top, bottom);
  const third = cross(bottom, from);
  return !((first < 0 || second < 0 || third < 0) && (first > 0 || second > 0 || third > 0));
}

export function createMenuTypeahead(
  group: RovingFocusController | undefined,
): (event: NativeKeyEvent) => void {
  const reset = createDelayed();
  let buffer = "";
  let alive = true;
  onCleanup(() => {
    alive = false;
    reset.cancel();
    buffer = "";
  });
  return (event: NativeKeyEvent) => {
    if (!alive || !group || event.isComposing || event.character == null) return;
    if (event.character === " " && buffer === "") return;
    buffer += event.character.toLowerCase();
    reset.after(1000, () => {
      buffer = "";
    });
    const characters = Array.from(buffer);
    const search =
      characters.length > 1 && characters.every(character => character === characters[0])
        ? characters[0]
        : buffer;
    if (
      search &&
      group.focusNext(node =>
        getTextContent(node)
          .replace(/^\p{White_Space}+/u, "")
          .toLowerCase()
          .startsWith(search),
      )
    ) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
}

export interface MenuRowOptions {
  role: NativeRole;
  readonly select: () => void;
  disabled?: (() => boolean) | undefined;
  extra?: NativeElementProps | ((node: NativeNode) => NativeElementProps) | undefined;
  overrides?: NativeElementProps | undefined;
  activateOnPress?: boolean | undefined;
  emitDataHooks?: boolean | undefined;
  enter?: ((activate: () => void) => void) | undefined;
  leave?: ((event: NativePointerBoundaryEvent, defocus: () => void) => void) | undefined;
  classGetter?: (() => string) | undefined;
}

export function createMenuRow(props: NativeElementProps, options: MenuRowOptions): NativeNode {
  const surface = useContext(MenuSurfaceContext);
  const rest = omit(
    props,
    "children",
    "ref",
    "onClick",
    "onPointerDown",
    "onPointerEnter",
    "onPointerLeave",
    "onFocus",
    "onBlur",
  );
  const node = createElement("control");
  const disabled = options.disabled ?? (() => !!props.disabled);
  const emitDataHooks = options.emitDataHooks ?? true;
  const item = surface?.roving.register(node, disabled);
  const [highlighted, setHighlighted] = createSignal(false);
  let within = false;
  let alive = true;
  onCleanup(() => {
    alive = false;
    within = false;
  });
  const activate = () => {
    item?.activate();
    focus(node);
  };
  const defocus = () => {
    if (!within) return;
    const target = surface?.node();
    if (target) focus(target);
  };
  const classLayer = options.classGetter
    ? [
        {
          get __zguiClass() {
            return options.classGetter!();
          },
        },
      ]
    : [];
  const dataHooksLayer = emitDataHooks
    ? [
        withDefaults(
          {
            get "data-highlighted"() {
              return highlighted() && !disabled() ? "true" : null;
            },
            get "data-disabled"() {
              return disabled() ? "true" : null;
            },
          },
          rest,
        ),
      ]
    : [];
  const extraProps = typeof options.extra === "function" ? options.extra(node) : options.extra;
  spread(
    node,
    mergeProps(
      rest,
      ...(extraProps ? [withDefaults(extraProps, rest)] : []),
      withDefaults(
        {
          role: options.role,
          get tabIndex() {
            return item?.tabIndex() ?? (disabled() ? -1 : 0);
          },
        },
        rest,
      ),
      ...dataHooksLayer,
      ...(options.overrides ? [options.overrides] : []),
      ...classLayer,
      {
        ref(value: NativeNode) {
          if (typeof props.ref === "function") props.ref(value);
        },
        onClick(event: NativeEvent) {
          if (callHandler(event, props.onClick) && alive && !disabled()) options.select();
        },
        onPointerDown(event: NativePointerDownEvent) {
          if (
            callHandler(event, props.onPointerDown) &&
            alive &&
            !disabled() &&
            event.button === "primary" &&
            (options.activateOnPress ?? true)
          ) {
            event.activateNow();
          }
        },
        onPointerEnter(event: NativePointerBoundaryEvent) {
          if (callHandler(event, props.onPointerEnter) && alive && !disabled()) {
            if (options.enter) options.enter(activate);
            else activate();
          }
        },
        onPointerLeave(event: NativePointerBoundaryEvent) {
          if (!callHandler(event, props.onPointerLeave) || !alive) return;
          if (options.leave) options.leave(event, defocus);
          else defocus();
        },
        onFocus(event: NativeFocusEvent) {
          within = true;
          if (emitDataHooks && alive && !disabled()) setHighlighted(true);
          if (callHandler(event, props.onFocus) && alive && !disabled()) item?.activate();
        },
        onBlur(event: NativeFocusEvent) {
          within = false;
          if (emitDataHooks) setHighlighted(false);
          props.onBlur?.(event);
        },
      },
    ),
    true,
  );
  return node;
}

export function provideMenuHierarchyRoot(
  state: OverlayState,
  onDismiss: () => void,
  source: { readonly children?: NativeChild } | (() => NativeChild),
): NativeChild {
  const children = new Set<MenuSubBranch>();
  let alive = true;
  let closingNow = false;
  let wasOpen = untrack(state.open);
  createEffect(
    () => state.open(),
    open => {
      if (open) wasOpen = true;
    },
  );
  onCleanup(() => {
    alive = false;
    children.clear();
  });
  const dismiss = () => {
    if (!alive) return;
    closingNow = true;
    try {
      onDismiss();
    } finally {
      closingNow = false;
    }
  };
  const hierarchy: MenuHierarchy = { state, dismissCascade: dismiss };
  const internal: MenuHierarchyInternal = {
    ownState: state,
    parent: null,
    children,
    isClosing() {
      return !alive || closingNow || (wasOpen && !untrack(state.open));
    },
    findDeepestOpenSub() {
      return findDeepestIn(children);
    },
    closeDeepestSub(reason) {
      const deepest = findDeepestIn(children);
      if (!deepest) return false;
      deepest.closeSelf(reason);
      return true;
    },
    closeSiblingSubs(except) {
      for (const sibling of [...children]) {
        if (sibling !== except) sibling.closeSubtree();
      }
    },
    cancelSiblingOpening(except) {
      for (const sibling of children) {
        if (sibling !== except) sibling.cancelOpening();
      }
    },
  };
  hierarchyInternals.set(hierarchy, internal);
  return provide(MenuHierarchyContext, hierarchy, source);
}

/**
 * Headless menu root providing OverlayContext and MenuHierarchyContext with zero Portal,
 * Popper, or stylesheet coupling. Suitable for WindowSurface, LayerSurface, and PopupSurface.
 */
export function MenuRootPrimitive(props: MenuRootPrimitiveProps = {}): NativeChild {
  const state = createOverlayState(props);
  return provide(OverlayContext, state, () =>
    provideMenuHierarchyRoot(
      state,
      () => {
        state.setOpen(false);
        props.onDismiss?.();
      },
      props,
    ),
  );
}

/**
 * Headless nested submenu root providing cascading dismissal, sibling replacement,
 * open-branch tracking, and hover-intent closing timer.
 */
export function MenuSubPrimitive(props: MenuSubPrimitiveProps = {}): NativeChild {
  const state = createOverlayState(props);
  const closing = createDelayed();
  const parent = useContext(MenuHierarchyContext);
  const parentInternal = internalHierarchy(parent);
  const children = new Set<MenuSubBranch>();
  const openings = new Set<DelayedController>();
  let focusFirstCallback: (() => boolean) | undefined;
  let alive = true;
  let closingNow = false;
  let wasOpen = untrack(state.open);
  let openMode: MenuSubOpenMode = "programmatic";
  let lastOpenOrder = untrack(state.open) ? ++subOpenSequence : 0;

  const cancelOpening = () => {
    for (const timer of openings) timer.cancel();
  };

  const cancelBranchClosing = () => {
    closing.cancel();
    parentInternal?.sub?.cancelBranchClosing();
  };

  const closeSubtree = () => {
    if (!alive) return;
    cancelOpening();
    closing.cancel();
    for (const child of [...children]) {
      child.closeSubtree();
    }
    if (untrack(state.open)) {
      closingNow = true;
      try {
        state.setOpen(false);
      } finally {
        closingNow = false;
      }
    }
  };

  const closeSelf = (reason: MenuSubCloseReason) => {
    if (!alive || !untrack(state.open)) return;
    cancelOpening();
    closing.cancel();
    const triggerBefore = untrack(state.trigger.node);
    const contentBefore = untrack(state.content.node);
    const wasSameDocument = sameDocument(triggerBefore, contentBefore);
    closingNow = true;
    try {
      state.setOpen(false);
      flush();
    } finally {
      closingNow = false;
    }
    if (!alive || untrack(state.open)) return;
    if (
      (reason === "keyboard" || reason === "escape") &&
      wasSameDocument &&
      !(parentInternal?.isClosing() ?? false)
    ) {
      const liveTrigger = untrack(state.trigger.node);
      if (liveTrigger && liveTrigger === triggerBefore) {
        focus(liveTrigger);
      }
    }
  };

  const internal: MenuHierarchyInternal = {
    ownState: state,
    parent: parentInternal,
    children,
    isClosing() {
      return (
        !alive ||
        closingNow ||
        (wasOpen && !untrack(state.open)) ||
        (parentInternal?.isClosing() ?? false)
      );
    },
    findDeepestOpenSub() {
      return findDeepestIn(children);
    },
    closeDeepestSub(reason) {
      const deepest = findDeepestIn(children);
      if (!deepest) return false;
      deepest.closeSelf(reason);
      return true;
    },
    closeSiblingSubs(except) {
      for (const sibling of [...children]) {
        if (sibling !== except) sibling.closeSubtree();
      }
    },
    cancelSiblingOpening(except) {
      for (const sibling of children) {
        if (sibling !== except) sibling.cancelOpening();
      }
    },
  };

  const branch: MenuSubBranch = {
    state,
    closing,
    hierarchy: internal,
    order: () => lastOpenOrder,
    openFrom(mode) {
      if (!alive) return;
      cancelBranchClosing();
      parentInternal?.cancelSiblingOpening(branch);
      openMode = mode;
      const wasMounted = untrack(state.content.node) !== null;
      if (untrack(state.open)) {
        lastOpenOrder = ++subOpenSequence;
        parentInternal?.closeSiblingSubs(branch);
        if (mode === "keyboard" && wasMounted) {
          focusFirstCallback?.();
        }
        return;
      }
      state.setOpen(true);
      if (untrack(state.open)) {
        lastOpenOrder = ++subOpenSequence;
        parentInternal?.closeSiblingSubs(branch);
        if (mode === "keyboard" && wasMounted) {
          focusFirstCallback?.();
        }
      }
    },
    shouldAutoFocus() {
      return openMode !== "hover";
    },
    cancelOpening,
    cancelBranchClosing,
    cancelSiblingOpening() {
      parentInternal?.cancelSiblingOpening(branch);
    },
    closeSelf,
    closeSubtree,
    registerOpening(timer) {
      openings.add(timer);
    },
    unregisterOpening(timer) {
      openings.delete(timer);
    },
    registerFocusFirst(callback) {
      focusFirstCallback = callback;
    },
    unregisterFocusFirst(callback) {
      if (focusFirstCallback === callback) focusFirstCallback = undefined;
    },
  };

  (internal as { sub?: MenuSubBranch }).sub = branch;
  parentInternal?.children.add(branch);

  createEffect(
    () => state.open(),
    open => {
      if (open) {
        wasOpen = true;
        lastOpenOrder = ++subOpenSequence;
        closing.cancel();
        parentInternal?.closeSiblingSubs(branch);
      } else {
        cancelOpening();
        closing.cancel();
        openMode = "programmatic";
      }
    },
  );

  onCleanup(() => {
    alive = false;
    cancelOpening();
    closing.cancel();
    openings.clear();
    children.clear();
    focusFirstCallback = undefined;
    parentInternal?.children.delete(branch);
  });

  const node: MenuHierarchy = {
    state: parent?.state ?? state,
    dismissCascade() {
      if (!alive) return;
      cancelOpening();
      closing.cancel();
      closingNow = true;
      try {
        state.setOpen(false);
        parent?.dismissCascade();
      } finally {
        closingNow = false;
      }
    },
    sub: { state, closing },
  };
  hierarchyInternals.set(node, internal);
  return provide(OverlayContext, state, () => provide(MenuHierarchyContext, node, props));
}

function attachPrimitiveChildren(
  node: NativeNode,
  props: { readonly children?: NativeChild | undefined },
): void {
  if (!($PROXY in props) && Object.getOwnPropertyDescriptor(props, "children")?.get === undefined) {
    if (props.children !== undefined) slot(node, props.children);
    return;
  }
  slot(
    node,
    memo(() => props.children),
  );
}

/**
 * Headless submenu trigger (`role="menuItem"`, `ariaHasPopup="menu"`) opening on named
 * `ArrowRight` / `Enter` / `Space` / click or hover after 120 ms, with same-document
 * DPI-aware hover corridor and cross-`PopupSurface` 180 ms grace timer.
 */
export function MenuSubTriggerPrimitive(props: MenuSubTriggerPrimitiveProps = {}): NativeNode {
  const hierarchy = useContext(MenuHierarchyContext);
  const internal = internalHierarchy(hierarchy);
  const sub = internal?.sub;
  const state = sub?.state ?? hierarchy?.sub?.state;
  const opening = createDelayed();
  const closing = sub?.closing ?? hierarchy?.sub?.closing ?? createDelayed();
  const rest = omit(props, "children", "onPointerMove", "onKeyDown");
  let alive = true;
  let scale = 1;
  let last: NativePoint = { x: 0, y: 0 };

  sub?.registerOpening(opening);
  createEffect(
    () => !!props.disabled,
    disabled => {
      if (disabled) {
        opening.cancel();
        closing.cancel();
      }
    },
  );
  onCleanup(() => {
    alive = false;
    opening.cancel();
    sub?.unregisterOpening(opening);
    if (!sub && !hierarchy?.sub) closing.cancel();
  });

  const device = (point: NativePoint): NativePoint => ({
    x: Math.fround(point.x * scale),
    y: Math.fround(point.y * scale),
  });

  const node = createMenuRow(
    mergeProps(rest, {
      onPointerMove(event: NativePointerEvent) {
        if (callHandler(event, props.onPointerMove) && alive) {
          last = device(event.position);
        }
      },
      onKeyDown(event: NativeKeyEvent) {
        if (
          !callHandler(event, props.onKeyDown) ||
          !alive ||
          props.disabled ||
          !state ||
          event.isComposing ||
          event.character != null
        ) {
          return;
        }
        if (event.key === "ArrowRight" || event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          event.stopPropagation();
          if (sub) sub.openFrom("keyboard");
          else state.setOpen(true);
        }
      },
    }),
    {
      role: "menuItem",
      disabled: () => !!props.disabled,
      activateOnPress: false,
      extra: triggerNode => ({
        ariaHasPopup: "menu" as const,
        get expanded() {
          return state?.open() ?? false;
        },
        get ariaControls() {
          const content = state?.content.node() ?? null;
          return sameDocument(triggerNode, content) ? content : null;
        },
        get "data-state"() {
          return state?.open() ? "open" : "closed";
        },
      }),
      select() {
        opening.cancel();
        closing.cancel();
        if (sub) sub.openFrom("pointer");
        else state?.setOpen(true);
      },
      enter(activate) {
        if (sub) {
          sub.cancelBranchClosing();
          sub.cancelSiblingOpening();
        } else {
          closing.cancel();
        }
        activate();
        if (alive && state) {
          opening.after(120, () => {
            if (!alive || props.disabled) return;
            if (sub) sub.openFrom("hover");
            else state.setOpen(true);
          });
        }
      },
      leave(event, defocus) {
        opening.cancel();
        if (!state?.open()) {
          defocus();
          return;
        }
        const surface = untrack(state.content.node);
        if (sameDocument(node, surface)) {
          const rect = surface ? getWindowRect(surface) : null;
          if (
            rect &&
            headingToward(last, device(event.position), {
              ...device(rect),
              width: Math.fround(rect.width * scale),
              height: Math.fround(rect.height * scale),
            })
          ) {
            return;
          }
        }
        closing.after(180, () => state.setOpen(false));
      },
    },
  );
  state?.trigger.ref(node);
  observeGeometry(node, "viewport", viewport => {
    scale = viewport.scale;
  });
  if ("children" in props) {
    attachPrimitiveChildren(node, props);
  }
  return node;
}

function createMenuSurfaceNode(
  props: MenuListPrimitiveProps,
  options: { readonly isSub: boolean },
): NativeNode {
  const hierarchy = useContext(MenuHierarchyContext);
  const internal = internalHierarchy(hierarchy);
  const sub = options.isSub ? internal?.sub : undefined;
  const state =
    (options.isSub
      ? (sub?.state ?? hierarchy?.sub?.state)
      : (hierarchy?.sub?.state ?? hierarchy?.state)) ?? useContext(OverlayContext);
  const orientation = () => props.orientation ?? "vertical";
  const roving = createRovingFocus(orientation, "active");
  const onTypeahead = createMenuTypeahead(roving);
  const rest = options.isSub
    ? omit(
        props,
        "children",
        "ref",
        "orientation",
        "autoFocus",
        "typeahead",
        "dismissOnEscape",
        "dismissOnOutsidePress",
        "dismissLayer",
        "onDismiss",
        "label",
        "onKeyDown",
        "onPointerEnter",
      )
    : omit(
        props,
        "children",
        "ref",
        "orientation",
        "autoFocus",
        "typeahead",
        "dismissOnEscape",
        "dismissOnOutsidePress",
        "dismissLayer",
        "onDismiss",
        "label",
        "onKeyDown",
      );
  const node = createElement("box");
  state?.content.ref(node);
  let alive = true;
  if (sub) {
    const focusFirst = () => roving.focusFirst();
    sub.registerFocusFirst(focusFirst);
    onCleanup(() => {
      alive = false;
      sub.unregisterFocusFirst(focusFirst);
    });
  } else {
    onCleanup(() => {
      alive = false;
    });
  }

  const dismiss = (reason: NativeDismissReason) => {
    if (!alive) return;
    if (reason === "escape" && internal?.closeDeepestSub("escape")) {
      return;
    }
    if (props.onDismiss) {
      props.onDismiss(reason);
      return;
    }
    if (sub) {
      sub.closeSelf(reason);
    } else {
      hierarchy?.dismissCascade();
    }
  };

  if (props.dismissLayer) {
    const layer = props.dismissLayer;
    createDismissLayer(node, dismiss, () => {
      const trigger = state?.trigger.node() ?? null;
      return {
        layer,
        dismissOnOutsidePress: props.dismissOnOutsidePress ?? true,
        dismissOnEscape: props.dismissOnEscape ?? true,
        exclude: sameDocument(node, trigger) ? trigger : null,
        leaving: state ? !state.open() : false,
      };
    });
  }

  const defaultAutoFocus = options.isSub ? (sub?.shouldAutoFocus() ?? true) : true;
  let focused = false;
  createEffect(
    () => ((props.autoFocus ?? defaultAutoFocus) && !focused ? roving.firstEnabled() : undefined),
    first => {
      if ((props.autoFocus ?? defaultAutoFocus) && !focused && first && roving.focusFirst()) {
        focused = true;
      }
    },
  );

  const subDefaults = options.isSub
    ? [
        withDefaults(
          {
            get ariaPopupFor() {
              const trigger = state?.trigger.node() ?? null;
              return sameDocument(node, trigger) ? trigger : null;
            },
          },
          rest,
        ),
      ]
    : [];

  const subPointerLayer = options.isSub
    ? [
        {
          onPointerEnter(event: NativePointerBoundaryEvent) {
            if (callHandler(event, props.onPointerEnter) && alive) {
              if (sub) sub.cancelBranchClosing();
              else hierarchy?.sub?.closing.cancel();
            }
          },
        },
      ]
    : [];

  spread(
    node,
    mergeProps(
      rest,
      ...subDefaults,
      withDefaults(
        {
          role: "menu" as const,
          tabIndex: -1,
          get ariaLabel() {
            return props.label ?? null;
          },
          get "data-orientation"() {
            return orientation();
          },
          get "data-state"() {
            return state ? (state.open() ? "open" : "closed") : "open";
          },
        },
        rest,
      ),
      ...subPointerLayer,
      {
        ref(value: NativeNode) {
          if (typeof props.ref === "function") props.ref(value);
        },
        onKeyDown(event: NativeKeyEvent) {
          if (!callHandler(event, props.onKeyDown) || !alive) return;
          if (event.isComposing) return;
          if (options.isSub && event.character == null && event.key === "ArrowLeft") {
            event.preventDefault();
            event.stopPropagation();
            if (!internal?.closeDeepestSub("keyboard")) {
              if (sub) sub.closeSelf("keyboard");
              else state?.setOpen(false);
            }
            return;
          }
          if (
            (props.dismissOnEscape ?? true) &&
            event.character == null &&
            event.key === "Escape"
          ) {
            event.preventDefault();
            event.stopPropagation();
            dismiss("escape");
            return;
          }
          if (callHandler(event, roving.onKeyDown) && (props.typeahead ?? true)) {
            onTypeahead(event);
          }
        },
      },
    ),
    true,
  );

  const surfaceValue: MenuSurfaceContextValue = {
    node: () => node,
    roving,
  };
  slot(node, provide(MenuSurfaceContext, surfaceValue, props));
  return node;
}

/**
 * Headless menu list container (`role="menu"`) with roving keyboard navigation, typeahead,
 * optional Escape/dismiss handling, and zero built-in styling.
 */
export function MenuListPrimitive(props: MenuListPrimitiveProps = {}): NativeNode {
  return createMenuSurfaceNode(props, { isSub: false });
}

/**
 * Headless submenu list container (`role="menu"`) sharing the same single-node roving,
 * typeahead, hover-grace cancellation, and deepest-branch `ArrowLeft`/`Escape` policy.
 * Caller owns conditional mounting, `Portal`/`Popper`, or `PopupSurface`.
 */
export function MenuSubContentPrimitive(props: MenuSubContentPrimitiveProps = {}): NativeNode {
  return createMenuSurfaceNode(props, { isSub: true });
}

/**
 * Headless menu item (`role="menuItem"`) with roving focus, hover highlight,
 * `data-highlighted` / `data-disabled` styling hooks, and zero stylesheets.
 */
export function MenuItemPrimitive(props: MenuItemPrimitiveProps = {}): NativeNode {
  const menu = useContext(MenuHierarchyContext);
  const rest = omit(props, "children", "closeOnSelect", "onSelect");
  const node = createMenuRow(rest, {
    role: "menuItem",
    disabled: () => !!props.disabled,
    select() {
      props.onSelect?.();
      if (props.closeOnSelect ?? true) {
        flush();
        menu?.dismissCascade();
      }
    },
  });
  attachPrimitiveChildren(node, props);
  return node;
}

/**
 * Headless checkbox menu item (`role="menuItemCheckBox"`) exposing `checked`,
 * `data-state="checked" | "unchecked"`, `data-highlighted`, and `data-disabled`.
 */
export function MenuCheckboxItemPrimitive(props: MenuCheckboxItemPrimitiveProps = {}): NativeNode {
  const menu = useContext(MenuHierarchyContext);
  const rest = omit(
    props,
    "children",
    "checked",
    "defaultChecked",
    "onCheckedChange",
    "closeOnSelect",
  );
  const [read, write] = createControllable<boolean>(
    {
      value: () => props.checked,
      defaultValue: () => props.defaultChecked,
      onChange: () => props.onCheckedChange,
    },
    false,
  );
  const validate = (value: boolean): boolean => {
    if (typeof value !== "boolean") {
      throw new TypeError("MenuCheckboxItemPrimitive checked must be a boolean");
    }
    return value;
  };
  validate(untrack(read));
  const checked = memo(() => validate(read()));
  const node = createMenuRow(rest, {
    role: "menuItemCheckBox",
    disabled: () => !!props.disabled,
    overrides: {
      get checked() {
        return checked();
      },
      get "data-state"() {
        return checked() ? "checked" : "unchecked";
      },
    },
    select() {
      write(!checked());
      if (props.closeOnSelect ?? true) {
        flush();
        menu?.dismissCascade();
      }
    },
  });
  attachPrimitiveChildren(node, props);
  return node;
}

/**
 * Headless radio group (`role="group"`) for `MenuRadioItemPrimitive`.
 */
export function MenuRadioGroupPrimitive(props: MenuRadioGroupPrimitiveProps = {}): NativeNode {
  const rest = omit(props, "children", "value", "defaultValue", "onValueChange", "label");
  const [read, choose] = createControllable<string>(
    {
      value: () => props.value,
      defaultValue: () => props.defaultValue,
      onChange: () => props.onValueChange,
    },
    "",
  );
  const validate = (current: string): string => {
    if (typeof current !== "string") {
      throw new TypeError("MenuRadioGroupPrimitive value must be a string");
    }
    return current;
  };
  validate(untrack(read));
  const value = memo(() => validate(read()));
  const node = createElement("box");
  spread(
    node,
    mergeProps(
      rest,
      withDefaults(
        {
          role: "group" as const,
          get ariaLabel() {
            return props.label ?? null;
          },
        },
        rest,
      ),
    ),
    true,
  );
  slot(node, provide(MenuRadioContext, { value, choose }, props));
  return node;
}

/**
 * Headless radio menu item (`role="menuItemRadio"`) exposing `checked`,
 * `data-state="checked" | "unchecked"`, `data-value`, `data-highlighted`, and `data-disabled`.
 */
export function MenuRadioItemPrimitive(props: MenuRadioItemPrimitiveProps): NativeNode {
  const menu = useContext(MenuHierarchyContext);
  const radio = useContext(MenuRadioContext);
  const rest = omit(props, "children", "value", "closeOnSelect");
  const validate = (current: string): string => {
    if (typeof current !== "string" || !current) {
      throw new TypeError("MenuRadioItemPrimitive value must be a non-empty string");
    }
    return current;
  };
  validate(props.value);
  const value = memo(() => validate(props.value));
  const checked = () => radio?.value() === value();
  const node = createMenuRow(rest, {
    role: "menuItemRadio",
    disabled: () => !!props.disabled,
    overrides: {
      get checked() {
        return checked();
      },
      get "data-state"() {
        return checked() ? "checked" : "unchecked";
      },
      get "data-value"() {
        return value();
      },
    },
    select() {
      radio?.choose(value());
      if (props.closeOnSelect ?? true) {
        flush();
        menu?.dismissCascade();
      }
    },
  });
  attachPrimitiveChildren(node, props);
  return node;
}
