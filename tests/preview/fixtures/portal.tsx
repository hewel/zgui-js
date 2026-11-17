import { Portal, Show, createContext, createEffect, createSignal, renderApp, trapFocus, useContext } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button, ThemeProvider } from "@hewel/zgui/components";

export const hostApiVersion = 1;

const Context = createContext("missing");

function Counter(props: { kind: string; hit: (kind: string) => void }) {
  const context = useContext(Context);
  const [count, setCount] = createSignal(0);
  return <Button testId={props.kind} style="position: absolute; left: 300px; top: 140px; width: 160px; height: 72px"
    onClick={() => { setCount(n => n + 1); props.hit(props.kind); }}><text>{props.kind} {context} {count()}</text></Button>;
}

function FocusPanel(props: { close: () => void }) {
  let node!: NativeNode;
  createEffect(() => { trapFocus(node); });
  return <column ref={value => { node = value; }} testId="focus-scope" style="position: absolute; left: 520px; top: 140px; width: 200px; gap: 8px">
    <Button testId="focus-first">First in portal</Button>
    <Button testId="focus-close" onClick={props.close}>Close focus scope</Button>
  </column>;
}

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [top, setTop] = createSignal(true);
    const [source, setSource] = createSignal(true);
    const [focused, setFocused] = createSignal(false);
    const [last, setLast] = createSignal("none");
    const [bubbles, setBubbles] = createSignal(0);
    return <ThemeProvider scheme="light"><Context.Provider value="kept">
      <column style={{ width: "220px", padding: "12px", gap: "8px", "background-color": "var(--zui-color-background)", color: "var(--zui-color-foreground)" }}>
        <Button testId="toggle-top" onClick={() => setTop(v => !v)}>Toggle toast</Button>
        <Button testId="focus-open" onClick={() => setFocused(true)}>Open focus scope</Button>
        <Button testId="toggle-source" onClick={() => setSource(v => !v)}>Toggle source</Button>
        <text testId="last">{last()}</text><text testId="bubbles">{bubbles()}</text>
        <Show when={source()}><box testId="source" onClick={() => setBubbles(n => n + 1)}
          style="position: relative; width: 120px; height: 80px; overflow: hidden; transform: translate(10px, 10px); background-color: var(--zui-color-muted)">
          <text>Clipped source</text>
          <Show when={top()}><Portal layer="toast"><Counter kind="toast" hit={setLast} /></Portal></Show>
          <Portal layer="modal"><Counter kind="modal" hit={setLast} /></Portal>
          <Show when={focused()}><Portal layer="modal"><FocusPanel close={() => setFocused(false)} /></Portal></Show>
        </box></Show>
      </column>
    </Context.Provider></ThemeProvider>;
  }, root);
}
