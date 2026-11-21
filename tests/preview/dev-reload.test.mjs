import assert from "node:assert/strict";
import { mkdir, readFile, symlink, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  pixelAt,
  readPng,
  readTree,
  runScenario,
} from "./harness.mjs";

function makeAppSource({ banner, version = 1, throwInMount = false }) {
  return `
import { Button, createDevState, renderApp, type NativeNode } from "@hewel/zgui";
import { createRouter, Outlet, RouterProvider, useLocation, useRouter } from "@hewel/zgui/router";

export const hostApiVersion = 1;

function Page(props: { name: string }) {
  return <label testId="route-page">{props.name}</label>;
}

function Shell(props: { banner: string }) {
  const router = useRouter();
  const loc = useLocation();
  const [count, setCount] = createDevState<number>("counter", 0, { version: ${version} });
  const [note, setNote] = createDevState<string>("note", "init", { version: ${version} });
  const [rejectMsg, setRejectMsg] = createDevState<string>("rejectMsg", "ok", { version: ${version} });

  return (
    <column style={{ width: "100%", height: "100%", padding: "16px", gap: "8px" }}>
      <label testId="banner">{props.banner}</label>
      <label testId="counter-val">{\`count:\${count()}\`}</label>
      <label testId="note-val">{\`note:\${note()}\`}</label>
      <label testId="reject-val">{rejectMsg()}</label>
      <label testId="router-loc">{\`\${loc().pathname}|\${router.canGoBack()}|\${router.canGoForward()}\`}</label>
      <Outlet />
      <row style={{ gap: "8px" }}>
        <Button testId="btn-inc" onClick={() => setCount(c => c + 1)}>Inc</Button>
        <Button testId="btn-note" onClick={() => setNote("saved-draft")}>Note</Button>
        <Button testId="btn-bad-state" onClick={() => {
          try {
            (setNote as unknown as (v: unknown) => void)({ fn: () => "closure" });
            setRejectMsg("accepted-bad");
          } catch {
            try {
              (setNote as unknown as (v: unknown) => void)(Promise.resolve(1));
              setRejectMsg("accepted-promise");
            } catch {
              setRejectMsg("rejected-non-plain");
            }
          }
        }}>Bad</Button>
        <Button testId="nav-docs" onClick={() => { void router.navigate("/docs/wave2"); }}>Docs</Button>
        <Button testId="nav-settings" onClick={() => { void router.navigate("/settings"); }}>Settings</Button>
        <Button testId="nav-back" onClick={() => { void router.back(); }}>Back</Button>
        <Button testId="nav-forward" onClick={() => { void router.forward(); }}>Forward</Button>
      </row>
    </column>
  );
}

export function mount(root: NativeNode): () => void {
  ${throwInMount ? 'throw new Error("intentional candidate mount failure");' : ""}
  const router = createRouter([
    { path: "/home", component: () => <Page name="Home" /> },
    { path: "/docs/:slug", component: () => <Page name="Docs" /> },
    { path: "/settings", component: () => <Page name="Settings" /> },
  ], { initialPath: "/home" });
  return renderApp(() => (
    <RouterProvider router={router}>
      <Shell banner="${banner}" />
    </RouterProvider>
  ), root);
}
`;
}

