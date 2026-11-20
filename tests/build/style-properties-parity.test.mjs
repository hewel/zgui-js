import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { workspaceRoot } from "../../packages/build/src/build.ts";

const stylesDtsPath = path.join(workspaceRoot, "packages/ui/src/styles.d.ts");
const propertiesRsPath = path.join(workspaceRoot, "crates/ui-bridge/src/properties.rs");

function extractTypeUnionStrings(source, typeName) {
  const match = source.match(new RegExp(`type\\s+${typeName}\\s*=([\\s\\S]*?);`));
  assert.ok(match, `missing type alias ${typeName} in styles.d.ts`);
  return [...match[1].matchAll(/"([^"]+)"/g)].map(m => m[1]);
}

function extractRustStringSlice(source, constName) {
  const match = source.match(new RegExp(`const\\s+${constName}:\\s*&\\[&str\\]\\s*=\\s*&\\[([\\s\\S]*?)\\];`));
  assert.ok(match, `missing const ${constName} in properties.rs`);
  return [...match[1].matchAll(/"([^"]+)"/g)].map(m => m[1]);
}

function extractRustPairSlice(source, constName) {
  const match = source.match(new RegExp(`const\\s+${constName}:\\s*&\\[\\(&str,\\s*&str\\)\\]\\s*=\\s*&\\[([\\s\\S]*?)\\];`));
  assert.ok(match, `missing const ${constName} in properties.rs`);
  return [...match[1].matchAll(/\("([^"]+)",\s*"([^"]+)"\)/g)].map(m => [m[1], m[2]]);
}

function assertUnique(items, label) {
  assert.equal(new Set(items).size, items.length, `${label} contains duplicate entries`);
}

function toCamelCase(kebab) {
  return kebab.replace(/-([a-z])/g, (_, ch) => ch.toUpperCase());
}

test("styles.d.ts hyphenated OrdinaryStyleName matches ui-bridge CAMEL_PROPERTIES bidirectionally", async () => {
  const [dts, rs] = await Promise.all([
    readFile(stylesDtsPath, "utf8"),
    readFile(propertiesRsPath, "utf8"),
  ]);

  const ordinary = extractTypeUnionStrings(dts, "OrdinaryStyleName");
  assertUnique(ordinary, "OrdinaryStyleName");

  const expectedPairs = ordinary
    .filter(name => name.includes("-"))
    .map(name => [toCamelCase(name), name])
    .sort((a, b) => a[0].localeCompare(b[0]));

  const actualPairs = extractRustPairSlice(rs, "CAMEL_PROPERTIES");
  assertUnique(actualPairs.map(([alias]) => alias), "CAMEL_PROPERTIES");

  const sortedActual = [...actualPairs].sort((a, b) => a[0].localeCompare(b[0]));
  assert.deepEqual(sortedActual, expectedPairs);
});

test("styles.d.ts Unitless matches ui-bridge UNITLESS bidirectionally", async () => {
  const [dts, rs] = await Promise.all([
    readFile(stylesDtsPath, "utf8"),
    readFile(propertiesRsPath, "utf8"),
  ]);

  const tsUnitless = extractTypeUnionStrings(dts, "Unitless");
  const rsUnitless = extractRustStringSlice(rs, "UNITLESS");

  assertUnique(tsUnitless, "Unitless");
  assertUnique(rsUnitless, "UNITLESS");
  assert.deepEqual([...rsUnitless].sort(), [...tsUnitless].sort());
});
