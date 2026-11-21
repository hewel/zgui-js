import { For, Image, Show } from "@hewel/zgui";
import { Badge, Checkbox } from "@hewel/zgui/components";
import type { BlockNode, CalloutKind, InlineNode } from "./markdown.js";

const CALLOUT_LABELS: Record<CalloutKind, string> = {
  NOTE: "Note",
  TIP: "Tip",
  IMPORTANT: "Important",
  WARNING: "Warning",
  CAUTION: "Caution",
};

function InlineFlow(props: {
  inlines: readonly InlineNode[];
  onLinkClick: (href: string, label: string) => void;
}) {
  return (
    <For each={props.inlines}>
      {node => {
        if (node.kind === "bold") {
          return <text class="md-inline md-bold">{node.text}</text>;
        }
        if (node.kind === "italic") {
          return <text class="md-inline md-italic">{node.text}</text>;
        }
        if (node.kind === "strike") {
          return <text class="md-inline md-strike">{node.text}</text>;
        }
        if (node.kind === "code") {
          return <text class="md-inline md-code-inline">{node.text}</text>;
        }
        if (node.kind === "link") {
          return (
            <text
              class="md-inline md-link"
              role="link"
              tabIndex={0}
              onClick={() => props.onLinkClick(node.href, node.label)}
            >
              {node.label}
            </text>
          );
        }
        return <text class="md-inline">{node.text}</text>;
      }}
    </For>
  );
}

export function MarkdownPreview(props: {
  blocks: readonly BlockNode[];
  fontSize: () => number;
  pipelineImg: () => string | null;
  highlightedLine: () => number | null;
  onToggleTask: (lineIndex: number) => void;
  onLinkClick: (href: string, label: string) => void;
}) {
  return (
    <column
      class="md-article"
      testId="md-preview-article"
      style={{ "font-size": `${props.fontSize()}px` }}
    >
      <For each={props.blocks}>
        {block => {
          if (block.kind === "heading") {
            return (
              <box
                class={`md-h md-h${block.level}`}
                data-highlighted={props.highlightedLine() === block.lineIndex ? "true" : "false"}
              >
                <InlineFlow inlines={block.inlines} onLinkClick={props.onLinkClick} />
              </box>
            );
          }

          if (block.kind === "paragraph") {
            return (
              <box class="md-p">
                <InlineFlow inlines={block.inlines} onLinkClick={props.onLinkClick} />
              </box>
            );
          }

          if (block.kind === "hr") {
            return <box class="md-hr" />;
          }

          if (block.kind === "blockquote") {
            return (
              <box class="md-quote">
                <InlineFlow inlines={block.inlines} onLinkClick={props.onLinkClick} />
              </box>
            );
          }

          if (block.kind === "callout") {
            return (
              <column class={`md-callout md-callout--${block.callout.toLowerCase()}`}>
                <text class={`md-callout-title md-callout-title--${block.callout.toLowerCase()}`}>
                  {CALLOUT_LABELS[block.callout]}
                </text>
                <For each={block.lines}>
                  {line => (
                    <box class="md-callout-body">
                      <InlineFlow inlines={line} onLinkClick={props.onLinkClick} />
                    </box>
                  )}
                </For>
              </column>
            );
          }

          if (block.kind === "code") {
            return (
              <column class="md-code-block">
                <row class="md-code-header">
                  <Badge variant="secondary">{block.lang}</Badge>
                  <text class="muted mono">{`${block.lines.length} lines`}</text>
                </row>
                <column class="md-code-body">
                  <For each={block.lines}>
                    {(tokens, idx) => (
                      <row class="md-code-line">
                        <text class="md-code-ln">{String(idx() + 1)}</text>
                        <box class="md-code-tokens">
                          <For each={tokens}>
                            {tok => <text class={`md-tok md-tok--${tok.kind}`}>{tok.text}</text>}
                          </For>
                        </box>
                      </row>
                    )}
                  </For>
                </column>
              </column>
            );
          }

          if (block.kind === "list") {
            return (
              <column class="md-list">
                <For each={block.items}>
                  {(item, idx) => (
                    <row class="md-list-item">
                      <Show
                        when={item.checked !== null}
                        fallback={
                          <text class="md-list-marker">
                            {block.ordered ? `${idx() + 1}.` : "•"}
                          </text>
                        }
                      >
                        <Checkbox
                          checked={item.checked === true}
                          onCheckedChange={() => props.onToggleTask(item.lineIndex)}
                          ariaLabel={`Toggle task at line ${item.lineIndex + 1}`}
                        />
                      </Show>
                      <box
                        class="md-list-content"
                        data-done={item.checked === true ? "true" : "false"}
                      >
                        <InlineFlow inlines={item.inlines} onLinkClick={props.onLinkClick} />
                      </box>
                    </row>
                  )}
                </For>
              </column>
            );
          }

          if (block.kind === "table") {
            const colTemplate = `repeat(${Math.max(1, block.headers.length)}, minmax(90px, 1fr))`;
            return (
              <column class="md-table">
                <box class="md-table-head" style={{ "grid-template-columns": colTemplate }}>
                  <For each={block.headers}>
                    {(cell, colIdx) => (
                      <box
                        class="md-table-cell md-table-th"
                        style={{ "text-align": block.alignments[colIdx()] ?? "left" }}
                      >
                        <InlineFlow inlines={cell} onLinkClick={props.onLinkClick} />
                      </box>
                    )}
                  </For>
                </box>
                <For each={block.rows}>
                  {(rowCells, rowIdx) => (
                    <box
                      class="md-table-row"
                      data-striped={rowIdx() % 2 === 1 ? "true" : "false"}
                      style={{ "grid-template-columns": colTemplate }}
                    >
                      <For each={rowCells}>
                        {(cell, colIdx) => (
                          <box
                            class="md-table-cell"
                            style={{ "text-align": block.alignments[colIdx()] ?? "left" }}
                          >
                            <InlineFlow inlines={cell} onLinkClick={props.onLinkClick} />
                          </box>
                        )}
                      </For>
                    </box>
                  )}
                </For>
              </column>
            );
          }

          // Image block
          return (
            <column class="md-figure">
              <Show
                when={block.src === "diagram://pipeline" && props.pipelineImg()}
                fallback={
                  <box class="md-image-fallback">
                    <text class="muted mono">{`[Image: ${block.alt || block.src}]`}</text>
                  </box>
                }
              >
                <Image src={props.pipelineImg()} class="md-diagram-img" />
              </Show>
              <Show when={block.alt}>
                <text class="md-figcaption">{block.alt}</text>
              </Show>
            </column>
          );
        }}
      </For>
    </column>
  );
}
