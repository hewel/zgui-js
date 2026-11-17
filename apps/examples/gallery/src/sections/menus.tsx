import { createSignal, getWindowRect } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Badge, Button, ContextMenu, ContextMenuContent, ContextMenuTrigger,
  DropdownMenu, DropdownMenuContent, DropdownMenuTrigger,
  MenuCheckboxItem, MenuGroup, MenuItem, MenuLabel, MenuRadioGroup, MenuRadioItem,
  MenuSeparator, MenuShortcut, MenuSub, MenuSubContent, MenuSubTrigger,
  Menubar, MenubarArrows, MenubarCheckboxItem, MenubarContent, MenubarGroup,
  MenubarItem, MenubarLabel, MenubarMenu, MenubarRadioGroup, MenubarRadioItem,
  MenubarSeparator, MenubarShortcut, MenubarTrigger,
  useContextMenuPlace,
} from "@hewel/zgui/components";
import { DemoRow, Panel } from "../shell.js";

function ContextPlaceButton(props: { getArea: () => NativeNode | undefined }) {
  const place = useContextMenuPlace();
  return (
    <Button
      testId="menus-context-open-at"
      size="sm"
      variant="outline"
      onClick={() => {
        const area = props.getArea();
        const rect = area ? getWindowRect(area) : null;
        place?.openAt(rect ? { x: rect.x + 28, y: rect.y + 28 } : { x: 240, y: 260 });
      }}
    >
      Open at corner (useContextMenuPlace)
    </Button>
  );
}

