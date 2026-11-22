import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  Menubar,
  MenubarCheckboxItem,
  MenubarContent,
  MenubarGroup,
  MenubarItem,
  MenubarLabel,
  MenubarMenu,
  MenubarRadioGroup,
  MenubarRadioItem,
  MenubarSeparator,
  MenubarTrigger,
  MenuItem,
  MenuSub,
  MenuSubContent,
  MenuSubTrigger,
  ThemeProvider,
} from "@hewel/zgui/components";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [wrap, setWrap] = createSignal(false);
    const [minimap, setMinimap] = createSignal(true);
    const [layout, setLayout] = createSignal("single");
    const [last, setLast] = createSignal("none");
    const [outsideClicks, setOutsideClicks] = createSignal(0);

    return (
      <ThemeProvider scheme="light">
        <box style="position: fixed; inset: 0; background-color: var(--zui-color-background); color: var(--zui-color-foreground)">
          <row style="position: fixed; left: 24px; top: 20px; gap: 16px; align-items: center">
            <Menubar testId="bar" label="Workspace menu">
              <MenubarMenu testId="file-menu" value="file">
                <MenubarTrigger testId="file-trigger">File</MenubarTrigger>
                <MenubarContent testId="file-content">
                  <MenubarItem testId="file-disabled" disabled onSelect={() => setLast("disabled")}>
                    Disabled
                  </MenubarItem>
                  <MenubarItem testId="file-new" shortcut="Ctrl+N" onSelect={() => setLast("new")}>
                    New file
                  </MenubarItem>
                  <MenubarItem
                    testId="file-open"
                    shortcut="Ctrl+O"
                    closeOnSelect={false}
                    onSelect={() => setLast("open")}
                  >
                    Open recent
                  </MenubarItem>
                  <MenubarSeparator />
                  <MenuSub>
                    <MenuSubTrigger testId="file-export">Export</MenuSubTrigger>
                    <MenuSubContent testId="file-export-sub" style="width: 170px">
                      <MenuItem testId="export-pdf" onSelect={() => setLast("export-pdf")}>
                        PDF document
                      </MenuItem>
                    </MenuSubContent>
                  </MenuSub>
                  <MenubarSeparator />
                  <MenubarItem
                    testId="file-quit"
                    shortcut="Ctrl+Q"
                    onSelect={() => setLast("quit")}
                  >
                    Quit
                  </MenubarItem>
                </MenubarContent>
              </MenubarMenu>

              <MenubarMenu testId="edit-menu" value="edit">
                <MenubarTrigger testId="edit-trigger">Edit</MenubarTrigger>
                <MenubarContent testId="edit-content">
                  <MenubarLabel>History</MenubarLabel>
                  <MenubarGroup label="Edit actions">
                    <MenubarItem
                      testId="edit-undo"
                      shortcut="Ctrl+Z"
                      onSelect={() => setLast("undo")}
                    >
                      Undo
                    </MenubarItem>
                    <MenubarItem
                      testId="edit-redo"
                      shortcut="Ctrl+Y"
                      onSelect={() => setLast("redo")}
                    >
                      Redo
                    </MenubarItem>
                  </MenubarGroup>
                </MenubarContent>
              </MenubarMenu>

              <MenubarMenu testId="tools-menu" value="tools">
                <MenubarTrigger testId="tools-trigger" disabled>
                  Tools
                </MenubarTrigger>
                <MenubarContent testId="tools-content">
                  <MenubarItem testId="tools-run">Inspect</MenubarItem>
                </MenubarContent>
              </MenubarMenu>

              <MenubarMenu testId="view-menu" value="view">
                <MenubarTrigger testId="view-trigger">View</MenubarTrigger>
                <MenubarContent testId="view-content">
                  <MenubarCheckboxItem
                    testId="view-wrap"
                    checked={wrap()}
                    onCheckedChange={v => {
                      setWrap(v);
                      setLast(v ? "wrap-on" : "wrap-off");
                    }}
                  >
                    Word wrap
                  </MenubarCheckboxItem>
                  <MenubarCheckboxItem
                    testId="view-minimap"
                    checked={minimap()}
                    closeOnSelect
                    onCheckedChange={v => {
                      setMinimap(v);
                      setLast(v ? "minimap-on" : "minimap-off");
                    }}
                  >
                    Show minimap
                  </MenubarCheckboxItem>
                  <MenubarSeparator />
                  <MenubarLabel>Layout</MenubarLabel>
                  <MenubarRadioGroup
                    label="Layout"
                    value={layout()}
                    onValueChange={v => {
                      setLayout(v);
                      setLast(`layout-${v}`);
                    }}
                  >
                    <MenubarRadioItem testId="layout-single" value="single">
                      Single pane
                    </MenubarRadioItem>
                    <MenubarRadioItem testId="layout-split" value="split">
                      Split view
                    </MenubarRadioItem>
                  </MenubarRadioGroup>
                </MenubarContent>
              </MenubarMenu>
            </Menubar>
            <Button
              testId="outside"
              variant="outline"
              onClick={() => {
                setOutsideClicks(n => n + 1);
                setLast("outside");
              }}
            >
              Outside action
            </Button>
            <row style="gap: 4px; align-items: center; font-size: 13px">
              <text style="color: var(--zui-color-muted-foreground)">Outside:</text>
              <text testId="outside-clicks">{outsideClicks()}</text>
            </row>
          </row>

          <column style="position: fixed; left: 420px; top: 96px; width: 320px; gap: 10px; padding: 16px; border: 1px solid var(--zui-color-border); border-radius: 10px; background-color: var(--zui-color-card); font-size: 14px">
            <text style="font-weight: 600">Menubar Diagnostics</text>
            <row style="justify-content: space-between">
              <text style="color: var(--zui-color-muted-foreground)">Wrap:</text>
              <text testId="wrap-state">{String(wrap())}</text>
            </row>
            <row style="justify-content: space-between">
              <text style="color: var(--zui-color-muted-foreground)">Minimap:</text>
              <text testId="minimap-state">{String(minimap())}</text>
            </row>
            <row style="justify-content: space-between">
              <text style="color: var(--zui-color-muted-foreground)">Layout:</text>
              <text testId="layout-state">{layout()}</text>
            </row>
            <row style="justify-content: space-between">
              <text style="color: var(--zui-color-muted-foreground)">Last:</text>
              <text testId="last-action">{last()}</text>
            </row>
          </column>
        </box>
      </ThemeProvider>
    );
  }, root);
}
