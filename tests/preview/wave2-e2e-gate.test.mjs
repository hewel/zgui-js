import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import { createTempWorkspace, runScenario, writeJson } from "./harness.mjs";

const COMPUTE_WORKER_SOURCE = `
declare function postMessage(payload: unknown): void;

export function onMessage(msg: { values: number[]; tag: string }) {
  const isolated =
    typeof (globalThis as Record<string, unknown>).__zgui === "undefined" &&
    typeof (globalThis as Record<string, unknown>).__zguiServices === "undefined" &&
    typeof (globalThis as Record<string, unknown>).__zguiDevState === "undefined" &&
    typeof (globalThis as Record<string, unknown>).setTimeout === "undefined";
  const total = msg.values.reduce((acc, n) => acc + n, 0);
  postMessage({
    tag: msg.tag,
    total,
    isolated,
    now: Date.now(),
    rand: Math.floor(Math.random() * 100000),
  });
}
`;

const LAZY_COMPUTE_ROUTE_SOURCE = `
import { Button, createSignal } from "@hewel/zgui";
import { spawnWorker } from "@hewel/zgui-services";

export function ComputeRoutePanel() {
  const [result, setResult] = createSignal("worker:pending");

  const worker = spawnWorker("compute", {
    onMessage(payload) {
      const data = payload as { tag: string; total: number; isolated: boolean };
      setResult(\`worker:\${data.tag}=\${data.total}|isolated=\${String(data.isolated)}\`);
    },
    onError(err) {
      setResult(\`worker-err:\${err.code}:\${err.message}\`);
    },
  });

  // Immediately post deterministic compute job on lazy route mount.
  worker.postMessage({ tag: "sum", values: [10, 20, 12] });

  // Verify non-plain payloads (closures / handles) are rejected synchronously.
  let nonPlainErr = "plain:unexpected-allow";
  try {
    worker.postMessage({ fn: () => "nope" });
  } catch {
    nonPlainErr = "plain:rejected";
  }

  return (
    <column style={{ gap: "6px" }}>
      <label testId="lazy-worker-result">{result()}</label>
      <label testId="lazy-nonplain-check">{nonPlainErr}</label>
      <Button
        testId="btn-worker-inflight"
        onClick={() => {
          worker.postMessage({ tag: "late-job", values: [1, 2, 3] });
        }}
      >
        Post Late Job
      </Button>
    </column>
  );
}
`;

