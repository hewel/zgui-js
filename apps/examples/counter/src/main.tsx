import { Button, createMemo, createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

let increment: (() => void) | undefined;

function Counter() {
  const [count, setCount] = createSignal(0);
  const doubled = createMemo(() => count() * 2);
  increment = () => { setCount(value => value + 1); };
  onCleanup(() => { increment = undefined; });
  return <column style={{ gap: "12px" }}>
    <text testId="count-label">Count: {count()}</text><text testId="doubled-label">Doubled: {doubled()}</text>
    <Button testId="increment" class="increment" style={{ padding: "8px", "border-radius": "6px", background: "#314665" }}
      onClick={() => setCount(value => value + 1)}>Increment</Button>
  </column>;
}

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <Counter />, root);
}

/** Deterministic native example harness; interactive input uses the Button above. */
export function advance(): void {
  if (!increment) throw new Error("Counter is not mounted");
  increment();
}
