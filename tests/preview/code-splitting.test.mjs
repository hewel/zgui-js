import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import { createTempWorkspace, runScenario, writeJson } from "./harness.mjs";

const LAZY_ROUTE_CSS = `
.card {
  padding: 12px;
  background-color: #1e3a5f;
  color: #ffffff;
}
`;

const LAZY_ROUTE_SOURCE = `
import styles from "./heavy-route.module.css";

// Substantial static payload to prove initial JS heap stays smaller before lazy import.
const TABLE = Array.from({ length: 4000 }, (_, i) => \`lazy-row-\${i}-\${"x".repeat(48)}\`);

export function HeavyPanel() {
  return (
    <box testId="lazy-panel" class={styles.card}>
      <label testId="lazy-panel-text">{\`Loaded \${TABLE.length} rows (\${TABLE[0]})\`}</label>
    </box>
  );
}
`;

const MAIN_SPLIT_SOURCE = `
import { Button, Show, createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import { createRouter, Outlet, RouterProvider, useRouter } from "@hewel/zgui/router";

export const hostApiVersion = 1;

function HomeView() {
  return <label testId="home-label">Home Ready</label>;
}

function HeavyRouteView() {
  const [Panel, setPanel] = createSignal<(() => unknown) | null>(null);
  const [status, setStatus] = createSignal("loading");

  void import("./heavy-route").then(mod => {
    setPanel(() => mod.HeavyPanel);
    setStatus("loaded");
  });

  return (
    <column style={{ gap: "8px" }}>
      <label testId="lazy-status">{status()}</label>
      <Show when={Panel()}>
        {Comp => (Comp()() as unknown as NativeNode)}
      </Show>
    </column>
  );
}

function AppShell() {
  const router = useRouter();
  return (
    <column style={{ width: "100%", height: "100%", padding: "16px", gap: "10px" }}>
      <row style={{ gap: "8px" }}>
        <Button testId="go-heavy" onClick={() => { void router.navigate("/heavy"); }}>
          Open Heavy Route
        </Button>
      </row>
      <Outlet />
    </column>
  );
}

export function mount(root: NativeNode): () => void {
  const router = createRouter([
    { path: "/", component: HomeView },
    { path: "/heavy", component: HeavyRouteView },
  ], { initialPath: "/" });
  return renderApp(() => (
    <RouterProvider router={router}>
      <AppShell />
    </RouterProvider>
  ), root);
}
`;

const MAIN_EAGER_SOURCE = `
import { renderApp, type NativeNode } from "@hewel/zgui";
import { HeavyPanel } from "./heavy-route";

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => (
    <column style={{ width: "100%", height: "100%", padding: "16px" }}>
      <label testId="home-label">Eager Ready</label>
      <HeavyPanel />
    </column>
  ), root);
}
`;

async function buildSplitFixture(appDir, name, mainSource) {
  await mkdir(path.join(appDir, "src"), { recursive: true });
  await writeFile(path.join(appDir, "src/heavy-route.module.css"), LAZY_ROUTE_CSS);
  await writeFile(path.join(appDir, "src/heavy-route.tsx"), LAZY_ROUTE_SOURCE);
  await writeFile(path.join(appDir, "src/main.tsx"), mainSource);
  await writeJson(path.join(appDir, "tsconfig.json"), {
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
    include: ["src/**/*.ts", "src/**/*.tsx"],
  });
  return buildApplication(name, { appDirectory: appDir });
}

const CAPS = ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect"];

