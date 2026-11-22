import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
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
  return renderApp(() => {
    const [flipped, setFlipped] = createSignal(false);
    return (
      <ThemeProvider scheme="light">
        <box style="position: fixed; inset: 0; background-color: var(--zui-color-background)" />
        <Popover defaultOpen>
          <PopoverTrigger
            testId="left-anchor"
            style="position: fixed; left: 20px; top: 240px; width: 80px; height: 36px"
          >
            Left
          </PopoverTrigger>
          <PopoverContent
            testId="right-surface"
            placement={{ side: "right", align: "start" }}
            offset={6}
            dismissOnOutsidePress={false}
            style="width: 160px; gap: 8px"
          >
            <PopoverHeader>
              <PopoverTitle>To the right</PopoverTitle>
            </PopoverHeader>
            <PopoverClose>Done</PopoverClose>
          </PopoverContent>
        </Popover>
        <Popover defaultOpen>
          <PopoverTrigger
            testId="right-anchor"
            style="position: fixed; right: 20px; top: 240px; width: 80px; height: 36px"
          >
            Right
          </PopoverTrigger>
          <PopoverContent
            testId="left-surface"
            placement={{ side: flipped() ? "right" : "left", align: "end" }}
            offset={10}
            dismissOnOutsidePress={false}
            style="width: 160px; gap: 8px"
          >
            <PopoverHeader>
              <PopoverTitle>To the left</PopoverTitle>
            </PopoverHeader>
            <Button testId="flip" size="sm" onClick={() => setFlipped(true)}>
              Flip
            </Button>
            <text testId="requested-side">{flipped() ? "right" : "left"}</text>
            <PopoverClose>Done</PopoverClose>
          </PopoverContent>
        </Popover>
      </ThemeProvider>
    );
  }, root);
}
