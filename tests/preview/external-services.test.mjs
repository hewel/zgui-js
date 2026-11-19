import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { test } from "node:test";
import {
  buildFixtureApp,
  createTempWorkspace,
  getNode,
  pixelAt,
  readPng,
  readTree,
  runScenario,
  writeJson,
} from "./harness.mjs";

const socketPath = "/zgui-preview-only/services.sock";
const program = "/zgui-preview-only/command";
const capabilities = { unixSockets: [socketPath], commands: [program], controlChannel: true };
const shell = (body, beforeOwner = "") => `
  import { Image, Show, createSignal, onCleanup, renderApp, type NativeNode } from "@hewel/zgui";
  import { connectUnixSocket, spawnCommand, onControlMessage, type ImageHandle } from "@hewel/zgui-services";
  export const hostApiVersion = 1;
  export function mount(root: NativeNode) {
    ${beforeOwner}
    return renderApp(() => { ${body} }, root);
  }
`;
const capture = { id: "shell", op: "capture", captureId: "shell" };
const calls = (service, method, count, errorCode) => ({
  id: `${service}-${method}`.toLowerCase(),
  op: "assert",
  serviceCall: { service, method, count, errorCode },
});
const text = (testId, equals) => ({
  id: `text-${testId}`,
  op: "assert",
  target: { surface: "main", testId },
  check: { property: "text", equals },
});

async function setup(t) {
  const { dir, cleanup } = await createTempWorkspace("preview-external-services-");
  t.after(cleanup);
  const image = await readFile(
    new URL("../../apps/examples/stylex/assets/red.png", import.meta.url),
  );
  return {
    build: (name, source, grants = capabilities) =>
      buildFixtureApp(path.join(dir, name), name, source, {
        rawSource: true,
        assets: { "socket.png": image },
        capabilities: grants,
      }),
    async run(name, bundle, services, steps, { events = [], allowFailure = false, runtimeEnvironment } = {}) {
      const fixture = path.join(dir, `${name}-fixture.json`);
      await writeJson(fixture, { schemaVersion: 1, id: name, services, events, runtimeEnvironment });
      return runScenario({
        projectDir: dir,
        name,
        bundle,
        fixture,
        steps,
        allowFailure,
        requiredCapabilities: [
          "services.fixtures",
          "pixels",
          "tree.inspect",
          "assets.native-readiness",
        ],
      });
    },
  };
}

