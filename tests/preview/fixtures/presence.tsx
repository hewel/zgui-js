import { Button, Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { createPresence } from "../../../packages/ui/src/components/presence.js";
declare const globalThis: { __zgui: { installStylesheet(name: string, css: string): void; removeStylesheet(name: string): void } };
export const hostApiVersion = 1;
const buttonStyle = { width: "150px", height: "32px", padding: "6px", background: "#374151", color: "#ffffff" };
const css = `
.presence { width: 300px; height: 140px; padding: 12px; gap: 8px; background-color: #bbf7d0; opacity: 1; }
.presence[data-mode="animated"][data-state="open"] { animation: presence-enter 80ms linear both; }
.presence[data-mode="animated"][data-state="closed"] { animation: presence-exit 180ms linear both; }
.presence[data-mode="zero"][data-state="closed"] { animation: presence-exit 0s both; }
.presence[data-mode="forever"][data-state="closed"] { animation: presence-exit 2s linear infinite; }
@keyframes presence-enter { from { opacity: 0; } to { opacity: 1; } }
@keyframes presence-exit { from { opacity: 1; } to { opacity: 0; } }
`;
function Panel(props: { open: boolean; mode: string }) {
  const presence = createPresence(() => props.open);
  function Body() {
    const [count, setCount] = createSignal(0);
    return <column testId="surface" class="presence" ref={presence.ref} data-mode={props.mode} data-state={presence.state()}
      onAnimationEnd={presence.onEnd} onAnimationCancel={presence.onEnd}
      onTransitionEnd={presence.onEnd} onTransitionCancel={presence.onEnd}>
      <text testId="state">{presence.state()}</text>
      <Button testId="counter" style={buttonStyle} onClick={() => setCount(n => n + 1)}><text>{count()}</text></Button>
    </column>;
  }
  return <Show when={presence.mounted()}><Body /></Show>;
}
export function mount(root: NativeNode) {
  return renderApp(() => {
    globalThis.__zgui.installStylesheet("presence-fixture", css);
    onCleanup(() => globalThis.__zgui.removeStylesheet("presence-fixture"));
    const [open, setOpen] = createSignal(false);
    const [source, setSource] = createSignal(true);
    const [mode, setMode] = createSignal("animated");
    return <column style={{ width: "740px", padding: "12px", gap: "8px", background: "#ffffff", color: "#18181b", "font-size": "14px" }}>
      <row style={{ gap: "8px" }}>
        <Button testId="open" style={buttonStyle} onClick={() => setOpen(true)}>Open</Button>
        <Button testId="close" style={buttonStyle} onClick={() => setOpen(false)}>Close</Button>
        <Button testId="source-toggle" style={buttonStyle} onClick={() => setSource(v => !v)}>Toggle source</Button>
      </row>
      <row style={{ gap: "8px" }}>
        <Button testId="animated" style={buttonStyle} onClick={() => setMode("animated")}>Animated</Button>
        <Button testId="none" style={buttonStyle} onClick={() => setMode("none")}>No motion</Button>
        <Button testId="zero" style={buttonStyle} onClick={() => setMode("zero")}>Zero duration</Button>
        <Button testId="forever" style={buttonStyle} onClick={() => setMode("forever")}>Endless motion</Button>
      </row>
      <Show when={source()}><Panel open={open()} mode={mode()} /></Show>
    </column>;
  }, root);
}
