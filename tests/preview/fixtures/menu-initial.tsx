import { createSignal, focus, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
  DialogTrigger,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  MenuItem,
  ThemeProvider,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    let trigger!: NativeNode;
    const [open, setOpen] = createSignal(true),
      [count, setCount] = createSignal(0);
    return (
      <ThemeProvider scheme="dark">
        <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
          <Dialog defaultOpen>
            <DialogTrigger testId="dialog-trigger">Dialog</DialogTrigger>
            <DialogContent testId="dialog" dismissControl={false} style="width: 440px">
              <DialogTitle>Menu inside dialog</DialogTitle>
              <Button testId="prepare" onClick={() => focus(trigger)}>
                Focus menu trigger
              </Button>
              <DropdownMenu defaultOpen onOpenChange={setOpen}>
                <DropdownMenuTrigger
                  testId="trigger"
                  ref={node => {
                    trigger = node;
                  }}
                >
                  Initial menu
                </DropdownMenuTrigger>
                <DropdownMenuContent testId="menu" ariaLabel="Nested actions" style="width: 220px">
                  <MenuItem
                    testId="keep"
                    closeOnSelect={false}
                    onSelect={() => setCount(value => value + 1)}
                  >
                    Keep open
                  </MenuItem>
                  <MenuItem testId="done">Done</MenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
              <DialogClose testId="close">Close dialog</DialogClose>
              <text testId="open">{open() ? "open" : "closed"}</text>
              <text testId="count">{count()}</text>
            </DialogContent>
          </Dialog>
        </box>
      </ThemeProvider>
    );
  }, root);
}
