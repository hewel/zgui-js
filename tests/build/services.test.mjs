import assert from "node:assert/strict";
import { test } from "node:test";

test("services defer host lookup and preserve the host request, receiver and synchronous errors", async () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  try {
    Object.defineProperty(globalThis, "__zguiServices", {
      configurable: true,
      get() { throw new Error("Host must not be accessed during module evaluation"); },
    });
    const { loadTextAsset, readTextAsset } = await import("@hewel/zgui-services");
    let cancellations = 0;
    const request = { promise: Promise.resolve("text"), cancel() { cancellations++; } };
    const failure = new Error("undeclared asset");
    const cancelledError = Object.assign(new Error("request cancelled"), { code: "Cancelled" });
    let pendingReject;
    const host = {
      readTextAsset(asset) {
        assert.equal(this, host);
        if (asset === "assets/missing.txt") throw failure;
        if (asset === "assets/cancelled.txt") {
          const cancelledPromise = new Promise((_, reject) => { pendingReject = reject; });
          return { promise: cancelledPromise, cancel() { cancellations++; pendingReject(cancelledError); } };
        }
        assert.equal(asset, "assets/message.txt");
        return request;
      },
    };
    Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value: host });
    const result = readTextAsset("assets/message.txt");
    assert.equal(result, request);
    assert.equal(result.promise, request.promise);
    result.cancel();
    assert.equal(cancellations, 1);
    assert.throws(() => readTextAsset("assets/missing.txt"), error => error === failure);

    const { createRoot, flush } = await import("solid-js");
    let dispose;
    const loaded = createRoot(d => {
      dispose = d;
      return loadTextAsset("assets/cancelled.txt");
    });
    flush();
    dispose();
    flush();
    assert.equal(cancellations, 2);
    assert.equal(await loaded, "");
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});

test("image asset handles preserve the host receiver, with explicit or owner-scoped release", async () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  try {
    Object.defineProperty(globalThis, "__zguiServices", {
      configurable: true,
      get() {
        throw new Error("Host must not be accessed during module evaluation");
      },
    });
    const { loadImageAsset, registerImageAsset } = await import("@hewel/zgui-services");
    const live = new Set();
    const failure = new Error("undeclared image asset");
    const host = {
      registerImageAsset(path) {
        assert.equal(this, host);
        if (path === "assets/missing.png") throw failure;
        const handle = {
          src: `zgui-blob://${path}`,
          release() {
            assert.equal(this, handle);
            live.delete(handle);
          },
        };
        live.add(handle);
        return handle;
      },
    };
    Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value: host });
    const { createRoot, flush } = await import("solid-js");
    let dispose;
    const [explicit, scoped] = createRoot(d => {
      dispose = d;
      return [registerImageAsset("assets/explicit.png"), loadImageAsset("assets/scoped.png")];
    });
    flush();
    assert.equal(explicit.src, "zgui-blob://assets/explicit.png");
    assert.equal(scoped.src, "zgui-blob://assets/scoped.png");
    assert.deepEqual([...live], [explicit, scoped]);
    dispose();
    flush();
    assert.deepEqual([...live], [explicit]);
    explicit.release();
    assert.equal(live.size, 0);
    const unscoped = loadImageAsset("assets/unscoped.png");
    assert.deepEqual([...live], [unscoped]);
    unscoped.release();
    assert.equal(live.size, 0);
    assert.throws(() => loadImageAsset("assets/missing.png"), error => error === failure);
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});

test("endpoint queries defer host access and preserve unbound and undeclared outcomes", async () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  try {
    Object.defineProperty(globalThis, "__zguiServices", {
      configurable: true,
      get() { throw new Error("Host must not be accessed during module evaluation"); },
    });
    const { getResolvedCapabilities, getAuthorizedEndpoint } = await import("@hewel/zgui-services");
    const denied = Object.assign(new Error("undeclared socket"), { code: "E_CAPABILITY" });
    const host = {
      getResolvedCapabilities() {
        assert.equal(this, host);
        return { imagePrefixes: [], unixSockets: [], namedSockets: { niri: null } };
      },
      getAuthorizedEndpoint(name) {
        assert.equal(this, host);
        if (name === "niri") return null;
        throw denied;
      },
    };
    Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value: host });
    assert.deepEqual(getResolvedCapabilities(), {
      imagePrefixes: [], unixSockets: [], namedSockets: { niri: null },
    });
    assert.equal(getAuthorizedEndpoint("niri"), null);
    assert.throws(() => getAuthorizedEndpoint("other"), error => error === denied);
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});