test("sealed JS code-splitting merges all CSS statically, lazy-loads verified chunks on navigation, and rejects tampered chunks", { timeout: 60000 }, async t => {
  const temp = await createTempWorkspace("preview-code-split-");
  t.after(() => temp.cleanup());

  const splitDir = path.join(temp.dir, "split-app");
  const eagerDir = path.join(temp.dir, "eager-app");
  const splitBuild = await buildSplitFixture(splitDir, "split-app", MAIN_SPLIT_SOURCE);
  const eagerBuild = await buildSplitFixture(eagerDir, "eager-app", MAIN_EAGER_SOURCE);

  // 1. Verify manifest seals closed chunk graph & all StyleX CSS is merged statically into app.css.
  assert.ok(Array.isArray(splitBuild.manifest.chunks) && splitBuild.manifest.chunks.length >= 1,
    `expected sealed lazy chunk(s) in manifest, got ${JSON.stringify(splitBuild.manifest.chunks)}`);
  for (const chunk of splitBuild.manifest.chunks) {
    assert.ok(splitBuild.manifest.artifactDigests[chunk.path], `missing digest for ${chunk.path}`);
    assert.ok(splitBuild.manifest.artifactDigests[chunk.sourceMap], `missing digest for ${chunk.sourceMap}`);
  }
  const mergedCss = await readFile(path.join(splitBuild.dist, "app.css"), "utf8");
  assert.ok(mergedCss.includes("#1e3a5f"), "lazy chunk CSS Module styles must be statically merged into app.css at build time");
  assert.equal(eagerBuild.manifest.chunks, undefined, "single-chunk package keeps chunks omitted for compatibility");

  // 2. Compare initial JS heap before loading lazy route vs eager baseline.
  const splitIdle = await runScenario({
    projectDir: temp.dir,
    bundle: splitBuild.dist,
    name: "split-idle",
    id: "split-idle",
    requiredCapabilities: CAPS,
    steps: [
      { id: "s1", op: "clock.advance", byMs: 20 },
      { id: "s2", op: "assert", target: { surface: "main", testId: "home-label" }, check: { property: "text", equals: "Home Ready" } },
    ],
  });
  const eagerIdle = await runScenario({
    projectDir: temp.dir,
    bundle: eagerBuild.dist,
    name: "eager-idle",
    id: "eager-idle",
    requiredCapabilities: CAPS,
    steps: [
      { id: "s1", op: "clock.advance", byMs: 20 },
      { id: "s2", op: "assert", target: { surface: "main", testId: "home-label" }, check: { property: "text", equals: "Eager Ready" } },
    ],
  });
  const splitMetrics = JSON.parse(await readFile(path.join(splitIdle.out, "metrics.json"), "utf8"));
  const eagerMetrics = JSON.parse(await readFile(path.join(eagerIdle.out, "metrics.json"), "utf8"));
  assert.ok(
    splitMetrics.jsHeapBytes < eagerMetrics.jsHeapBytes,
    `expected split initial JS heap (${splitMetrics.jsHeapBytes}) < eager (${eagerMetrics.jsHeapBytes})`,
  );

  // 3. Navigating to /heavy dynamically loads the verified chunk and renders it cleanly.
  const nav = await runScenario({
    projectDir: temp.dir,
    bundle: splitBuild.dist,
    name: "split-nav",
    id: "split-nav",
    requiredCapabilities: CAPS,
    steps: [
      { id: "n1", op: "clock.advance", byMs: 20 },
      { id: "n2", op: "pointer.click", target: { surface: "main", testId: "go-heavy" }, button: "primary" },
      { id: "n3", op: "clock.advance", byMs: 40 },
      { id: "n4", op: "assert", target: { surface: "main", testId: "lazy-status" }, check: { property: "text", equals: "loaded" } },
      { id: "n5", op: "assert", target: { surface: "main", testId: "lazy-panel" }, check: { property: "exists", equals: true } },
    ],
  });
  assert.equal(nav.result.teardown?.clean, true);

  // 4. Tampering with the lazy chunk rejects preview preflight cleanly (E_BUNDLE).
  const firstChunkPath = path.join(splitBuild.dist, splitBuild.manifest.chunks[0].path);
  await writeFile(firstChunkPath, "export const tampered = true;\n");
  const tampered = await runScenario({
    projectDir: temp.dir,
    bundle: splitBuild.dist,
    name: "split-tampered",
    id: "split-tampered",
    allowFailure: true,
    requiredCapabilities: CAPS,
    steps: [{ id: "t1", op: "clock.advance", byMs: 20 }],
  });
  assert.equal(tampered.result.firstError?.code, "E_BUNDLE");
});
