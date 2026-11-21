import assert from "node:assert/strict";
import { mkdir, readFile, realpath, writeFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import { buildApplication, workspaceRoot } from "../../packages/build/src/build.ts";
import {
  createTempWorkspace,
  getNode,
  nativeHostBinary,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

const WORKER_SOURCE = `
declare function postMessage(value: unknown): void;

export function onMessage(payload: { values: number[] }) {
  const sum = payload.values.reduce((acc, n) => acc + n, 0);
  postMessage({ sum });
}
`;

const LAZY_CHUNK_SOURCE = `
export function LazyGreeting() {
  return <text testId="lazy-text">Lazy Bytecode Chunk OK</text>;
}
`;

const MAIN_SOURCE = `
import { Button, Show, createSignal, renderApp, type NativeNode } from "@hewel/zgui";
import { spawnWorker } from "@hewel/zgui-services";

export const hostApiVersion = 1;

function BytecodeApp() {
  const [LazyComp, setLazyComp] = createSignal<(() => unknown) | null>(null);
  const [workerResult, setWorkerResult] = createSignal("worker:idle");

  const worker = spawnWorker("calc", {
    onMessage(payload) {
      const data = payload as { sum: number };
      setWorkerResult(\`worker:sum=\${data.sum}\`);
    },
  });

  return (
    <column style={{ width: "100%", height: "100%", padding: "16px", gap: "8px" }}>
      <text testId="entry-status">Bytecode Entry Ready</text>
      <text testId="worker-status">{workerResult()}</text>
      <Show when={LazyComp()}>{Comp => Comp()() as unknown as NativeNode}</Show>
      <row style={{ gap: "8px" }}>
        <Button
          testId="btn-lazy"
          onClick={() => {
            void import("./lazy-chunk").then(mod => {
              setLazyComp(() => mod.LazyGreeting);
            });
          }}
        >
          Load Chunk
        </Button>
        <Button
          testId="btn-worker"
          onClick={() => {
            worker.postMessage({ values: [10, 20, 12] });
          }}
        >
          Run Worker
        </Button>
      </row>
    </column>
  );
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <BytecodeApp />, root);
}
`;

const FAILING_SOURCE = `
import { renderApp, type NativeNode } from "@hewel/zgui";

export const hostApiVersion = 1;

function explode(): string {
  throw new Error("bytecode-mapped-crash");
}

export function mount(root: NativeNode): () => void {
  return renderApp(() => <text>{explode()}</text>, root);
}
`;

test("bytecode packages (--bytecode) execute entry, lazy chunk, and background worker with clean teardown", async t => {
  const work = await createTempWorkspace("preview-bytecode-");
  t.after(() => work.cleanup());

  const appDir = path.join(work.dir, "app");
  await mkdir(path.join(appDir, "src/workers"), { recursive: true });
  await writeFile(path.join(appDir, "src/main.tsx"), MAIN_SOURCE);
  await writeFile(path.join(appDir, "src/lazy-chunk.tsx"), LAZY_CHUNK_SOURCE);
  await writeFile(path.join(appDir, "src/workers/calc.ts"), WORKER_SOURCE);
  await writeJson(path.join(appDir, "package.json"), {
    name: "bytecode-demo",
    private: true,
    type: "module",
    zgui: {
      workers: { calc: "src/workers/calc.ts" },
    },
  });
  await writeJson(path.join(appDir, "tsconfig.json"), {
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
    include: ["src/**/*.ts", "src/**/*.tsx"],
  });

  const { dist, manifest } = await buildApplication("bytecode-demo", {
    appDirectory: appDir,
    bytecode: true,
    hostExecutable: nativeHostBinary,
  });

  assert.equal(manifest.entry, "app.qjsb");
  assert.equal(manifest.bytecodeFormat, "qjs-bytecode-v1");
  assert.match(manifest.quickjsAbi ?? "", /^qjs-ng-/);
  assert.equal(Object.hasOwn(manifest.artifactDigests, "app.mjs"), false);
  assert.equal(Object.hasOwn(manifest.artifactDigests, "app.qjsb"), true);
  assert.ok((manifest.chunks ?? []).length >= 1);
  assert.ok((manifest.chunks ?? []).every(c => c.path.endsWith(".qjsb")));
  assert.ok((manifest.workers ?? []).every(w => w.path.endsWith(".qjsb")));

  const { out } = await runScenario({
    projectDir: work.dir,
    bundle: dist,
    name: "bytecode-exec",
    requiredCapabilities: ["pixels", "tree.inspect"],
    steps: [
      {
        id: "click-lazy",
        op: "pointer.click",
        target: { surface: "main", testId: "btn-lazy" },
        button: "primary",
      },
      { id: "wait-lazy", op: "clock.advance", byMs: 30 },
      {
        id: "click-worker",
        op: "pointer.click",
        target: { surface: "main", testId: "btn-worker" },
        button: "primary",
      },
      { id: "wait-worker", op: "clock.advance", byMs: 60 },
      { id: "cap", op: "capture", captureId: "done" },
    ],
  });

  const tree = await readTree(out, "done");
  assert.equal(getNode(tree, "entry-status").text, "Bytecode Entry Ready");
  assert.equal(getNode(tree, "lazy-text").text, "Lazy Bytecode Chunk OK");
  assert.equal(getNode(tree, "worker-status").text, "worker:sum=42");

  // Tamper .qjsb bytes in place and verify fail-closed E_BUNDLE rejection.
  const realDist = await realpath(dist);
  const bcBytes = Buffer.from(await readFile(path.join(realDist, "app.qjsb")));
  bcBytes[0] ^= 0xff;
  await writeFile(path.join(realDist, "app.qjsb"), bcBytes);

  const tampered = await runScenario({
    projectDir: work.dir,
    bundle: dist,
    name: "bytecode-tampered",
    requiredCapabilities: ["pixels", "tree.inspect"],
    allowFailure: true,
    steps: [{ id: "cap", op: "capture", captureId: "never" }],
  });
  assert.notEqual(tampered.result.exitCode, 0);
  assert.match(tampered.result.firstError?.message ?? "", /integrity mismatch/i);
});

test("bytecode packages preserve sidecar sourcemap diagnostic remapping on thrown exceptions", async t => {
  const work = await createTempWorkspace("preview-bytecode-remap-");
  t.after(() => work.cleanup());

  const appDir = path.join(work.dir, "crash-app");
  await mkdir(path.join(appDir, "src"), { recursive: true });
  await writeFile(path.join(appDir, "src/main.tsx"), FAILING_SOURCE);
  await writeJson(path.join(appDir, "package.json"), {
    name: "bytecode-crash",
    private: true,
    type: "module",
  });
  await writeJson(path.join(appDir, "tsconfig.json"), {
    extends: path.join(workspaceRoot, "packages/ui/tsconfig.native.json"),
    include: ["src/**/*.ts", "src/**/*.tsx"],
  });

  const { dist } = await buildApplication("bytecode-crash", {
    appDirectory: appDir,
    bytecode: true,
    hostExecutable: nativeHostBinary,
  });

  const crashed = await runScenario({
    projectDir: work.dir,
    bundle: dist,
    name: "bytecode-crash",
    requiredCapabilities: ["pixels", "tree.inspect"],
    allowFailure: true,
    steps: [{ id: "cap", op: "capture", captureId: "never" }],
  });

  assert.notEqual(crashed.result.exitCode, 0);
  const diagText = await readFile(path.join(crashed.out, "diagnostics.jsonl"), "utf8");
  assert.match(diagText, /bytecode-mapped-crash/);
  assert.match(diagText, /app\/src\/main\.tsx:7:/);
});
