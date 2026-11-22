import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
  DialogTrigger,
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
  ThemeProvider,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(
    () => (
      <ThemeProvider scheme="light">
        <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
          <Dialog defaultOpen>
            <DialogTrigger testId="dialog-trigger">Open dialog</DialogTrigger>
            <DialogContent testId="dialog" dismissControl={false} style="width: 440px">
              <DialogTitle>Hover surfaces</DialogTitle>
              <Tooltip defaultOpen>
                <TooltipTrigger testId="tip-trigger">
                  <Button testId="tip-button">Tooltip trigger</Button>
                </TooltipTrigger>
                <TooltipContent testId="tip">Tooltip preview</TooltipContent>
              </Tooltip>
              <HoverCard defaultOpen>
                <HoverCardTrigger testId="card-trigger">Card trigger</HoverCardTrigger>
                <HoverCardContent testId="card" ariaLabel="Card preview">
                  Card preview
                </HoverCardContent>
              </HoverCard>
              <DialogClose testId="close">Close dialog</DialogClose>
            </DialogContent>
          </Dialog>
        </box>
      </ThemeProvider>
    ),
    root,
  );
}
