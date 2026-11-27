import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { workspaceRoot } from "../../packages/build/src/build.ts";

const llmsPath = path.join(workspaceRoot, "llms.txt");

function extractValueExports(source) {
  const names = new Set();
  for (const match of source.matchAll(/^export\s+\{([^}]+)\}\s+from\s+["'][^"']+["'];/gm)) {
    for (const part of match[1].split(",")) {
      const trimmed = part.trim();
      if (!trimmed || trimmed.startsWith("type ")) continue;
      const alias = trimmed.match(/\bas\s+(\w+)$/);
      names.add(alias ? alias[1] : trimmed);
    }
  }
  for (const match of source.matchAll(/^export\s+(?:async\s+)?function\s+(\w+)\s*\(/gm)) {
    names.add(match[1]);
  }
  return names;
}

function extractBacktickTokens(markdown) {
  const tokens = new Set();
  for (const match of markdown.matchAll(/`([^`]+)`/g)) {
    tokens.add(match[1].trim());
  }
  return tokens;
}

test("llms.txt covers every @hewel/zgui/components, @hewel/zgui/primitives, @hewel/zgui, @hewel/zgui/shell-host, @hewel/zgui/router, and @hewel/zgui-services export", async () => {
  const [
    llms,
    jsxRuntime,
    uiIndex,
    shellHostIndex,
    componentsIndex,
    primitivesIndex,
    routerIndex,
    servicesFiles,
  ] = await Promise.all([
    readFile(llmsPath, "utf8"),
    readFile(path.join(workspaceRoot, "packages/ui/src/jsx-runtime.ts"), "utf8"),
    readFile(path.join(workspaceRoot, "packages/ui/src/index.ts"), "utf8"),
    readFile(path.join(workspaceRoot, "packages/ui/src/shell-host.ts"), "utf8"),
    readFile(path.join(workspaceRoot, "packages/ui/src/components/index.ts"), "utf8"),
    readFile(path.join(workspaceRoot, "packages/ui/src/primitives/index.ts"), "utf8"),
    readFile(path.join(workspaceRoot, "packages/ui/src/router/index.ts"), "utf8"),
    Promise.all([
      readFile(path.join(workspaceRoot, "packages/services/src/index.ts"), "utf8"),
      readFile(path.join(workspaceRoot, "packages/services/src/activation.ts"), "utf8"),
      readFile(path.join(workspaceRoot, "packages/services/src/network.ts"), "utf8"),
      readFile(path.join(workspaceRoot, "packages/services/src/files.ts"), "utf8"),
      readFile(path.join(workspaceRoot, "packages/services/src/grants.ts"), "utf8"),
      readFile(path.join(workspaceRoot, "packages/services/src/locale.ts"), "utf8"),
      readFile(path.join(workspaceRoot, "packages/services/src/sqlite.ts"), "utf8"),
      readFile(path.join(workspaceRoot, "packages/services/src/worker.ts"), "utf8"),
    ]),
  ]);

  const intrinsicBlock = jsxRuntime.match(/interface IntrinsicElements\s*\{([^}]+)\}/s);
  assert.ok(intrinsicBlock, "packages/ui/src/jsx-runtime.ts must declare IntrinsicElements");
  const intrinsicTags = [...intrinsicBlock[1].matchAll(/^\s*([a-z][a-z0-9-]*)\s*:/gm)].map(
    match => match[1],
  );
  const intrinsicLine = llms
    .split("\n")
    .find(line => line.startsWith("- **Intrinsic Native Elements**:"));
  assert.ok(intrinsicLine, "llms.txt must have an Intrinsic Native Elements line");
  const documentedTags = [
    ...new Set([...intrinsicLine.matchAll(/`<([a-z][a-z0-9-]*)>`/g)].map(match => match[1])),
  ];
  assert.deepEqual(
    documentedTags.sort(),
    intrinsicTags.sort(),
    "llms.txt Intrinsic Native Elements tags must match jsx-runtime.ts",
  );
  assert.match(llms, /\bdocumentSelection\b/, "llms.txt must document documentSelection prop");

  const documented = extractBacktickTokens(llms);

  for (const [label, source] of [
    ["@hewel/zgui", uiIndex],
    ["@hewel/zgui/shell-host", shellHostIndex],
    ["@hewel/zgui/components", componentsIndex],
    ["@hewel/zgui/primitives", primitivesIndex],
    ["@hewel/zgui/router", routerIndex],
    ["@hewel/zgui-services", servicesFiles.join("\n")],
  ]) {
    const missing = [...extractValueExports(source)].filter(name => !documented.has(name)).sort();
    assert.deepEqual(missing, [], `llms.txt is missing ${label} exports: ${missing.join(", ")}`);
  }

  for (const cmd of [
    "init",
    "dev",
    "build",
    "typecheck",
    "preview",
    "package",
    "doctor",
    "inspect",
    "smoke",
  ]) {
    assert.match(llms, new RegExp(`\`zgui-js ${cmd}\\b`), `llms.txt must document zgui-js ${cmd}`);
  }
  assert.match(
    llms,
    /--op tree\|profile/,
    "llms.txt must document `--op tree|profile` flag grammar rather than positional tree|profile",
  );
});

test("active agent entry docs (AGENTS.md and README.md) reference only existing local paths", async () => {
  for (const doc of ["AGENTS.md", "README.md"]) {
    const content = await readFile(path.join(workspaceRoot, doc), "utf8");
    for (const match of content.matchAll(/\]\(([^)]+)\)/g)) {
      const target = match[1].split("#")[0];
      if (!target || /^(?:https?:|mailto:|file:)/.test(target)) continue;
      const resolved = path.resolve(workspaceRoot, target);
      assert.ok(existsSync(resolved), `${doc} references missing local path: ${target}`);
    }
  }
});
