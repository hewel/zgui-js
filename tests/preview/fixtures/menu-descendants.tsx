import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger, MenuItem, MenuLabel, MenuTypeahead, ThemeProvider } from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => <ThemeProvider scheme="light"><box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
    <DropdownMenu><DropdownMenuTrigger testId="trigger" style="position: fixed; left: 32px; top: 24px">Descendant focus</DropdownMenuTrigger>
      <DropdownMenuContent testId="menu" ariaLabel="Descendant actions" style="width: 280px">
        <MenuLabel testId="label">Actions</MenuLabel>
        <MenuItem testId="row-a" closeOnSelect={false}><text>Alpha</text><control testId="child" tabIndex={0}>Inner control</control></MenuItem>
        <MenuItem testId="disabled" disabled>Disabled</MenuItem>
        <MenuItem testId="row-b" closeOnSelect={false}>Beta</MenuItem>
        <MenuItem testId="row-c" closeOnSelect={false}>Charlie</MenuItem>
        <control testId="loose" tabIndex={0}>Loose focus</control>
        <MenuTypeahead onKeyDown={event => event.stopPropagation()}><control testId="blocked" tabIndex={0}>Blocked propagation</control></MenuTypeahead>
      </DropdownMenuContent>
    </DropdownMenu>
  </box></ThemeProvider>, root);
}
