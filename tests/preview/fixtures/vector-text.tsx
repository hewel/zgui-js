import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => <column style="position: fixed; inset: 0; background-color: #ffffff; color: #000000; font-size: 24px">
    <box testId="plain" style="position: fixed; left: 24px; top: 24px; width: 320px; height: 56px"><text>Readable glyphs</text></box>
    <box testId="scaled" style="position: fixed; left: 24px; top: 120px; width: 320px; height: 56px; transform: scale(0.95); transform-origin: left top"><text>Readable glyphs</text></box>
    <box testId="faded" style="position: fixed; left: 24px; top: 216px; width: 320px; height: 56px; opacity: 0.5"><text>Readable glyphs</text></box>
  </column>, root);
}
