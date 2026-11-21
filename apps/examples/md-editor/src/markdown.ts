export type InlineNode =
  | { readonly kind: "text"; readonly text: string }
  | { readonly kind: "bold"; readonly text: string }
  | { readonly kind: "italic"; readonly text: string }
  | { readonly kind: "strike"; readonly text: string }
  | { readonly kind: "code"; readonly text: string }
  | { readonly kind: "link"; readonly label: string; readonly href: string };

export type CalloutKind = "NOTE" | "TIP" | "IMPORTANT" | "WARNING" | "CAUTION";
export type TableAlign = "left" | "center" | "right";

export interface ListItem {
  readonly inlines: readonly InlineNode[];
  readonly checked: boolean | null;
  readonly lineIndex: number;
}

export interface CodeToken {
  readonly kind: "plain" | "keyword" | "string" | "comment" | "number" | "type";
  readonly text: string;
}

export type BlockNode =
  | {
      readonly kind: "heading";
      readonly level: 1 | 2 | 3 | 4 | 5 | 6;
      readonly text: string;
      readonly inlines: readonly InlineNode[];
      readonly lineIndex: number;
    }
  | { readonly kind: "paragraph"; readonly inlines: readonly InlineNode[] }
  | {
      readonly kind: "code";
      readonly lang: string;
      readonly code: string;
      readonly lines: readonly (readonly CodeToken[])[];
    }
  | { readonly kind: "blockquote"; readonly inlines: readonly InlineNode[] }
  | {
      readonly kind: "callout";
      readonly callout: CalloutKind;
      readonly lines: readonly (readonly InlineNode[])[];
    }
  | { readonly kind: "list"; readonly ordered: boolean; readonly items: readonly ListItem[] }
  | {
      readonly kind: "table";
      readonly headers: readonly (readonly InlineNode[])[];
      readonly alignments: readonly TableAlign[];
      readonly rows: readonly (readonly (readonly InlineNode[])[])[];
    }
  | { readonly kind: "image"; readonly alt: string; readonly src: string }
  | { readonly kind: "hr" };

export interface HeadingOutline {
  readonly level: number;
  readonly text: string;
  readonly lineIndex: number;
}

export interface ParsedDocument {
  readonly blocks: readonly BlockNode[];
  readonly headings: readonly HeadingOutline[];
  readonly taskTotal: number;
  readonly taskDone: number;
}

const INLINE_RE = /(`[^`\n]+`|\*\*[^*\n]+?\*\*|~~[^~\n]+?~~|\*[^*\n]+?\*|\[[^\]\n]+\]\([^)\n]+\))/g;

export function parseInlines(source: string): InlineNode[] {
  const nodes: InlineNode[] = [];
  let cursor = 0;
  INLINE_RE.lastIndex = 0;
  for (let match = INLINE_RE.exec(source); match !== null; match = INLINE_RE.exec(source)) {
    if (match.index > cursor) {
      nodes.push({ kind: "text", text: source.slice(cursor, match.index) });
    }
    const token = match[0];
    if (token.startsWith("`") && token.endsWith("`")) {
      nodes.push({ kind: "code", text: token.slice(1, -1) });
    } else if (token.startsWith("**") && token.endsWith("**")) {
      nodes.push({ kind: "bold", text: token.slice(2, -2) });
    } else if (token.startsWith("~~") && token.endsWith("~~")) {
      nodes.push({ kind: "strike", text: token.slice(2, -2) });
    } else if (token.startsWith("*") && token.endsWith("*")) {
      nodes.push({ kind: "italic", text: token.slice(1, -1) });
    } else if (token.startsWith("[")) {
      const split = token.indexOf("](");
      if (split > 0) {
        nodes.push({
          kind: "link",
          label: token.slice(1, split),
          href: token.slice(split + 2, -1),
        });
      } else {
        nodes.push({ kind: "text", text: token });
      }
    }
    cursor = match.index + token.length;
  }
  if (cursor < source.length) {
    nodes.push({ kind: "text", text: source.slice(cursor) });
  }
  return nodes.length > 0 ? nodes : [{ kind: "text", text: source }];
}

const KEYWORDS = new Set([
  "import",
  "export",
  "from",
  "const",
  "let",
  "var",
  "function",
  "return",
  "if",
  "else",
  "for",
  "while",
  "type",
  "interface",
  "readonly",
  "async",
  "await",
  "pub",
  "fn",
  "struct",
  "enum",
  "impl",
  "match",
  "use",
  "mod",
  "mut",
  "true",
  "false",
  "null",
  "undefined",
]);

const CODE_TOKEN_RE =
  /(\/\/.*$|#.*$|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`|\b\d+(?:\.\d+)?\b|\b[A-Z][A-Za-z0-9_]*\b|\b[a-z_][a-zA-Z0-9_]*\b)/g;

