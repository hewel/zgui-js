import { For, Show, createDelay, createMemo, createSignal, observeGeometry } from "@hewel/zgui";
import type { NativeNode, NativeScrollPosition, NativeValueEvent } from "@hewel/zgui";
import {
  Badge,
  Button,
  Progress,
  Separator,
  Slider,
  SplitPane,
  SplitPaneHandle,
  SplitPanePanel,
  Textarea,
  TreeView,
  toast,
} from "@hewel/zgui/components";
import { useBeforeLeave, useNavigate } from "@hewel/zgui/router";
import { MarkdownPreview } from "../preview.js";
import { useStudio } from "../studio-context.js";

export function EditorView() {
  const studio = useStudio();
  const navigate = useNavigate();
  const [scrollEl, setScrollEl] = createSignal<NativeNode | null>(null);
  const [scrollInfo, setScrollInfo] = createSignal<NativeScrollPosition | null>(null);
  const [previewReady, setPreviewReady] = createSignal(false);
  const previewTimer = createDelay();
  previewTimer.schedule(0, () => setPreviewReady(true));
  let bypassLeaveOnce = false;

  observeGeometry(scrollEl, "scrollPosition", setScrollInfo);

  useBeforeLeave(event => {
    if (bypassLeaveOnce) {
      bypassLeaveOnce = false;
      studio.setPendingLeavePath(null);
      return;
    }
    if (studio.guardNavigation() && studio.activeDoc().dirty === true) {
      event.preventDefault();
      studio.setPendingLeavePath(event.to.pathname);
      toast.warning("Unsaved edits in current document", {
        id: "unsaved-nav-guard",
        description: `Save before leaving for ${event.to.pathname}, or discard navigation guard.`,
        action: {
          label: "Leave Anyway",
          onClick: () => {
            bypassLeaveOnce = true;
            const target = studio.pendingLeavePath();
            studio.setPendingLeavePath(null);
            if (target) void navigate(target);
          },
        },
      });
    }
  });

  const taskPct = createMemo(() => {
    const p = studio.parsedResult().parsed;
    return p.taskTotal > 0 ? Math.round((p.taskDone / p.taskTotal) * 100) : 0;
  });

  const outlineExpanded = createMemo(() => studio.outlineTree().map(n => n.id));

  const effectiveSplitSizes = createMemo<readonly number[]>(() => {
    const saved = studio.splitSizes();
    const sidebar = Math.max(0, Math.min(38, saved[0] ?? 22));
    const rest = Math.max(0, 100 - sidebar);
    const mode = studio.viewMode();
    if (mode === "preview") return [sidebar, 0, rest];
    if (mode === "editor") return [sidebar, rest, 0];
    return saved;
  });

  const handleSplitSizesChange = (next: readonly number[]) => {
    if (studio.viewMode() === "split") {
      studio.setSplitSizes(next);
      return;
    }
    const prev = studio.splitSizes();
    const nextSidebar = Math.max(0, Math.min(38, next[0] ?? prev[0] ?? 22));
    const prevMain = (prev[1] ?? 39) + (prev[2] ?? 39);
    const editorShare = prevMain > 0 ? (prev[1] ?? 39) / prevMain : 0.5;
    const nextMain = Math.max(0, 100 - nextSidebar);
    const nextEditor = Math.round(nextMain * editorShare * 10) / 10;
    const nextPreview = Math.max(0, Math.round((nextMain - nextEditor) * 10) / 10);
    studio.setSplitSizes([nextSidebar, nextEditor, nextPreview]);
  };

  const handleEditorInput = (event: NativeValueEvent) => {
    studio.setSelectionRange(event.selection);
    studio.updateActiveContent(event.value);
  };

  return (
    <column class="mde-view" testId="view-editor">
      <row class="mde-toolbar">
        <row style={{ gap: "6px", "align-items": "center", "flex-wrap": "wrap" }}>
          <text class="muted">Format:</text>
          <Button
            testId="fmt-bold"
            size="xs"
            variant="outline"
            onClick={() => studio.wrapSelection("**", "**", "bold")}
          >
            B
          </Button>
          <Button
            testId="fmt-italic"
            size="xs"
            variant="outline"
            onClick={() => studio.wrapSelection("*", "*", "italic")}
          >
            I
          </Button>
          <Button
            testId="fmt-strike"
            size="xs"
            variant="outline"
            onClick={() => studio.wrapSelection("~~", "~~", "strike")}
          >
            S
          </Button>
          <Button
            testId="fmt-code"
            size="xs"
            variant="outline"
            onClick={() => studio.wrapSelection("`", "`", "code")}
          >
            Code
          </Button>
          <Button
            testId="fmt-link"
            size="xs"
            variant="outline"
            onClick={() => studio.wrapSelection("[", "](https://zgui.dev)", "link")}
          >
            Link
          </Button>
          <Separator orientation="vertical" />
          <Button
            testId="ins-h2"
            size="xs"
            variant="outline"
            onClick={() => studio.appendBlock("## New Section\n\nParagraph text here.", "Heading")}
          >
            H2
          </Button>
          <Button
            testId="ins-task"
            size="xs"
            variant="outline"
            onClick={() => studio.appendBlock("- [ ] New interactive task item", "Task")}
          >
            Task
          </Button>
          <Button
            testId="ins-code"
            size="xs"
            variant="outline"
            onClick={() =>
              studio.appendBlock('```ts\nconst greeting: string = "Hello zgui";\n```', "Code Block")
            }
          >
            Code Block
          </Button>
          <Separator orientation="vertical" />
          <Button
            testId="btn-paste-image"
            size="xs"
            variant="secondary"
            onClick={() => void studio.pasteClipboardImage()}
          >
            Paste Image
          </Button>
          <Button testId="btn-stress-doc" size="xs" variant="ghost" onClick={studio.loadStressDoc}>
            120-Block Stress
          </Button>
        </row>

        <row style={{ gap: "8px", "align-items": "center" }}>
          <Button
            testId="mode-split"
            size="xs"
            variant={studio.viewMode() === "split" ? "default" : "outline"}
            onClick={() => studio.setViewMode("split")}
          >
            Split
          </Button>
          <Button
            testId="mode-editor"
            size="xs"
            variant={studio.viewMode() === "editor" ? "default" : "outline"}
            onClick={() => studio.setViewMode("editor")}
          >
            Editor
          </Button>
          <Button
            testId="mode-preview"
            size="xs"
            variant={studio.viewMode() === "preview" ? "default" : "outline"}
            onClick={() => studio.setViewMode("preview")}
          >
            Preview
          </Button>
          <Separator orientation="vertical" />
          <text class="muted mono">{`Size: ${studio.fontSize()}px`}</text>
          <box style={{ width: "92px" }}>
            <Slider
              value={studio.fontSize()}
              onValueChange={studio.setFontSize}
              min={13}
              max={20}
              step={1}
              label="Preview font size"
            />
          </box>
        </row>
      </row>

      <Show when={studio.pendingLeavePath()}>
        {target => (
          <row class="mde-guard-banner" testId="nav-guard-banner">
            <text>{`Unsaved edits blocked leaving for ${target()}.`}</text>
            <row style={{ gap: "6px" }}>
              <Button
                testId="btn-guard-save-leave"
                size="xs"
                onClick={() => {
                  void studio.saveActiveDocument().then(saved => {
                    if (saved) {
                      const dest = studio.pendingLeavePath();
                      studio.setPendingLeavePath(null);
                      if (dest) void navigate(dest);
                    }
                  });
                }}
              >
                Save & Leave
              </Button>
              <Button
                testId="btn-guard-force-leave"
                size="xs"
                variant="outline"
                onClick={() => {
                  bypassLeaveOnce = true;
                  const dest = studio.pendingLeavePath();
                  studio.setPendingLeavePath(null);
                  if (dest) void navigate(dest);
                }}
              >
                Leave Anyway
              </Button>
              <Button
                testId="btn-guard-cancel"
                size="xs"
                variant="ghost"
                onClick={() => studio.setPendingLeavePath(null)}
              >
                Stay
              </Button>
            </row>
          </row>
        )}
      </Show>

      <box class="mde-body">
        <SplitPane
          testId="studio-split-pane"
          sizes={effectiveSplitSizes()}
          onSizesChange={handleSplitSizesChange}
          label="Markdown Studio workspace split"
        >
          <SplitPanePanel
            testId="studio-sidebar-panel"
            minSize={15}
            maxSize={38}
            collapsible
            collapsedSize={0}
          >
            <scroll class="mde-sidebar" testId="studio-sidebar">
              <Show when={previewReady()}>
                <column style={{ gap: "12px" }}>
                  <column style={{ gap: "6px" }}>
                    <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                      <text class="muted" style={{ "font-weight": "600" }}>
                        {studio.workspaceName()
                          ? `Workspace: ${studio.workspaceName()}`
                          : "Workspace"}
                      </text>
                      <Button
                        testId="btn-open-folder-side"
                        size="xs"
                        variant="outline"
                        onClick={() => void studio.openWorkspaceFolder()}
                      >
                        Folder
                      </Button>
                    </row>
                    <Show
                      when={studio.workspaceTree().length > 0}
                      fallback={
                        <text class="muted" testId="workspace-empty-hint">
                          No folder opened. Click Folder to mount a local workspace.
                        </text>
                      }
                    >
                      <TreeView
                        testId="workspace-tree"
                        items={studio.workspaceTree()}
                        selectedId={studio.activeDocId()}
                        expandedIds={studio.workspaceExpanded()}
                        onExpandedChange={studio.setWorkspaceExpanded}
                        onActivate={(_, node) => {
                          if (node?.meta?.kind === "file" && node.meta.docId) {
                            studio.setActiveDocId(node.meta.docId);
                            studio.setStatusMsg(`Opened workspace file ${node.meta.relativePath}`);
                          }
                        }}
                        label="Workspace files"
                      />
                    </Show>
                  </column>

                  <Separator />

                  <column style={{ gap: "4px" }}>
                    <text class="muted" style={{ "font-weight": "600" }}>
                      {`Outline (${studio.parsedResult().parsed.headings.length})`}
                    </text>
                    <TreeView
                      testId="outline-tree"
                      items={studio.outlineTree()}
                      expandedIds={outlineExpanded()}
                      onActivate={(_, node) => {
                        if (node?.meta) {
                          studio.jumpToHeading(node.meta.lineIndex, node.meta.title);
                        }
                      }}
                      emptyText="No headings"
                      label="Document outline"
                    />
                  </column>

                  <Separator />

                  <Show when={studio.parsedResult().parsed.taskTotal > 0}>
                    <column style={{ gap: "6px" }}>
                      <row style={{ "justify-content": "space-between" }}>
                        <text class="muted" style={{ "font-weight": "600" }}>
                          Task Progress
                        </text>
                        <text class="muted mono">
                          {`${studio.parsedResult().parsed.taskDone}/${studio.parsedResult().parsed.taskTotal}`}
                        </text>
                      </row>
                      <Progress value={taskPct()} max={100} label="Markdown tasks progress" />
                    </column>
                    <Separator />
                  </Show>

                  <column style={{ gap: "6px" }}>
                    <text class="muted" style={{ "font-weight": "600" }}>
                      {`Samples & Files (${studio.docs().filter(d => d.source !== "workspace").length})`}
                    </text>
                    <For each={studio.docs().filter(d => d.source !== "workspace")}>
                      {doc => (
                        <box
                          testId={`doc-${doc.id}`}
                          class="mde-doc-btn"
                          role="button"
                          tabIndex={0}
                          data-active={studio.activeDoc().id === doc.id ? "true" : "false"}
                          onClick={() => {
                            studio.setActiveDocId(doc.id);
                            studio.setStatusMsg(`Switched to "${doc.title}"`);
                          }}
                        >
                          <text>{`${doc.title}${doc.dirty ? " *" : ""}`}</text>
                          <Badge variant="secondary">{doc.badge}</Badge>
                        </box>
                      )}
                    </For>
                  </column>
                </column>
              </Show>
            </scroll>
          </SplitPanePanel>

          <SplitPaneHandle testId="split-handle-sidebar" withHandle label="Resize sidebar" />

          <SplitPanePanel
            testId="studio-editor-panel"
            minSize={studio.viewMode() === "preview" ? 0 : 20}
            collapsible
            collapsedSize={0}
            collapsed={studio.viewMode() === "preview"}
          >
            <Show when={studio.viewMode() !== "preview"}>
              <column class="mde-editor-pane">
                <row class="mde-pane-head">
                  <text style={{ "font-weight": "600" }}>MARKDOWN SOURCE</text>
                  <text class="mono" testId="selection-readout">
                    {`sel ${studio.selectionRange().start}:${studio.selectionRange().end}`}
                  </text>
                </row>
                <column class="mde-textarea-wrap">
                  <Textarea
                    ref={studio.setEditorEl}
                    testId="md-source-editor"
                    class="mde-textarea"
                    label="Markdown source editor"
                    placeholder="Type Markdown here..."
                    value={studio.activeDoc().content}
                    onInput={handleEditorInput}
                  />
                </column>
              </column>
            </Show>
          </SplitPanePanel>

          <SplitPaneHandle
            testId="split-handle-preview"
            withHandle
            disabled={studio.viewMode() !== "split"}
            style={studio.viewMode() !== "split" ? { display: "none" } : undefined}
            label="Resize editor and preview"
          />

          <SplitPanePanel
            testId="studio-preview-panel"
            minSize={studio.viewMode() === "editor" ? 0 : 20}
            collapsible
            collapsedSize={0}
            collapsed={studio.viewMode() === "editor"}
          >
            <Show when={studio.viewMode() !== "editor"}>
              <column class="mde-preview-pane">
                <row class="mde-pane-head">
                  <text style={{ "font-weight": "600" }}>LIVE NATIVE PREVIEW</text>
                  <text class="mono" testId="preview-scroll-readout">
                    {scrollInfo()
                      ? `scrollY=${Math.round(scrollInfo()!.offset.y)} / ${Math.round(scrollInfo()!.contentSize.height)}`
                      : "scroll ready"}
                  </text>
                </row>
                <scroll
                  class="mde-preview-scroll"
                  ref={node => setScrollEl(node)}
                  testId="md-preview-scroll"
                >
                  <Show when={previewReady()}>
                    <MarkdownPreview
                      blocks={studio.parsedResult().parsed.blocks}
                      fontSize={studio.fontSize}
                      pipelineImg={studio.pipelineImg}
                      pastedImg={studio.pastedImg}
                      highlightedLine={studio.highlightedLine}
                      onToggleTask={studio.toggleTask}
                      onLinkClick={(href, label) =>
                        studio.setStatusMsg(`Link: "${label}" -> ${href}`)
                      }
                      onCopyCode={(code, lang) => void studio.copyCodeBlock(code, lang)}
                      onClearPastedImg={studio.clearPastedImage}
                      onHeadingNode={studio.registerHeadingNode}
                    />
                  </Show>
                </scroll>
              </column>
            </Show>
          </SplitPanePanel>
        </SplitPane>
      </box>
    </column>
  );
}
