import { createEffect, createRoot, createSignal, flush, onCleanup, type Setter } from "solid-js";

export function runSmoke(): string {
  const events: string[] = [];
  let setCount!: Setter<number>;
  let dispose!: () => void;

  function expect(expected: string): void {
    const actual = events.join(",");
    if (actual !== expected) {
      throw new Error(`Solid smoke: expected ${expected}; received ${actual}`);
    }
  }

  createRoot(disposeRoot => {
    dispose = disposeRoot;
    const [count, updateCount] = createSignal(0);
    setCount = updateCount;
    onCleanup(() => events.push("root:cleanup"));
    createEffect(
      () => count(),
      value => {
        events.push(`effect:${value}`);
        return () => events.push(`cleanup:${value}`);
      },
    );
  });
  flush();

  try {
    expect("effect:0");
    setCount(1);
    flush();
    expect("effect:0,cleanup:0,effect:1");
    setCount(1);
    flush();
    expect("effect:0,cleanup:0,effect:1");
    setCount(2);
    setCount(3);
    flush();
    expect("effect:0,cleanup:0,effect:1,cleanup:1,effect:3");
    dispose();
    flush();
    expect("effect:0,cleanup:0,effect:1,cleanup:1,effect:3,cleanup:3,root:cleanup");
    setCount(4);
    flush();
    expect("effect:0,cleanup:0,effect:1,cleanup:1,effect:3,cleanup:3,root:cleanup");
    return "solid-smoke:ok";
  } finally {
    dispose();
    flush();
  }
}
