import { Show, createSignal, mountStylesheet, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  Popover,
  PopoverContent,
  PopoverTrigger,
  ThemeProvider,
} from "@hewel/zgui/components";
export const hostApiVersion = 1;
const buttonStyle = {
  width: "150px",
  height: "32px",
  padding: "6px",
  background: "#374151",
  color: "#ffffff",
};
const css = `
.zui-surface.presence { width: 300px; height: 140px; padding: 12px; gap: 8px; background-color: #bbf7d0; opacity: 1; }
.zui-surface.presence[data-mode="none"],
.zui-surface.presence[data-mode="none"][data-state="closed"] { animation: none; }
.zui-surface.presence[data-mode="animated"][data-state="open"] { animation: presence-enter 80ms linear both; }
.zui-surface.presence[data-mode="animated"][data-state="closed"] { animation: presence-exit 180ms linear both; }
.zui-surface.presence[data-mode="zero"][data-state="open"] { animation: none; }
.zui-surface.presence[data-mode="zero"][data-state="closed"] { animation: presence-exit 0s both; }
.zui-surface.presence[data-mode="forever"][data-state="open"] { animation: none; }
.zui-surface.presence[data-mode="forever"][data-state="closed"] { animation: presence-exit 2s linear infinite; }
@keyframes presence-enter { from { opacity: 0; } to { opacity: 1; } }
@keyframes presence-exit { from { opacity: 1; } to { opacity: 0; } }
`;
function Body(props: { open: boolean }) {
  const [count, setCount] = createSignal(0);
  return (
    <>
      <text testId="state">{props.open ? "open" : "closed"}</text>
      <Button testId="counter" style={buttonStyle} onClick={() => setCount(n => n + 1)}>
        <text>{count()}</text>
      </Button>
    </>
  );
}
function Panel(props: { open: boolean; mode: string }) {
  return (
    <Popover open={props.open}>
      <PopoverTrigger testId="anchor" style={{ width: "80px", height: "24px" }}>
        Anchor
      </PopoverTrigger>
      <PopoverContent
        testId="surface"
        class="presence"
        data-mode={props.mode}
        placement={{ side: "bottom", align: "start" }}
        dismissOnOutsidePress={false}
        dismissOnEscape={false}
      >
        <Body open={props.open} />
      </PopoverContent>
    </Popover>
  );
}
export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet("presence-fixture", css);
    const [open, setOpen] = createSignal(false);
    const [source, setSource] = createSignal(true);
    const [mode, setMode] = createSignal("animated");
    return (
      <ThemeProvider>
        <column
          style={{
            width: "740px",
            padding: "12px",
            gap: "8px",
            background: "#ffffff",
            color: "#18181b",
            "font-size": "14px",
          }}
        >
          <row style={{ gap: "8px" }}>
            <Button testId="open" style={buttonStyle} onClick={() => setOpen(true)}>
              Open
            </Button>
            <Button testId="close" style={buttonStyle} onClick={() => setOpen(false)}>
              Close
            </Button>
            <Button testId="source-toggle" style={buttonStyle} onClick={() => setSource(v => !v)}>
              Toggle source
            </Button>
          </row>
          <row style={{ gap: "8px" }}>
            <Button testId="animated" style={buttonStyle} onClick={() => setMode("animated")}>
              Animated
            </Button>
            <Button testId="none" style={buttonStyle} onClick={() => setMode("none")}>
              No motion
            </Button>
            <Button testId="zero" style={buttonStyle} onClick={() => setMode("zero")}>
              Zero duration
            </Button>
            <Button testId="forever" style={buttonStyle} onClick={() => setMode("forever")}>
              Endless motion
            </Button>
          </row>
          <Show when={source()}>
            <Panel open={open()} mode={mode()} />
          </Show>
        </column>
      </ThemeProvider>
    );
  }, root);
}
