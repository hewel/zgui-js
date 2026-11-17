import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Dialog, DialogContent, DialogTitle, DropdownMenu, DropdownMenuContent, DropdownMenuTrigger, MenuItem, MenuSub, MenuSubContent, MenuSubTrigger, ThemeProvider } from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => <ThemeProvider scheme="light"><box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
    <Dialog defaultOpen><DialogContent testId="dialog" style="width: 480px"><DialogTitle>Initial menu stack</DialogTitle>
      <DropdownMenu defaultOpen><DropdownMenuTrigger testId="root-trigger">Files</DropdownMenuTrigger>
        <DropdownMenuContent testId="menu" style="width: 180px">
          <MenuSub defaultOpen><MenuSubTrigger testId="branch">Export</MenuSubTrigger>
            <MenuSubContent testId="submenu" style="width: 180px">
              <MenuSub defaultOpen><MenuSubTrigger testId="deep-trigger">More</MenuSubTrigger>
                <MenuSubContent testId="deep" style="width: 180px"><MenuItem testId="leaf" closeOnSelect={false}>PDF</MenuItem></MenuSubContent>
              </MenuSub>
            </MenuSubContent>
          </MenuSub>
        </DropdownMenuContent>
      </DropdownMenu>
    </DialogContent></Dialog>
  </box></ThemeProvider>, root);
}
