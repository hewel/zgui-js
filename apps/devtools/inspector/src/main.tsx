import {
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  renderApp,
  type NativeNode,
} from "@hewel/zgui";
import {
  Badge,
  Button,
  Separator,
  SplitPane,
  SplitPaneHandle,
  SplitPanePanel,
  ThemeProvider,
  TreeView,
  type TreeNodeData,
} from "@hewel/zgui/components";
import {
  connectUnixSocket,
  getAuthorizedEndpoint,
  getResolvedCapabilities,
  type UnixSocket,
} from "@hewel/zgui-services";
import styles from "./Inspector.module.css";

interface Bounds {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

interface BoxEdges {
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
  readonly left: number;
}

interface BoxModel {
  readonly borderBox: Bounds;
  readonly contentBox: Bounds;
  readonly padding: BoxEdges;
  readonly border: BoxEdges;
  readonly margin: BoxEdges;
}

interface ComputedStyleSummary {
  readonly display: string;
  readonly position: string;
  readonly visibility: string;
  readonly opacity: number;
  readonly fontSize: number;
}

interface InspectorNode {
  readonly token: string;
  readonly parent?: string | null;
  readonly kind: string;
  readonly testId?: string | null;
  readonly classes?: readonly string[];
  readonly source?: string | null;
  readonly role?: string | null;
  readonly text?: string;
  readonly redacted?: boolean;
  readonly boundsLogical?: Bounds | null;
  readonly boxModel?: BoxModel | null;
  readonly computedStyle?: ComputedStyleSummary | null;
}

interface FramePhases {
  readonly styleUs: number;
  readonly layoutUs: number;
  readonly paintUs: number;
  readonly totalUs: number;
}

interface InspectorSurface {
  readonly surface: string;
  readonly coherent?: boolean;
  readonly coherenceStatus?: string;
  readonly excluded?: boolean;
  readonly reason?: string;
  readonly frame?: {
    readonly frameOrdinal?: number | null;
    readonly phases?: FramePhases;
  };
  readonly nodes?: readonly InspectorNode[];
}

interface PickerTarget {
  readonly surface: string;
  readonly token: string;
  readonly source?: string | null;
}

interface PickerSnapshot {
  readonly active?: boolean;
  readonly forbidden?: boolean;
  readonly reason?: string;
  readonly hovered?: PickerTarget | null;
  readonly selected?: PickerTarget | null;
}

interface InspectorSnapshot {
  readonly readOnly?: boolean;
  readonly error?: string;
  readonly picker?: PickerSnapshot;
  readonly surfaces?: readonly InspectorSurface[];
  readonly session?: {
    readonly generation?: number;
    readonly resources?: {
      readonly nodes?: number;
      readonly callbacks?: number;
      readonly timers?: number;
      readonly unixSockets?: number;
    };
  };
  readonly profiler?: {
    readonly frameCount?: number;
    readonly frames?: readonly {
      readonly ordinal?: number;
      readonly styleUs?: number;
      readonly layoutUs?: number;
      readonly paintUs?: number;
      readonly totalUs?: number;
    }[];
  };
}

interface FlatSelection {
  readonly surface: string;
  readonly node: InspectorNode;
}

function formatEdges(edges: BoxEdges): string {
  return `${Math.round(edges.top)} ${Math.round(edges.right)} ${Math.round(edges.bottom)} ${Math.round(edges.left)}`;
}

function formatBounds(bounds: Bounds): string {
  return `${Math.round(bounds.x)}, ${Math.round(bounds.y)} (${Math.round(bounds.width)}x${Math.round(bounds.height)})`;
}

function buildSurfaceTree(surfaces: readonly InspectorSurface[]): {
  readonly items: readonly TreeNodeData<FlatSelection>[];
  readonly byId: ReadonlyMap<string, FlatSelection>;
  readonly defaultExpanded: readonly string[];
} {
  const byId = new Map<string, FlatSelection>();
  const defaultExpanded: string[] = [];
  const items: TreeNodeData<FlatSelection>[] = [];

  for (const surface of surfaces) {
    const surfaceId = `surface:${surface.surface}`;
    defaultExpanded.push(surfaceId);
    const nodes = surface.nodes ?? [];
    const childrenByParent = new Map<string, InspectorNode[]>();
    const roots: InspectorNode[] = [];
    const knownTokens = new Set(nodes.map(n => n.token));

    for (const node of nodes) {
      const id = `${surface.surface}:${node.token}`;
      byId.set(id, { surface: surface.surface, node });
      if (node.parent && knownTokens.has(node.parent)) {
        const list = childrenByParent.get(node.parent) ?? [];
        list.push(node);
        childrenByParent.set(node.parent, list);
      } else {
        roots.push(node);
      }
    }

    const toTreeNode = (node: InspectorNode): TreeNodeData<FlatSelection> => {
      const id = `${surface.surface}:${node.token}`;
      const childNodes = childrenByParent.get(node.token) ?? [];
      if (childNodes.length > 0) defaultExpanded.push(id);
      const suffix = node.testId
        ? ` #${node.testId}`
        : node.classes && node.classes.length > 0
          ? ` .${node.classes[0]}`
          : "";
      return {
        id,
        label: `<${node.kind}>${suffix}`,
        badge: node.redacted ? "redacted" : node.source ? "src" : undefined,
        meta: { surface: surface.surface, node },
        children: childNodes.length > 0 ? childNodes.map(toTreeNode) : undefined,
      };
    };

    items.push({
      id: surfaceId,
      label: surface.excluded
        ? `${surface.surface} (locked)`
        : `${surface.surface} (${nodes.length})`,
      badge: surface.coherenceStatus,
      children: roots.map(toTreeNode),
    });
  }

  return { items, byId, defaultExpanded };
}

function InspectorApp() {
  const [socketPath, setSocketPath] = createSignal<string | null>(null);
  const [snapshot, setSnapshot] = createSignal<InspectorSnapshot | null>(null);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [expandedIds, setExpandedIds] = createSignal<readonly string[]>([]);
  const [status, setStatus] = createSignal("Initializing...");
  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const openSockets = new Set<UnixSocket>();
  onCleanup(() => {
    for (const sock of openSockets) {
      sock.close();
    }
    openSockets.clear();
  });

  const resolveSocket = (): string | null => {
    const named = getAuthorizedEndpoint("devInspect");
    if (named) return named;
    const caps = getResolvedCapabilities();
    return caps.unixSockets[0] ?? null;
  };

  const requestInspector = (
    payload: Record<string, unknown>,
    onReply: (reply: InspectorSnapshot) => void,
    overridePath?: string,
  ): void => {
    const path = overridePath ?? socketPath();
    if (!path) {
      setErrorMessage("No --dev-inspect socket bound (pass --socket to zgui-js inspect --ui).");
      setStatus("Disconnected");
      return;
    }
    setBusy(true);
    setErrorMessage(null);
    let settled = false;
    const sock = connectUnixSocket(path, {
      framing: "line",
      onMessage(message) {
        if (settled) return;
        settled = true;
        sock.close();
        openSockets.delete(sock);
        setBusy(false);
        try {
          const parsed = JSON.parse(message) as InspectorSnapshot;
          if (parsed.error) {
            setErrorMessage(parsed.error);
          }
          onReply(parsed);
        } catch (error) {
          setErrorMessage(error instanceof Error ? error.message : String(error));
        }
      },
      onClose(err) {
        openSockets.delete(sock);
        if (!settled) {
          settled = true;
          setBusy(false);
          if (err) {
            setErrorMessage(`${err.code}: ${err.message}`);
            setStatus("Error");
          }
        }
      },
    });
    openSockets.add(sock);
    sock.send(JSON.stringify(payload));
  };

  const refreshSnapshot = (overridePath?: string): void => {
    setStatus("Refreshing...");
    requestInspector(
      { op: "snapshot" },
      data => {
        setSnapshot(data);
        const surfaces = data.surfaces ?? [];
        const built = buildSurfaceTree(surfaces);
        if (expandedIds().length === 0 && built.defaultExpanded.length > 0) {
          setExpandedIds(built.defaultExpanded);
        }
        const picked = data.picker?.selected;
        if (picked) {
          const pickedId = `${picked.surface}:${picked.token}`;
          if (built.byId.has(pickedId)) {
            setSelectedId(pickedId);
          }
        } else if (!selectedId()) {
          const first = built.byId.keys().next().value;
          if (first) setSelectedId(first);
        }
        setStatus("Connected");
      },
      overridePath,
    );
  };

  const togglePicker = (): void => {
    requestInspector({ op: "pick", action: "toggle" }, data => {
      setSnapshot(prev => (prev ? { ...prev, picker: data.picker } : data));
      setStatus(data.picker?.active ? "Picker active" : "Connected");
    });
  };

  const selectRemoteNode = (selection: FlatSelection): void => {
    requestInspector(
      {
        op: "pick",
        action: "select",
        surface: selection.surface,
        token: selection.node.token,
      },
      data => {
        setSnapshot(prev => (prev ? { ...prev, picker: data.picker } : data));
      },
    );
  };

  createEffect(
    () => null,
    () => {
      const resolved = resolveSocket();
      setSocketPath(resolved);
      if (resolved) {
        refreshSnapshot(resolved);
      } else {
        setStatus("No socket bound");
        setErrorMessage(
          "Start target app with --dev-inspect <socket> and launch zgui-js inspect --ui --socket <socket>.",
        );
      }
    },
  );

  const treeModel = createMemo(() => buildSurfaceTree(snapshot()?.surfaces ?? []));

  const selected = createMemo<FlatSelection | null>(() => {
    const id = selectedId();
    if (!id) return null;
    return treeModel().byId.get(id) ?? null;
  });

  const latestFrame = createMemo(() => {
    const frames = snapshot()?.profiler?.frames;
    if (frames && frames.length > 0) {
      const last = frames[frames.length - 1];
      if (last) {
        return {
          styleUs: last.styleUs ?? 0,
          layoutUs: last.layoutUs ?? 0,
          paintUs: last.paintUs ?? 0,
          totalUs: last.totalUs ?? 0,
        };
      }
    }
    const firstSurface = snapshot()?.surfaces?.[0];
    return firstSurface?.frame?.phases ?? null;
  });

  const pickerActive = () => Boolean(snapshot()?.picker?.active);

  return (
    <ThemeProvider scheme="dark">
      <box class={styles.shell} testId="inspector-shell">
        <box class={styles.toolbar} testId="inspector-toolbar">
          <box class={styles.toolbarGroup}>
            <text class={styles.title} testId="inspector-title">
              zgui-js Inspector
            </text>
            <Badge variant={pickerActive() ? "default" : "secondary"} testId="inspector-status">
              {status()}
            </Badge>
          </box>
          <box class={styles.toolbarGroup}>
            <Button
              size="sm"
              variant={pickerActive() ? "default" : "outline"}
              testId="inspector-pick-toggle"
              onClick={togglePicker}
            >
              {pickerActive() ? "Cancel Pick" : "Pick Element"}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              testId="inspector-refresh"
              disabled={busy()}
              onClick={() => refreshSnapshot()}
            >
              Refresh
            </Button>
          </box>
        </box>

        <Show when={errorMessage()}>
          {message => (
            <box class={styles.errorBanner} testId="inspector-error">
              <text>{message()}</text>
            </box>
          )}
        </Show>

        <SplitPane class={styles.workspace} direction="horizontal" defaultSizes={[44, 56]}>
          <SplitPanePanel minSize={24}>
            <box class={styles.pane} testId="inspector-tree-pane">
              <box class={styles.paneHeader}>
                <text class={styles.sectionTitle}>NATIVE TREE</text>
                <text class={styles.muted} testId="inspector-node-count">
                  {`${treeModel().byId.size} nodes`}
                </text>
              </box>
              <scroll class={styles.scrollArea}>
                <TreeView
                  testId="inspector-tree"
                  items={treeModel().items}
                  selectedId={selectedId()}
                  expandedIds={expandedIds()}
                  onExpandedChange={ids => setExpandedIds(ids)}
                  onSelect={(id, item) => {
                    setSelectedId(id);
                    if (item?.meta) {
                      selectRemoteNode(item.meta);
                    }
                  }}
                  emptyText="No surfaces captured"
                />
              </scroll>
            </box>
          </SplitPanePanel>

          <SplitPaneHandle withHandle />

          <SplitPanePanel minSize={30}>
            <scroll class={styles.scrollArea}>
              <box class={styles.pane} testId="inspector-details-pane">
                <text class={styles.sectionTitle}>SELECTED NODE</text>
                <Show
                  when={selected()}
                  fallback={
                    <box class={styles.card}>
                      <text class={styles.muted}>
                        Select a node from the tree or use Pick Element.
                      </text>
                    </box>
                  }
                >
                  {sel => {
                    const node = () => sel().node;
                    const boxModel = () => node().boxModel ?? null;
                    const computed = () => node().computedStyle ?? null;
                    return (
                      <box class={styles.card} testId="inspector-node-card">
                        <box class={styles.row}>
                          <text class={styles.mono} testId="inspector-node-kind">
                            {`<${node().kind}> (${node().token})`}
                          </text>
                          <Show when={node().role}>
                            {role => <Badge variant="secondary">{role()}</Badge>}
                          </Show>
                        </box>

                        <Show when={node().source}>
                          {src => (
                            <box class={styles.row}>
                              <text class={styles.muted}>Source</text>
                              <text class={styles.source} testId="inspector-node-source">
                                {src()}
                              </text>
                            </box>
                          )}
                        </Show>

                        <Show when={(node().classes?.length ?? 0) > 0}>
                          <box class={styles.row}>
                            <text class={styles.muted}>Classes</text>
                            <text class={styles.mono} testId="inspector-node-classes">
                              {(node().classes ?? []).map(c => `.${c}`).join(" ")}
                            </text>
                          </box>
                        </Show>

                        <Show when={node().redacted}>
                          <text class={styles.redacted} testId="inspector-node-redacted">
                            [REDACTED — secure field text hidden]
                          </text>
                        </Show>

                        <Show when={boxModel()}>
                          {bm => (
                            <box class={styles.boxDiagram} testId="inspector-box-model">
                              <text class={styles.sectionTitle}>BOX MODEL (CSS PX)</text>
                              <box class={styles.row}>
                                <text class={styles.muted}>Border box</text>
                                <text class={styles.mono}>{formatBounds(bm().borderBox)}</text>
                              </box>
                              <box class={styles.row}>
                                <text class={styles.muted}>Content box</text>
                                <text class={styles.mono}>{formatBounds(bm().contentBox)}</text>
                              </box>
                              <box class={styles.row}>
                                <text class={styles.muted}>Padding</text>
                                <text class={styles.mono}>{formatEdges(bm().padding)}</text>
                              </box>
                              <box class={styles.row}>
                                <text class={styles.muted}>Border</text>
                                <text class={styles.mono}>{formatEdges(bm().border)}</text>
                              </box>
                              <box class={styles.row}>
                                <text class={styles.muted}>Margin</text>
                                <text class={styles.mono}>{formatEdges(bm().margin)}</text>
                              </box>
                            </box>
                          )}
                        </Show>

                        <Show when={computed()}>
                          {style => (
                            <box class={styles.boxDiagram} testId="inspector-computed-style">
                              <text class={styles.sectionTitle}>COMPUTED STYLE</text>
                              <box class={styles.row}>
                                <text class={styles.muted}>display / position</text>
                                <text class={styles.mono}>
                                  {`${style().display} / ${style().position}`}
                                </text>
                              </box>
                              <box class={styles.row}>
                                <text class={styles.muted}>visibility / opacity</text>
                                <text class={styles.mono}>
                                  {`${style().visibility} / ${style().opacity}`}
                                </text>
                              </box>
                              <box class={styles.row}>
                                <text class={styles.muted}>font-size</text>
                                <text
                                  class={styles.mono}
                                >{`${Math.round(style().fontSize)}px`}</text>
                              </box>
                            </box>
                          )}
                        </Show>
                      </box>
                    );
                  }}
                </Show>

                <Separator />

                <text class={styles.sectionTitle}>FRAME TIMINGS &amp; RESOURCES</text>
                <box class={styles.card} testId="inspector-profiler-card">
                  <Show
                    when={latestFrame()}
                    fallback={<text class={styles.muted}>No frame phase timings recorded.</text>}
                  >
                    {frame => (
                      <box>
                        <box class={styles.row}>
                          <text class={styles.muted}>Style / Layout / Paint</text>
                          <text class={styles.mono} testId="inspector-frame-phases">
                            {`${frame().styleUs}us / ${frame().layoutUs}us / ${frame().paintUs}us`}
                          </text>
                        </box>
                        <box class={styles.row}>
                          <text class={styles.muted}>Frame total</text>
                          <text class={styles.mono} testId="inspector-frame-total">
                            {`${frame().totalUs}us`}
                          </text>
                        </box>
                      </box>
                    )}
                  </Show>
                  <box class={styles.row}>
                    <text class={styles.muted}>Generation / Live nodes</text>
                    <text class={styles.mono} testId="inspector-session-stats">
                      {`gen ${snapshot()?.session?.generation ?? 0} | ${snapshot()?.session?.resources?.nodes ?? 0} nodes`}
                    </text>
                  </box>
                </box>
              </box>
            </scroll>
          </SplitPanePanel>
        </SplitPane>
      </box>
    </ThemeProvider>
  );
}

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <InspectorApp />, root);
}