test("Unix sockets and control subscriptions preserve native delivery and follow their Solid owner", async () => {
  const { connectUnixSocket, onControlMessage } = await import("@hewel/zgui-services");
  const { createRoot, flush } = await import("solid-js");
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  const messages = [];
  const image = { src: "zgui-blob://sample", release() {} };
  const socketOptions = {
    framing: "length",
    onMessage(text) {
      messages.push(text);
    },
    onBinaryImage(handle) {
      messages.push(handle);
    },
    onClose(error) {
      messages.push(error);
    },
  };
  let socketClosed = false;
  let controlHandler;
  const socketHandle = {
    send(text) {
      assert.equal(this, socketHandle, "send retains its native receiver");
      assert.equal(text, "ping", "send forwards the original text");
    },
    close() {
      assert.equal(this, socketHandle, "owner cleanup closes with the native receiver");
      socketClosed = true;
    },
  };
  const host = {
    connectUnixSocket(path, options) {
      assert.equal(this, host, "socket registration retains its native receiver");
      assert.equal(path, "/run/example.sock", "the path is unchanged");
      assert.equal(options, socketOptions, "framing and callbacks are forwarded intact");
      return socketHandle;
    },
    onControlMessage(handler) {
      assert.equal(this, host, "control registration retains its native receiver");
      controlHandler = handler;
      return () => {
        controlHandler = undefined;
      };
    },
  };
  try {
    Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value: host });
    let dispose;
    const socket = createRoot(d => {
      dispose = d;
      onControlMessage(args => (args.length ? { args } : undefined));
      return connectUnixSocket("/run/example.sock", socketOptions);
    });
    flush();
    socket.send("ping");
    socketOptions.onMessage("pong");
    socketOptions.onBinaryImage(image);
    socketOptions.onClose(null);
    assert.deepEqual(
      messages,
      ["pong", image, null],
      "socket callbacks receive text, image and close",
    );
    assert.deepEqual(
      controlHandler(["space separated", ""]),
      { args: ["space separated", ""] },
      "control arguments and JSON values are unmodified",
    );
    assert.equal(controlHandler([]), undefined, "undefined is forwarded for an empty reply");
    dispose();
    flush();
    assert.equal(socketClosed, true, "the Solid owner closes its socket");
    assert.equal(controlHandler, undefined, "the Solid owner unsubscribes its control handler");

    socketClosed = false;
    const explicit = connectUnixSocket("/run/example.sock", socketOptions);
    const unsubscribe = onControlMessage(() => "plain text");
    assert.equal(controlHandler(["status"]), "plain text", "text is passed through to the host");
    assert.equal(socketClosed, false, "unowned sockets remain open until explicitly closed");
    explicit.close();
    unsubscribe();
    assert.equal(socketClosed, true, "unowned sockets can be closed explicitly");
    assert.equal(controlHandler, undefined, "unowned handlers can be removed explicitly");
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});

test("commands keep the native handles and cancel owner-scoped work without masking failures", async () => {
  const { spawnCommand } = await import("@hewel/zgui-services");
  const { createRoot, flush } = await import("solid-js");
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  const args = ["argument with spaces", ";literal"];
  const streamed = [];
  const streamOptions = {
    stream: true,
    onStdout(line) {
      streamed.push(line);
    },
    onStderr(line) {
      streamed.push(line);
    },
    onExit(status) {
      streamed.push(status);
    },
    onError(error) {
      streamed.push(error);
    },
  };
  const cancelled = Object.assign(new Error("command cancelled"), { code: "Cancelled" });
  const failure = Object.assign(new Error("unable to start command"), { code: "Io" });
  let reject;
  let cancels = 0;
  let kills = 0;
  const request = {
    promise: new Promise((_, fail) => {
      reject = fail;
    }),
    cancel() {
      assert.equal(this, request, "buffered cleanup cancels with the native receiver");
      reject(cancelled);
    },
  };
  const stream = {
    cancel() {
      assert.equal(this, stream, "stream cleanup cancels with the native receiver");
      cancels++;
    },
    kill() {
      assert.equal(this, stream, "kill retains its native receiver");
      kills++;
    },
  };
  const host = {
    spawnCommand(program, values, options) {
      assert.equal(this, host, "command registration retains its native receiver");
      assert.equal(values, args, "arguments are forwarded intact");
      if (program === "missing") {
        assert.equal(options.stream, false, "explicit buffered mode reaches the host");
        return { promise: Promise.reject(failure), cancel() {} };
      }
      assert.equal(program, "/usr/bin/example", "the executable is forwarded intact");
      if (options === undefined) return request;
      assert.equal(options, streamOptions, "stream callbacks are forwarded intact");
      return stream;
    },
  };
  try {
    Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value: host });
    let dispose;
    const [buffered, streaming] = createRoot(d => {
      dispose = d;
      return [
        spawnCommand("/usr/bin/example", args),
        spawnCommand("/usr/bin/example", args, streamOptions),
      ];
    });
    flush();
    assert.equal(buffered, request, "the buffered return value is the original request");
    assert.equal(buffered.promise, request.promise, "the buffered promise is not replaced");
    assert.equal(streaming, stream, "the streaming return value is the original handle");
    streamOptions.onStdout("output");
    streamOptions.onStderr("warning");
    streamOptions.onExit(9);
    streamOptions.onError(failure);
    assert.deepEqual(
      streamed,
      ["output", "warning", 9, failure],
      "stream callbacks receive separate lines, exit status and host errors",
    );
    streaming.kill();
    assert.equal(kills, 1, "kill is directly available on the stream");
    dispose();
    flush();
    assert.equal(cancels, 1, "the Solid owner cancels its streaming command");
    await Bun.sleep(0);
    await assert.rejects(
      buffered.promise,
      error => error === cancelled,
      "cancellation still rejects the original promise",
    );

    const explicit = spawnCommand("/usr/bin/example", args, streamOptions);
    assert.equal(cancels, 1, "without an owner the command is not automatically cancelled");
    explicit.cancel();
    assert.equal(cancels, 2, "unowned commands can be explicitly cancelled");

    let disposeFailure;
    const failed = createRoot(d => {
      disposeFailure = d;
      return spawnCommand("missing", args, { stream: false });
    });
    await assert.rejects(
      failed.promise,
      error => error === failure,
      "ordinary command failures remain observable",
    );
    disposeFailure();
    flush();
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});