export function tokenizeCodeLine(line: string, lang: string): CodeToken[] {
  const isHashComment = lang === "bash" || lang === "sh" || lang === "toml" || lang === "yaml";
  const tokens: CodeToken[] = [];
  let cursor = 0;
  CODE_TOKEN_RE.lastIndex = 0;
  for (let m = CODE_TOKEN_RE.exec(line); m !== null; m = CODE_TOKEN_RE.exec(line)) {
    const raw = m[0];
    if (raw.startsWith("#") && !isHashComment) continue;
    if (m.index > cursor) {
      tokens.push({ kind: "plain", text: line.slice(cursor, m.index) });
    }
    if (raw.startsWith("//") || raw.startsWith("#")) {
      tokens.push({ kind: "comment", text: raw });
    } else if (raw.startsWith('"') || raw.startsWith("'") || raw.startsWith("`")) {
      tokens.push({ kind: "string", text: raw });
    } else if (/^\d/.test(raw)) {
      tokens.push({ kind: "number", text: raw });
    } else if (/^[A-Z]/.test(raw)) {
      tokens.push({ kind: "type", text: raw });
    } else if (KEYWORDS.has(raw)) {
      tokens.push({ kind: "keyword", text: raw });
    } else {
      tokens.push({ kind: "plain", text: raw });
    }
    cursor = m.index + raw.length;
  }
  if (cursor < line.length) {
    tokens.push({ kind: "plain", text: line.slice(cursor) });
  }
  return tokens.length > 0 ? tokens : [{ kind: "plain", text: " " }];
}

function splitTableRow(line: string): string[] {
  const trimmed = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  return trimmed.split("|").map(cell => cell.trim());
}

function parseTableDelimiter(line: string): TableAlign[] | null {
  const trimmed = line.trim();
  if (!trimmed.includes("|") || !trimmed.includes("-")) return null;
  const cells = splitTableRow(trimmed);
  if (cells.length === 0) return null;
  const aligns: TableAlign[] = [];
  for (const cell of cells) {
    if (!/^:?-{2,}:?$/.test(cell)) return null;
    if (cell.startsWith(":") && cell.endsWith(":")) aligns.push("center");
    else if (cell.endsWith(":")) aligns.push("right");
    else aligns.push("left");
  }
  return aligns;
}

const CALLOUT_RE = /^>\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*(.*)$/;
const HEADING_RE = /^(#{1,6})\s+(.+)$/;
const IMAGE_RE = /^!\[([^\]]*)\]\(([^)]+)\)$/;
const UNORDERED_RE = /^\s*[-*+]\s+(.*)$/;
const ORDERED_RE = /^\s*\d+\.\s+(.*)$/;
const TASK_RE = /^\[([ xX])\]\s+(.*)$/;

