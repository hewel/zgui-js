import {
  batch,
  createEffect,
  createRoot,
  createSignal,
  onCleanup,
  type Setter,
} from "solid-js";

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

  createRoot((disposeRoot) => {
    dispose = disposeRoot;
    const [count, updateCount] = createSignal(0);
    setCount = updateCount;
    onCleanup(() => events.push("root:cleanup"));
    createEffect(() => {
      const value = count();
      events.push(`effect:${value}`);
      onCleanup(() => events.push(`cleanup:${value}`));
    });
  });

  try {
    // Initial effects flush as createRoot returns, without host jobs or timers.
    expect("effect:0");
    setCount(1);
    expect("effect:0,cleanup:0,effect:1");
    setCount(1);
    expect("effect:0,cleanup:0,effect:1");
    batch(() => {
      setCount(2);
      setCount(3);
    });
    expect("effect:0,cleanup:0,effect:1,cleanup:1,effect:3");
    dispose();
    expect("effect:0,cleanup:0,effect:1,cleanup:1,effect:3,cleanup:3,root:cleanup");
    setCount(4);
    expect("effect:0,cleanup:0,effect:1,cleanup:1,effect:3,cleanup:3,root:cleanup");
    return "solid-smoke:ok";
  } finally {
    dispose();
  }
}
