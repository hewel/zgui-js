import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
  DialogTrigger,
  Input,
  ThemeProvider,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(
    () => (
      <ThemeProvider scheme="light">
        <Dialog defaultOpen>
          <DialogTrigger testId="initial-launch">Outer</DialogTrigger>
          <DialogContent testId="initial-outer" dismissControl={false}>
            <DialogTitle>Initially open</DialogTitle>
            <Input testId="initial-name" label="Outer input" />
            <Dialog defaultOpen>
              <DialogTrigger testId="initial-inner-trigger">Inner</DialogTrigger>
              <DialogContent
                testId="initial-inner"
                dismissControl={false}
                style={{ "max-width": "340px" }}
              >
                <DialogTitle>Nested on mount</DialogTitle>
                <Input testId="initial-inner-name" label="Inner input" />
                <DialogClose testId="initial-inner-close">Back</DialogClose>
              </DialogContent>
            </Dialog>
            <DialogClose testId="initial-outer-close">Close outer</DialogClose>
          </DialogContent>
        </Dialog>
      </ThemeProvider>
    ),
    root,
  );
}
