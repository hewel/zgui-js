import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REVISION = "ee29c7dc66206eb04ba7bcf514058a278c9070ba";
const workspaceRoot = fileURLToPath(new URL("../../../", import.meta.url));

export interface ResourceSnapshot {
  revision: string;
  styles: Record<string, { className: string; css: string }>;
  themes: Record<string, { light: string; dark: string }>;
  tokens: string[];
  icons: Record<string, { name: string; d: string; viewBox: number }>;
}

function assertRecord(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`Snapshot ${label} must be an object`);
  }
  return value as Record<string, unknown>;
}
function assertString(value: unknown, label: string): string {
  if (typeof value !== "string") throw new Error(`Snapshot ${label} must be a string`);
  return value;
}
function validateSnapshot(value: unknown): asserts value is ResourceSnapshot {
  const snapshot = value as Record<string, unknown>;
  if (snapshot.revision !== REVISION) {
    throw new Error(
      `Snapshot revision ${String(snapshot.revision)} does not match pinned ${REVISION}`,
    );
  }
  const styles = assertRecord(snapshot.styles, "styles");
  const themes = assertRecord(snapshot.themes, "themes");
  if (
    !Array.isArray(snapshot.tokens) ||
    snapshot.tokens.some((token: unknown) => typeof token !== "string")
  ) {
    throw new Error("Snapshot tokens must be a string array");
  }
  const icons = assertRecord(snapshot.icons, "icons");
  for (const [name, value] of Object.entries(styles)) {
    if (!/^[A-Z][A-Za-z0-9]*$/.test(name)) throw new Error(`Unexpected style export name: ${name}`);
    const sheet = assertRecord(value, `styles.${name}`);
    assertString(sheet.className, `styles.${name}.className`);
    assertString(sheet.css, `styles.${name}.css`);
  }
  for (const [name, value] of Object.entries(themes)) {
    const theme = assertRecord(value, `themes.${name}`);
    assertString(theme.light, `themes.${name}.light`);
    assertString(theme.dark, `themes.${name}.dark`);
  }
  for (const [name, icon] of Object.entries(icons)) {
    if (!/^[A-Z][A-Z0-9_]*$/.test(name)) throw new Error(`Unexpected icon export name: ${name}`);
    const data = assertRecord(icon, `icons.${name}`);
    assertString(data.name, `icons.${name}.name`);
    assertString(data.d, `icons.${name}.d`);
    if (typeof data.viewBox !== "number" || !Number.isFinite(data.viewBox) || data.viewBox <= 0) {
      throw new Error(`Snapshot icons.${name}.viewBox must be a positive number`);
    }
  }
}

