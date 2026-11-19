import { createSignal, For, LayerSurface, renderShell, useMonitors } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

export function mount() {
  return renderShell(() => {
    const monitors = useMonitors();
    const [count, setCount] = createSignal(0);
    return <For each={monitors()} keyed={monitor => monitor.id}>{monitor => {
      const [hit, setHit] = createSignal<NativeNode>();
      const input = (): NativeNode[] | "empty" => {
        const node = hit();
        return node ? [node] : "empty";
      };
      return <LayerSurface name={monitor().id} output={monitor().id} anchor={["top", "left", "right"]} height={48}
        inputRegion={input()}>
        <column style="position: fixed; left: 0; top: 0; width: 100%; height: 100%; background: #123456">
          <box ref={node => { setHit(node); }} style={{ position: "fixed", left: "0px", top: "0px", width: "80px", height: "24px",
            pointerEvents: "none", display: count() === 0 ? "block" : "none" }} />
          <control testId="counter" style="width: 96px; height: 24px"
            onClick={() => setCount(value => value + 1)}>{monitor().name}:{count()}</control>
        </column>
      </LayerSurface>;
    }}</For>;
  });
}
