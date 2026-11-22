import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  MenuItem,
  MenuSub,
  MenuSubContent,
  MenuSubTrigger,
  ThemeProvider,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [open, setOpen] = createSignal(false),
      [requests, setRequests] = createSignal(0),
      [last, setLast] = createSignal("none");
    return (
      <ThemeProvider scheme="light">
        <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
          <DropdownMenu open={true}>
            <DropdownMenuTrigger
              testId="root-trigger"
              style="position: fixed; left: 32px; top: 24px"
            >
              Held root
            </DropdownMenuTrigger>
            <DropdownMenuContent testId="menu" style="width: 220px">
              <MenuSub
                open={open()}
                onOpenChange={value => {
                  setRequests(v => v + 1);
                  setLast(value ? "open" : "closed");
                }}
              >
                <MenuSubTrigger testId="branch">Controlled export</MenuSubTrigger>
                <MenuSubContent testId="submenu" style="width: 220px">
                  <MenuItem testId="keep" closeOnSelect={false}>
                    Keep
                  </MenuItem>
                </MenuSubContent>
              </MenuSub>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button
            testId="accept"
            style="position: fixed; left: 520px; top: 24px"
            onClick={() => setOpen(v => !v)}
          >
            Accept toggle
          </Button>
          <Button
            testId="outside"
            variant="outline"
            style="position: fixed; left: 680px; top: 24px"
          >
            Outside
          </Button>
          <column style="position: fixed; left: 32px; top: 340px; gap: 12px; font-size: 14px">
            <text>Root held open for controlled-branch checks</text>
            <text testId="open">{open() ? "open" : "closed"}</text>
            <text testId="requests">{requests()}</text>
            <text testId="last">{last()}</text>
          </column>
        </box>
      </ThemeProvider>
    );
  }, root);
}
