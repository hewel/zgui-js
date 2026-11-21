import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

const networkAppSource = `
import { Button, createSignal, createTimeout, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  connectWebSocket,
  createScopedFetch,
  httpRequest,
  openEventStream,
} from "@hewel/zgui-services";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [originDenied, setOriginDenied] = createSignal("PENDING");
    const [gestureDenied, setGestureDenied] = createSignal("PENDING");
    const [httpStatus, setHttpStatus] = createSignal("PENDING");
    const [fetchStatus, setFetchStatus] = createSignal("PENDING");
    const [sseStatus, setSseStatus] = createSignal("PENDING");
    const [timerTicks, setTimerTicks] = createSignal(0);
    const [wsStatus, setWsStatus] = createSignal("PENDING");

    // 1. Unallowlisted origin denied with E_CAPABILITY.
    try {
      httpRequest({ url: "https://evil.example.com/secret" });
      setOriginDenied("UNEXPECTED_ALLOW");
    } catch (err) {
      const code = err instanceof Error && "code" in err ? String(err.code) : "UNKNOWN";
      setOriginDenied(\`DENIED:\${code}\`);
    }

    // 2. Pull-driven chunked HTTP request.
    const req = httpRequest({ url: "https://api.example.com/chunks" });
    void (async () => {
      const resp = await req.promise;
      const chunks: string[] = [];
      for (;;) {
        const next = await resp.readChunk();
        if (next === null) break;
        let text = "";
        for (let i = 0; i < next.length; i++) text += String.fromCharCode(next[i]!);
        chunks.push(text);
      }
      setHttpStatus(\`status=\${resp.status},chunks=\${chunks.join("|")}\`);
    })();

    // 3. Explicit createScopedFetch (never mutates globalThis, omits Response.clone).
    const scopedFetch = createScopedFetch({ baseUrl: "https://api.example.com" });
    void (async () => {
      const hasGlobalFetch = "fetch" in globalThis;
      const resp = await scopedFetch("/profile");
      const hasClone = "clone" in resp;
      const data = await resp.json<{ user: string }>();
      setFetchStatus(
        \`global=\${String(hasGlobalFetch)},clone=\${String(hasClone)},user=\${data.user}\`,
      );
    })();

    // 4. SSE flood (64 messages) while a timer runs concurrently to verify fairness.
    let sseCount = 0;
    let lastId = "";
    openEventStream(
      { url: "https://api.example.com/events" },
      {
        onMessage(ev) {
          sseCount += 1;
          if (ev.id) lastId = ev.id;
          setSseStatus(\`count=\${sseCount},last=\${lastId},data=\${ev.data}\`);
        },
        onClose() {
          setSseStatus(\`closed:count=\${sseCount},last=\${lastId}\`);
        },
      },
    );
    createTimeout(() => setTimerTicks(t => t + 1), 5);
    createTimeout(() => setTimerTicks(t => t + 1), 15);

    // 5. Full-duplex WebSocket with echo + backpressure verification.
    const wsEcho = connectWebSocket(
      { url: "wss://stream.example.com/echo" },
      {
        onOpen(info) {
          wsEcho.send("ping-1");
          setWsStatus(\`open:\${info.protocol ?? "none"}\`);
        },
        onMessage(msg) {
          if (msg.kind === "text") {
            setWsStatus(prev => \`\${prev}|msg:\${msg.data}\`);
          }
        },
      },
    );

    const wsStalled = connectWebSocket(
      { url: "wss://stream.example.com/stalled" },
      {
        onOpen() {
          let bpCode = "NONE";
          try {
            wsStalled.send("a");
            wsStalled.send("b");
            wsStalled.send("c");
          } catch (err) {
            bpCode = err instanceof Error && "code" in err ? String(err.code) : "ERR";
          }
          setWsStatus(prev => \`\${prev}|bp:\${bpCode}\`);
          wsStalled.close(1000, "done");
        },
        onMessage() {},
      },
    );

    return (
      <column style="width: 100%; height: 100%; padding: 12px; gap: 6px">
        <text testId="origin-denied">{originDenied()}</text>
        <text testId="gesture-denied">{gestureDenied()}</text>
        <text testId="http-status">{httpStatus()}</text>
        <text testId="fetch-status">{fetchStatus()}</text>
        <text testId="sse-status">{sseStatus()}</text>
        <text testId="timer-ticks">{String(timerTicks())}</text>
        <text testId="ws-status">{wsStatus()}</text>
        <Button
          testId="btn-gesture-deny"
          onClick={() => {
            // User gesture must NEVER grant network authority to unallowlisted origins.
            try {
              httpRequest({ url: "https://unauthorized.example.com/data" });
              setGestureDenied("UNEXPECTED_ALLOW");
            } catch (err) {
              const code = err instanceof Error && "code" in err ? String(err.code) : "UNKNOWN";
              setGestureDenied(\`DENIED:\${code}\`);
            }
          }}
        >
          Check Gesture
        </Button>
      </column>
    );
  }, root);
}
`;

