import { For, Show, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

type Item = { id: string; title: string };
let next: (() => void) | undefined;
let cleanedItems = 0;
let cleanedBranches = 0;

function ItemView(props: { item: Item }) {
  onCleanup(() => { cleanedItems += 1; });
  return <row><label>{props.item.id}</label><text>{props.item.title}</text></row>;
}

function ConditionalContent() {
  onCleanup(() => { cleanedBranches += 1; });
  return <text>Fresh conditional branch</text>;
}

function Lists() {
  const original: Item[] = [
    { id: "a", title: "Alpha" },
    { id: "b", title: "Beta" },
    { id: "c", title: "Gamma" },
  ];
  const [items, setItems] = createSignal(original);
  const [visible, setVisible] = createSignal(true);
  const [slots, setSlots] = createSignal(["first", "second"]);
  let stage = 0;
  next = () => {
    stage = (stage + 1) % 4;
    if (stage === 1) setItems([original[2], original[0], original[1]]);
    if (stage === 2) setVisible(false);
    if (stage === 3) {
      setVisible(true);
      setItems([original[1], original[2]]);
      setSlots(["updated", "second"]);
    }
    if (stage === 0) {
      setItems(original);
      setSlots(["first", "second"]);
    }
  };
  onCleanup(() => { next = undefined; });
  return <column>
    <For each={items()}>{item => <ItemView item={item} />}</For>
    <Show when={visible()}><ConditionalContent /></Show>
    <For each={slots()} keyed={false}>{(value, index) => <text>{index}: {value()}</text>}</For>
  </column>;
}

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  cleanedItems = 0;
  cleanedBranches = 0;
  return renderApp(() => <Lists />, root);
}

/** Call once per host turn/frame to exercise moves, delayed hide/show, then item removal. */
export function advance(): void {
  if (!next) throw new Error("Lists are not mounted");
  next();
}

export function cleanupCounts(): { items: number; branches: number } {
  return { items: cleanedItems, branches: cleanedBranches };
}
