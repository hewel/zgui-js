import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import { createTempWorkspace, runScenario } from "./harness.mjs";

test(
  "QuickJS remap_diagnostic recovers authored TSX file, line, and column for a compiled JSX getter throw",
  { timeout: 60000 },
  async t => {
    const temp = await createTempWorkspace("preview-compiler-sourcemap-");
    t.after(() => temp.cleanup());

    const appDir = path.join(temp.dir, "app");
    await mkdir(path.join(appDir, "src"), { recursive: true });
    await writeFile(path.join(appDir, "package.json"), '{"type":"module"}\n');
    await writeFile(
      path.join(appDir, "tsconfig.json"),
      JSON.stringify({
        compilerOptions: {
          target: "ES2022",
          module: "ESNext",
          moduleResolution: "Bundler",
          lib: ["ES2022"],
          types: [],
          strict: true,
          noEmit: true,
          jsx: "preserve",
          jsxImportSource: "@hewel/zgui",
        },
        include: ["src"],
      }),
    );
    await writeFile(
      path.join(appDir, "src/main.tsx"),
      [
        'import { renderApp } from "@hewel/zgui";',
        'import type { NativeNode } from "@hewel/zgui";',
        "export const hostApiVersion = 1;",
        "function App() {",
        "  return (",
        "    <text>{(() => {",
        '      throw new Error("qjs-jsx-getter-boom");',
        "    })()}</text>",
        "  );",
        "}",
        "export function mount(root: NativeNode): () => void {",
        "  return renderApp(() => <App />, root);",
        "}",
        "",
      ].join("\n"),
    );

    const { dist } = await buildApplication("qjs-map", {
      root: workspaceRoot,
      appDirectory: appDir,
    });
    const { result, out } = await runScenario({
      projectDir: temp.dir,
      bundle: dist,
      name: "compiler-sourcemap",
      id: "compiler-sourcemap",
      width: 320,
      height: 200,
      scale: 1,
      allowFailure: true,
      requiredCapabilities: ["virtual-time"],
      steps: [{ id: "s1", op: "clock.advance", byMs: 10 }],
    });

    assert.equal(result.exitCode, 3);
    assert.equal(result.firstError?.code, "E_JS_EXCEPTION");
    assert.equal(result.teardown?.clean, true, JSON.stringify(result.teardown));

    const diagLines = (await readFile(path.join(out, "diagnostics.jsonl"), "utf8"))
      .trim()
      .split("\n")
      .map(line => JSON.parse(line));
    const entry = diagLines.find(item => item.diagnostic?.message === "qjs-jsx-getter-boom");
    assert.ok(entry, "expected remapped exception diagnostic in diagnostics.jsonl");
    assert.equal(entry.diagnostic.mappingApplied, true);
    assert.match(entry.diagnostic.mappedStack, /app\/src\/main\.tsx:7:17\b/);
    assert.match(entry.diagnostic.mappedStack, /App \(app\/src\/main\.tsx:6:11\)/);
    assert.ok(!entry.diagnostic.mappedStack.includes(temp.dir));
    assert.ok(!entry.diagnostic.mappedStack.includes(workspaceRoot));
  },
);