export function adaptUpstreamCss(name: string, className: string, css: string): string {
  switch (name) {
    case "TextareaStyle":
      return css.replace("overflow: hidden;", "overflow-x: hidden;\n        overflow-y: auto;");
    case "TabsStyle":
      return css
        .replaceAll(
          '.zui-tabs__list[data-variant="default"] .zui-tabs__trigger',
          '.zui-tabs__trigger[data-variant="default"]',
        )
        .replaceAll(
          '.zui-tabs__list[data-variant="line"] .zui-tabs__trigger',
          '.zui-tabs__trigger[data-variant="line"]',
        )
        .replaceAll(
          `.${className}[data-orientation="vertical"] .zui-tabs__list`,
          '.zui-tabs__list[data-orientation="vertical"]',
        )
        .replaceAll(
          `.${className}[data-orientation="vertical"] .zui-tabs__trigger`,
          '.zui-tabs__trigger[data-orientation="vertical"]',
        )
        .replaceAll(
          `.${className}[data-orientation="horizontal"] .zui-tabs__trigger`,
          '.zui-tabs__trigger[data-orientation="horizontal"]',
        );
    case "AccordionStyle":
      return css
        .replaceAll(".zui-accordion__content .zui-collapsible__measure", ".zui-accordion__measure")
        .replaceAll(
          '.zui-accordion__trigger[data-state="open"] .zui-accordion__chevron',
          '.zui-accordion__chevron[data-state="open"]',
        );
    case "CollapsibleStyle":
      return `${css}\n.zui-collapsible__content[data-collapsed] { visibility: hidden; }`;
    case "TableStyle":
      return css
        .replace(
          `.${className}[data-sticky-header="true"] .zui-table__head`,
          '.zui-table__head[data-sticky="true"]',
        )
        .replace(
          `.${className} .zui-table__row[data-selected="true"] .zui-table__cell`,
          '.zui-table__cell[data-selected="true"]',
        )
        .replace(
          ".zui-table__row:hover .zui-table__cell",
          ".zui-table__row:hover > .zui-table__cell",
        )
        .replace(
          ".zui-table__footer .zui-table__cell",
          ".zui-table__footer > .zui-table__row > .zui-table__cell",
        )
        .replaceAll(
          /(\.zui-table__(?:body|footer)) (\.zui-table__row:(?:first|last)-child)/g,
          "$1 > $2",
        )
        .replaceAll(
          /(\.zui-table__row:(?:first|last)-child) \.zui-table__cell/g,
          "$1 > .zui-table__cell",
        );
    case "DataTableStyle":
      return `${css
        .replace(
          "border: 1px solid var(--zui-color-border);\n        border-radius: var(--zui-radius-md);\n        max-height:",
          "border: none;\n        --zgui-scrollbar-mode: overlay;\n        max-height:",
        )
        .replace(
          '.zui-data-table__grid[data-virtualized="true"] .zui-table__body .zui-table__cell',
          '.zui-data-table__layout-row[data-fixed="true"] > .zui-table__row > .zui-table__cell',
        )}
.zui-data-table__layout-row { flex-shrink: 0; }
.zui-data-table__frame {
    width: 100%;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    border: 1px solid var(--zui-color-border);
    border-radius: var(--zui-radius-md);
}`;
    case "DatePickerStyle":
      return css.replace(
        ".zui-date-picker__surface {\n        width: auto;",
        ".zui-popover.zui-date-picker__surface {\n        width: max-content;",
      );
    default:
      return css;
  }
}

export async function syncUiResources(
  snapshotPath: string,
  outDir = path.join(workspaceRoot, "packages/ui/src/components/generated"),
): Promise<number> {
  const snapshot: unknown = await Bun.file(path.resolve(snapshotPath)).json();
  validateSnapshot(snapshot);
  await mkdir(outDir, { recursive: true });

  const literal = (value: string) => JSON.stringify(value);
  const outputs = new Map<string, string>();
  for (const name of Object.keys(snapshot.styles).sort()) {
    const sheet = snapshot.styles[name];
    const adaptedCss = adaptUpstreamCss(name, sheet.className, sheet.css);
    outputs.set(
      `${name}.ts`,
      `export const sheet = { name: ${literal(name)}, className: ${literal(sheet.className)}, css: ${literal(adaptedCss)} } as const;\n`,
    );
  }
  const themeEntries = Object.keys(snapshot.themes)
    .sort()
    .map(name => `${JSON.stringify(name)}: ${JSON.stringify(snapshot.themes[name])}`)
    .join(", ");
  outputs.set(
    "themes.ts",
    [
      `export const upstreamRevision = ${literal(REVISION)} as const;`,
      `export const themes = { ${themeEntries} } as const;`,
      `export const themeTokens = ${JSON.stringify(snapshot.tokens)} as const;`,
      "",
    ].join("\n"),
  );
  const iconExports = Object.keys(snapshot.icons)
    .sort()
    .map(name => {
      const icon = snapshot.icons[name];
      return `export const ${name} = { name: ${literal(icon.name)}, viewBox: ${icon.viewBox}, d: ${literal(icon.d)} } as const;`;
    })
    .join("\n");
  outputs.set("icons.ts", `${iconExports}\n`);

  for (const [filename, content] of [...outputs].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
    await Bun.write(path.join(outDir, filename), content);
  }
  return outputs.size;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  let snapshotPath: string | undefined;
  let outDir: string | undefined;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--out") {
      outDir = args[++i];
      if (!outDir) throw new Error("--out requires a directory");
    } else if (!snapshotPath) {
      snapshotPath = args[i];
    } else {
      throw new Error(`Unexpected argument: ${args[i]}`);
    }
  }
  if (!snapshotPath) {
    throw new Error(
      "Usage: bun packages/build/src/sync-ui-resources.ts <snapshot.json> [--out DIR]",
    );
  }
  const count = await syncUiResources(snapshotPath, outDir ? path.resolve(outDir) : undefined);
  console.log(`Wrote ${count} generated files from ${snapshotPath}`);
}
