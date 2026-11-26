import { createSignal } from "@hewel/zgui";
import {
  Badge,
  Button,
  Combobox,
  CommandPalette,
  CommandPaletteProvider,
  Kbd,
  KbdGroup,
  Select,
  SplitPane,
  SplitPaneHandle,
  SplitPanePanel,
  ToastProvider,
  TreeItem,
  TreeView,
  toast,
  useCommandPalette,
} from "@hewel/zgui/components";
import type { CommandItem, SelectOption, TreeNodeData } from "@hewel/zgui/components";
import { DemoRow, Panel } from "../shell.js";

const FRAMEWORK_OPTIONS: readonly SelectOption[] = [
  { value: "zgui", label: "zgui-js", description: "Native Wayland + QuickJS UI" },
  { value: "solid", label: "Solid 2.0", description: "Fine-grained signals" },
  { value: "stylo", label: "Stylo CSS", description: "Parallel selector engine" },
  { value: "vello", label: "Vello GPU", description: "Compute shader vector renderer" },
  { value: "taffy", label: "Taffy Layout", description: "Flexbox & Grid layout" },
];

const WORKSPACE_TREE: readonly TreeNodeData[] = [
  {
    id: "docs",
    label: "docs",
    children: [
      { id: "docs/intro.md", label: "intro.md", badge: "MD" },
      { id: "docs/architecture.md", label: "architecture.md", badge: "MD" },
    ],
  },
  {
    id: "src",
    label: "src",
    children: [
      { id: "src/main.tsx", label: "main.tsx", badge: "TSX" },
      { id: "src/theme.ts", label: "theme.ts", badge: "TS" },
    ],
  },
  { id: "README.md", label: "README.md" },
];

function CommandPaletteShowcase(props: {
  lastCommand: () => string;
  onRun: (command: CommandItem) => void;
}) {
  const palette = useCommandPalette();
  return (
    <Panel
      title="CommandPalette"
      note="Modal fuzzy command search over Dialog, VirtualList, Kbd badges, and provider-scoped registries"
      wide
    >
      <DemoRow label="Palette trigger">
        <Button
          testId="workbench-command-trigger"
          size="sm"
          variant="outline"
          onClick={() => palette.setOpen(true)}
        >
          <text>Open Command Palette</text>
          <KbdGroup>
            <Kbd>Ctrl</Kbd>
            <Kbd>K</Kbd>
          </KbdGroup>
        </Button>
        <Badge testId="workbench-command-last" variant="secondary">
          <text>{props.lastCommand()}</text>
        </Badge>
      </DemoRow>
      <CommandPalette
        testId="workbench-command-palette"
        inputTestId="workbench-command-input"
        listTestId="workbench-command-list"
        placeholder="Search workbench commands..."
        onSelect={props.onRun}
      />
    </Panel>
  );
}