test(
  "preview scripts drive a reactive shell, verified binary images and exact control replies",
  { timeout: 45000 },
  async t => {
    const { build, run } = await setup(t);
    const bundle = await build(
      "external-reactive-shell",
      shell(`
    const [state, setState] = createSignal("initial");
    const [src, setSrc] = createSignal<string>();
    const [output, setOutput] = createSignal("waiting");
    const [stream, setStream] = createSignal("");
    const [fault, setFault] = createSignal("waiting");
    let handle: ImageHandle | undefined;
    const socket = connectUnixSocket(${JSON.stringify(socketPath)}, {
      framing: "length",
      onMessage(message) { setState(message); socket.send("ack:" + message); },
      onBinaryImage(next) { handle = next; setSrc(next.src); },
      onClose() { setState(previous => previous + ":closed"); },
    });
    const buffered = spawnCommand(${JSON.stringify(program)}, ["a b", "$literal"]);
    buffered.promise.then(result => setOutput([result.status, result.stdout, result.stderr].join("|")));
    spawnCommand(${JSON.stringify(program)}, ["stream"], {
      stream: true,
      onStdout(line) { setStream(previous => previous + "out:" + line + ";"); },
      onStderr(line) { setStream(previous => previous + "err:" + line + ";"); },
      onExit(status) { setStream(previous => previous + "exit:" + status); },
    });
    const failed = spawnCommand(${JSON.stringify(program)}, ["fail"]);
    failed.promise.catch(error => setFault(error.code));
    onControlMessage(args => {
      if (args[0] === "get") return { state: state(), output: output() };
      setState(args.join(":"));
      return "plain reply";
    });
    onCleanup(() => handle?.release());
    return <column>
      <text testId="state">{state()}</text><text testId="output">{output()}</text>
      <text testId="stream">{stream()}</text><text testId="fault">{fault()}</text>
      <Show when={src()} keyed>{url => <Image testId="socket-image" src={url} style={{ width: "24px", height: "24px" }} />}</Show>
    </column>;
  `),
    );
    const services = {
      sockets: [
        {
          path: socketPath,
          framing: "length",
          send: ["ack:ready"],
          events: [
            { atMs: 1, kind: "message", text: "ready" },
            { atMs: 2, kind: "image", asset: "assets/socket.png" },
            { atMs: 6, kind: "close" },
          ],
        },
      ],
      commands: [
        {
          program,
          args: ["a b", "$literal"],
          mode: "buffered",
          events: [
            {
              atMs: 3,
              kind: "output",
              status: 7,
              stdout: "direct stdout",
              stderr: "direct stderr",
            },
          ],
        },
        {
          program,
          args: ["stream"],
          mode: "streaming",
          events: [
            { atMs: 2, kind: "stdout", text: "first" },
            { atMs: 2, kind: "stderr", text: "second" },
            { atMs: 3, kind: "exit", status: null },
          ],
        },
        {
          program,
          args: ["fail"],
          mode: "buffered",
          events: [{ atMs: 2, kind: "error", error: { code: "Io", message: "scripted error" } }],
        },
      ],
      control: {},
    };
    const { out, result } = await run(
      "external-services",
      bundle,
      services,
      [
        { id: "advance", op: "clock.advance", byMs: 30 },
        text("state", "ready:closed"),
        text("output", "7|direct stdout|direct stderr"),
        text("stream", "out:first;err:second;exit:null"),
        text("fault", "Io"),
        {
          id: "emit",
          op: "service.emit",
          channel: "control.message",
          value: { args: ["visible", "state"], reply: { text: "plain reply" } },
        },
        calls("sockets", "connectUnixSocket", 1),
        calls("sockets", "send", 1),
        calls("commands", "spawnCommand", 3, "Io"),
        calls("control", "onControlMessage", 1),
        calls("control", "reply", 2),
        { id: "render-control-state", op: "clock.advance", byMs: 20 },
        capture,
      ],
      {
        events: [
          {
            atMs: 10,
            channel: "control.message",
            value: {
              args: ["get"],
              reply: { text: '{"state":"ready:closed","output":"7|direct stdout|direct stderr"}' },
            },
          },
        ],
      },
    );
    assert.equal(
      result.teardown.clean,
      true,
      "all handles are retired after the shell is disposed",
    );
    const tree = await readTree(out, "shell");
    assert.equal(
      getNode(tree, "state").text,
      "visible:state",
      "service.emit updates the captured tree",
    );
    const { x, y, width, height } = getNode(tree, "socket-image").boundsDevice;
    const png = await readPng(path.join(out, "captures/shell.png"));
    assert.deepEqual(
      pixelAt(png, Math.floor(x + width / 2), Math.floor(y + height / 2)),
      [255, 0, 0, 255],
      "binary socket event draws declared package image bytes",
    );
    const { records } = JSON.parse(await readFile(path.join(out, "services.json"), "utf8"));
    assert.ok(
      records.some(({ kind, detail }) => kind === "arguments" && detail.message === "ack:ready"),
      "external trace retains exact outgoing socket text",
    );
    assert.ok(
      records.some(({ kind, detail }) => kind === "response" && detail.text === "plain reply"),
      "external trace retains exact outgoing control reply",
    );
  },
);

test(
  "caught capability denials and exact send mismatches are sticky preview failures",
  { timeout: 45000 },
  async t => {
    const { build, run } = await setup(t);
    const source = shell(`
    try {
      connectUnixSocket(${JSON.stringify(socketPath)}, { framing: "line", onMessage() {} }).send("different");
    } catch {}
    return <text>caught</text>;
  `);
    const scripts = {
      sockets: [{ path: socketPath, framing: "line", send: ["expected"], events: [] }],
    };
    const denied = await build("fixture-denied-app", source, {});
    const mismatched = await build("fixture-mismatched-app", source);
    for (const [name, bundle] of [
      ["manifest-denial", denied],
      ["send-mismatch", mismatched],
    ]) {
      const { result } = await run(name, bundle, scripts, [capture], { allowFailure: true });
      assert.equal(result.firstError?.code, "E_FIXTURE_CONTRACT", name);
      assert.equal(result.captureCount, 0, `${name} fails despite the JavaScript catch`);
    }
  },
);

