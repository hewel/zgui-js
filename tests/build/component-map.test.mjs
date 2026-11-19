import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { workspaceRoot } from "../../packages/build/src/build.ts";

const docPath = path.join(workspaceRoot, "docs/development/component-map.md");
const componentsDir = path.join(workspaceRoot, "packages/ui/src/components");
const indexPath = path.join(componentsDir, "index.ts");
const generatedDir = path.join(componentsDir, "generated");
const gallerySrcDir = path.join(workspaceRoot, "apps/examples/gallery/src");

function sectionBody(markdown, heading) {
  const marker = `## ${heading}\n`;
  const start = markdown.indexOf(marker);
  assert.notEqual(start, -1, `missing section "## ${heading}" in component-map.md`);
  const rest = markdown.slice(start + marker.length);
  const next = rest.search(/\n## /);
  return next === -1 ? rest : rest.slice(0, next);
}

function firstTableColumn(section) {
  const rows = section
    .split("\n")
    .filter(line => line.startsWith("| `"))
    .map(line => {
      const cell = line.match(/^\|\s*`([^`]+)`\s*\|/);
      return cell?.[1] ?? null;
    })
    .filter(Boolean);
  return new Set(rows);
}

async function listTsxFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...await listTsxFiles(full));
    else if (entry.name.endsWith(".tsx")) files.push(full);
  }
  return files;
}

test("component-map.md matches @hewel/zgui/components value exports bidirectionally", async () => {
  const [markdown, indexSource] = await Promise.all([
    readFile(docPath, "utf8"),
    readFile(indexPath, "utf8"),
  ]);
  const docExports = firstTableColumn(sectionBody(markdown, "Exported Component & Helper Inventory"));

  const codeExports = new Set();
  for (const match of indexSource.matchAll(/^export\s+\{([^}]+)\}\s+from\s+["'][^"']+["'];/gm)) {
    for (const part of match[1].split(",")) {
      const trimmed = part.trim();
      if (!trimmed || trimmed.startsWith("type ")) continue;
      const alias = trimmed.match(/\bas\s+(\w+)$/);
      codeExports.add(alias ? alias[1] : trimmed);
    }
  }

  assert.deepEqual([...docExports].sort(), [...codeExports].sort());
});

test("component-map.md matches literal gallery testId vocabulary bidirectionally", async () => {
  const markdown = await readFile(docPath, "utf8");
  const docTestIds = firstTableColumn(sectionBody(markdown, "Gallery `testId` Vocabulary"));

  const codeTestIds = new Set();
  for (const file of await listTsxFiles(gallerySrcDir)) {
    const source = await readFile(file, "utf8");
    for (const match of source.matchAll(/testId="([^"]+)"/g)) {
      codeTestIds.add(match[1]);
    }
  }

  assert.deepEqual([...docTestIds].sort(), [...codeTestIds].sort());
});

test("component-map.md lists every unported generated stylesheet bidirectionally", async () => {
  const [markdown, generatedFiles, componentFiles] = await Promise.all([
    readFile(docPath, "utf8"),
    readdir(generatedDir),
    readdir(componentsDir),
  ]);

  const bullets = [...sectionBody(markdown, "Unported Upstream Stylesheets").matchAll(/^- `([^`]+)`$/gm)].map(m => m[1]);
  const docUnported = new Set(bullets);

  const usedStyles = new Set();
  for (const file of componentFiles) {
    if (!file.endsWith(".ts")) continue;
    const source = await readFile(path.join(componentsDir, file), "utf8");
    for (const match of source.matchAll(/\.\/generated\/([^"'/]+)\.js/g)) {
      usedStyles.add(match[1]);
    }
  }

  const codeUnported = new Set(
    generatedFiles
      .filter(name => name.endsWith("Style.ts"))
      .map(name => name.slice(0, -3))
      .filter(stem => !usedStyles.has(stem)),
  );

  assert.deepEqual([...docUnported].sort(), [...codeUnported].sort());
});
