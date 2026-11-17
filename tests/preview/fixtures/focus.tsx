import { Show, createEffect, createSignal, renderApp, trapFocus } from "@hewel/zgui";
import type { NativeChild, NativeNode } from "@hewel/zgui";
import { Button, ThemeProvider } from "@hewel/zgui/components";

export const hostApiVersion = 1;

function Scope(props: { testId: string; children?: NativeChild }) {
  let node!: NativeNode;
  createEffect(() => { trapFocus(node); });
  return <column ref={value => { node = value; }} testId={props.testId} style={{ gap: "8px" }}>{props.children}</column>;
}

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [open, setOpen] = createSignal(false);
    const [inner, setInner] = createSignal(false);
    return <ThemeProvider scheme="light"><column style={{ width: "400px", padding: "16px", gap: "12px", "background-color": "var(--zui-color-background)", color: "var(--zui-color-foreground)" }}>
      <Button testId="launch" onClick={() => setOpen(true)}>Open focus scope</Button>
      <Show when={open()}><Scope testId="outer">
        <Button testId="first">First</Button>
        <Button testId="inner-launch" onClick={() => setInner(true)}>Open nested scope</Button>
        <Button testId="outer-close" onClick={() => setOpen(false)}>Close outer</Button>
        <Show when={inner()}><Scope testId="inner">
          <Button testId="inner-first">Nested first</Button>
          <Button testId="inner-close" onClick={() => setInner(false)}>Close nested</Button>
        </Scope></Show>
      </Scope></Show>
      <Button testId="after" onClick={() => setOpen(false)}>Outside scope</Button>
    </column></ThemeProvider>;
  }, root);
}
