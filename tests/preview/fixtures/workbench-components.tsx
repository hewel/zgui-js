import { createSignal, renderApp, setReducedMotion } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  Button,
  Combobox,
  Select,
  SplitPane,
  SplitPaneHandle,
  SplitPanePanel,
  ThemeProvider,
  ToastProvider,
  TreeView,
  toast,
  useToast,
} from "@hewel/zgui/components";
import type { ColorScheme, SelectOption, TreeNodeData } from "@hewel/zgui/components";

export const hostApiVersion = 1;

const LONG_OPTIONS: readonly SelectOption[] = Array.from({ length: 24 }, (_, idx) => ({
  value: `opt-${idx + 1}`,
  label: `Option ${String(idx + 1).padStart(2, "0")}`,
  description: idx % 2 === 0 ? "Even entry" : "Odd entry",
}));

const TREE_ITEMS: readonly TreeNodeData[] = [
  {
    id: "workspace",
    label: "workspace",
    children: [
      {
        id: "workspace/notes",
        label: "notes",
        children: [
          { id: "workspace/notes/alpha.md", label: "alpha.md" },
          { id: "workspace/notes/beta.md", label: "beta.md" },
        ],
      },
      { id: "workspace/todo.md", label: "todo.md" },
    ],
  },
  { id: "changelog.md", label: "changelog.md" },
];

const LARGE_TREE: readonly TreeNodeData[] = Array.from({ length: 120 }, (_, i) => ({
  id: `node-${i}`,
  label: `Node ${String(i).padStart(3, "0")}`,
}));

function ToastCountBadge() {
  const controller = useToast();
  return <text testId="toast-count">{`toasts:${controller.items().length}`}</text>;
}

