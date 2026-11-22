import {
  LayerSurface,
  PopupSurface,
  WindowSurface,
  createSpring,
  cx,
  renderApp,
} from "@hewel/zgui";
import type { BlurRegion, NativeNode, SpringOptions, SpringPhysicsOptions } from "@hewel/zgui";
import { Button, ThemeProvider } from "@hewel/zgui/components";
import {
  MenuCheckboxItemPrimitive,
  MenuItemPrimitive,
  MenuListPrimitive,
  MenuRadioGroupPrimitive,
  MenuRadioItemPrimitive,
  MenuRootPrimitive,
  createOverlayState,
  createPresence,
  createRetainedPresence,
  createRovingFocus,
} from "@hewel/zgui/primitives";
import type { RetainedPresenceOptions } from "@hewel/zgui/primitives";
import { createRouter, RouterProvider, Outlet } from "@hewel/zgui/router";
import { httpRequest, spawnWorker } from "@hewel/zgui-services";

const maybeDisabled: boolean | undefined = undefined;
const maybeChecked: boolean | undefined = undefined;
const maybeClass: string | undefined = undefined;
const maybeOpen: boolean | undefined = undefined;
const maybeBlur: BlurRegion | undefined = undefined;

export function strictOptionalProbe(root: NativeNode) {
  const router = createRouter([{ path: "/", component: () => <text>home</text> }]);
  const overlay = createOverlayState({ open: maybeOpen, defaultOpen: false });
  const presence = createPresence(() => overlay.open());
  const springPhysics: SpringPhysicsOptions = {
    stiffness: undefined,
    damping: undefined,
    mass: undefined,
    precision: undefined,
    velocityPrecision: undefined,
  };
  const springOpts: SpringOptions = { ...springPhysics, initial: undefined };
  const spring = createSpring(() => (overlay.open() ? 1 : 0), springOpts);
  spring.configure({ stiffness: undefined, damping: undefined });
  const retainedOpts: RetainedPresenceOptions = { settled: undefined };
  const retained = createRetainedPresence(() => (overlay.open() ? "ok" : null), retainedOpts);
  const roving = createRovingFocus(() => "vertical");
  void [
    presence.mounted(),
    spring.settled(),
    retained.present(),
    retained.value(),
    roving.active(),
  ];
  void httpRequest;
  void spawnWorker;

  return renderApp(
    () => (
      <ThemeProvider>
        <RouterProvider router={router}>
          <box class={cx("base", maybeClass)}>
            <Button disabled={maybeDisabled}>Save</Button>
            <MenuRootPrimitive open={maybeOpen} defaultOpen={false}>
              <MenuListPrimitive>
                <MenuItemPrimitive disabled={maybeDisabled} onSelect={() => {}}>
                  Open
                </MenuItemPrimitive>
                <MenuCheckboxItemPrimitive checked={maybeChecked} disabled={maybeDisabled}>
                  Wrap
                </MenuCheckboxItemPrimitive>
                <MenuRadioGroupPrimitive value="a">
                  <MenuRadioItemPrimitive value="a" disabled={maybeDisabled}>
                    Option A
                  </MenuRadioItemPrimitive>
                </MenuRadioGroupPrimitive>
              </MenuListPrimitive>
            </MenuRootPrimitive>
            <Outlet />
            <WindowSurface
              name="glass-win"
              title="Glass"
              width={300}
              height={200}
              blurRegion={maybeBlur}
            />
            <LayerSurface name="glass-layer" blurRegion={maybeBlur} />
            <PopupSurface
              name="glass-pop"
              parent="glass-layer"
              anchorRect={{ x: 0, y: 0, width: 10, height: 10 }}
              width={100}
              blurRegion={maybeBlur}
            />
          </box>
        </RouterProvider>
      </ThemeProvider>
    ),
    root,
  );
}
