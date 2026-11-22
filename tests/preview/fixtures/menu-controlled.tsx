import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  MenuCheckboxItem,
  MenuRadioGroup,
  MenuRadioItem,
  ThemeProvider,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [open, setOpen] = createSignal(false),
      [requests, setRequests] = createSignal(0);
    const [checks, setChecks] = createSignal(0),
      [radios, setRadios] = createSignal(0);
    return (
      <ThemeProvider scheme="light">
        <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
          <DropdownMenu open={open()} onOpenChange={() => setRequests(value => value + 1)}>
            <DropdownMenuTrigger
              testId="trigger"
              style="position: fixed; left: 32px; top: 24px; width: 180px; height: 36px"
            >
              Controlled
            </DropdownMenuTrigger>
            <DropdownMenuContent testId="menu" ariaLabel="Controlled actions" style="width: 240px">
              <MenuCheckboxItem
                testId="flag"
                checked={true}
                onCheckedChange={() => setChecks(value => value + 1)}
              >
                Flag
              </MenuCheckboxItem>
              <MenuRadioGroup value="a" onValueChange={() => setRadios(value => value + 1)}>
                <MenuRadioItem testId="radio-a" value="a">
                  Alpha
                </MenuRadioItem>
                <MenuRadioItem testId="radio-b" value="b">
                  Beta
                </MenuRadioItem>
              </MenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button
            testId="accept"
            style="position: fixed; left: 360px; top: 24px"
            onClick={() => setOpen(value => !value)}
          >
            Accept toggle
          </Button>
          <column style="position: fixed; left: 360px; top: 90px; gap: 12px">
            <text testId="open">{open() ? "open" : "closed"}</text>
            <text testId="requests">{requests()}</text>
            <text testId="checks">{checks()}</text>
            <text testId="radios">{radios()}</text>
          </column>
        </box>
      </ThemeProvider>
    );
  }, root);
}
