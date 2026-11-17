import { createSignal, onCleanup, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Input, Popover, PopoverClose, PopoverContent, PopoverTitle, PopoverTrigger, ThemeProvider } from "@hewel/zgui/components";
import { Popper } from "__POPPER_MODULE__";
declare const globalThis: { __zgui: { installStylesheet(name: string, css: string): void; removeStylesheet(name: string): void } };
export const hostApiVersion = 1;
const css = `:root { background-color: #ffffff; color: #111827; font-size: 14px; }
.popover-geometry-page { position: fixed; inset: 0; background-color: #ffffff; color: #111827; }
.popover-geometry-anchor { position: fixed; right: 12px; bottom: 12px; width: 80px; height: 32px; }
.popover-size-anchor { position: fixed; left: 40.2px; top: 40.2px; width: 100.2px; height: 20.2px; background: #374151; }
.popover-sized { width: var(--zui-popper-anchor-width); height: var(--zui-popper-anchor-width); background: #0f766e; }`;
export function mount(root: NativeNode) {
  return renderApp(() => {
    globalThis.__zgui.installStylesheet("popover-geometry-fixture", css);
    onCleanup(() => globalThis.__zgui.removeStylesheet("popover-geometry-fixture"));
    const [anchor, setAnchor] = createSignal<NativeNode | null>(null);
    return <ThemeProvider scheme="light">
      <box class="popover-geometry-page" />
      <box testId="size-anchor" class="popover-size-anchor" ref={setAnchor} />
      <Popper anchor={anchor()}><box testId="sized" class="popover-sized" /></Popper>
      <Popover defaultOpen>
        <PopoverTrigger testId="cap-anchor" class="popover-geometry-anchor">Details</PopoverTrigger>
        <PopoverContent testId="cap-surface" style="height: 700px; gap: 8px">
          <column style="flex-shrink: 0; gap: 8px">
            <PopoverTitle>Geometry</PopoverTitle>
            <Input testId="cap-input" label="Measurement" defaultValue="Measured" />
            <column testId="tall-content" style="height: 1000px; flex-shrink: 0; gap: 180px">
              <text>First detail</text><text>Second detail</text><text>Third detail</text><text>Last detail</text>
              <PopoverClose testId="cap-close">Close</PopoverClose>
            </column>
          </column>
        </PopoverContent>
      </Popover>
    </ThemeProvider>;
  }, root);
}
