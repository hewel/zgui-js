import { cx, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
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
  createRovingFocus,
} from "@hewel/zgui/primitives";
import { createRouter, RouterProvider, Outlet } from "@hewel/zgui/router";
import { httpRequest, spawnWorker } from "@hewel/zgui-services";

const maybeDisabled: boolean | undefined = undefined;
const maybeChecked: boolean | undefined = undefined;
const maybeClass: string | undefined = undefined;
const maybeOpen: boolean | undefined = undefined;

export function strictOptionalProbe(root: NativeNode) {
  const router = createRouter([{ path: "/", component: () => <text>home</text> }]);
  const overlay = createOverlayState({ open: maybeOpen, defaultOpen: false });
  const presence = createPresence(() => overlay.open());
  const roving = createRovingFocus(() => "vertical");
  void presence.mounted();
  void roving.active();
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
          </box>
        </RouterProvider>
      </ThemeProvider>
    ),
    root,
  );
}