export function WorkbenchSection(props: { onAction: (label: string) => void }) {
  const [splitSizes, setSplitSizes] = createSignal<readonly number[]>([36, 64]);
  const [selectedFramework, setSelectedFramework] = createSignal<string>("zgui");
  const [comboFramework, setComboFramework] = createSignal<string>("solid");
  const [selectedTree, setSelectedTree] = createSignal<string>("docs/intro.md");
  const [expandedTree, setExpandedTree] = createSignal<readonly string[]>(["docs", "src"]);
  const [lastCommand, setLastCommand] = createSignal<string>("Last: none");

  const galleryCommands: readonly CommandItem[] = [
    {
      id: "file.new",
      label: "New Document",
      group: "File",
      shortcut: ["Ctrl", "N"],
      keywords: ["create", "buffer"],
    },
    {
      id: "file.open",
      label: "Open Workspace",
      group: "File",
      shortcut: ["Ctrl", "O"],
      keywords: ["folder", "project"],
    },
    {
      id: "file.save",
      label: "Save All Changes",
      group: "File",
      shortcut: ["Ctrl", "S"],
      keywords: ["write", "disk"],
    },
    {
      id: "view.split",
      label: "Toggle Split Preview",
      group: "View",
      shortcut: ["Ctrl", "\\"],
      keywords: ["pane", "layout"],
    },
    {
      id: "view.theme",
      label: "Switch Color Scheme",
      group: "View",
      shortcut: ["Ctrl", "Shift", "T"],
      keywords: ["dark", "light"],
    },
    {
      id: "tools.export",
      label: "Export Snapshot",
      group: "Tools",
      description: "Capture PNG + accessibility tree",
      disabled: true,
    },
  ];

  return (
    <CommandPaletteProvider commands={galleryCommands}>
      <row class="panel-grid" testId="section-workbench">
        <Panel
          title="SplitPane"
          note="Draggable horizontal split panes with pointer capture, keyboard resize, and collapsible bounds"
          wide
        >
          <box class="split-frame">
            <SplitPane
              testId="workbench-split-pane"
              sizes={splitSizes()}
              onSizesChange={next => {
                setSplitSizes(next);
                props.onAction(`Split ${Math.round(next[0] ?? 0)}/${Math.round(next[1] ?? 0)}`);
              }}
              label="Workbench split demo"
            >
              <SplitPanePanel minSize={20} collapsible collapsedSize={0}>
                <column class="split-pane-body">
                  <text class="demo-strong">Navigator</text>
                  <Badge testId="workbench-split-left" variant="secondary">
                    <text>{`${Math.round(splitSizes()[0] ?? 0)}%`}</text>
                  </Badge>
                </column>
              </SplitPanePanel>
              <SplitPaneHandle
                testId="workbench-split-handle"
                withHandle
                label="Resize navigator"
              />
              <SplitPanePanel minSize={30}>
                <column class="split-pane-body">
                  <text class="demo-strong">Editor Surface</text>
                  <Badge testId="workbench-split-right" variant="outline">
                    <text>{`${Math.round(splitSizes()[1] ?? 0)}%`}</text>
                  </Badge>
                </column>
              </SplitPanePanel>
            </SplitPane>
          </box>
        </Panel>

        <Panel
          title="Select & Combobox"
          note="Anchored listbox overlays with keyboard navigation, scrollIntoView reveal, typeahead, and search filtering"
        >
          <DemoRow label="Select">
            <Select
              testId="workbench-select"
              listTestId="workbench-select-list"
              options={FRAMEWORK_OPTIONS}
              value={selectedFramework()}
              onValueChange={value => {
                setSelectedFramework(value);
                props.onAction(`Select ${value}`);
              }}
              label="Choose engine"
            />
            <Badge testId="workbench-select-value" variant="outline">
              <text>{selectedFramework()}</text>
            </Badge>
          </DemoRow>
          <DemoRow label="Combobox">
            <Combobox
              testId="workbench-combobox"
              inputTestId="workbench-combobox-input"
              listTestId="workbench-combobox-list"
              options={FRAMEWORK_OPTIONS}
              value={comboFramework()}
              onValueChange={value => {
                setComboFramework(value);
                props.onAction(`Combobox ${value}`);
              }}
              placeholder="Filter components..."
              label="Filter engine"
            />
            <Badge testId="workbench-combobox-value" variant="outline">
              <text>{comboFramework()}</text>
            </Badge>
          </DemoRow>
        </Panel>

        <Panel
          title="Toast Notifications"
          note="OverlayLayer::Toast stack with owner-scoped auto-dismiss, hover-pause, actions, and reduced motion"
        >
          <DemoRow label="Triggers">
            <Button
              testId="workbench-toast-success"
              size="sm"
              onClick={() => {
                toast.success("Workspace saved", {
                  description: "All markdown notes synced cleanly.",
                  action: {
                    label: "Undo",
                    onClick: () => props.onAction("Toast Undo"),
                  },
                });
                props.onAction("Toast Success");
              }}
            >
              Success Toast
            </Button>
            <Button
              testId="workbench-toast-error"
              size="sm"
              variant="destructive"
              onClick={() => {
                toast.error("Sync failed", { description: "Disk permission denied." });
                props.onAction("Toast Error");
              }}
            >
              Error Toast
            </Button>
            <Button
              testId="workbench-toast-dismiss"
              size="sm"
              variant="outline"
              onClick={() => {
                toast.dismiss();
                props.onAction("Toast Dismissed");
              }}
            >
              Dismiss All
            </Button>
          </DemoRow>
        </Panel>

        <Panel
          title="TreeView & TreeItem"
          note="Keyboard-navigable hierarchical tree supporting data-driven items and declarative TreeItem children"
        >
          <DemoRow label="Data-driven workspace tree">
            <Badge testId="workbench-tree-selected" variant="secondary">
              <text>{`Selected: ${selectedTree()}`}</text>
            </Badge>
          </DemoRow>
          <TreeView
            testId="workbench-tree"
            items={WORKSPACE_TREE}
            selectedId={selectedTree()}
            onSelect={id => {
              setSelectedTree(id);
              props.onAction(`Tree ${id}`);
            }}
            expandedIds={expandedTree()}
            onExpandedChange={setExpandedTree}
            label="Workspace files"
          />
          <DemoRow label="Declarative TreeItem">
            <TreeView
              testId="workbench-tree-declarative"
              defaultExpandedIds={["outline"]}
              label="Outline"
            >
              <TreeItem itemId="outline" label="Outline">
                <TreeItem itemId="h1" label="Introduction" />
                <TreeItem itemId="h2" label="Implementation" />
              </TreeItem>
            </TreeView>
          </DemoRow>
        </Panel>
        <CommandPaletteShowcase
          lastCommand={lastCommand}
          onRun={cmd => {
            setLastCommand(`Last: ${cmd.label}`);
            props.onAction(`Command ${cmd.id}`);
          }}
        />
        <ToastProvider />
      </row>
    </CommandPaletteProvider>
  );
}
