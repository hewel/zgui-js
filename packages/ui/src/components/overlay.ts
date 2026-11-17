import { createContext, createEffect, createMemo, createSignal, onCleanup, useContext, untrack } from "#solid";
import { createElement, insertNode, setProp } from "../renderer/index.js";
import { allocatePortal } from "../portal.js";
import type { NativeChild, NativeNode, NativeOverlayLayer } from "../types.js";
import { createControllable } from "./state.js";
import { provide, useStylesheet } from "./shared.js";
import { portalThemeScope, useTheme } from "./theme.js";
import { sheet } from "./generated/OverlayStyle.js";

export interface OverlayRootProps {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  children?: NativeChild;
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
export const OverlayContext = createContext<OverlayState>();
export function OverlayRoot(props: OverlayRootProps): NativeChild {
  return provide(OverlayContext, createOverlayState(props), props);
}

interface Elevation { layer: NativeOverlayLayer; depth: number; }
const ElevationContext = createContext<Elevation>();
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
    setProp(box, "--zui-overlay-depth", String(elevation.depth));
    const built = untrack(build);
    if (Array.isArray(built)) {
      for (const item of built) if (item != null) insertNode(box, item as NativeNode);
    } else if (built != null) {
      insertNode(box, built as NativeNode);
    }
    return box;
  };
  const root = allocatePortal(elevation.layer);
  createEffect(() => {
    provide(ElevationContext, elevation, () => {
      const content = body();
      insertNode(root, theme ? portalThemeScope(theme, content) : content);
      return null;
    });
  });
  return root;
}
