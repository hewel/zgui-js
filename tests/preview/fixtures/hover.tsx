import { Show, createSignal, focus, onCleanup, renderApp, untrack } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  ThemeProvider,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const [dark, setDark] = createSignal(false),
      [alive, setAlive] = createSignal(true);
    const [aOpen, setAOpen] = createSignal(false),
      [bOpen, setBOpen] = createSignal(false);
    const bumps = new Map<string, () => void>();
    let first!: NativeNode;
    function Body(props: { id: string; label: string }) {
      const id = untrack(() => props.id);
      const [count, setCount] = createSignal(0);
      const bump = () => setCount(n => n + 1);
      bumps.set(id, bump);
      onCleanup(() => {
        if (bumps.get(id) === bump) bumps.delete(id);
      });
      return (
        <text testId={props.id + "-value"}>
          {props.label} {count()}
        </text>
      );
    }
    return (
      <ThemeProvider scheme={dark() ? "dark" : "light"}>
        <box
          style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)"
          onKeyDown={event => {
            if (event.key === "Home") setDark(value => !value);
            if (event.key === "End") bumps.get("a")?.();
            if (event.key === "Delete") {
              setAlive(value => !value);
              setAOpen(false);
            }
          }}
        >
          <Button testId="outside" style="position: fixed; left: 20px; top: 20px; width: 120px">
            Outside
          </Button>
          <Button
            testId="program"
            style="position: fixed; left: 160px; top: 20px; width: 160px"
            disabled={!alive()}
            onClick={() => focus(first)}
          >
            Focus first
          </Button>
          <text style="position: fixed; left: 20px; top: 80px">
            Home: theme; End: change first hint; Delete: toggle first source
          </text>
          <text testId="a-state" style="position: fixed; left: 20px; top: 120px">
            {aOpen() ? "open" : "closed"}
          </text>
          <text testId="b-state" style="position: fixed; left: 160px; top: 120px">
            {bOpen() ? "open" : "closed"}
          </text>
          <TooltipProvider delay={200} closeDelay={120}>
            <Show when={alive()}>
              <Tooltip onOpenChange={setAOpen}>
                <TooltipTrigger
                  testId="a-trigger"
                  style="position: fixed; left: 100px; top: 200px; width: 100px; height: 36px"
                >
                  <Button
                    testId="a-button"
                    ref={node => {
                      first = node;
                    }}
                    style="width: 100px; height: 36px"
                  >
                    First
                  </Button>
                </TooltipTrigger>
                <TooltipContent testId="a-surface" offset={0} style="width: 160px">
                  <Body id="a" label="Alpha" />
                </TooltipContent>
              </Tooltip>
            </Show>
            <Tooltip onOpenChange={setBOpen}>
              <TooltipTrigger
                testId="b-trigger"
                style="position: fixed; left: 280px; top: 200px; width: 100px; height: 36px"
              >
                <Button testId="b-button" style="width: 100px; height: 36px">
                  Second
                </Button>
              </TooltipTrigger>
              <TooltipContent testId="b-surface" style="width: 160px">
                <Body id="b" label="Beta" />
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </box>
      </ThemeProvider>
    );
  }, root);
}
