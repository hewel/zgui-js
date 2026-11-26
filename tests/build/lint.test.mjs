import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { promisify } from "node:util";
import { workspaceRoot } from "../../packages/build/src/build.ts";

const execFileAsync = promisify(execFile);
const oxlintBin = path.join(workspaceRoot, "node_modules/oxlint/bin/oxlint");
const oxfmtBin = path.join(workspaceRoot, "node_modules/oxfmt/bin/oxfmt");
const configPath = path.join(workspaceRoot, ".oxlintrc.json");
const fmtConfigPath = path.join(workspaceRoot, ".oxfmtrc.json");

async function runOxlint(args) {
  try {
    const result = await execFileAsync(
      process.execPath,
      [oxlintBin, "--deny-warnings", "-c", configPath, ...args],
      {
        cwd: workspaceRoot,
        encoding: "utf8",
      },
    );
    return { status: 0, output: `${result.stdout}\n${result.stderr}` };
  } catch (error) {
    return {
      status: error.code ?? 1,
      output: `${error.stdout ?? ""}\n${error.stderr ?? ""}`,
    };
  }
}

// Repository-wide oxlint is enforced directly by `bun run lint` in `bun run check` and CI;
// this test file verifies custom rule diagnostics, standalone downstream presets, and oxfmt.
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
    code: "declare const Ctx: { Provider(props: unknown): unknown };\nexport const x = Ctx.Provider({ value: 1 });\n",
    expected: /zgui\(no-raw-insert-or-provider\)/,
    guidance: /provide\(\)/,
  },
  {
    name: "zgui/no-raw-insert-or-provider (callable Context call)",
    file: "raw-callable-context.ts",
    code: "declare function MyContext(props: { value: number }): unknown;\nexport const x = MyContext({ value: 1 });\n",
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
    name: "zgui/no-raw-insert-or-provider (JSX callable Context explicit children prop)",
    file: "jsx-callable-context.tsx",
    code: 'declare function MyContext(props: { value: number; children: unknown }): unknown;\nexport const el = <MyContext value={1} children="x" />;\n',
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
    name: "zgui/no-getter-object-spread (direct)",
    file: "getter-spread-direct.ts",
    code: "export const bad = { ...{ get ariaHidden() { return true; } } };\n",
    expected: /zgui\(no-getter-object-spread\)/,
    guidance: /mergeProps\(\.\.\.\) or Object\.defineProperty\(\.\.\.\)/,
  },
  {
    name: "zgui/no-getter-object-spread (conditional branch)",
    file: "getter-spread-cond.ts",
    code: 'export function bad(cond: boolean, open: () => boolean) { return { ...(cond ? { get ariaLabelledBy() { return open() ? "a" : "b"; } } : {}) }; }\n',
    expected: /zgui\(no-getter-object-spread\)/,
    guidance: /get ariaLabelledBy\(\)/,
  },
  {
    name: "zgui/no-cleanup-in-ref (observeGeometry in ref arrow)",
    file: "cleanup-in-ref.tsx",
    code: 'import { observeGeometry } from "@hewel/zgui";\nexport const V = () => <scroll ref={el => observeGeometry(el, "scrollPosition", () => {})} />;\n',
    expected: /zgui\(no-cleanup-in-ref\)/,
    guidance: /CLEANUP_IN_FORBIDDEN_SCOPE/,
  },
  {
    name: "zgui/no-cleanup-in-ref (spawnCommand in ref block)",
    file: "service-in-ref.tsx",
    code: 'import { spawnCommand } from "@hewel/zgui-services";\nexport const V = () => <box ref={() => { spawnCommand("/usr/bin/uname", ["-a"]); }} />;\n',
    expected: /zgui\(no-cleanup-in-ref\)/,
    guidance: /ref=\{setNode\}/,
  },
  {
    name: "zgui/no-cleanup-in-ref (spawnWorker in ref arrow)",
    file: "worker-in-ref.tsx",
    code: 'import { spawnWorker } from "@hewel/zgui-services";\nexport const V = () => <box ref={() => spawnWorker("compute")} />;\n',
    expected: /zgui\(no-cleanup-in-ref\)/,
    guidance: /ref=\{setNode\}/,
  },
  {
    name: "zgui/no-cleanup-in-ref (registerImageAsset in ref arrow)",
    file: "register-image-in-ref.tsx",
    code: 'import { registerImageAsset } from "@hewel/zgui-services";\nexport const V = () => <box ref={() => registerImageAsset("hero.png")} />;\n',
    expected: /zgui\(no-cleanup-in-ref\)/,
    guidance: /ref=\{setNode\}/,
  },
  {
    name: "zgui/no-top-level-service-call (named import)",
    file: "top-level-service.ts",
    code: 'import { spawnCommand } from "@hewel/zgui-services";\nexport const req = spawnCommand("/usr/bin/uname", ["-s"]);\n',
    expected: /zgui\(no-top-level-service-call\)/,
    guidance: /module top level before mount\(\)/,
  },
  {
    name: "zgui/no-top-level-service-call (namespace import)",
    file: "top-level-service-ns.ts",
    code: 'import * as svc from "@hewel/zgui-services";\nexport const caps = svc.getResolvedCapabilities();\n',
    expected: /zgui\(no-top-level-service-call\)/,
    guidance: /inside mount\(\)/,
  },
  {
    name: "zgui/no-top-level-service-call (spawnWorker)",
    file: "top-level-worker.ts",
    code: 'import { spawnWorker } from "@hewel/zgui-services";\nexport const w = spawnWorker("compute");\n',
    expected: /zgui\(no-top-level-service-call\)/,
    guidance: /module top level before mount\(\)/,
  },
  {
    name: "zgui/no-top-level-service-call (registerImageAsset)",
    file: "top-level-register-image.ts",
    code: 'import { registerImageAsset } from "@hewel/zgui-services";\nexport const img = registerImageAsset("hero.png");\n',
    expected: /zgui\(no-top-level-service-call\)/,
    guidance: /module top level before mount\(\)/,
  },
  {
    name: "zgui/no-solid-web-imports (solid-js/web)",
    file: "solid-web.ts",
    code: 'import { render } from "solid-js/web";\nexport const r = render;\n',
    expected: /zgui\(no-solid-web-imports\)/,
    guidance: /@hewel\/zgui/,
  },
  {
    name: "zgui/no-solid-web-imports (solid-js)",
    file: "solid-direct.ts",
    code: 'import { createSignal } from "solid-js";\nexport const s = createSignal;\n',
    expected: /zgui\(no-solid-web-imports\)/,
    guidance: /@hewel\/zgui/,
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
    guidance: /createSpring\(\)/,
  },
  {
    name: "typescript/await-thenable (tsrs type-aware linting)",
    file: "await-non-promise.ts",
    code: "export async function check() { const value = 42; return await value; }\n",
    expected: /typescript\(await-thenable\)/,
    guidance: /not Promise-like|non-Promise/,
  },
  {
    name: "zgui/no-static-component-stylesheet (named inside component)",
    file: "static-sheet-named.tsx",
    code: 'import { mountStylesheet } from "@hewel/zgui";\nexport function App() { mountStylesheet("a", ".x { gap: 4px; }"); return <box />; }\n',
    expected: /zgui\(no-static-component-stylesheet\)/,
    guidance: /\*\.module\.css.*StyleDependencies.*Stylist/,
  },
  {
    name: "zgui/no-static-component-stylesheet (aliased + const binding)",
    file: "static-sheet-alias.tsx",
    code: 'import { mountStylesheet as install } from "@hewel/zgui";\nexport function App() { const css = `.x { gap: 4px; }`; install("a", css as string); return <box />; }\n',
    expected: /zgui\(no-static-component-stylesheet\)/,
    guidance: /src\/app\.css/,
  },
  {
    name: "zgui/no-static-component-stylesheet (namespace mountShellStylesheet)",
    file: "static-sheet-ns.tsx",
    code: 'import * as zgui from "@hewel/zgui";\nexport function App() { zgui.mountShellStylesheet("a", ":root { padding: 0; }"); return <box />; }\n',
    expected: /zgui\(no-static-component-stylesheet\)/,
    guidance: /Stylist/,
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

test("exported @hewel/zgui/oxlint/recommended preset works standalone for downstream consumer apps", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-consumer-lint-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const consumerConfig = path.join(dir, ".oxlintrc.json");
  await writeFile(
    consumerConfig,
    JSON.stringify({
      extends: [path.join(workspaceRoot, "packages/ui/oxlint/recommended.json")],
    }),
    "utf8",
  );

  // Positive fixture: accessor ref pattern + service call inside mount() + @hewel/zgui imports.
  const goodFile = path.join(dir, "good.tsx");
  await writeFile(
    goodFile,
    `import { createDelay, createSignal, observeGeometry, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { spawnCommand } from "@hewel/zgui-services";

export function makeExtra(cond: boolean) {
  return { ...(cond ? { role: "region" as const } : {}), get id() { return "s"; } };
}

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [scroller, setScroller] = createSignal<NativeNode | null>(null);
    observeGeometry(scroller, "scrollPosition", () => {});
    const delay = createDelay();
    delay.schedule(50, () => { spawnCommand("/usr/bin/uname", ["-s"]); });
    void makeExtra(true);
    return <scroll ref={setScroller}><text>ok</text></scroll>;
  }, root);
}
`,
    "utf8",
  );
  const goodResult = await execFileAsync(
    process.execPath,
    [oxlintBin, "--deny-warnings", "-c", consumerConfig, goodFile],
    { cwd: dir, encoding: "utf8" },
  );
  assert.ok(goodResult.stdout.includes("Found 0 warnings and 0 errors"), goodResult.stdout);

  // Negative fixture: ref cleanup + top-level service + solid-js/web import + getter spread all caught by preset.
  const badFile = path.join(dir, "bad.tsx");
  await writeFile(
    badFile,
    `import { createSignal } from "solid-js";
import { observeGeometry } from "@hewel/zgui";
import { spawnCommand } from "@hewel/zgui-services";
export const early = spawnCommand("/usr/bin/uname", ["-a"]);
export const badSpread = { ...(true ? { get x() { return 1; } } : {}) };
export const View = () => {
  const [x] = createSignal(0);
  return <scroll ref={el => observeGeometry(el, "scrollPosition", () => {})}><text>{x()}</text></scroll>;
};
`,
    "utf8",
  );
  let badOutput = "";
  let badStatus = 0;
  try {
    await execFileAsync(
      process.execPath,
      [oxlintBin, "--deny-warnings", "-c", consumerConfig, badFile],
      {
        cwd: dir,
        encoding: "utf8",
      },
    );
  } catch (error) {
    badStatus = error.code ?? 1;
    badOutput = `${error.stdout ?? ""}\n${error.stderr ?? ""}`;
  }
  assert.notEqual(badStatus, 0, "consumer preset must reject bad fixture");
  assert.match(badOutput, /zgui\(no-solid-web-imports\)/);
  assert.match(badOutput, /zgui\(no-top-level-service-call\)/);
  assert.match(badOutput, /zgui\(no-cleanup-in-ref\)/);
  assert.match(badOutput, /zgui\(no-getter-object-spread\)/);
});

