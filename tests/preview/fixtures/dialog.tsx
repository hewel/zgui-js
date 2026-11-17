import { Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button, Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger, Input, ThemeProvider } from "@hewel/zgui/components";
declare const globalThis: { __zgui: { installStylesheet(name: string, css: string): void; removeStylesheet(name: string): void } };
export const hostApiVersion = 1;
const css = `:root { display: block; overflow-y: scroll; background-color: var(--zui-color-background); color: var(--zui-color-foreground); }
.dialog-page { width: 100%; height: 2400px; padding: 240px 24px 24px; gap: 160px; background-color: var(--zui-color-background); color: var(--zui-color-foreground); font-size: 14px; }
.dialog-tools { position: fixed; left: 12px; top: 12px; width: 200px; padding: 12px; gap: 8px; background-color: var(--zui-color-background); color: var(--zui-color-foreground); font-size: 14px; }`;
function Body(props: { remove: () => void; theme: () => void; policy: boolean; togglePolicy: () => void }) {
  const [count, setCount] = createSignal(0);
  return <>
    <DialogHeader><DialogTitle>Rename project</DialogTitle><DialogDescription>Choose a name for this project.</DialogDescription></DialogHeader>
    <Input testId="name" label="Project name" defaultValue="Alpha" />
    <Button testId="counter" variant="secondary" onClick={() => setCount(n => n + 1)}><text>{`Count ${count()}`}</text></Button>
    <scroll testId="inner-scroll" tabIndex={-1} style="height: 96px; flex-shrink: 0; overflow: auto; border: 1px solid var(--zui-color-border)">
      <column testId="inner-content" style="height: 600px; padding: 12px; gap: 90px">
        <text>First detail</text><text>Second detail</text><text>Third detail</text><text>Final detail</text>
      </column>
    </scroll>
    <row style={{ gap: "8px" }}>
      <Button testId="policy" size="sm" variant="outline" onClick={props.togglePolicy}><text>{props.policy ? "Lock close" : "Allow close"}</text></Button>
      <Button testId="theme" size="sm" variant="outline" onClick={props.theme}>Theme</Button>
      <Button testId="remove" size="sm" variant="outline" onClick={props.remove}>Remove</Button>
    </row>
    <Dialog><DialogTrigger testId="nested-open" variant="outline">Confirm change</DialogTrigger>
      <DialogContent testId="nested-surface" style={{ "max-width": "340px" }}>
        <DialogHeader><DialogTitle>Confirm change</DialogTitle><DialogDescription>Review the new project name.</DialogDescription></DialogHeader>
        <Input testId="nested-name" label="Confirmation" defaultValue="Confirmed" />
        <DialogFooter><DialogClose testId="nested-close">Back to editor</DialogClose></DialogFooter>
      </DialogContent>
    </Dialog>
    <DialogFooter><DialogClose testId="cancel" variant="outline">Cancel</DialogClose><DialogClose testId="save">Save</DialogClose></DialogFooter>
  </>;
}
export function mount(root: NativeNode) {
  return renderApp(() => {
    globalThis.__zgui.installStylesheet("dialog-fixture", css);
    onCleanup(() => globalThis.__zgui.removeStylesheet("dialog-fixture"));
    const [source, setSource] = createSignal(true);
    const [open, setOpen] = createSignal(false);
    const [dark, setDark] = createSignal(false);
    const [policy, setPolicy] = createSignal(true);
    const [background, setBackground] = createSignal(0);
    return <ThemeProvider scheme="light">
      <column testId="page" class="dialog-page"><text>Background content</text><text>Second section</text><text>Third section</text><text>Final section</text></column>
      <column class="dialog-tools">
        <Button testId="background" variant="outline" onClick={() => setBackground(n => n + 1)}>Background action</Button>
        <Button testId="source-toggle" variant="outline" onClick={() => setSource(v => !v)}>Toggle source</Button>
        <Show when={source()}><ThemeProvider scheme={dark() ? "dark" : "light"}>
          <Dialog open={open()} onOpenChange={setOpen}>
            <DialogTrigger testId="launch">Rename project</DialogTrigger>
            <DialogContent testId="surface" dismissOnOutsidePress={policy()} dismissOnEscape={policy()}>
              <Body remove={() => setSource(false)} theme={() => setDark(v => !v)} policy={policy()} togglePolicy={() => setPolicy(v => !v)} />
            </DialogContent>
          </Dialog>
        </ThemeProvider></Show>
        <Button testId="after" variant="ghost">After source</Button>
        <text testId="requested">{open() ? "open" : "closed"}</text>
        <text testId="background-count">{String(background())}</text>
      </column>
    </ThemeProvider>;
  }, root);
}
