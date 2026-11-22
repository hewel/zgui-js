import { omit, untrack, useContext } from "#solid";
import { createElement, memo, mergeProps, setProp } from "../renderer/index.js";
import { trapFocus } from "../nodes.js";
import type {
  NativeAnimationEvent,
  NativeChild,
  NativeElementProps,
  NativeNode,
  NativeTransitionEvent,
} from "../types.js";
import { createPresence } from "../primitives/overlay.js";
import {
  headingToward,
  MenuHierarchyContext,
  MenuSubContentPrimitive,
  MenuSubPrimitive,
  MenuSubTriggerPrimitive,
  shouldAutoFocusMenuSub,
} from "../primitives/menu.js";
import { overlayPortal, raiseSurface, type OverlayRootProps } from "./overlay.js";
import { Popper } from "./popper.js";
import { append, classes, slot, useStylesheet, withDefaults } from "./shared.js";
import { Icon } from "./icons.js";
import { CHEVRON_RIGHT } from "./generated/icons.js";
import { sheet as menuSheet } from "./generated/MenuStyle.js";
import { sheet as overlaySheet } from "./generated/OverlayStyle.js";
import type { MenuContentProps } from "./menu.js";

export { headingToward };

export type MenuSubProps = OverlayRootProps;
export interface MenuSubTriggerProps extends NativeElementProps {
  inset?: boolean;
}
export type MenuSubContentProps = MenuContentProps;

export function MenuSub(props: MenuSubProps = {}): NativeChild {
  return MenuSubPrimitive(props);
}

export function MenuSubTrigger(props: MenuSubTriggerProps = {}): NativeNode {
  useStylesheet(menuSheet);
  const rest = omit(props, "children", "inset");
  const node = MenuSubTriggerPrimitive(
    mergeProps(
      rest,
      withDefaults(
        {
          get "data-inset"() {
            return props.inset ? "true" : null;
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes("zui-menu__item", props.inset && "zui-menu__item--inset");
        },
      },
    ),
  );
  slot(
    node,
    memo(() => props.children),
  );
  append(node, Icon({ icon: CHEVRON_RIGHT, class: "zui-menu__chevron" }));
  return node;
}

export function MenuSubContent(props: MenuSubContentProps = {}): NativeChild {
  useStylesheet(menuSheet);
  useStylesheet(overlaySheet);
  const hierarchy = useContext(MenuHierarchyContext);
  const state = hierarchy?.sub?.state;
  const presence = createPresence(() => state?.open() ?? false);
  const elevation = raiseSurface("popover");
  const holder = createElement("box");
  setProp(holder, "__zguiInvariantStyle", { display: "none" });
  slot(holder, () =>
    presence.mounted()
      ? untrack(() =>
          overlayPortal(elevation, () => {
            const rest = omit(
              props,
              "placement",
              "offset",
              "orientation",
              "onAnimationEnd",
              "onAnimationCancel",
              "onTransitionEnd",
              "onTransitionCancel",
            );
            const surface = MenuSubContentPrimitive(
              mergeProps(rest, withDefaults({ ariaModal: true }, rest), {
                get orientation() {
                  return props.orientation ?? undefined;
                },
                dismissLayer: elevation.layer,
                autoFocus: false,
                get "data-state"() {
                  return presence.state();
                },
                get __zguiClass() {
                  return classes("zui-surface", menuSheet.className, "zui-menu", "zui-menu--sub");
                },
                onAnimationEnd(event: NativeAnimationEvent) {
                  props.onAnimationEnd?.(event);
                  presence.onEnd(event);
                },
                onAnimationCancel(event: NativeAnimationEvent) {
                  props.onAnimationCancel?.(event);
                  presence.onEnd(event);
                },
                onTransitionEnd(event: NativeTransitionEvent) {
                  props.onTransitionEnd?.(event);
                  presence.onEnd(event);
                },
                onTransitionCancel(event: NativeTransitionEvent) {
                  props.onTransitionCancel?.(event);
                  presence.onEnd(event);
                },
              }),
            );
            presence.ref(surface);
            trapFocus(surface, { autoFocus: shouldAutoFocusMenuSub(hierarchy) });
            return Popper({
              get anchor() {
                return state!.trigger.node();
              },
              get placement() {
                return props.placement ?? ({ side: "right", align: "start" } as const);
              },
              get offset() {
                return props.offset ?? -4;
              },
              children: surface,
            });
          }),
        )
      : null,
  );
  return holder;
}
