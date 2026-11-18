import { batch, createContext, createSignal, onCleanup, splitProps, untrack, useContext } from "#solid";
import { createElement, memo, mergeProps, spread } from "../renderer/index.js";
import { getWindowRect } from "../nodes.js";
import type {
  Accessor, NativeChild, NativeContextMenuEvent, NativeElementProps, NativeNode, NativePointerDownEvent,
} from "../types.js";
import { MenuContent, MenuContext, menuRoot } from "./menu.js";
import type { MenuContentProps } from "./menu.js";
import type { OverlayRootProps, OverlayState } from "./overlay.js";
import { callHandler } from "./state.js";
import { append, classes, element, provide, slot, useStylesheet } from "./shared.js";
import { sheet } from "./generated/ContextMenuStyle.js";

export interface ContextMenuPoint {
  readonly x: number;
  readonly y: number;
}
export interface ContextMenuPlace {
  openAt(position: ContextMenuPoint): void;
}
export type ContextMenuProps = OverlayRootProps;
export type ContextMenuTriggerProps = NativeElementProps;
export type ContextMenuContentProps = MenuContentProps;

interface AnchorRecord {
  point: Accessor<ContextMenuPoint>;
  setArea(node: NativeNode | null): void;
}

const PlaceContext = createContext<ContextMenuPlace>();
const anchorState = new WeakMap<OverlayState, AnchorRecord>();

export function useContextMenuPlace(): ContextMenuPlace | undefined {
  return useContext(PlaceContext);
}

export function ContextMenu(props: ContextMenuProps = {}): NativeChild {
  return menuRoot(mergeProps(props, {
    get children() {
      const state = useContext(MenuContext)!.state;
      const [point, setPoint] = createSignal<ContextMenuPoint>({ x: 0, y: 0 });
      const [area, setArea] = createSignal<NativeNode | null>(null);
      let alive = true;
      onCleanup(() => { alive = false; });
      anchorState.set(state, { point, setArea });
      const openAt = (position: ContextMenuPoint) => {
        if (!position || !Number.isFinite(position.x) || !Number.isFinite(position.y)) {
          throw new RangeError("ContextMenu position must have finite x and y coordinates");
        }
        if (!alive) return;
        const host = untrack(area);
        const rect = host ? getWindowRect(host) : null;
        const x = rect ? position.x - rect.x : position.x;
        const y = rect ? position.y - rect.y : position.y;
        batch(() => {
          setPoint({ x, y });
          state.setOpen(true);
        });
      };
      return provide(PlaceContext, { openAt }, props);
    },
  }));
}

export function ContextMenuTrigger(props: ContextMenuTriggerProps = {}): NativeNode {
  useStylesheet(sheet);
  const state = useContext(MenuContext)?.state;
  const place = useContextMenuPlace();
  const record = state ? anchorState.get(state) : undefined;
  const [own, rest] = splitProps(props, ["children", "ref", "onContextMenu", "onPointerDown"]);
  const area = createElement("box");
  record?.setArea(area);
  let alive = true;
  onCleanup(() => {
    alive = false;
    record?.setArea(null);
  });
  spread(area, mergeProps({
    ariaHasPopup: "menu",
    get expanded() { return state?.open() ?? false; },
    get ariaControls() { return state?.content.node() ?? null; },
    get "data-state"() { return state?.open() ? "open" : "closed"; },
  }, rest, {
    get __zguiClass() { return classes(sheet.className, "zui-context-menu__area"); },
    onContextMenu(event: NativeContextMenuEvent) {
      if (!callHandler(event, own.onContextMenu) || !alive || rest.disabled) return;
      event.preventDefault();
      place?.openAt(event.position);
    },
    onPointerDown(event: NativePointerDownEvent) {
      if (!callHandler(event, own.onPointerDown) || !alive || rest.disabled) return;
      if (event.button === "secondary") {
        event.preventDefault();
        place?.openAt(event.position);
      }
    },
  }), true);
  if (typeof own.ref === "function") own.ref(area);
  const anchor = element("box", {
    class: "zui-context-menu__anchor",
    ariaHidden: true,
    get style() {
      const current = record?.point() ?? { x: 0, y: 0 };
      return { left: `${current.x}px`, top: `${current.y}px` };
    },
  });
  state?.trigger.ref(anchor);
  append(area, anchor);
  slot(area, memo(() => own.children), anchor);
  return area;
}

export function ContextMenuContent(props: ContextMenuContentProps = {}): NativeChild {
  return MenuContent(mergeProps({ placement: { side: "bottom", align: "start" } as const, offset: 0 }, props));
}