function makeGateAppSource({ genTag, throwInMount = false, openSse = false, hasLazyCompute = false }) {
  return `
import { Button, Show, createDevState, createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import { createRouter, Outlet, RouterProvider, useLocation, useRouter } from "@hewel/zgui/router";
import { httpRequest, openEventStream, spawnWorker } from "@hewel/zgui-services";

export const hostApiVersion = 1;

function HomeView() {
  return <label testId="view-name">home</label>;
}

function WorkspaceView() {
  return <label testId="view-name">workspace</label>;
}

${
  hasLazyCompute
    ? `
function ComputeHostView() {
  const [Panel, setPanel] = createSignal<(() => unknown) | null>(null);
  void import("./lazy-compute").then(mod => {
    setPanel(() => mod.ComputeRoutePanel);
  });
  return (
    <column style={{ gap: "6px" }}>
      <label testId="view-name">compute</label>
      <Show when={Panel()}>
        {Comp => (Comp()() as unknown as NativeNode)}
      </Show>
    </column>
  );
}
`
    : `
function ComputeHostView() {
  return <label testId="view-name">compute-placeholder</label>;
}
`
}

function GateShell() {
  const router = useRouter();
  const loc = useLocation();
  const [draft, setDraft] = createDevState<string>("draft", "empty");
  const [clicks, setClicks] = createSignal(0);
  const [sseLog, setSseLog] = createSignal("sse:none");

  ${
    openSse
      ? `
  const received: string[] = [];
  openEventStream(
    { url: "https://api.example.com/stream" },
    {
      onMessage(ev) {
        received.push(ev.data);
        setSseLog(\`sse:\${received.join(",")}\`);
      },
    },
  );
`
      : ""
  }

  return (
    <column style={{ width: "100%", height: "100%", padding: "16px", gap: "8px" }}>
      <label testId="gen-tag">${genTag}</label>
      <label testId="draft-val">{\`draft:\${draft()}\`}</label>
      <label testId="clicks-val">{\`clicks:\${clicks()}\`}</label>
      <label testId="sse-log">{sseLog()}</label>
      <label testId="route-path">{loc().pathname}</label>
      <Outlet />
      <row style={{ gap: "8px" }}>
        <Button testId="btn-draft" onClick={() => setDraft("wave2-draft")}>Save Draft</Button>
        <Button testId="btn-click" onClick={() => setClicks(c => c + 1)}>Ping</Button>
        <Button testId="nav-workspace" onClick={() => { void router.navigate("/workspace"); }}>Workspace</Button>
        <Button testId="nav-compute" onClick={() => { void router.navigate("/compute"); }}>Compute</Button>
      </row>
    </column>
  );
}

export function mount(root: NativeNode): () => void {
  ${
    throwInMount
      ? `
  // Attempt both an unscripted outbound HTTP send and a background worker spawn before failing mount.
  // Staged deferral must prevent both from executing so zero external sends leak on rejected reload.
  const req = httpRequest({ url: "https://api.example.com/rejected-side-effect" });
  void req.promise.catch(() => {});
  const w = spawnWorker("compute");
  w.postMessage({ tag: "must-not-run", values: [99] });
  throw new Error("intentional rejected reload candidate");
`
      : ""
  }
  const router = createRouter([
    { path: "/", component: HomeView },
    { path: "/workspace", component: WorkspaceView },
    { path: "/compute", component: ComputeHostView },
  ], { initialPath: "/" });
  return renderApp(() => (
    <RouterProvider router={router}>
      <GateShell />
    </RouterProvider>
  ), root);
}
`;
}

async function buildGateApp(dir, { genTag, throwInMount = false, openSse = false, hasLazyCompute = false }) {
  await mkdir(path.join(dir, "src/workers"), { recursive: true });
  await writeFile(path.join(dir, "src/workers/compute.ts"), COMPUTE_WORKER_SOURCE);
  if (hasLazyCompute) {
    await writeFile(path.join(dir, "src/lazy-compute.tsx"), LAZY_COMPUTE_ROUTE_SOURCE);
  }
  await writeFile(
    path.join(dir, "src/main.tsx"),
    makeGateAppSource({ genTag, throwInMount, openSse, hasLazyCompute }),
  );
  await writeJson(path.join(dir, "package.json"), {
    name: "wave2-gate-app",
    private: true,
    zgui: {
      id: "dev.test.wave2-gate",
      capabilities: {
        network: {
          origins: ["https://api.example.com"],
        },
      },
      workers: {
        compute: "src/workers/compute.ts",
      },
    },
  });
  await writeJson(path.join(dir, "tsconfig.json"), {
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
    include: ["src/**/*.ts", "src/**/*.tsx"],
  });
  return buildApplication("wave2-gate-app", { appDirectory: dir, development: true });
}

let stepSeq = 0;
const sid = () => `g-${++stepSeq}`;
const click = testId => ({ id: sid(), op: "pointer.click", target: { surface: "main", testId }, button: "primary" });
const check = (testId, property, equals) => ({ id: sid(), op: "assert", target: { surface: "main", testId }, check: { property, equals } });
const advance = byMs => ({ id: sid(), op: "clock.advance", byMs });
const reload = (bundle, expect = "accepted") => ({ id: sid(), op: "lifecycle.reload", bundle, expect });

const CAPS = [
  "pixels", "virtual-time", "selectors.test-id", "input.pointer", "tree.inspect", "lifecycle.reload",
];

test("Wave 2 Final Architectural Gate: SSE stream + rejected reload zero-send + accepted reload preserving dev state, lazy chunk, lockstep worker, retirement isolation, and clean teardown", { timeout: 90000 }, async t => {
  const temp = await createTempWorkspace("preview-wave2-gate-");
  t.after(() => temp.cleanup());

  const v1 = await buildGateApp(path.join(temp.dir, "v1"), { genTag: "gen-1", openSse: true });
  const v2Rejected = await buildGateApp(path.join(temp.dir, "v2-rejected"), { genTag: "gen-2-bad", throwInMount: true });
  const v2Accepted = await buildGateApp(path.join(temp.dir, "v2-accepted"), { genTag: "gen-2-ok", hasLazyCompute: true });
  const v3Final = await buildGateApp(path.join(temp.dir, "v3-final"), { genTag: "gen-3-final" });

  // Verify v2Accepted sealed both the lazy chunk and the compute worker in manifest.json.
  assert.ok(Array.isArray(v2Accepted.manifest.chunks) && v2Accepted.manifest.chunks.length >= 1);
  assert.deepEqual(v2Accepted.manifest.workers, [
    { name: "compute", path: "workers/compute.mjs", sourceMap: "workers/compute.mjs.map" },
  ]);

  const fixturePath = path.join(temp.dir, "fixture.json");
  await writeJson(fixturePath, {
    schemaVersion: 1,
    id: "wave2-gate-fixture",
    services: {
      network: {
        http: [],
        sse: [
          {
            url: "https://api.example.com/stream",
            atMs: 0,
            events: [
              { kind: "message", atMs: 10, data: "tick-1" },
              { kind: "message", atMs: 50, data: "tick-2" },
              { kind: "message", atMs: 120, data: "late-after-reload" },
            ],
          },
        ],
      },
    },
    events: [],
  });

  const { result, out } = await runScenario({
    projectDir: temp.dir,
    bundle: v1.dist,
    fixture: fixturePath,
    name: "wave2-gate",
    id: "wave2-gate",
    requiredCapabilities: CAPS,
    steps: [
      // 1. Gen 1 opens SSE stream, receives tick-1, mutates dev state & navigates router.
      advance(20),
      check("gen-tag", "text", "gen-1"),
      check("sse-log", "text", "sse:tick-1"),
      click("btn-draft"),
      click("nav-workspace"),
      advance(10),
      check("draft-val", "text", "draft:wave2-draft"),
      check("route-path", "text", "/workspace"),

      // 2. Rejected reload candidate fails mount; staged deferral ensures zero external HTTP sends
      //    (unscripted /rejected-side-effect would trigger E_FIXTURE_UNMATCHED if ever dispatched)
      //    and Gen 1 stays responsive and receives tick-2.
      reload(v2Rejected.dist, "rejected"),
      click("btn-click"),
      advance(35),
      check("gen-tag", "text", "gen-1"),
      check("clicks-val", "text", "clicks:1"),
      check("sse-log", "text", "sse:tick-1,tick-2"),

      // 3. Accepted reload mounts Gen 2, retires Gen 1 SSE stream, preserves dev state & router,
      //    ignores late SSE event at t=120ms, lazy-loads code-split chunk, and runs lockstep worker.
      reload(v2Accepted.dist, "accepted"),
      advance(80),
      check("gen-tag", "text", "gen-2-ok"),
      check("draft-val", "text", "draft:wave2-draft"),
      check("route-path", "text", "/workspace"),
      check("sse-log", "text", "sse:none"),

      click("nav-compute"),
      advance(40),
      check("route-path", "text", "/compute"),
      check("lazy-worker-result", "text", "worker:sum=42|isolated=true"),
      check("lazy-nonplain-check", "text", "plain:rejected"),

      // 4. Post an in-flight worker job and immediately reload to Gen 3; retired worker
      //    cannot mutate Gen 3's mount region, and Gen 3 still preserves dev state cleanly.
      click("btn-worker-inflight"),
      reload(v3Final.dist, "accepted"),
      advance(40),
      check("gen-tag", "text", "gen-3-final"),
      check("draft-val", "text", "draft:wave2-draft"),
      check("sse-log", "text", "sse:none"),
    ],
  });

  assert.equal(result.teardown?.clean, true, JSON.stringify(result.teardown));

  // Double-check fixture service trace recorded zero outbound calls for the rejected candidate.
  const traceText = await readFile(path.join(out, "services.json"), "utf8");
  assert.equal(traceText.includes("rejected-side-effect"), false, "rejected candidate must never emit outbound HTTP");
});

test("compute worker teardown detects leaked workers, build blocks UI imports, and tampered worker rejects with E_BUNDLE", { timeout: 60000 }, async t => {
  const temp = await createTempWorkspace("preview-worker-guards-");
  t.after(() => temp.cleanup());

  // 1. Build-time guard: worker importing @hewel/zgui-services must fail compilation.
  const badDir = path.join(temp.dir, "bad-worker-app");
  await mkdir(path.join(badDir, "src/workers"), { recursive: true });
  await writeFile(
    path.join(badDir, "src/workers/bad.ts"),
    'import { readTextAsset } from "@hewel/zgui-services"; export function onMessage() { postMessage(Boolean(readTextAsset)); }\n',
  );
  await writeFile(
    path.join(badDir, "src/main.tsx"),
    'import { renderApp, type NativeNode } from "@hewel/zgui"; export const hostApiVersion = 1; export function mount(root: NativeNode) { return renderApp(() => <label>ok</label>, root); }\n',
  );
  await writeJson(path.join(badDir, "package.json"), {
    name: "bad-worker-app",
    private: true,
    zgui: { id: "dev.test.bad-worker", workers: { bad: "src/workers/bad.ts" } },
  });
  await writeJson(path.join(badDir, "tsconfig.json"), {
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
    include: ["src/main.tsx"],
  });
  await assert.rejects(
    () => buildApplication("bad-worker-app", { appDirectory: badDir }),
    /Compute worker cannot import UI, Solid, or host service module/,
  );

  // 2. Leaked worker outside Solid owner cleanup fails teardown with resources.workers === 1.
  const leakDir = path.join(temp.dir, "leak-worker-app");
  await mkdir(path.join(leakDir, "src/workers"), { recursive: true });
  await writeFile(path.join(leakDir, "src/workers/compute.ts"), COMPUTE_WORKER_SOURCE);
  await writeFile(
    path.join(leakDir, "src/main.tsx"),
    `
import { renderApp, type NativeNode } from "@hewel/zgui";
import { spawnWorker } from "@hewel/zgui-services";
export const hostApiVersion = 1;
export function mount(root: NativeNode): () => void {
  // Spawn outside renderApp owner without terminating on cleanup -> intentional leak.
  const w = spawnWorker("compute");
  w.postMessage({ tag: "leak", values: [1, 2] });
  return renderApp(() => <label testId="status">leaked</label>, root);
}
`,
  );
  await writeJson(path.join(leakDir, "package.json"), {
    name: "leak-worker-app",
    private: true,
    zgui: { id: "dev.test.leak-worker", workers: { compute: "src/workers/compute.ts" } },
  });
  await writeJson(path.join(leakDir, "tsconfig.json"), {
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
    include: ["src/**/*.ts", "src/**/*.tsx"],
  });
  const leakBuild = await buildApplication("leak-worker-app", { appDirectory: leakDir });
  const leakRun = await runScenario({
    projectDir: temp.dir,
    bundle: leakBuild.dist,
    name: "worker-leak",
    id: "worker-leak",
    allowFailure: true,
    requiredCapabilities: ["pixels", "virtual-time", "selectors.test-id", "tree.inspect"],
    steps: [advance(20)],
  });
  assert.equal(leakRun.result.teardown?.clean, false, "leaked worker must fail teardown");
  assert.equal(leakRun.result.teardown?.resources?.workers, 1);

  // 3. Tampering with sealed worker file rejects preflight with E_BUNDLE.
  await writeFile(path.join(leakBuild.dist, "workers/compute.mjs"), "export function onMessage() {}\n");
  const tamperedRun = await runScenario({
    projectDir: temp.dir,
    bundle: leakBuild.dist,
    name: "worker-tampered",
    id: "worker-tampered",
    allowFailure: true,
    requiredCapabilities: ["pixels", "virtual-time"],
    steps: [advance(10)],
  });
  assert.equal(tamperedRun.result.firstError?.code, "E_BUNDLE");
});