export function parseMarkdown(source: string): ParsedDocument {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const blocks: BlockNode[] = [];
  const headings: HeadingOutline[] = [];
  let taskTotal = 0;
  let taskDone = 0;

  let i = 0;
  while (i < lines.length) {
    const raw = lines[i]!;
    const trimmed = raw.trim();

    if (!trimmed) {
      i += 1;
      continue;
    }

    // Fenced code block
    if (trimmed.startsWith("```")) {
      const lang = trimmed.slice(3).trim().toLowerCase();
      const codeLines: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i]!.trim().startsWith("```")) {
        codeLines.push(lines[i]!);
        i += 1;
      }
      if (i < lines.length) i += 1;
      const code = codeLines.join("\n");
      blocks.push({
        kind: "code",
        lang: lang || "text",
        code,
        lines: codeLines.map(line => tokenizeCodeLine(line, lang)),
      });
      continue;
    }

    // Horizontal rule
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(trimmed)) {
      blocks.push({ kind: "hr" });
      i += 1;
      continue;
    }

    // Standalone image
    const imgMatch = trimmed.match(IMAGE_RE);
    if (imgMatch) {
      blocks.push({ kind: "image", alt: imgMatch[1]!, src: imgMatch[2]! });
      i += 1;
      continue;
    }

    // ATX Heading
    const headingMatch = trimmed.match(HEADING_RE);
    if (headingMatch) {
      const level = headingMatch[1]!.length as 1 | 2 | 3 | 4 | 5 | 6;
      const text = headingMatch[2]!.trim();
      headings.push({ level, text, lineIndex: i });
      blocks.push({
        kind: "heading",
        level,
        text,
        inlines: parseInlines(text),
        lineIndex: i,
      });
      i += 1;
      continue;
    }

    // GitHub Callout or Blockquote
    if (trimmed.startsWith(">")) {
      const calloutMatch = trimmed.match(CALLOUT_RE);
      if (calloutMatch) {
        const callout = calloutMatch[1] as CalloutKind;
        const bodyLines: InlineNode[][] = [];
        if (calloutMatch[2]!.trim()) {
          bodyLines.push(parseInlines(calloutMatch[2]!.trim()));
        }
        i += 1;
        while (i < lines.length && lines[i]!.trim().startsWith(">")) {
          const content = lines[i]!.trim().replace(/^>\s?/, "");
          if (content) bodyLines.push(parseInlines(content));
          i += 1;
        }
        blocks.push({ kind: "callout", callout, lines: bodyLines });
        continue;
      }
      const quoteParts: string[] = [];
      while (i < lines.length && lines[i]!.trim().startsWith(">")) {
        quoteParts.push(lines[i]!.trim().replace(/^>\s?/, ""));
        i += 1;
      }
      blocks.push({ kind: "blockquote", inlines: parseInlines(quoteParts.join(" ")) });
      continue;
    }

    // GFM Table
    if (trimmed.includes("|") && i + 1 < lines.length) {
      const aligns = parseTableDelimiter(lines[i + 1]!);
      if (aligns) {
        const headerCells = splitTableRow(trimmed).map(parseInlines);
        i += 2;
        const rows: InlineNode[][][] = [];
        while (i < lines.length && lines[i]!.trim().includes("|")) {
          rows.push(splitTableRow(lines[i]!).map(parseInlines));
          i += 1;
        }
        blocks.push({ kind: "table", headers: headerCells, alignments: aligns, rows });
        continue;
      }
    }

    // Unordered / Task list
    if (UNORDERED_RE.test(raw)) {
      const items: ListItem[] = [];
      while (i < lines.length) {
        const m = lines[i]!.match(UNORDERED_RE);
        if (!m) break;
        const body = m[1]!;
        const task = body.match(TASK_RE);
        if (task) {
          const checked = task[1]!.toLowerCase() === "x";
          taskTotal += 1;
          if (checked) taskDone += 1;
          items.push({ inlines: parseInlines(task[2]!), checked, lineIndex: i });
        } else {
          items.push({ inlines: parseInlines(body), checked: null, lineIndex: i });
        }
        i += 1;
      }
      blocks.push({ kind: "list", ordered: false, items });
      continue;
    }

    // Ordered list
    if (ORDERED_RE.test(raw)) {
      const items: ListItem[] = [];
      while (i < lines.length) {
        const m = lines[i]!.match(ORDERED_RE);
        if (!m) break;
        items.push({ inlines: parseInlines(m[1]!), checked: null, lineIndex: i });
        i += 1;
      }
      blocks.push({ kind: "list", ordered: true, items });
      continue;
    }

    // Paragraph (coalesce consecutive plain lines)
    const para: string[] = [trimmed];
    i += 1;
    while (i < lines.length) {
      const next = lines[i]!;
      const nextTrim = next.trim();
      if (
        !nextTrim ||
        nextTrim.startsWith("```") ||
        nextTrim.startsWith(">") ||
        HEADING_RE.test(nextTrim) ||
        UNORDERED_RE.test(next) ||
        ORDERED_RE.test(next) ||
        IMAGE_RE.test(nextTrim) ||
        /^(-{3,}|\*{3,}|_{3,})$/.test(nextTrim) ||
        (nextTrim.includes("|") &&
          i + 1 < lines.length &&
          parseTableDelimiter(lines[i + 1]!) !== null)
      ) {
        break;
      }
      para.push(nextTrim);
      i += 1;
    }
    blocks.push({ kind: "paragraph", inlines: parseInlines(para.join(" ")) });
  }

  return { blocks, headings, taskTotal, taskDone };
}

export function toggleTaskAtLine(source: string, lineIndex: number): string {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const target = lines[lineIndex];
  if (target === undefined) return source;
  if (/^\s*[-*+]\s+\[ \]/.test(target)) {
    lines[lineIndex] = target.replace("[ ]", "[x]");
  } else if (/^\s*[-*+]\s+\[[xX]\]/.test(target)) {
    lines[lineIndex] = target.replace(/\[[xX]\]/, "[ ]");
  }
  return lines.join("\n");
}

/** Convert a native UTF-8 byte offset from NativeValueEvent.selection to a JS UTF-16 index. */
export function utf8OffsetToUtf16(text: string, byteOffset: number): number {
  let bytes = 0;
  let i = 0;
  while (i < text.length && bytes < byteOffset) {
    const code = text.codePointAt(i)!;
    const step = code > 0xffff ? 2 : 1;
    const width = code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
    if (bytes + width > byteOffset) break;
    bytes += width;
    i += step;
  }
  return i;
}