function WorkbenchFixture() {
  const [scheme, setScheme] = createSignal<ColorScheme>("light");
  const [sizes, setSizes] = createSignal<readonly number[]>([40, 60]);
  const [collapsed, setCollapsed] = createSignal(false);
  const [selectVal, setSelectVal] = createSignal<string>("opt-1");
  const [comboVal, setComboVal] = createSignal<string>("opt-1");
  const [treeSelected, setTreeSelected] = createSignal<string>("workspace");
  const [treeActivated, setTreeActivated] = createSignal<string>("none");
  const [treeExpanded, setTreeExpanded] = createSignal<readonly string[]>(["workspace"]);
  const [virtSelected, setVirtSelected] = createSignal<string>("node-0");
  const [toastActionLog, setToastActionLog] = createSignal("none");
  const [reduced, setReduced] = createSignal(false);

  return (
    <ThemeProvider scheme={scheme()}>
      <ToastProvider
        maxVisible={2}
        defaultDuration={200}
        corner="bottom-right"
        regionTestId="wb-toast-region"
      >
        <column
          testId="wb-root"
          style={{
            width: "100%",
            height: "100%",
            padding: "12px",
            gap: "10px",
            backgroundColor: "var(--zui-color-background)",
            color: "var(--zui-color-foreground)",
            boxSizing: "border-box",
          }}
        >
          <row style={{ gap: "8px", alignItems: "center" }}>
            <Button
              testId="toggle-scheme"
              size="xs"
              onClick={() => setScheme(s => (s === "light" ? "dark" : "light"))}
            >
              {`Scheme:${scheme()}`}
            </Button>
            <Button
              testId="toggle-reduced"
              size="xs"
              variant="outline"
              onClick={() => {
                const next = !reduced();
                setReduced(next);
                setReducedMotion(next);
              }}
            >
              {`Reduced:${reduced() ? "on" : "off"}`}
            </Button>
            <Button
              testId="btn-toast-success"
              size="xs"
              onClick={() => {
                toast.success("Saved file", {
                  id: "save-1",
                  description: "Written atomically.",
                  action: {
                    label: "Undo",
                    onClick: () => setToastActionLog("undo-clicked"),
                  },
                });
              }}
            >
              Toast Success
            </Button>
            <Button
              testId="btn-toast-stack"
              size="xs"
              variant="secondary"
              onClick={() => {
                toast.info("First", { id: "t-1", duration: 0 });
                toast.warning("Second", { id: "t-2", duration: 0 });
                toast.error("Third", { id: "t-3", duration: 0 });
              }}
            >
              Toast Stack 3
            </Button>
            <Button
              testId="btn-toast-promise"
              size="xs"
              variant="outline"
              onClick={() => {
                void toast.promise(Promise.resolve("ok"), {
                  loading: "Syncing...",
                  success: value => `Synced:${value}`,
                  error: "Failed",
                });
              }}
            >
              Toast Promise
            </Button>
            <Button
              testId="btn-toast-clear"
              size="xs"
              variant="ghost"
              onClick={() => toast.dismiss()}
            >
              Clear Toasts
            </Button>
            <ToastCountBadge />
            <text testId="toast-action-log">{toastActionLog()}</text>
          </row>

          <row style={{ gap: "12px", alignItems: "center" }}>
            <Select
              testId="wb-select"
              listTestId="wb-select-list"
              options={LONG_OPTIONS}
              value={selectVal()}
              onValueChange={setSelectVal}
              label="Long select"
            />
            <text testId="wb-select-val">{`sel:${selectVal()}`}</text>

            <Combobox
              testId="wb-combo"
              inputTestId="wb-combo-input"
              listTestId="wb-combo-list"
              options={LONG_OPTIONS}
              value={comboVal()}
              onValueChange={setComboVal}
              placeholder="Filter options..."
              emptyText="Nothing matched"
              label="Filter options"
            />
            <text testId="wb-combo-val">{`combo:${comboVal()}`}</text>
          </row>

          <box
            style={{
              width: "100%",
              height: "150px",
              border: "1px solid var(--zui-color-border)",
              borderRadius: "6px",
              overflow: "hidden",
            }}
          >
            <SplitPane testId="wb-split" sizes={sizes()} onSizesChange={setSizes} keyboardStep={5}>
              <SplitPanePanel
                testId="wb-split-left"
                minSize={20}
                maxSize={75}
                collapsible
                collapsedSize={0}
                onCollapseChange={setCollapsed}
              >
                <column style={{ padding: "8px", gap: "4px" }}>
                  <text testId="wb-split-readout">{`left:${Math.round(sizes()[0] ?? 0)},collapsed:${collapsed()}`}</text>
                </column>
              </SplitPanePanel>
              <SplitPaneHandle testId="wb-split-handle" withHandle label="Resize split" />
              <SplitPanePanel testId="wb-split-right" minSize={20}>
                <column style={{ padding: "8px", gap: "4px" }}>
                  <text testId="wb-split-right-readout">{`right:${Math.round(sizes()[1] ?? 0)}`}</text>
                </column>
              </SplitPanePanel>
            </SplitPane>
          </box>

          <row style={{ gap: "12px", flexGrow: 1, minHeight: "0px" }}>
            <column style={{ width: "360px", gap: "6px" }}>
              <text testId="wb-tree-state">{`sel:${treeSelected()},act:${treeActivated()},exp:${treeExpanded().join("|")}`}</text>
              <box
                style={{
                  border: "1px solid var(--zui-color-border)",
                  borderRadius: "6px",
                  padding: "4px",
                  height: "200px",
                  overflow: "auto",
                }}
              >
                <TreeView
                  testId="wb-tree"
                  items={TREE_ITEMS}
                  selectedId={treeSelected()}
                  onSelect={setTreeSelected}
                  onActivate={setTreeActivated}
                  expandedIds={treeExpanded()}
                  onExpandedChange={setTreeExpanded}
                  label="Workspace tree"
                />
              </box>
            </column>

            <column style={{ width: "360px", gap: "6px" }}>
              <text testId="wb-vtree-state">{`vsel:${virtSelected()}`}</text>
              <box
                style={{
                  display: "flex",
                  flexDirection: "column",
                  border: "1px solid var(--zui-color-border)",
                  borderRadius: "6px",
                  padding: "4px",
                  height: "200px",
                  overflow: "hidden",
                }}
              >
                <TreeView
                  testId="wb-vtree"
                  items={LARGE_TREE}
                  selectedId={virtSelected()}
                  onSelect={setVirtSelected}
                  virtualize={{ itemHeight: 28, overscan: 3, initialHeight: 192 }}
                  label="Virtualized tree"
                />
              </box>
            </column>
          </row>
        </column>
      </ToastProvider>
    </ThemeProvider>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <WorkbenchFixture />, root);
}
