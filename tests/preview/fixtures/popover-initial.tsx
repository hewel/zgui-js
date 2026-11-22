import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
  DialogTrigger,
  Input,
  Popover,
  PopoverClose,
  PopoverContent,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
  ThemeProvider,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(
    () => (
      <ThemeProvider scheme="light">
        <Dialog defaultOpen>
          <DialogTrigger testId="dialog-launch">Edit item</DialogTrigger>
          <DialogContent testId="dialog" dismissControl={false} style="width: 360px; gap: 8px">
            <DialogTitle>Initially open dialog</DialogTitle>
            <Input testId="dialog-name" label="Dialog name" defaultValue="Dialog" />
            <Popover defaultOpen>
              <PopoverTrigger testId="outer-trigger">Item details</PopoverTrigger>
              <PopoverContent testId="outer" style="gap: 8px">
                <PopoverHeader>
                  <PopoverTitle>Item details</PopoverTitle>
                </PopoverHeader>
                <Input testId="outer-name" label="Outer name" defaultValue="Outer" />
                <Popover defaultOpen>
                  <PopoverTrigger testId="inner-trigger">Nested details</PopoverTrigger>
                  <PopoverContent testId="inner" style="gap: 8px">
                    <PopoverHeader>
                      <PopoverTitle>Nested details</PopoverTitle>
                    </PopoverHeader>
                    <Input testId="inner-name" label="Inner name" defaultValue="Inner" />
                    <PopoverClose testId="inner-close">Back</PopoverClose>
                  </PopoverContent>
                </Popover>
                <PopoverClose testId="outer-close">Close details</PopoverClose>
              </PopoverContent>
            </Popover>
            <DialogClose testId="dialog-close">Close dialog</DialogClose>
          </DialogContent>
        </Dialog>
      </ThemeProvider>
    ),
    root,
  );
}
