import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { promisify } from "node:util";
import { workspaceRoot } from "../../packages/build/src/build.mjs";

const execFileAsync = promisify(execFile);
const oxlintBin = path.join(workspaceRoot, "node_modules/oxlint/bin/oxlint");
const configPath = path.join(workspaceRoot, ".oxlintrc.json");

async function runOxlint(args) {
  try {
    const result = await execFileAsync(process.execPath, [oxlintBin, "--deny-warnings", "-c", configPath, ...args], {
      cwd: workspaceRoot,
      encoding: "utf8",
    });
    return { status: 0, output: `${result.stdout}\n${result.stderr}` };
  } catch (error) {
    return {
      status: error.code ?? 1,
      output: `${error.stdout ?? ""}\n${error.stderr ?? ""}`,
    };
  }
}

test("repository passes oxlint with zero warnings or errors", async () => {
  const result = await runOxlint(["."]);
  assert.equal(result.status, 0, result.output);
});

const negativeCases = [
  {
    name: "zgui/no-direct-bridge",
    file: "bridge.ts",
    code: 'export function leak() { return globalThis.__zgui.createElement("box"); }\n',
    expected: /zgui\(no-direct-bridge\)/,
    guidance: /mountStylesheet/,
  },
  {
    name: "zgui/no-raw-insert-or-provider (insert call)",
    file: "raw-insert.ts",
    code: 'declare function insert(p: unknown, c: unknown): void;\nexport function bad(p: unknown) { insert(p, "x"); }\n',
    expected: /zgui\(no-raw-insert-or-provider\)/,
    guidance: /append\(\) or slot\(\)/,
  },
  {
    name: "zgui/no-raw-insert-or-provider (Context.Provider call)",
    file: "raw-provider.ts",
    code: 'declare const Ctx: { Provider(props: unknown): unknown };\nexport const x = Ctx.Provider({ value: 1 });\n',
    expected: /zgui\(no-raw-insert-or-provider\)/,
    guidance: /provide\(\)/,
  },
  {
    name: "zgui/no-raw-insert-or-provider (JSX Provider explicit children prop)",
    file: "jsx-provider.tsx",
    code: 'declare const Ctx: { Provider(props: { value: number; children: unknown }): unknown };\nexport const el = <Ctx.Provider value={1} children="x" />;\n',
    expected: /zgui\(no-raw-insert-or-provider\)/,
    guidance: /explicit children prop/,
  },
  {
    name: "zgui/no-unthemed-primitives-with-components",
    file: "mixed-ui.tsx",
    code: 'import { Button } from "@hewel/zgui";\nimport { Badge } from "@hewel/zgui/components";\nexport const View = () => <><Button>Raw</Button><Badge>Ok</Badge></>;\n',
    expected: /zgui\(no-unthemed-primitives-with-components\)/,
    guidance: /@hewel\/zgui\/components/,
  },
  {
    name: "zgui/no-internal-package-imports",
    file: "internal-import.ts",
    code: 'import { shared } from "../../packages/ui/src/components/shared.js";\nexport const x = shared;\n',
    expected: /zgui\(no-internal-package-imports\)/,
    guidance: /public package entry points/,
  },
  {
    name: "zgui/no-lint-suppression",
    file: "suppress.ts",
    code: `// ${"oxlint"}-disable-next-line no-unused-vars\nexport const ok = 1;\n`,
    expected: /zgui\(no-lint-suppression\)/,
    guidance: /inline disable comments/,
  },
  {
    name: "zgui/no-essay-comments",
    file: "essay.ts",
    code: "/*\n * 1\n * 2\n * 3\n * 4\n * 5\n * 6\n */\nexport const ok = 1;\n",
    expected: /zgui\(no-essay-comments\)/,
    guidance: /concise \(<= 6 lines\)/,
  },
  {
    name: "no-restricted-globals (setTimeout)",
    file: "timer.ts",
    code: "export function delay() { return setTimeout(() => {}, 10); }\n",
    expected: /eslint\(no-restricted-globals\)/,
    guidance: /createDelay\(\)/,
  },
  {
    name: "no-restricted-globals (requestAnimationFrame)",
    file: "raf.ts",
    code: "export function tick() { return requestAnimationFrame(() => {}); }\n",
    expected: /eslint\(no-restricted-globals\)/,
    guidance: /createPresence\(\)/,
  },
];

test("oxlint rules reject architectural violations with actionable teaching messages", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-lint-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  for (const item of negativeCases) {
    const file = path.join(dir, item.file);
    await writeFile(file, item.code, "utf8");
    const result = await runOxlint([file]);
    assert.notEqual(result.status, 0, `${item.name} should fail lint`);
    assert.match(result.output, item.expected, item.name);
    assert.match(result.output, item.guidance, item.name);
  }
});