test("exported @hewel/zgui/oxlint/strict preset enforces prefer-stylesheet without false positives", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-strict-lint-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const strictConfig = path.join(dir, ".oxlintrc.json");
  await writeFile(
    strictConfig,
    JSON.stringify({
      extends: [path.join(workspaceRoot, "packages/ui/oxlint/strict.json")],
    }),
    "utf8",
  );

  // Positive fixture: dynamic motion, ratios, progress, custom props, and non-zgui components pass.
  const goodFile = path.join(dir, "good-strict.tsx");
  await writeFile(
    goodFile,
    `import { createSignal, mountShellStylesheet, mountStylesheet } from "@hewel/zgui";
import * as zguiNs from "@hewel/zgui";
import { Button as ZButton } from "@hewel/zgui/components";

mountStylesheet("app-static", ".card { padding: 8px; }");
mountShellStylesheet("shell-static", ":root { margin: 0; }");
zguiNs.mountStylesheet("ns-static", ".ns { gap: 4px; }");

function Bar(props: { style?: string }) {
  return <box data-kind={props.style} />;
}

export function GoodView(props: {
  style?: Record<string, string>;
  display?: string;
  spring: () => string;
  ratio: () => number;
  pct: () => number;
  canvasW: () => number;
  canvasH: () => number;
  measured: () => number;
  ready: () => boolean;
  hidden: () => boolean;
  tone: () => string;
}) {
  const [color] = createSignal("#fff");
  const mountDynamic = () => {
    mountStylesheet("dyn", \`.dyn { color: \${color()}; }\`);
  };
  const shadowedFn = (mountStylesheet: (n: string, c: string) => void) => {
    mountStylesheet("local", ".x { gap: 4px; }");
  };
  const shadowedNs = (zguiNs: { mountStylesheet: (n: string, c: string) => void }) => {
    zguiNs.mountStylesheet("local", ".y { gap: 4px; }");
  };
  const Button = (p: { style?: string }) => <box data-s={p.style} />;
  const mutated = { width: "100px" };
  mutated.width = \`\${props.canvasW()}px\`;
  let mutableStr = "gap: 4px";
  mutableStr = "gap: 8px";
  void mountDynamic;
  void shadowedFn;
  void shadowedNs;

  return (
    <column style={props.style}>
      <box style={{ transform: props.spring(), opacity: props.ratio() }} />
      <box style={{ flexGrow: props.ratio(), width: \`\${props.pct()}%\` }} />
      <box style={{ width: \`\${props.canvasW()}px\`, height: \`\${props.canvasH()}px\` }} />
      <box style={{ "--progress": props.pct(), "--tone": props.tone() }} />
      <box style={{ "--tone": "accent", "--progress": props.pct() }} />
      <box style={{ width: props.ready() ? \`\${props.measured()}px\` : "0px" }} />
      <box style={{ display: props.hidden() ? "none" : props.display }} />
      <box style={null} />
      <box style={{ width: null, height: undefined }} />
      <box style={{ width: undefined, transform: props.spring() }} />
      <box style={{ ...props.style }} />
      <box style={mutated} />
      <box style={mutableStr} />
      <Bar style="floating" />
      <Button style="width: 120px" />
      <ZButton style={{ transform: props.spring() }}>Go</ZButton>
    </column>
  );
}
`,
    "utf8",
  );
  const goodResult = await execFileAsync(
    process.execPath,
    [oxlintBin, "--deny-warnings", "-c", strictConfig, goodFile],
    { cwd: dir, encoding: "utf8" },
  );
  assert.ok(goodResult.stdout.includes("Found 0 warnings and 0 errors"), goodResult.stdout);

  // Negative cases: each strict violation triggers prefer-stylesheet or no-static-component-stylesheet.
  const strictNegativeCases = [
    {
      name: "static string literal",
      file: "neg-str.tsx",
      code: 'export const V = () => <box style="width: 220px" />;\n',
      expected: /zgui\(prefer-stylesheet\)/,
      guidance: /\*\.module\.css/,
    },
    {
      name: "static template and const string",
      file: "neg-const-str.tsx",
      code: "const css = `gap: 8px`;\nexport const V = () => <row style={css} />;\n",
      expected: /zgui\(prefer-stylesheet\)/,
      guidance: /src\/app\.css/,
    },
    {
      name: "purely static object with spread and unary minus",
      file: "neg-obj.tsx",
      code: 'const base = { padding: "8px" };\nexport const V = () => <box style={{ ...base, zIndex: -1 }} />;\n',
      expected: /zgui\(prefer-stylesheet\)/,
      guidance: /padding, zIndex/,
    },
    {
      name: "top-level conditional static object",
      file: "neg-top-cond.tsx",
      code: 'export const V = (open: () => boolean) => <box style={open() ? { display: "flex" } : null} />;\n',
      expected: /zgui\(prefer-stylesheet\)/,
      guidance: /display/,
    },
    {
      name: "zgui component named, aliased, primitive, and namespace imports",
      file: "neg-imports.tsx",
      code: `import { WindowSurface as Win } from "@hewel/zgui";
import * as UI from "@hewel/zgui/components";
import { Trigger } from "@hewel/zgui/primitives";
export const V = () => (
  <>
    <Win style="gap: 4px" />
    <UI.Button style={{ width: "100px" }} />
    <Trigger style={{ padding: "4px" }} />
  </>
);
`,
      expected: /zgui\(prefer-stylesheet\)/,
      guidance: /Found 0 warnings and 3 errors/,
    },
    {
      name: "mixed object reports static ordinary and --zgui-* properties only",
      file: "neg-mixed.tsx",
      code: 'export const V = (spring: () => string) => <box style={{ gap: "8px", "--zgui-ring": "2px", "--tone": "ok", transform: spring() }} />;\n',
      expected: /zgui\(prefer-stylesheet\)/,
      guidance: /gap, --zgui-ring/,
    },
    {
      name: "finite static conditional on ordinary CSS property",
      file: "neg-cond-prop.tsx",
      code: 'export const V = (active: () => boolean) => <text style={{ color: active() ? "var(--a)" : "var(--b)" }}>hi</text>;\n',
      expected: /zgui\(prefer-stylesheet\)/,
      guidance: /class=\{cx\(\.\.\.\)\} or \[data-\*\]/,
    },
    {
      name: "static stylesheet in component via strict preset",
      file: "neg-sheet.tsx",
      code: 'import { mountStylesheet } from "@hewel/zgui";\nexport const V = () => { mountStylesheet("s", ".a{}"); return <box />; };\n',
      expected: /zgui\(no-static-component-stylesheet\)/,
      guidance: /StyleDependencies/,
    },
  ];

  for (const item of strictNegativeCases) {
    const file = path.join(dir, item.file);
    await writeFile(file, item.code, "utf8");
    let output = "";
    let status = 0;
    try {
      await execFileAsync(
        process.execPath,
        [oxlintBin, "--deny-warnings", "-c", strictConfig, file],
        {
          cwd: dir,
          encoding: "utf8",
        },
      );
    } catch (error) {
      status = error.code ?? 1;
      output = `${error.stdout ?? ""}\n${error.stderr ?? ""}`;
    }
    assert.notEqual(status, 0, `${item.name} should fail strict lint`);
    assert.match(output, item.expected, item.name);
    assert.match(output, item.guidance, item.name);
  }
});

test("oxfmt checks and formats files with repository configuration", async t => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "zgui-fmt-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const file = path.join(dir, "sample.ts");
  await writeFile(file, "export const add=(x:number)=>x+1\n", "utf8");
  await assert.rejects(
    execFileAsync(process.execPath, [oxfmtBin, "--check", "-c", fmtConfigPath, file], {
      cwd: workspaceRoot,
    }),
  );
  await execFileAsync(process.execPath, [oxfmtBin, "-c", fmtConfigPath, file], {
    cwd: workspaceRoot,
  });
  await execFileAsync(process.execPath, [oxfmtBin, "--check", "-c", fmtConfigPath, file], {
    cwd: workspaceRoot,
  });
});