export function MenusSection(props: { onAction: (label: string) => void }) {
  const [autosave, setAutosave] = createSignal(true);
  const [density, setDensity] = createSignal("cosy");
  const [wrapLines, setWrapLines] = createSignal(false);
  const [gridSnap, setGridSnap] = createSignal(true);
  const [toolStep, setToolStep] = createSignal(0);
  const tools = ["Select", "Move", "Measure"] as const;

  let areaNode: NativeNode | undefined;

  return (
    <row class="panel-grid" testId="section-menus">
      <Panel
        title="DropdownMenu, MenuSub & Typeahead"
        note="Items, checkboxes, radio groups, shortcuts, nested submenus, and character jump"
      >
        <DemoRow label="Account dropdown">
          <DropdownMenu>
            <DropdownMenuTrigger testId="menus-dropdown-trigger" variant="outline">
              Account menu
            </DropdownMenuTrigger>
            <DropdownMenuContent testId="menus-dropdown-content" style="width: 220px">
              <MenuLabel>Signed in as ada</MenuLabel>
              <MenuSeparator />
              <MenuGroup>
                <MenuItem shortcut="Ctrl+," onSelect={() => props.onAction("Menu: Settings")}>
                  Settings
                </MenuItem>
                <MenuItem onSelect={() => props.onAction("Menu: Duplicate")}>
                  <text>Duplicate</text>
                  <MenuShortcut>Ctrl+D</MenuShortcut>
                </MenuItem>
                <MenuItem disabled>Team billing (locked)</MenuItem>
              </MenuGroup>
              <MenuSeparator />
              <MenuCheckboxItem
                checked={autosave()}
                closeOnSelect={false}
                onCheckedChange={value => {
                  setAutosave(value);
                  props.onAction(`Autosave ${value ? "on" : "off"}`);
                }}
              >
                Autosave changes
              </MenuCheckboxItem>
              <MenuSeparator />
              <MenuRadioGroup
                value={density()}
                onValueChange={value => {
                  setDensity(value);
                  props.onAction(`Density: ${value}`);
                }}
              >
                <MenuRadioItem value="compact">Compact</MenuRadioItem>
                <MenuRadioItem value="cosy">Cosy</MenuRadioItem>
                <MenuRadioItem value="spacious">Spacious</MenuRadioItem>
              </MenuRadioGroup>
              <MenuSeparator />
              <MenuSub>
                <MenuSubTrigger>Export</MenuSubTrigger>
                <MenuSubContent style="width: 170px">
                  <MenuItem onSelect={() => props.onAction("Export: PNG")}>PNG snapshot</MenuItem>
                  <MenuItem onSelect={() => props.onAction("Export: JSON")}>Tree JSON</MenuItem>
                </MenuSubContent>
              </MenuSub>
              <MenuSeparator />
              <MenuItem destructive onSelect={() => props.onAction("Menu: Sign out")}>
                Sign out
              </MenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Badge variant="outline">
            <text>{`autosave=${String(autosave())} density=${density()}`}</text>
          </Badge>
        </DemoRow>
        <text class="muted-caption">
          While open, press letters (e.g. s, c, e) for MenuTypeahead or ArrowRight to open Export.
        </text>
      </Panel>

      <Panel
        title="ContextMenu"
        note="Right-click, touch long-press, or open via useContextMenuPlace() coordinates"
      >
        <ContextMenu>
          <column class="demo-stack">
            <ContextMenuTrigger testId="menus-context-trigger" ref={el => { areaNode = el; }}>
              <column class="dashed-frame">
                <text style={{ "font-weight": "600" }}>Right-click or long-press here</text>
                <text class="muted-caption">Opens anchored at the pointer coordinate</text>
              </column>
            </ContextMenuTrigger>
            <row class="demo-items">
              <ContextPlaceButton getArea={() => areaNode} />
              <Badge variant="secondary">
                <text>{`Snap: ${gridSnap() ? "on" : "off"}`}</text>
              </Badge>
            </row>
          </column>
          <ContextMenuContent testId="menus-context-content" style="width: 210px">
            <MenuItem shortcut="Ctrl+V" onSelect={() => props.onAction("Context: Paste")}>
              Paste node
            </MenuItem>
            <MenuCheckboxItem
              checked={gridSnap()}
              closeOnSelect={false}
              onCheckedChange={value => {
                setGridSnap(value);
                props.onAction(`Grid snap ${value ? "on" : "off"}`);
              }}
            >
              Snap to grid
            </MenuCheckboxItem>
            <MenuSub>
              <MenuSubTrigger>Inspect</MenuSubTrigger>
              <MenuSubContent style="width: 165px">
                <MenuItem onSelect={() => props.onAction("Inspect: Bounds")}>Bounds</MenuItem>
                <MenuItem onSelect={() => props.onAction("Inspect: Styles")}>Styles</MenuItem>
              </MenuSubContent>
            </MenuSub>
            <MenuSeparator />
            <MenuItem destructive onSelect={() => props.onAction("Context: Delete")}>
              Delete selection
            </MenuItem>
          </ContextMenuContent>
        </ContextMenu>
      </Panel>

      <Panel
        wide
        title="Editor Menubar & MenubarArrows"
        note="Horizontal bar with cross-menu Left/Right traversal, checkboxes, radios, and custom MenubarArrows strip"
      >
        <row class="demo-items">
          <Menubar testId="menus-editor-bar" label="Editor menubar">
            <MenubarMenu value="file">
              <MenubarTrigger>File</MenubarTrigger>
              <MenubarContent>
                <MenubarItem shortcut="Ctrl+N" onSelect={() => props.onAction("Editor: New")}>
                  New document
                </MenubarItem>
                <MenubarItem shortcut="Ctrl+O" onSelect={() => props.onAction("Editor: Open")}>
                  Open...
                </MenubarItem>
                <MenubarSeparator />
                <MenuSub>
                  <MenuSubTrigger>Export as</MenuSubTrigger>
                  <MenuSubContent style="width: 170px">
                    <MenuItem onSelect={() => props.onAction("Editor: PDF")}>PDF</MenuItem>
                    <MenuItem onSelect={() => props.onAction("Editor: SVG")}>SVG</MenuItem>
                  </MenuSubContent>
                </MenuSub>
              </MenubarContent>
            </MenubarMenu>
            <MenubarMenu value="edit">
              <MenubarTrigger>Edit</MenubarTrigger>
              <MenubarContent>
                <MenubarLabel>History</MenubarLabel>
                <MenubarItem shortcut="Ctrl+Z" onSelect={() => props.onAction("Editor: Undo")}>
                  Undo
                </MenubarItem>
                <MenubarItem onSelect={() => props.onAction("Editor: Redo")}>
                  <text>Redo</text>
                  <MenubarShortcut>Ctrl+Y</MenubarShortcut>
                </MenubarItem>
              </MenubarContent>
            </MenubarMenu>
            <MenubarMenu value="view">
              <MenubarTrigger testId="menus-editor-view">View</MenubarTrigger>
              <MenubarContent testId="menus-editor-view-content">
                <MenubarCheckboxItem
                  checked={wrapLines()}
                  onCheckedChange={value => {
                    setWrapLines(value);
                    props.onAction(`Wrap ${value ? "on" : "off"}`);
                  }}
                >
                  Word wrap
                </MenubarCheckboxItem>
                <MenubarSeparator />
                <MenubarGroup>
                  <MenubarLabel>Layout density</MenubarLabel>
                  <MenubarRadioGroup value={density()} onValueChange={setDensity}>
                    <MenubarRadioItem value="compact">Compact</MenubarRadioItem>
                    <MenubarRadioItem value="cosy">Cosy</MenubarRadioItem>
                    <MenubarRadioItem testId="menus-editor-spacious" value="spacious">Spacious</MenubarRadioItem>
                  </MenubarRadioGroup>
                </MenubarGroup>
              </MenubarContent>
            </MenubarMenu>
          </Menubar>

          <MenubarArrows
            bar={{
              step(direction) {
                const next = (toolStep() + direction + tools.length) % tools.length;
                setToolStep(next);
                props.onAction(`Tool stepped: ${tools[next]}`);
                return true;
              },
            }}
          >
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                const next = (toolStep() + 1) % tools.length;
                setToolStep(next);
                props.onAction(`Tool: ${tools[next]}`);
              }}
            >
              <text>{`MenubarArrows tool: ${tools[toolStep()]} (Left/Right)`}</text>
            </Button>
          </MenubarArrows>
        </row>
      </Panel>
    </row>
  );
}
