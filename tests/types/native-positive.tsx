import {
  Image, Button, TextField, TextArea, For, Index, Show, batch, createContext, createEffect, createMemo,
  createSignal, onCleanup, onMount, renderApp, splitProps, untrack, useContext,
  focus, getTextContent, getWindowRect, isAnimating, lockScroll, mountStylesheet, observeGeometry, precedes,
} from "@hewel/zgui";
import type { Component, Context, NativeAnimationEvent, NativeChild, NativeContextMenuEvent, NativeFocusOptions, NativeNode, NativePointerBoundaryEvent, NativeRole, NativeTextControlProps, NativeTransitionEvent, NativeViewport, NativeWindowRect, ParentComponent, TextFieldProps, TextAreaProps, VectorProps } from "@hewel/zgui";

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
  <control ariaRadioGroup={assigned} />
  <Button class="primary" title="Increase" disabled={false} ariaLabel="Increase counter" ariaKeyShortcuts="Control+I"
    style={{ "font-size": "20px", padding: "8px", "--accent": "blue" }}
    onClick={event => { event.preventDefault(); setCount(value => value + 1); }}
    onKeyDownCapture={event => { const character: string | null = event.character; console.log(character, event.key, event.repeat, event.modifiers.shift); }}
    onPointerDown={event => { console.log(event.position.x, event.pointerId); event.capturePointer(); }}
    onPointerMove={event => { event.releasePointer(); }}
    onPointerMoveCapture={event => { console.log(event.coalesced.length); }}
    onPointerEnter={(event: NativePointerBoundaryEvent) => { console.log(event.type, event.position.x, event.pointerId); }}
    onContextMenu={(event: NativeContextMenuEvent) => { console.log(event.type, event.position.x, event.pointerId); event.preventDefault(); }}
    onPointerLeave={event => { console.log(event.position.y, event.pointerKind); }}
    onPointerEnterCapture={event => { console.log(event.target, event.currentTarget); }}
    onPointerLeaveCapture={event => { console.log(event.coalesced.length); }}
    onPointerCancel={event => { event.releasePointer(); }}
    onPointerCancelCapture={event => { console.log(event.pointerKind); }}>Increase</Button>
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
  <Image src={optional()} ariaLabel="Package icon" style={{ width: "32px", height: "32px" }} />
  <image src={null} />
  <vector d="M0 0 L8 8 L8 0 Z" viewBox="0 0 16 16" data-icon="save" role="image" ariaLabel="Save" />
  <vector d={null} viewBox={undefined} />
  <control checked="mixed" expanded={false} selected open={true} invalid={null} required
    ariaHidden={false} ariaModal orientation="vertical" data-action="save" data-count-1_x="two" role="slider" />
  <box onAnimationEnd={event => {
      const name: string = event.animationName;
      const elapsed: number = event.elapsedTime;
      const pseudo: string = event.pseudoElement;
      console.log(name, elapsed, pseudo, event.target === event.currentTarget);
    }}
    onAnimationStartCapture={(event: NativeAnimationEvent) => { event.stopPropagation(); }}
    onTransitionRun={event => { console.log(event.propertyName, event.elapsedTime); }}
    onTransitionCancelCapture={(event: NativeTransitionEvent) => console.log(event.pseudoElement)} />
  <For each={[{ id: 1 }]}>{(item, index) => <text>{item.id}: {index()}</text>}</For>
  <Index each={["a"]}>{(item, index) => <text>{index}: {item()}</text>}</Index>
  <Show when={count()}>{value => <text>{value()}</text>}</Show>
  <Show when={{ name: "value" }} keyed>{value => <text>{value.name}</text>}</Show>
  <Show when={count() > 0}><text>Created lazily by Universal compilation</text></Show>
</Container>;

const Theme: Context<{ accent: string }> = createContext({ accent: "blue" });
const MaybeTheme = createContext<string>();
interface SplitControlProps { title: string; detail?: string; flag?: boolean; }
const ThemedLabel: Component<SplitControlProps> = props => {
  const theme = useContext(Theme);
  const [picked, rest] = splitProps(props, ["flag"]);
  const flag: boolean | undefined = picked.flag;
  const title: string = rest.title;
  return <text>{title}{flag}{theme.accent}{rest.detail}</text>;
};
const provided = <Theme.Provider value={{ accent: "red" }}>
  <ThemedLabel title="labeled" flag detail="extra" />
  <text>native children</text>
</Theme.Provider>;
const roles: NativeRole[] = ["alert", "checkBox", "menuItemRadio", "scrollView", "tabPanel", "tooltip"];
const vectorProps: VectorProps = { d: "M0 0", viewBox: "0 0 8 8", role: "image" };
const children: NativeChild = [tree, provided, "text", 42, false, null, undefined, () => [true, "accessor"]];
const nativeTextProps: NativeTextControlProps = { value: null, defaultValue: "seed", readOnly: false, role: "textInput" };
const fieldProps: TextFieldProps = { value: optional(), disabled: false };
const areaProps: TextAreaProps = { defaultValue: "one\n二", onInput: event => console.log(event.selection.start) };
const rect: NativeWindowRect | null = getWindowRect(assigned);
const textContent: string = getTextContent(assigned);
void textContent;
const ordered: boolean = precedes(assigned, assigned);
const moving: boolean = isAnimating(assigned);
void moving;
const unlockScroll: () => void = lockScroll(assigned);
unlockScroll();
const unmountStylesheet: () => void = mountStylesheet("positive-test", ":root { color: #111827; }");
unmountStylesheet();
const reveal: NativeFocusOptions = { reveal: true };
focus(assigned);
focus(assigned, reveal);
focus(assigned, { reveal: false });
const cancelBox: () => void = observeGeometry(assigned, "borderBox", rect => rect.x.toFixed(2));
const cancelBorder: () => void = observeGeometry(assigned, "borderSize", size => size.width.toFixed(2));
const cancelContent: () => void = observeGeometry(assigned, "contentSize", size => size.width.toFixed(2));
const cancelScroll: () => void = observeGeometry(assigned, "scrollPosition", position =>
  console.log(position.offset.x, position.offset.y, position.contentSize.width, position.scrollport.height));
const cancelViewport: () => void = observeGeometry(assigned, "viewport", viewport => {
  const snapshot: NativeViewport = viewport;
  return snapshot.width + snapshot.height + snapshot.scale;
});
void [cancelBox, cancelBorder, cancelContent, cancelScroll, cancelViewport];
void rect;
void ordered;
const fallbackTheme: string | undefined = MaybeTheme.defaultValue;
void [nativeTextProps, fieldProps, areaProps, roles, vectorProps, fallbackTheme];
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

type AssertBoundary<T extends true> = T;
export type BoundaryHasNoPointerActions = AssertBoundary<
  Extract<"releasePointer" | "capturePointer" | "activateNow", keyof NativePointerBoundaryEvent> extends never ? true : false
>;
export type ContextMenuHasNoPointerActions = AssertBoundary<
  Extract<"releasePointer" | "capturePointer" | "activateNow", keyof NativeContextMenuEvent> extends never ? true : false
>;
