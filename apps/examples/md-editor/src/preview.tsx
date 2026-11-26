import { For, Image, Show } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { Badge, Button, Checkbox } from "@hewel/zgui/components";
import type { BlockNode, CalloutKind, InlineNode } from "./markdown.js";

const CALLOUT_LABELS: Record<CalloutKind, string> = {
  NOTE: "Note",
  TIP: "Tip",
  IMPORTANT: "Important",
  WARNING: "Warning",
  CAUTION: "Caution",
};

function renderInlines(
  inlines: readonly InlineNode[],
  onLinkClick: (href: string, label: string) => void,
) {
  if (inlines.length === 1 && inlines[0]!.kind === "text") {
    return inlines[0]!.text;
  }
  return inlines.map(node => {
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
          onClick={() => onLinkClick(node.href, node.label)}
        >
          {node.label}
        </text>
      );
    }
    return node.text;
  });
}

export function MarkdownPreview(props: {
  blocks: readonly BlockNode[];
  fontSize: () => number;
  pipelineImg: () => string | null;
  pastedImg: () => string | null;
  highlightedLine: () => number | null;
  onToggleTask: (lineIndex: number) => void;
  onLinkClick: (href: string, label: string) => void;
  onCopyCode: (code: string, lang: string) => void;
  onClearPastedImg: () => void;
  onHeadingNode: (lineIndex: number, node: NativeNode) => void;
}) {
  return (
    <column
      class="md-article"
      testId="md-preview-article"
      style={{ "font-size": `${props.fontSize()}px` }}
    >
      <Show when={props.pastedImg()}>
        {src => (
          <column class="md-figure md-pasted-card" testId="pasted-image-card">
            <row
              style={{ width: "100%", "justify-content": "space-between", "align-items": "center" }}
            >
              <Badge variant="secondary">Clipboard Image Preview</Badge>
              <Button
                testId="btn-clear-pasted-img"
                size="xs"
                variant="ghost"
                onClick={props.onClearPastedImg}
              >
                Dismiss Image
              </Button>
            </row>
            <Image testId="pasted-image-el" src={src()} class="md-diagram-img" />
          </column>
        )}
      </Show>

      <For each={props.blocks}>
        {(block, blockIdx) => {
          if (block.kind === "heading") {
            return (
              <box
                ref={node => props.onHeadingNode(block.lineIndex, node)}
                testId={`preview-heading-${block.lineIndex}`}
                class={`md-h md-h${block.level}`}
                data-highlighted={props.highlightedLine() === block.lineIndex ? "true" : "false"}
              >
                {renderInlines(block.inlines, props.onLinkClick)}
              </box>
            );
          }

          if (block.kind === "paragraph") {
            return <box class="md-p">{renderInlines(block.inlines, props.onLinkClick)}</box>;
          }

          if (block.kind === "hr") {
            return <box class="md-hr" />;
          }

          if (block.kind === "blockquote") {
            return <box class="md-quote">{renderInlines(block.inlines, props.onLinkClick)}</box>;
          }

          if (block.kind === "callout") {
            return (
              <column class={`md-callout md-callout--${block.callout.toLowerCase()}`}>
                <text class={`md-callout-title md-callout-title--${block.callout.toLowerCase()}`}>
                  {CALLOUT_LABELS[block.callout]}
                </text>
                {block.lines.map(line => (
                  <box class="md-callout-body">{renderInlines(line, props.onLinkClick)}</box>
                ))}
              </column>
            );
          }

          if (block.kind === "code") {
            return (
              <column class="md-code-block">
                <row class="md-code-header">
                  <row style={{ gap: "8px", "align-items": "center" }}>
                    <Badge variant="secondary">{block.lang}</Badge>
                    <text class="muted mono">{`${block.lines.length} lines`}</text>
                  </row>
                  <Button
                    testId={`copy-code-${blockIdx()}`}
                    size="xs"
                    variant="outline"
                    onClick={() => props.onCopyCode(block.code, block.lang)}
                  >
                    Copy
                  </Button>
                </row>
                <column class="md-code-body">
                  {block.lines.map((tokens, idx) => (
                    <row class="md-code-line">
                      <text class="md-code-ln">{String(idx + 1)}</text>
                      <box class="md-code-tokens">
                        {tokens.map(tok => (
                          <text class={`md-tok md-tok--${tok.kind}`}>{tok.text}</text>
                        ))}
                      </box>
                    </row>
                  ))}
                </column>
              </column>
            );
          }

          if (block.kind === "list") {
            return (
              <column class="md-list">
                {block.items.map((item, idx) => (
                  <row class="md-list-item">
                    {item.checked !== null ? (
                      <Checkbox
                        checked={item.checked === true}
                        onCheckedChange={() => props.onToggleTask(item.lineIndex)}
                        ariaLabel={`Toggle task at line ${item.lineIndex + 1}`}
                      />
                    ) : (
                      <text class="md-list-marker">{block.ordered ? `${idx + 1}.` : "•"}</text>
                    )}
                    <box
                      class="md-list-content"
                      data-done={item.checked === true ? "true" : "false"}
                    >
                      {renderInlines(item.inlines, props.onLinkClick)}
                    </box>
                  </row>
                ))}
              </column>
            );
          }

          if (block.kind === "table") {
            const colTemplate = `repeat(${Math.max(1, block.headers.length)}, minmax(90px, 1fr))`;
            return (
              <column class="md-table">
                <box class="md-table-head" style={{ "grid-template-columns": colTemplate }}>
                  {block.headers.map((cell, colIdx) => (
                    <box
                      class="md-table-cell md-table-th"
                      style={{ "text-align": block.alignments[colIdx] ?? "left" }}
                    >
                      {renderInlines(cell, props.onLinkClick)}
                    </box>
                  ))}
                </box>
                {block.rows.map((rowCells, rowIdx) => (
                  <box
                    class="md-table-row"
                    data-striped={rowIdx % 2 === 1 ? "true" : "false"}
                    style={{ "grid-template-columns": colTemplate }}
                  >
                    {rowCells.map((cell, colIdx) => (
                      <box
                        class="md-table-cell"
                        style={{ "text-align": block.alignments[colIdx] ?? "left" }}
                      >
                        {renderInlines(cell, props.onLinkClick)}
                      </box>
                    ))}
                  </box>
                ))}
              </column>
            );
          }

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
                <box class="md-diagram-frame">
                  <Image
                    testId="pipeline-diagram-el"
                    src={props.pipelineImg()}
                    class="md-diagram-svg"
                  />
                </box>
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
