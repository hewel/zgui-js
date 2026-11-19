import { createCanvasScene, renderApp, type NativeNode } from "@hewel/zgui";
import { Button } from "@hewel/zgui/components";

export const hostApiVersion = 1;

const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 80">
  <rect x="8" y="8" width="20" height="20" fill="#ff0000" stroke="#0000ff" stroke-width="8"/>
  <rect x="44" y="8" width="24" height="20" fill="#00ff00"/>
</svg>`;
const d = "M0 0 H80 V80 H0 Z";
const viewBox = "0 0 80 80";
const style = "width: 80px; height: 80px; color: #ff00ff";
const tile = (fill: string) => ({ kind: "rect" as const, x: 8, y: 8, width: 24, height: 24, fill });

export function mount(root: NativeNode) {
  return renderApp(() => {
    const scene = createCanvasScene();
    scene.update([tile("#00ffff")]);
    const owned = createCanvasScene();
    owned.update([tile("#0000ff")]);
    return <column style="position: fixed; inset: 0; padding: 16px; gap: 16px; background-color: #ffffff">
      <row style="gap: 16px">
        <vector testId="svg" style={style} viewBox={viewBox} d={d} svg={svg} />
        <vector testId="canvas-svg" style={style} viewBox={viewBox} d={d} svg={svg} canvas={scene} />
        <vector testId="canvas-path" style={style} viewBox={viewBox} d={d} canvas={scene} />
        <vector testId="owned" style={style} viewBox={viewBox} canvas={owned} />
      </row>
      <row style="gap: 8px">
        <Button testId="replace" onClick={() => scene.update([tile("#ffff00")])}>Replace</Button>
        <Button testId="translate" onClick={() => scene.update(null, [1, 0, 0, 1, 32, 0])}>Translate</Button>
        <Button testId="clear" onClick={() => scene.update([])}>Clear</Button>
        <Button testId="dispose" onClick={() => {
          scene.dispose();
          owned.dispose();
        }}>Dispose</Button>
      </row>
    </column>;
  }, root);
}
