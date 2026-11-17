import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button, ContextMenu, ContextMenuContent, ContextMenuTrigger, useContextMenuPlace,
  MenuCheckboxItem, MenuItem, MenuLabel, MenuRadioGroup, MenuRadioItem, MenuSeparator,
  MenuSub, MenuSubContent, MenuSubTrigger, ThemeProvider,
} from "@hewel/zgui/components";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [open, setOpen] = createSignal(false);
    const [grid, setGrid] = createSignal(false);
    const [zoom, setZoom] = createSignal("100");
    const [last, setLast] = createSignal("none");
    const [btnClicks, setBtnClicks] = createSignal(0);
    const [req, setReq] = createSignal("none");

    function ProgrammaticTrigger() {
      const place = useContextMenuPlace();
      return <Button testId="place-btn" variant="outline"
        style="position: fixed; left: 560px; top: 18px"
        onClick={() => place?.openAt({ x: 320, y: 240 })}>
        Open at 320,240
      </Button>;
    }

    return <ThemeProvider scheme="light">
      <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
        <row style="position: fixed; left: 80px; top: 18px; gap: 16px; align-items: center; font-size: 14px">
          <Button testId="outside" variant="outline" onClick={() => setLast("outside")}>Outside</Button>
          <text testId="open-state">{open() ? "open" : "closed"}</text>
          <text testId="last-action">{last()}</text>
          <text testId="btn-clicks">{btnClicks()}</text>
          <text testId="menu-req">{req()}</text>
        </row>
        <ContextMenu onOpenChange={setOpen}>
          <ProgrammaticTrigger />
          <ContextMenuTrigger testId="area"
            onContextMenu={event => setReq(`${Math.round(event.position.x)},${Math.round(event.position.y)}:${event.pointerKind}`)}
            style="position: fixed; left: 80px; top: 72px; width: 690px; height: 495px; padding: 20px; border: 1px solid var(--zui-color-border); border-radius: 12px; background-color: var(--zui-color-card); gap: 12px">
            <text style="font-size: 16px; font-weight: 600">Canvas Workspace</text>
            <text style="font-size: 13px; color: var(--zui-color-muted-foreground)">
              Right-click or long-press anywhere in this offset area to open actions.
            </text>
            <row style="gap: 12px">
              <Button testId="canvas-btn" variant="secondary" onClick={() => setBtnClicks(n => n + 1)}>
                Canvas Button
              </Button>
            </row>
          </ContextMenuTrigger>
          <ContextMenuContent testId="menu" ariaLabel="Canvas actions" style="width: 220px">
            <MenuLabel testId="menu-label">Canvas</MenuLabel>
            <MenuItem testId="copy" shortcut="Ctrl+C" onSelect={() => setLast("copy")}>Copy selection</MenuItem>
            <MenuItem testId="paste" shortcut="Ctrl+V" closeOnSelect={false} onSelect={() => setLast("paste")}>Paste in place</MenuItem>
            <MenuSeparator />
            <MenuCheckboxItem testId="grid" checked={grid()} closeOnSelect={false}
              onCheckedChange={v => { setGrid(v); setLast(v ? "grid-on" : "grid-off"); }}>
              Show grid
            </MenuCheckboxItem>
            <MenuRadioGroup label="Zoom" value={zoom()} onValueChange={v => { setZoom(v); setLast(`zoom-${v}`); }}>
              <MenuRadioItem testId="zoom-100" value="100">100%</MenuRadioItem>
              <MenuRadioItem testId="zoom-200" value="200">200%</MenuRadioItem>
            </MenuRadioGroup>
            <MenuSub>
              <MenuSubTrigger testId="export-sub">Export as</MenuSubTrigger>
              <MenuSubContent testId="export-menu" style="width: 180px">
                <MenuItem testId="export-png" onSelect={() => setLast("export-png")}>PNG image</MenuItem>
              </MenuSubContent>
            </MenuSub>
          </ContextMenuContent>
        </ContextMenu>
      </box>
    </ThemeProvider>;
  }, root);
}