let serial = 0;
const id = () => `s-${++serial}`;
const click = testId => ({ id: id(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const check = (testId, property, equals) => ({ id: id(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const advance = byMs => ({ id: id(), op: "clock.advance", byMs });
const reload = (bundle, expect = "accepted") => ({ id: id(), op: "lifecycle.reload", bundle, expect });

const CAPS = [
  "pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect", "lifecycle.reload",
];

test("dev reload preserves createDevState + router stack, rolls back rejected candidates, and cold-starts on schema/capability mismatch", { timeout: 60000 }, async t => {
  const temp = await createTempWorkspace("preview-dev-reload-");
  t.after(() => temp.cleanup());

  const v1 = await buildFixtureApp(path.join(temp.dir, "v1"), "dev-reload-app", makeAppSource({ banner: "v1" }), { rawSource: true, development: true });
  const v2Bad = await buildFixtureApp(path.join(temp.dir, "v2-bad"), "dev-reload-app", makeAppSource({ banner: "v2-bad", throwInMount: true }), { rawSource: true, development: true });
  const v2Good = await buildFixtureApp(path.join(temp.dir, "v2-good"), "dev-reload-app", makeAppSource({ banner: "v2-good" }), { rawSource: true, development: true });
  const v3VersionBump = await buildFixtureApp(path.join(temp.dir, "v3-ver"), "dev-reload-app", makeAppSource({ banner: "v3-ver", version: 2 }), { rawSource: true, development: true });
  const v4CapChange = await buildFixtureApp(
    path.join(temp.dir, "v4-cap"),
    "dev-reload-app",
    makeAppSource({ banner: "v4-cap", version: 2 }),
    { rawSource: true, development: true, capabilities: { clipboard: { read: true } } },
  );

  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle: v1,
    name: "dev-reload-flow",
    id: "dev-reload-flow",
    requiredCapabilities: CAPS,
    steps: [
      advance(40),
      check("banner", "text", "v1"),
      check("counter-val", "text", "count:0"),
      check("router-loc", "text", "/home|false|false"),

      // Mutate dev state & navigate history stack
      click("btn-inc"),
      click("btn-inc"),
      click("btn-inc"),
      click("btn-note"),
      click("btn-bad-state"),
      click("nav-docs"),
      click("nav-settings"),
      click("nav-back"),
      advance(20),
      check("counter-val", "text", "count:3"),
      check("note-val", "text", "note:saved-draft"),
      check("reject-val", "text", "rejected-non-plain"),
      check("router-loc", "text", "/docs/wave2|true|true"),

      // 1. Rejected reload rolls back cleanly; predecessor stays mounted and interactive
      reload(v2Bad, "rejected"),
      advance(20),
      check("banner", "text", "v1"),
      click("btn-inc"),
      advance(20),
      check("counter-val", "text", "count:4"),
      check("router-loc", "text", "/docs/wave2|true|true"),

      // 2. Accepted reload preserves counter, note, AND router location + history stack
      reload(v2Good, "accepted"),
      advance(40),
      check("banner", "text", "v2-good"),
      check("counter-val", "text", "count:4"),
      check("note-val", "text", "note:saved-draft"),
      check("router-loc", "text", "/docs/wave2|true|true"),
      // Forward still reaches /settings from preserved history stack!
      click("nav-forward"),
      advance(20),
      check("router-loc", "text", "/settings|true|false"),

      // 3. Entry version bump resets createDevState slots while router stays preserved
      reload(v3VersionBump, "accepted"),
      advance(40),
      check("banner", "text", "v3-ver"),
      check("counter-val", "text", "count:0"),
      check("note-val", "text", "note:init"),
      check("router-loc", "text", "/settings|true|false"),

      // 4. Capability digest mismatch forces full cold start (including router)
      click("btn-inc"),
      advance(20),
      check("counter-val", "text", "count:1"),
      reload(v4CapChange, "accepted"),
      advance(40),
      check("banner", "text", "v4-cap"),
      check("counter-val", "text", "count:0"),
      check("router-loc", "text", "/home|false|false"),
    ],
  });

  assert.equal(result.teardown?.clean, true);
});

test("production build degrades createDevState and router dev preservation to plain no-ops", { timeout: 45000 }, async t => {
  const temp = await createTempWorkspace("preview-prod-reload-");
  t.after(() => temp.cleanup());

  const prodV1 = await buildFixtureApp(path.join(temp.dir, "prod-v1"), "prod-reload-app", makeAppSource({ banner: "prod-v1" }), { rawSource: true, development: false });
  const prodV2 = await buildFixtureApp(path.join(temp.dir, "prod-v2"), "prod-reload-app", makeAppSource({ banner: "prod-v2" }), { rawSource: true, development: false });

  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle: prodV1,
    name: "prod-reload-noop",
    id: "prod-reload-noop",
    requiredCapabilities: CAPS,
    steps: [
      advance(40),
      click("btn-inc"),
      click("btn-inc"),
      click("nav-docs"),
      advance(20),
      check("counter-val", "text", "count:2"),
      check("router-loc", "text", "/docs/wave2|true|false"),
      reload(prodV2, "accepted"),
      advance(40),
      check("banner", "text", "prod-v2"),
      check("counter-val", "text", "count:0"),
      check("router-loc", "text", "/home|false|false"),
    ],
  });

  assert.equal(result.teardown?.clean, true);
});

test("CSS-only live reload hot-swaps Stylo stylesheets and updates pixels without restarting QuickJS or losing local signals (#82)", { timeout: 60000 }, async t => {
  const temp = await createTempWorkspace("preview-css-live-reload-");
  t.after(() => temp.cleanup());

  const makeTsx = label => `
    import { createSignal, renderApp, type NativeNode } from "@hewel/zgui";
    import styles from "./Swatch.module.css";
    export const hostApiVersion = 1;
    function App() {
      const [localCount, setLocalCount] = createSignal(0);
      return (
        <box style={{ display: "flex", "flex-direction": "column", gap: "12px", padding: "20px" }}>
          <text testId="build-label">${label}</text>
          <text testId="local-count">local:{localCount()}</text>
          <box testId="btn-inc" onClick={() => setLocalCount(n => n + 1)}><text>Inc</text></box>
          <box testId="swatch" class={styles.swatch} />
        </box>
      );
    }
    export function mount(root: NativeNode) { return renderApp(() => <App />, root); }
  `;
  const buildVariant = async (dirName, { label, rgb }) => {
    const dir = path.join(temp.dir, dirName);
    await mkdir(path.join(dir, "src"), { recursive: true });
    await mkdir(path.join(dir, "node_modules/@hewel"), { recursive: true });
    await symlink(path.join(workspaceRoot, "packages/ui"), path.join(dir, "node_modules/@hewel/zgui"), "dir");
    await symlink(path.join(workspaceRoot, "node_modules/solid-js"), path.join(dir, "node_modules/solid-js"), "dir");
    await writeFile(path.join(dir, "package.json"), JSON.stringify({ name: "css-hot-app", type: "module" }));
    await writeFile(path.join(dir, "tsconfig.json"), JSON.stringify({
      extends: "@hewel/zgui/tsconfig.native.json",
      include: ["src"],
    }));
    await writeFile(path.join(dir, "src/Swatch.module.css"), `.swatch { width: 120px; height: 60px; background: ${rgb}; }\n`);
    await writeFile(path.join(dir, "src/main.tsx"), makeTsx(label));
    const { dist } = await buildApplication("css-hot-app", { root: workspaceRoot, appDirectory: dir, development: true });
    return dist;
  };

  // Identical TSX + identical CSS class names, different CSS color -> pure CSS-only successor!
  const v1Red = await buildVariant("v1-red", { label: "v1", rgb: "rgb(220, 40, 40)" });
  const v2Green = await buildVariant("v2-green", { label: "v1", rgb: "rgb(30, 185, 85)" });
  // Modified TSX -> full staged JS reload!
  const v3Js = await buildVariant("v3-js", { label: "v3", rgb: "rgb(40, 100, 220)" });

  const { result, out } = await runScenario({
    projectDir: temp.dir,
    bundle: v1Red,
    name: "css-only-live-reload",
    id: "css-only-live-reload",
    width: 480,
    height: 320,
    scale: 1,
    devState: true,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect"],
    steps: [
      advance(40),
      click("btn-inc"),
      click("btn-inc"),
      advance(20),
      check("local-count", "text", "local:2"),
      { id: "c-red", op: "capture", captureId: "before-css", surface: "main" },
      reload(v2Green, "accepted"),
      advance(20),
      // Local Solid signal (NOT createDevState) MUST stay at 2 because QuickJS never restarted!
      check("local-count", "text", "local:2"),
      check("build-label", "text", "v1"),
      { id: "c-green", op: "capture", captureId: "after-css", surface: "main" },
      reload(v3Js, "accepted"),
      advance(40),
      // JS change restarts QuickJS so local signal resets to 0 and label becomes v3.
      check("build-label", "text", "v3"),
      check("local-count", "text", "local:0"),
    ],
  });

  assert.equal(result.teardown?.clean, true, JSON.stringify(result.teardown));

  // Verify steps.jsonl recorded cssOnly: true on generation 1 for the CSS reload, and generation 2 for JS reload.
  const steps = (await readFile(path.join(out, "steps.jsonl"), "utf8"))
    .trim()
    .split("\n")
    .map(line => JSON.parse(line));
  const reloadSteps = steps.filter(s => s.op === "lifecycle.reload");
  assert.equal(reloadSteps.length, 2);
  assert.equal(reloadSteps[0].result?.generation, 1, JSON.stringify(reloadSteps[0]));
  assert.equal(reloadSteps[0].result?.cssOnly, true, JSON.stringify(reloadSteps[0]));
  assert.equal(reloadSteps[1].result?.generation, 2, JSON.stringify(reloadSteps[1]));
  assert.equal(reloadSteps[1].result?.cssOnly, undefined, JSON.stringify(reloadSteps[1]));

  // Verify rendered swatch pixels genuinely changed from red to green!
  const tree = await readTree(out, "after-css");
  const swatch = getNode(tree, "swatch");
  const cx = Math.round(swatch.boundsLogical.x + swatch.boundsLogical.width / 2);
  const cy = Math.round(swatch.boundsLogical.y + swatch.boundsLogical.height / 2);
  const beforePx = pixelAt(await readPng(path.join(out, "captures/before-css.png")), cx, cy);
  const afterPx = pixelAt(await readPng(path.join(out, "captures/after-css.png")), cx, cy);
  assert.ok(beforePx[0] > 180 && beforePx[1] < 80, `expected red before CSS reload, got rgba(${beforePx.join(", ")})`);
  assert.ok(afterPx[1] > 150 && afterPx[0] < 80, `expected green after CSS reload, got rgba(${afterPx.join(", ")})`);
});