test("preview network services enforce allowlists, stream HTTP/SSE/WS with backpressure, keep timers fair under flood, and teardown cleanly", async t => {
  const temp = await createTempWorkspace("preview-net-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "net-preview",
    networkAppSource,
    {
      rawSource: true,
      capabilities: {
        network: {
          origins: ["https://api.example.com", "wss://stream.example.com"],
        },
      },
    },
  );
  const floodEvents = [];
  for (let i = 1; i <= 64; i++) {
    floodEvents.push({
      kind: "message",
      atMs: Math.floor(i / 4),
      event: "tick",
      data: `ev-${i}`,
      id: String(i),
    });
  }
  floodEvents.push({ kind: "close", atMs: 25 });

  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "network-main",
    services: {
      network: {
        http: [
          {
            method: "GET",
            url: "https://api.example.com/chunks",
            status: 200,
            headers: { "content-type": "text/plain" },
            chunks: [
              { atMs: 2, text: "hello-" },
              { atMs: 2, text: "world" },
            ],
          },
          {
            method: "GET",
            url: "https://api.example.com/profile",
            status: 200,
            headers: { "content-type": "application/json" },
            responseBody: JSON.stringify({ user: "ada" }),
          },
        ],
        sse: [
          {
            url: "https://api.example.com/events",
            atMs: 1,
            status: 200,
            events: floodEvents,
          },
        ],
        websockets: [
          {
            url: "wss://stream.example.com/echo",
            atMs: 1,
            protocol: "zgui-v1",
            echoSends: true,
            expectedSends: ["ping-1"],
            events: [{ kind: "text", atMs: 5, data: "server-hi" }],
          },
          {
            url: "wss://stream.example.com/stalled",
            atMs: 2,
            outboundQueueLimit: 2,
            stallSends: true,
            events: [],
          },
        ],
      },
    },
    events: [],
  });

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "network-main",
    requiredCapabilities: [],
    steps: [
      { id: "advance-flood", op: "clock.advance", byMs: 40 },
      {
        id: "click-gesture",
        op: "pointer.click",
        button: "primary",
        target: { surface: "main", testId: "btn-gesture-deny" },
      },
      { id: "after-click", op: "clock.advance", byMs: 16 },
      { id: "cap-done", op: "capture", captureId: "done", surface: "main" },
    ],
  });
  assert.equal(result.teardown.clean, true, JSON.stringify(result.teardown));

  const tree = await readTree(out, "done");
  assert.equal(getNode(tree, "origin-denied")?.text, "DENIED:E_CAPABILITY");
  assert.equal(getNode(tree, "gesture-denied")?.text, "DENIED:E_CAPABILITY");
  assert.equal(getNode(tree, "http-status")?.text, "status=200,chunks=hello-|world");
  assert.equal(getNode(tree, "fetch-status")?.text, "global=false,clone=false,user=ada");
  assert.equal(getNode(tree, "sse-status")?.text, "closed:count=64,last=64");
  assert.equal(getNode(tree, "timer-ticks")?.text, "2");
  const wsText = getNode(tree, "ws-status")?.text ?? "";
  assert.match(wsText, /open:zgui-v1/);
  assert.match(wsText, /msg:ping-1/);
  assert.match(wsText, /msg:server-hi/);
  assert.match(wsText, /bp:E_BACKPRESSURE/);
});

test("preview network fails hard on unmatched fixture requests without touching live network", async t => {
  const temp = await createTempWorkspace("preview-net-unmatched-");
  t.after(() => temp.cleanup());

  const source = `
import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { httpRequest } from "@hewel/zgui-services";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  return renderApp(() => {
    const req = httpRequest({ url: "https://api.example.com/unscripted" });
    void req.promise.catch(() => {});
    return <text testId="status">started</text>;
  }, root);
}
`;
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "net-unmatched", source, {
    rawSource: true,
    capabilities: {
      network: {
        origins: ["https://api.example.com"],
      },
    },
  });
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "network-unmatched",
    services: { network: { http: [] } },
    events: [],
  });
  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "network-unmatched",
    requiredCapabilities: [],
    allowFailure: true,
    steps: [{ id: "boot", op: "clock.advance", byMs: 16 }],
  });
  assert.notEqual(result.exitCode, 0, "unmatched fixture request must fail hard");
  assert.equal(result.firstError?.code, "E_FIXTURE_UNMATCHED");
});

test("preview network teardown catches leaked unclosed SSE/WS registrations", async t => {
  const temp = await createTempWorkspace("preview-net-leak-");
  t.after(() => temp.cleanup());

  const source = `
import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
export const hostApiVersion = 1;
export function mount(root: NativeNode) {
  const svc = (globalThis as unknown as {
    __zguiServices: {
      openEventStream(opts: { url: string }, cbs: { onMessage(): void }): { close(): void };
    };
  }).__zguiServices;
  svc.openEventStream({ url: "https://api.example.com/stay-open" }, { onMessage() {} });
  return renderApp(() => <text testId="status">leaked</text>, root);
}
`;
  const bundle = await buildFixtureApp(path.join(temp.dir, "app"), "net-leak", source, {
    rawSource: true,
    capabilities: {
      network: {
        origins: ["https://api.example.com"],
      },
    },
  });
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "network-leak",
    services: {
      network: {
        sse: [{ url: "https://api.example.com/stay-open", atMs: 0, events: [] }],
      },
    },
    events: [],
  });
  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "network-leak",
    requiredCapabilities: [],
    allowFailure: true,
    steps: [
      { id: "boot", op: "clock.advance", byMs: 16 },
      { id: "cap", op: "capture", captureId: "leak", surface: "main" },
    ],
  });
  assert.equal(result.exitCode, 9, "leaked open SSE stream must fail teardown");
  assert.equal(result.firstError?.code, "E_TEARDOWN");
  assert.equal(result.teardown.clean, false);
  assert.equal(result.teardown.resources.eventStreams, 1);
});
