import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
  ThemeProvider,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [open, setOpen] = createSignal(false);
    let mounts = 0;
    function Body() {
      const [value] = createSignal(++mounts);
      return <text testId="card-value">Preview {value()}</text>;
    }
    return (
      <ThemeProvider scheme="light">
        <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
          <Button testId="before" style="position: fixed; left: 20px; top: 20px">
            Before
          </Button>
          <HoverCard onOpenChange={setOpen}>
            <HoverCardTrigger
              testId="card-trigger"
              style="position: fixed; left: 100px; top: 360px; width: 160px; height: 36px"
            >
              <text>Profile preview</text>
            </HoverCardTrigger>
            <HoverCardContent testId="card" ariaLabel="Profile preview" style="width: 240px">
              <Body />
            </HoverCardContent>
          </HoverCard>
          <Button testId="after" style="position: fixed; left: 160px; top: 20px">
            After
          </Button>
          <text testId="state" style="position: fixed; left: 20px; top: 80px">
            {open() ? "open" : "closed"}
          </text>
        </box>
      </ThemeProvider>
    );
  }, root);
}