test("preview connects only to resolved sockets and reports unbound and undeclared names separately", async t => {
  const { build, run } = await setup(t);
  const host = "/zgui-preview-only/runtime/z-shell/host-wayland-2.sock";
  const niri = "/zgui-preview-only/compositor.sock";
  const runtimeEnvironment = {
    xdgRuntimeDir: "/zgui-preview-only/runtime", waylandDisplay: "wayland-2",
    homeDir: "/zgui-preview-only/home", namedSockets: { niri, ambient: "/not-authorized.sock" },
  };
  const imports = `
    import { getAuthorizedEndpoint, getResolvedCapabilities } from "@hewel/zgui-services";
    const snapshot = getResolvedCapabilities();
  `;
  const grants = {
    unixSockets: ["$XDG_RUNTIME_DIR/z-shell/host-${WAYLAND_DISPLAY}.sock"],
    imagePrefixes: ["$XDG_DATA_HOME/icons"],
    namedSockets: ["niri", "optional", "constructor"],
  };
  const bundle = await build("portable-sockets", imports + shell(`
    const named = getAuthorizedEndpoint("niri");
    const literal = snapshot.unixSockets[0];
    if (!named || !literal || getAuthorizedEndpoint("optional") !== null
      || getAuthorizedEndpoint("constructor") !== null
      || Object.keys(snapshot.namedSockets).join(",") !== "constructor,niri,optional"
      || snapshot.imagePrefixes[0] !== "/zgui-preview-only/home/.local/share/icons") {
      throw new Error("unexpected resolved capabilities");
    }
    Object.assign(snapshot.namedSockets, { niri: "/tampered.sock" });
    Object.assign(snapshot.unixSockets, { 0: "/tampered.sock" });
    const fresh = getResolvedCapabilities();
    if (fresh.namedSockets.niri !== named || fresh.unixSockets[0] !== literal) {
      throw new Error("snapshot mutation changed capability authority");
    }
    const [status, setStatus] = createSignal("");
    for (const path of [literal, named]) {
      connectUnixSocket(path, { framing: "line", onMessage(message) { setStatus(s => s + message); } });
    }
    return <text testId="status">{status()}</text>;
  `), grants);
  const sockets = [[host, "host"], [niri, "niri"]].map(([path, text]) => ({
    path, framing: "line", send: [], events: [{ atMs: 0, kind: "message", text }],
  }));
  await run("resolved-sockets", bundle, { sockets }, [
    text("status", "hostniri"), calls("sockets", "connectUnixSocket", 2), capture,
  ], { runtimeEnvironment });

  const denied = await build("undeclared-endpoint", imports + shell(`
    try {
      getAuthorizedEndpoint("ambient");
      throw new Error("undeclared environment binding was exposed");
    } catch (error) {
      if (!(error instanceof Error) || !("code" in error) || error.code !== "E_CAPABILITY") throw error;
    }
    return <text>caught</text>;
  `), grants);
  const { result } = await run("undeclared-endpoint", denied, {}, [capture], { runtimeEnvironment, allowFailure: true });
  assert.equal(result.firstError?.code, "E_CAPABILITY");
  assert.equal(result.captureCount, 0, "catching the capability error cannot clear the fixture failure");
});

test(
  "preview teardown reports residual external handles without waiting for fixture deadlines",
  { timeout: 45000 },
  async t => {
    const { build, run } = await setup(t);
    const bundle = await build(
      "leaked-external-handles",
      shell(
        "return <text>leaked handles</text>;",
        `
    connectUnixSocket(${JSON.stringify(socketPath)}, { framing: "line", onMessage() {} });
    spawnCommand(${JSON.stringify(program)}, [], { stream: true });
    onControlMessage(() => "unused");
  `,
      ),
    );
    const { result } = await run(
      "external-residuals",
      bundle,
      {
        sockets: [
          { path: socketPath, framing: "line", send: [], events: [{ atMs: 10000, kind: "close" }] },
        ],
        commands: [
          {
            program,
            args: [],
            mode: "streaming",
            events: [{ atMs: 10000, kind: "exit", status: 0 }],
          },
        ],
        control: {},
      },
      [capture],
      { allowFailure: true },
    );
    assert.equal(result.captureCount, 1, "fixture activity does not block capture");
    assert.equal(result.exitCode, 9, "the run fails on teardown, not fixture finalization");
    assert.equal(
      result.firstError?.code,
      "E_TEARDOWN",
      "leaked callbacks are not silently cleared before counting",
    );
    for (const name of ["unixSockets", "childProcesses", "controlListeners"]) {
      assert.equal(
        result.teardown.resources[name],
        1,
        `teardown reports one retained ${name} registration`,
      );
    }
  },
);
