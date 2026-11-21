import {
  For,
  Show,
  createDelay,
  createEffect,
  createMemo,
  createSignal,
  observeGeometry,
  renderApp,
} from "@hewel/zgui";
import type { NativeNode, NativeScrollPosition, NativeValueEvent } from "@hewel/zgui";
import {
  Badge,
  Button,
  Progress,
  Separator,
  Slider,
  Textarea,
  ThemeProvider,
} from "@hewel/zgui/components";
import type { ColorScheme, ThemePreset } from "@hewel/zgui/components";
import { registerImageBytes } from "@hewel/zgui-services";
import { parseMarkdown, toggleTaskAtLine, utf8OffsetToUtf16 } from "./markdown.js";
import { MarkdownPreview } from "./preview.js";
import { INITIAL_DOCS, buildPipelineSvg, buildStressMarkdown, encodeUtf8Bytes } from "./samples.js";
import type { MarkdownDoc } from "./samples.js";

type ViewMode = "split" | "editor" | "preview";

const PRESETS: readonly ThemePreset[] = ["base", "ocean", "midnight", "ember"];

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <MarkdownStudioApp />, root);
}

function MarkdownStudioApp() {
  const [scheme, setScheme] = createSignal<ColorScheme>("dark");
  const [preset, setPreset] = createSignal<ThemePreset>("base");
  const [viewMode, setViewMode] = createSignal<ViewMode>("split");
  const [showSidebar, setShowSidebar] = createSignal(true);
  const [fontSize, setFontSize] = createSignal(15);

  const [docs, setDocs] = createSignal<readonly MarkdownDoc[]>(INITIAL_DOCS);
  const [activeDocId, setActiveDocId] = createSignal(INITIAL_DOCS[0]!.id);
  const [selectionBytes, setSelectionBytes] = createSignal<{ start: number; end: number }>({
    start: 0,
    end: 0,
  });
  const [highlightedLine, setHighlightedLine] = createSignal<number | null>(null);
  const [scrollEl, setScrollEl] = createSignal<NativeNode | null>(null);
  const [scrollInfo, setScrollInfo] = createSignal<NativeScrollPosition | null>(null);
  const [saveState, setSaveState] = createSignal<"saved" | "editing">("saved");
  const [statusMsg, setStatusMsg] = createSignal(
    "Ready — edit Markdown on the left or toggle task checkboxes in the live preview.",
  );

  observeGeometry(scrollEl, "scrollPosition", setScrollInfo);

  const saveTimer = createDelay();
  const highlightTimer = createDelay();

  const [pipelineImg, setPipelineImg] = createSignal<string | null>(null);
  createEffect(
    () => scheme() === "dark",
    isDark => {
      const svg = buildPipelineSvg(isDark);
      const handle = registerImageBytes(encodeUtf8Bytes(svg));
      setPipelineImg(handle.src);
      return () => {
        handle.release();
      };
    },
  );

  const activeDoc = createMemo(() => docs().find(d => d.id === activeDocId()) ?? docs()[0]!);

  const parsedResult = createMemo(() => {
    const t0 = Date.now();
    const parsed = parseMarkdown(activeDoc().content);
    const parseMs = Date.now() - t0;
    return { parsed, parseMs };
  });

  const stats = createMemo(() => {
    const text = activeDoc().content;
    const lines = text ? text.split("\n").length : 0;
    const words = text.trim() ? text.trim().split(/\s+/).length : 0;
    const readMin = Math.max(1, Math.ceil(words / 220));
    return { chars: text.length, words, lines, readMin };
  });

  const setDocContent = (nextContent: string, note?: string) => {
    const id = activeDoc().id;
    setDocs(prev => prev.map(d => (d.id === id ? { ...d, content: nextContent } : d)));
    setSaveState("editing");
    saveTimer.schedule(650, () => {
      setSaveState("saved");
      if (note) setStatusMsg(note);
    });
  };

  const handleEditorInput = (event: NativeValueEvent) => {
    setSelectionBytes(event.selection);
    setDocContent(event.value);
  };

  const wrapSelection = (before: string, after: string, fallbackText: string) => {
    const content = activeDoc().content;
    const sel = selectionBytes();
    const start = utf8OffsetToUtf16(content, sel.start);
    const end = utf8OffsetToUtf16(content, sel.end);
    const selected = content.slice(start, end) || fallbackText;
    const next = `${content.slice(0, start)}${before}${selected}${after}${content.slice(end)}`;
    setDocContent(next, `Inserted ${before.trim() || "snippet"}`);
    setStatusMsg(`Formatted selection (${start}:${end})`);
  };

  const appendBlock = (snippet: string, label: string) => {
    const content = activeDoc().content.trimEnd();
    setDocContent(`${content}\n\n${snippet}\n`, `Appended ${label}`);
    setStatusMsg(`Appended ${label}`);
  };

  const handleToggleTask = (lineIndex: number) => {
    const updated = toggleTaskAtLine(activeDoc().content, lineIndex);
    setDocContent(updated, `Toggled task at line ${lineIndex + 1}`);
    setStatusMsg(`Toggled task checkbox on line ${lineIndex + 1}`);
  };

  const handleLinkClick = (href: string, label: string) => {
    setStatusMsg(`Link clicked: "${label}" -> ${href}`);
  };

  const focusHeading = (lineIndex: number, title: string) => {
    setHighlightedLine(lineIndex);
    setStatusMsg(`Outline: "${title}" (line ${lineIndex + 1})`);
    highlightTimer.schedule(1800, () => setHighlightedLine(null));
  };

  const loadStressDoc = () => {
    const existing = docs().find(d => d.id === "stress");
    if (!existing) {
      const stressDoc: MarkdownDoc = {
        id: "stress",
        title: "120-Block Stress Doc",
        badge: "Stress",
        content: buildStressMarkdown(),
      };
      setDocs(prev => [...prev, stressDoc]);
    }
    setActiveDocId("stress");
    setStatusMsg("Loaded 120-block Markdown stress document");
  };

  const cyclePreset = () => {
    const idx = PRESETS.indexOf(preset());
    setPreset(PRESETS[(idx + 1) % PRESETS.length]!);
  };

  const taskPct = createMemo(() => {
    const p = parsedResult().parsed;
    return p.taskTotal > 0 ? Math.round((p.taskDone / p.taskTotal) * 100) : 0;
  });

  return (
    <ThemeProvider scheme={scheme()} light={{ preset: preset() }} dark={{ preset: preset() }}>
      <column class="mde-root" testId="md-editor-root">
        <row class="mde-topbar">
          <row style={{ gap: "10px", "align-items": "center" }}>
            <Button size="xs" variant="outline" onClick={() => setShowSidebar(v => !v)}>
              {showSidebar() ? "Hide Outline" : "Show Outline"}
            </Button>
            <text style={{ "font-weight": "700" }}>zgui Markdown Studio</text>
            <Badge variant={saveState() === "editing" ? "default" : "secondary"}>
              {saveState() === "editing" ? "Editing..." : "Saved"}
            </Badge>
            <Badge variant="outline">
              {`Parse: ${parsedResult().parseMs}ms · ${parsedResult().parsed.blocks.length} blocks`}
            </Badge>
          </row>

          <row style={{ gap: "6px", "align-items": "center" }}>
            <Button
              testId="mode-split"
              size="xs"
              variant={viewMode() === "split" ? "default" : "outline"}
              onClick={() => setViewMode("split")}
            >
              Split
            </Button>
            <Button
              testId="mode-editor"
              size="xs"
              variant={viewMode() === "editor" ? "default" : "outline"}
              onClick={() => setViewMode("editor")}
            >
              Editor
            </Button>
            <Button
              testId="mode-preview"
              size="xs"
              variant={viewMode() === "preview" ? "default" : "outline"}
              onClick={() => setViewMode("preview")}
            >
              Preview
            </Button>
            <Separator orientation="vertical" />
            <Button size="xs" variant="outline" onClick={loadStressDoc}>
              120-Block Stress
            </Button>
            <Button size="xs" variant="outline" onClick={cyclePreset}>
              {`Preset: ${preset()}`}
            </Button>
            <Button
              size="xs"
              variant="outline"
              onClick={() => setScheme(s => (s === "dark" ? "light" : "dark"))}
            >
              {scheme() === "dark" ? "Light" : "Dark"}
            </Button>
          </row>
        </row>

        <row class="mde-toolbar">
          <row style={{ gap: "6px", "align-items": "center", "flex-wrap": "wrap" }}>
            <text class="muted">Insert:</text>
            <Button size="xs" variant="outline" onClick={() => wrapSelection("**", "**", "bold")}>
              B
            </Button>
            <Button size="xs" variant="outline" onClick={() => wrapSelection("*", "*", "italic")}>
              I
            </Button>
            <Button size="xs" variant="outline" onClick={() => wrapSelection("~~", "~~", "strike")}>
              S
            </Button>
            <Button size="xs" variant="outline" onClick={() => wrapSelection("`", "`", "code")}>
              Code
            </Button>
            <Button
              size="xs"
              variant="outline"
              onClick={() => wrapSelection("[", "](https://zgui.dev)", "link")}
            >
              Link
            </Button>
            <Separator orientation="vertical" />
            <Button
              size="xs"
              variant="outline"
              onClick={() => appendBlock("## New Section\n\nParagraph text here.", "Heading")}
            >
              H2
            </Button>
            <Button
              size="xs"
              variant="outline"
              onClick={() => appendBlock("- [ ] New interactive task item", "Task")}
            >
              Task
            </Button>
            <Button
              size="xs"
              variant="outline"
              onClick={() =>
                appendBlock("> [!NOTE]\n> Callout note inserted from toolbar.", "Callout")
              }
            >
              Callout
            </Button>
            <Button
              size="xs"
              variant="outline"
              onClick={() =>
                appendBlock("| Column A | Column B |\n| :--- | ---: |\n| Value 1 | 42 |", "Table")
              }
            >
              Table
            </Button>
            <Button
              size="xs"
              variant="outline"
              onClick={() =>
                appendBlock('```ts\nconst greeting: string = "Hello zgui";\n```', "Code Block")
              }
            >
              Code Block
            </Button>
          </row>

          <row style={{ gap: "8px", "align-items": "center" }}>
            <text class="muted mono">{`Size: ${fontSize()}px`}</text>
            <box style={{ width: "110px" }}>
              <Slider
                value={fontSize()}
                onValueChange={setFontSize}
                min={13}
                max={19}
                step={1}
                label="Preview font size"
              />
            </box>
          </row>
        </row>

        <row class="mde-body">
          <Show when={showSidebar()}>
            <column class="mde-sidebar">
              <column style={{ gap: "6px" }}>
                <text class="muted" style={{ "font-weight": "600" }}>
                  Documents
                </text>
                <For each={docs()}>
                  {doc => (
                    <box
                      testId={`doc-${doc.id}`}
                      class="mde-doc-btn"
                      role="button"
                      tabIndex={0}
                      data-active={activeDoc().id === doc.id ? "true" : "false"}
                      onClick={() => {
                        setActiveDocId(doc.id);
                        setStatusMsg(`Opened "${doc.title}"`);
                      }}
                    >
                      <text>{doc.title}</text>
                      <Badge variant="secondary">{doc.badge}</Badge>
                    </box>
                  )}
                </For>
              </column>

              <Separator />

              <Show when={parsedResult().parsed.taskTotal > 0}>
                <column style={{ gap: "6px" }}>
                  <row style={{ "justify-content": "space-between" }}>
                    <text class="muted" style={{ "font-weight": "600" }}>
                      Task Progress
                    </text>
                    <text class="muted mono">
                      {`${parsedResult().parsed.taskDone}/${parsedResult().parsed.taskTotal}`}
                    </text>
                  </row>
                  <Progress value={taskPct()} max={100} label="Markdown tasks progress" />
                </column>
                <Separator />
              </Show>

              <column style={{ gap: "4px" }}>
                <text class="muted" style={{ "font-weight": "600" }}>
                  {`Outline (${parsedResult().parsed.headings.length})`}
                </text>
                <For each={parsedResult().parsed.headings}>
                  {h => (
                    <box
                      class="mde-toc-item"
                      role="button"
                      tabIndex={0}
                      style={{ "padding-left": `${(h.level - 1) * 10 + 8}px` }}
                      onClick={() => focusHeading(h.lineIndex, h.text)}
                    >
                      <text>{h.text}</text>
                    </box>
                  )}
                </For>
              </column>
            </column>
          </Show>

          <row class="mde-panes">
            <Show when={viewMode() !== "preview"}>
              <column class="mde-editor-pane">
                <row class="mde-pane-head">
                  <text style={{ "font-weight": "600" }}>MARKDOWN SOURCE</text>
                  <text class="mono">
                    {`sel ${selectionBytes().start}:${selectionBytes().end} (UTF-8)`}
                  </text>
                </row>
                <column class="mde-textarea-wrap">
                  <Textarea
                    testId="md-source-editor"
                    class="mde-textarea"
                    label="Markdown source editor"
                    placeholder="Type Markdown here..."
                    value={activeDoc().content}
                    onInput={handleEditorInput}
                  />
                </column>
              </column>
            </Show>

            <Show when={viewMode() !== "editor"}>
              <column class="mde-preview-pane">
                <row class="mde-pane-head">
                  <text style={{ "font-weight": "600" }}>LIVE NATIVE PREVIEW</text>
                  <text class="mono">
                    {scrollInfo()
                      ? `scrollY=${Math.round(scrollInfo()!.offset.y)} / ${Math.round(scrollInfo()!.contentSize.height)}`
                      : "scroll ready"}
                  </text>
                </row>
                <scroll
                  class="mde-preview-scroll"
                  ref={node => {
                    setScrollEl(node);
                  }}
                  testId="md-preview-scroll"
                >
                  <MarkdownPreview
                    blocks={parsedResult().parsed.blocks}
                    fontSize={fontSize}
                    pipelineImg={pipelineImg}
                    highlightedLine={highlightedLine}
                    onToggleTask={handleToggleTask}
                    onLinkClick={handleLinkClick}
                  />
                </scroll>
              </column>
            </Show>
          </row>
        </row>

        <row class="mde-statusbar">
          <text>{statusMsg()}</text>
          <text class="mono">
            {`${stats().words} words · ${stats().lines} lines · ${stats().chars} chars · ~${stats().readMin} min read`}
          </text>
        </row>
      </column>
    </ThemeProvider>
  );
}
