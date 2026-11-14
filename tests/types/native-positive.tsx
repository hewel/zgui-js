import {
  Button, TextField, TextArea, For, Index, Show, batch, createEffect, createMemo, createSignal,
  onCleanup, onMount, renderApp, untrack,
} from "@zgui/ui";
import type { Component, NativeChild, NativeNode, NativeTextControlProps, ParentComponent, TextFieldProps, TextAreaProps } from "@zgui/ui";

const Greeting: Component<{ name: string }> = props => <text>{props.name}</text>;
const Container: ParentComponent = props => <box>{props.children}</box>;
const [count, setCount] = createSignal(0);
const [optional, setOptional] = createSignal<string>();
setOptional();
const doubled = createMemo(() => count() * 2);
const previous = createMemo(value => value + count(), 0);
createEffect(() => { console.log(doubled(), previous()); });
createEffect(value => value + count(), 0);
batch(() => { setCount(value => value + 1); setOptional("native"); });
untrack(() => optional());

let borrowed: NativeNode | undefined;
let assigned!: NativeNode;
const tree = <Container>
  <row ref={node => { borrowed = node; }}><Greeting name="Native" /></row>
  <column><stack><label>{count()}</label></stack></column>
  <scroll><control><text>{optional()}</text></control></scroll>
  <text ref={assigned}>Variable ref</text>
  <Button class="primary" title="Increase" disabled={false} ariaLabel="Increase counter"
    style={{ "font-size": "20px", padding: "8px", "--accent": "blue" }}
    onClick={event => { event.preventDefault(); setCount(value => value + 1); }}
    onKeyDownCapture={event => { console.log(event.key, event.repeat, event.modifiers.shift); }}
    onPointerDown={event => { console.log(event.position.x, event.pointerId); }}>Increase</Button>
  <TextField value={optional()} defaultValue="initial" readOnly={false}
    ariaLabel="Name" ariaDescription="Native single-line field"
    onInput={event => {
      setOptional(event.value);
      const start: number = event.selection.start;
      const end: number = event.selection.end;
      const kind: "input" | "committed" = event.valueChange;
      console.log(start, end, kind, event.target === event.currentTarget);
    }} />
  <TextArea value={null} defaultValue={undefined} readOnly={null} disabled={false}
    onChange={event => console.log(event.value, event.selection)} />
  <field role="textInput" value={undefined} defaultValue="native" tabIndex={0} />
  <editor role="multilineTextInput" defaultValue={"one\n二"} readOnly />
  <For each={[{ id: 1 }]}>{(item, index) => <text>{item.id}: {index()}</text>}</For>
  <Index each={["a"]}>{(item, index) => <text>{index}: {item()}</text>}</Index>
  <Show when={count()}>{value => <text>{value()}</text>}</Show>
  <Show when={{ name: "value" }} keyed>{value => <text>{value.name}</text>}</Show>
  <Show when={count() > 0}><text>Created lazily by Universal compilation</text></Show>
</Container>;

const children: NativeChild = [tree, "text", 42, false, null, undefined, () => [true, "accessor"]];
const nativeTextProps: NativeTextControlProps = { value: null, defaultValue: "seed", readOnly: false, role: "textInput" };
const fieldProps: TextFieldProps = { value: optional(), disabled: false };
const areaProps: TextAreaProps = { defaultValue: "one\n二", onInput: event => console.log(event.selection.start) };
void [nativeTextProps, fieldProps, areaProps];
export function mount(root: NativeNode): () => void {
  return renderApp(() => {
    onMount(() => {
      const timer = setTimeout(() => console.info("timer"), 1);
      const frame = requestAnimationFrame(timestamp => console.debug(timestamp));
      queueMicrotask(() => console.warn("microtask"));
      onCleanup(() => { clearTimeout(timer); cancelAnimationFrame(frame); });
    });
    return children;
  }, root);
}
void borrowed;
