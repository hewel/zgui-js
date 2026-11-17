import { Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Button, Input, Popover, PopoverClose, PopoverContent, PopoverDescription, PopoverHeader, PopoverTitle, PopoverTrigger, ThemeProvider } from "@hewel/zgui/components";
declare const globalThis: { __zgui: { installStylesheet(name: string, css: string): void; removeStylesheet(name: string): void } };
export const hostApiVersion = 1;
const css = `:root { display: block; padding: 0; overflow-y: scroll; background-color: var(--zui-color-background); color: var(--zui-color-foreground); font-size: 14px; }
.popover-page { position: relative; width: 100%; height: 1800px; padding: 280px 24px 24px; gap: 220px; background-color: var(--zui-color-background); color: var(--zui-color-foreground); }
.popover-tools { position: fixed; left: 12px; top: 12px; width: 200px; padding: 12px; gap: 8px; background-color: var(--zui-color-background); color: var(--zui-color-foreground); }
.popover-anchor-scope { position: absolute; left: 440px; top: 420px; width: 128px; height: 44px; padding: 4px; overflow: hidden; }`;
function Body(props: { remove: () => void; theme: () => void; move: () => void; policy: boolean; togglePolicy: () => void }) {
  const [count, setCount] = createSignal(0);
  return <>
    <PopoverHeader><PopoverTitle testId="title">Dimensions</PopoverTitle><PopoverDescription>Adjust this item.</PopoverDescription></PopoverHeader>
    <Input testId="name" label="Item name" defaultValue="Alpha" />
    <Button testId="counter" variant="secondary" onClick={() => setCount(n => n + 1)}><text>{`Count ${count()}`}</text></Button>
    <row style={{ gap: "8px" }}>
      <Button testId="policy" size="sm" variant="outline" onClick={props.togglePolicy}><text>{props.policy ? "Lock close" : "Allow close"}</text></Button>
      <Button testId="theme" size="sm" variant="outline" onClick={props.theme}>Theme</Button>
    </row>
    <row style={{ gap: "8px" }}>
      <Button testId="move" size="sm" variant="outline" onClick={props.move}>Move anchor</Button>
      <Button testId="remove" size="sm" variant="outline" onClick={props.remove}>Remove</Button>
    </row>
    <PopoverClose testId="cancel" variant="outline">Done</PopoverClose>
  </>;
}
export function mount(root: NativeNode) {
  return renderApp(() => {
    globalThis.__zgui.installStylesheet("popover-fixture", css);
    onCleanup(() => globalThis.__zgui.removeStylesheet("popover-fixture"));
    const [source, setSource] = createSignal(true);
    const [open, setOpen] = createSignal(false);
    const [dark, setDark] = createSignal(false);
    const [moved, setMoved] = createSignal(false);
    const [policy, setPolicy] = createSignal(true);
    const [background, setBackground] = createSignal(0);
    const [ends, setEnds] = createSignal(0);
    return <ThemeProvider scheme="light">
      <column testId="page" class="popover-page">
        <text>Background content</text><text>Second section</text><text>Third section</text><text>Final section</text>
        <Show when={source()}><ThemeProvider scheme={dark() ? "dark" : "light"}>
          <box testId="anchor-scope" class="popover-anchor-scope" style={moved() ? "transform: translate(30px, -20px)" : "transform: translate(0px, 0px)"}>
            <Popover open={open()} onOpenChange={setOpen}>
              <PopoverTrigger testId="launch" style="width: 120px; height: 36px">Edit item</PopoverTrigger>
              <PopoverContent testId="surface" placement={{ side: "top", align: "center" }} style="gap: 8px"
                dismissOnOutsidePress={policy()} dismissOnEscape={policy()}
                onAnimationEnd={event => { if (event.target === event.currentTarget) setEnds(n => n + 1); }}>
                <Body remove={() => setSource(false)} theme={() => setDark(v => !v)} move={() => setMoved(v => !v)}
                  policy={policy()} togglePolicy={() => setPolicy(v => !v)} />
              </PopoverContent>
            </Popover>
          </box>
        </ThemeProvider></Show>
      </column>
      <column class="popover-tools">
        <Button testId="background" variant="outline" onClick={() => setBackground(n => n + 1)}>Background action</Button>
        <Button testId="source-toggle" variant="outline" onClick={() => setSource(v => !v)}>Toggle source</Button>
        <Button testId="after" variant="ghost">After source</Button>
        <text testId="requested">{open() ? "open" : "closed"}</text>
        <text testId="background-count">{String(background())}</text>
        <text testId="ends">{String(ends())}</text>
      </column>
    </ThemeProvider>;
  }, root);
}
