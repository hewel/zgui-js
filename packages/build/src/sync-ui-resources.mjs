import { mkdir, writeFile } from "node:fs/promises";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REVISION = "80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a";
const workspaceRoot = fileURLToPath(new URL("../../../", import.meta.url));
const snapshotPath = process.argv[2];
if (!snapshotPath) throw new Error("Usage: sync-ui-resources.mjs <snapshot.json>");

const snapshot = JSON.parse(await readFile(path.resolve(snapshotPath), "utf8"));
if (snapshot.revision !== REVISION) {
  throw new Error(`Snapshot revision ${snapshot.revision} does not match pinned ${REVISION}`);
}

function assertRecord(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`Snapshot ${label} must be an object`);
  }
  return value;
}
function assertString(value, label) {
  if (typeof value !== "string") throw new Error(`Snapshot ${label} must be a string`);
  return value;
}
assertRecord(snapshot.styles, "styles");
assertRecord(snapshot.themes, "themes");
if (!Array.isArray(snapshot.tokens) || snapshot.tokens.some(token => typeof token !== "string")) {
  throw new Error("Snapshot tokens must be a string array");
}
assertRecord(snapshot.icons, "icons");
for (const [name, sheet] of Object.entries(snapshot.styles)) {
  if (!/^[A-Z][A-Za-z0-9]*$/.test(name)) throw new Error(`Unexpected style export name: ${name}`);
  assertString(assertRecord(sheet, `styles.${name}`).className, `styles.${name}.className`);
  assertString(sheet.css, `styles.${name}.css`);
}
for (const [name, theme] of Object.entries(snapshot.themes)) {
  assertString(assertRecord(theme, `themes.${name}`).light, `themes.${name}.light`);
  assertString(theme.dark, `themes.${name}.dark`);
}
for (const [name, icon] of Object.entries(snapshot.icons)) {
  if (!/^[A-Z][A-Z0-9_]*$/.test(name)) throw new Error(`Unexpected icon export name: ${name}`);
  const data = assertRecord(icon, `icons.${name}`);
  assertString(data.name, `icons.${name}.name`);
  assertString(data.d, `icons.${name}.d`);
  if (typeof data.viewBox !== "number" || !Number.isFinite(data.viewBox) || data.viewBox <= 0) {
    throw new Error(`Snapshot icons.${name}.viewBox must be a positive number`);
  }
}

const generated = path.join(workspaceRoot, "packages/ui/src/components/generated");
await mkdir(generated, { recursive: true });

const literal = value => JSON.stringify(value);
const outputs = new Map();
for (const name of Object.keys(snapshot.styles).sort()) {
  const sheet = snapshot.styles[name];
  outputs.set(`${name}.ts`,
    `export const sheet = { name: ${literal(name)}, className: ${literal(sheet.className)}, css: ${literal(sheet.css)} } as const;\n`);
}
const themeEntries = Object.keys(snapshot.themes).sort()
  .map(name => `${JSON.stringify(name)}: ${JSON.stringify(snapshot.themes[name])}`)
  .join(", ");
outputs.set("themes.ts", [
  `export const upstreamRevision = ${literal(REVISION)} as const;`,
  `export const themes = { ${themeEntries} } as const;`,
  `export const themeTokens = ${JSON.stringify(snapshot.tokens)} as const;`,
  "",
].join("\n"));
const iconExports = Object.keys(snapshot.icons).sort()
  .map(name => {
    const icon = snapshot.icons[name];
    return `export const ${name} = { name: ${literal(icon.name)}, viewBox: ${icon.viewBox}, d: ${literal(icon.d)} } as const;`;
  })
  .join("\n");
outputs.set("icons.ts", `${iconExports}\n`);

for (const [filename, content] of [...outputs.entries()].sort()) {
  await writeFile(path.join(generated, filename), content);
}
console.log(`Wrote ${outputs.size} generated files from ${snapshotPath}`);
