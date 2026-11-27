import assert from "node:assert/strict";
import { test } from "node:test";

test("services defer host lookup and preserve the host request, receiver and synchronous errors", async () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  try {
    Object.defineProperty(globalThis, "__zguiServices", {
      configurable: true,
      get() {
        throw new Error("Host must not be accessed during module evaluation");
      },
    });
    const { loadTextAsset, readTextAsset } = await import("@hewel/zgui-services");
    let cancellations = 0;
    const request = {
      promise: Promise.resolve("text"),
      cancel() {
        cancellations++;
      },
    };
    const failure = new Error("undeclared asset");
    const cancelledError = Object.assign(new Error("request cancelled"), { code: "Cancelled" });
    let pendingReject;
    const host = {
      readTextAsset(asset) {
        assert.equal(this, host);
        if (asset === "assets/missing.txt") throw failure;
        if (asset === "assets/cancelled.txt") {
          const cancelledPromise = new Promise((_, reject) => {
            pendingReject = reject;
          });
          return {
            promise: cancelledPromise,
            cancel() {
              cancellations++;
              pendingReject(cancelledError);
            },
          };
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
    assert.throws(
      () => readTextAsset("assets/missing.txt"),
      error => error === failure,
    );

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
    assert.throws(
      () => loadImageAsset("assets/missing.png"),
      error => error === failure,
    );
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});

test("byte image handles release with their owner unless explicitly registered", async () => {
  const { loadImageBytes, registerImageBytes } = await import("@hewel/zgui-services");
  const { createRoot, flush } = await import("solid-js");
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  const bytes = new Uint8Array([1, 2, 3]).subarray(1);
  const live = new Set();
  const failure = new RangeError("image bytes are empty");
  const host = {
    registerImageBytes(input) {
      assert.equal(this, host);
      if (input.length === 0) throw failure;
      assert.equal(input, bytes, "forward the view without copying it in JS");
      const handle = {
        src: `zgui-blob://${live.size}`,
        release() {
          live.delete(this);
        },
      };
      live.add(handle);
      return handle;
    },
  };
  Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value: host });
  try {
    const unscoped = loadImageBytes(bytes);
    let dispose;
    const [explicit, scoped] = createRoot(d => {
      dispose = d;
      return [registerImageBytes(bytes), loadImageBytes(bytes)];
    });
    flush();
    assert.deepEqual([...live], [unscoped, explicit, scoped]);
    dispose();
    flush();
    assert.deepEqual([...live], [unscoped, explicit]);
    explicit.release();
    unscoped.release();
    assert.equal(live.size, 0);
    assert.throws(
      () => loadImageBytes(new Uint8Array()),
      error => error === failure,
    );
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
      get() {
        throw new Error("Host must not be accessed during module evaluation");
      },
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
      imagePrefixes: [],
      unixSockets: [],
      namedSockets: { niri: null },
    });
    assert.equal(getAuthorizedEndpoint("niri"), null);
    assert.throws(
      () => getAuthorizedEndpoint("other"),
      error => error === denied,
    );
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});

test("locale calls resolve the current session and preserve synchronous range errors", async () => {
  const { getSystemTimeZone, getSystemLocale, fromZonedDateTime, collatorSortIndices } =
    await import("@hewel/zgui-services");
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  const preflight = new TypeError("host work is unavailable during preflight");
  const ambiguous = new RangeError("ambiguous wall time");
  const install = value =>
    Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value });
  try {
    install({
      getSystemTimeZone() {
        throw preflight;
      },
    });
    assert.throws(getSystemTimeZone, error => error === preflight);
    install({
      getSystemTimeZone: () => "Asia/Shanghai",
      getSystemLocale: () => "zh-CN",
      fromZonedDateTime() {
        throw ambiguous;
      },
      collatorSortIndices: () => Uint32Array.of(1, 0),
    });
    assert.deepEqual([getSystemTimeZone(), getSystemLocale()], ["Asia/Shanghai", "zh-CN"]);
    assert.throws(
      () =>
        fromZonedDateTime({ year: 2026, month: 11, day: 1, hour: 1 }, "America/New_York", {
          disambiguation: "reject",
        }),
      error => error === ambiguous,
    );
    const values = ["boot-10", "boot-2"];
    const indices = collatorSortIndices(values);
    assert.ok(indices instanceof Uint32Array);
    assert.deepEqual(
      Array.from(indices, i => values[i]),
      ["boot-2", "boot-10"],
    );
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

test("clipboard services preserve receiver, options, owner cancellation, and image handle cleanup", async () => {
  const {
    clipboardReadImage,
    clipboardReadText,
    readClipboardImage,
    readClipboardText,
    writeClipboardImage,
    writeClipboardText,
  } = await import("@hewel/zgui-services");
  const { createRoot, flush } = await import("solid-js");
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  const cancelledError = Object.assign(new Error("clipboard request cancelled"), {
    code: "Cancelled",
  });
  let textCancels = 0;
  let imageReleases = 0;
  let lastWriteText = null;
  let lastWriteImage = null;
  let resolveLateImage;
  const host = {
    readClipboardText() {
      assert.equal(this, host);
      let rejectFn;
      const promise = new Promise((_, reject) => {
        rejectFn = reject;
      });
      return {
        promise,
        cancel() {
          textCancels++;
          rejectFn(cancelledError);
        },
      };
    },
    writeClipboardText(text, options) {
      assert.equal(this, host);
      lastWriteText = { text, options };
      return { promise: Promise.resolve(), cancel() {} };
    },
    readClipboardImage() {
      assert.equal(this, host);
      const promise = new Promise(resolve => {
        resolveLateImage = resolve;
      });
      return { promise, cancel() {} };
    },
    writeClipboardImage(source, options) {
      assert.equal(this, host);
      lastWriteImage = { source, options };
      return { promise: Promise.resolve(), cancel() {} };
    },
  };
  try {
    Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value: host });
    await writeClipboardText("secret-value", { secret: true }).promise;
    assert.deepEqual(lastWriteText, { text: "secret-value", options: { secret: true } });

    const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47]);
    await writeClipboardImage(bytes, { secret: false }).promise;
    assert.deepEqual(lastWriteImage, { source: bytes, options: { secret: false } });

    const raw = readClipboardText();
    raw.cancel();
    assert.equal(textCancels, 1);
    await assert.rejects(raw.promise, err => err === cancelledError);

    let disposeText;
    const scopedText = createRoot(d => {
      disposeText = d;
      return clipboardReadText();
    });
    flush();
    disposeText();
    flush();
    assert.equal(textCancels, 2);
    assert.equal(await scopedText, null);

    let disposeImg;
    const scopedImg = createRoot(d => {
      disposeImg = d;
      return clipboardReadImage();
    });
    flush();
    disposeImg();
    flush();
    // Late resolution after owner cleanup must release the ImageHandle immediately.
    resolveLateImage({
      src: "zgui-blob://1-1",
      release() {
        imageReleases++;
      },
    });
    assert.equal(await scopedImg, null);
    assert.equal(imageReleases, 1);

    const rawImg = readClipboardImage();
    resolveLateImage({
      src: "zgui-blob://1-2",
      release() {
        imageReleases++;
      },
    });
    const imgHandle = await rawImg.promise;
    assert.equal(imgHandle?.src, "zgui-blob://1-2");
    imgHandle?.release();
    assert.equal(imageReleases, 2);
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});

test("SQLite openDatabase and openKvStore close with owner and roll back on transaction failure or timeout", async () => {
  const { openDatabase, openKvStore } = await import("@hewel/zgui-services");
  const { createRoot, flush } = await import("solid-js");
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  let closedCount = 0;
  let rollbacks = 0;
  let commits = 0;
  let triggerTimeout;
  const rows = new Map();
  const host = {
    openDatabase(name) {
      assert.equal(this, host);
      return {
        name,
        query(sql, params) {
          if (sql.includes("FROM _zgui_kv WHERE namespace = ?1 AND key = ?2")) {
            const val = rows.get(`${params[0]}:${params[1]}`);
            return {
              promise: Promise.resolve(val === undefined ? [] : [{ value_json: val }]),
              cancel() {},
            };
          }
          return { promise: Promise.resolve([]), cancel() {} };
        },
        execute(sql, params) {
          if (sql.includes("INSERT INTO _zgui_kv")) {
            rows.set(`${params[0]}:${params[1]}`, params[2]);
            return {
              promise: Promise.resolve({ rowsAffected: 1, lastInsertRowid: 1 }),
              cancel() {},
            };
          }
          if (sql.includes("DELETE FROM _zgui_kv WHERE namespace = ?1 AND key = ?2")) {
            const existed = rows.delete(`${params[0]}:${params[1]}`);
            return {
              promise: Promise.resolve({ rowsAffected: existed ? 1 : 0, lastInsertRowid: null }),
              cancel() {},
            };
          }
          return {
            promise: Promise.resolve({ rowsAffected: 0, lastInsertRowid: null }),
            cancel() {},
          };
        },
        batch() {
          return { promise: Promise.resolve([]), cancel() {} };
        },
        __beginTransaction() {
          const timeoutPromise = new Promise((_, reject) => {
            triggerTimeout = reject;
          });
          return {
            beginPromise: Promise.resolve(),
            timeoutPromise,
            query() {
              return { promise: Promise.resolve([]), cancel() {} };
            },
            execute() {
              return {
                promise: Promise.resolve({ rowsAffected: 1, lastInsertRowid: 1 }),
                cancel() {},
              };
            },
            commit() {
              commits++;
              return { promise: Promise.resolve(), cancel() {} };
            },
            rollback() {
              rollbacks++;
              return { promise: Promise.resolve(), cancel() {} };
            },
            abort() {},
          };
        },
        close() {
          closedCount++;
        },
      };
    },
  };
  try {
    Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value: host });
    let dispose;
    const { db, kv } = createRoot(d => {
      dispose = d;
      return { db: openDatabase("main"), kv: openKvStore("main", "ns") };
    });
    flush();

    // Commit path
    const res = await db.transaction(async tx => {
      await tx.execute("INSERT INTO t VALUES (1)");
      return 42;
    });
    assert.equal(res, 42);
    assert.equal(commits, 1);

    // JS throw auto-rollback
    await assert.rejects(
      db.transaction(async () => {
        throw new Error("boom");
      }),
      /boom/,
    );
    assert.equal(rollbacks, 1);

    // Host timeout race rejects immediately with E_TX_TIMEOUT
    const timeoutErr = Object.assign(new Error("timed out"), { code: "E_TX_TIMEOUT" });
    const pendingTx = db.transaction(async () => new Promise(() => {}));
    await Bun.sleep(0);
    triggerTimeout(timeoutErr);
    await assert.rejects(pendingTx, err => err === timeoutErr);

    // KV wrapper roundtrip
    await kv.set("k1", { ok: true });
    assert.deepEqual(await kv.get("k1"), { ok: true });
    assert.equal(await kv.delete("k1"), true);
    assert.equal(await kv.get("k1"), null);

    dispose();
    flush();
    assert.equal(closedCount, 2);
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});

test("files openAuthorizedDirectory and watchDirectory register Solid owner cleanup and forward options", async () => {
  const { openAuthorizedDirectory, watchDirectory } = await import("@hewel/zgui-services");
  const { createRoot, flush } = await import("solid-js");
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  let closed = 0;
  let stopped = 0;
  let seenWatchOptions = null;
  const host = {
    openAuthorizedDirectory(path) {
      assert.equal(this, host);
      assert.equal(path, "$XDG_DOCUMENTS_DIR/notes");
      return {
        kind: "dir",
        name: "notes",
        mode: "readwrite",
        close() {
          closed++;
        },
      };
    },
    watchDirectory(dir, options, cb) {
      assert.equal(this, host);
      assert.equal(dir.name, "notes");
      seenWatchOptions = options;
      cb([{ path: "a.md", kind: "modify" }]);
      return () => {
        stopped++;
      };
    },
  };
  try {
    Object.defineProperty(globalThis, "__zguiServices", { configurable: true, value: host });
    const received = [];
    let dispose;
    createRoot(d => {
      dispose = d;
      const dir = openAuthorizedDirectory("$XDG_DOCUMENTS_DIR/notes");
      watchDirectory(dir, { recursive: true, debounceMs: 80 }, events => {
        received.push(...events);
      });
    });
    flush();
    assert.deepEqual(seenWatchOptions, { recursive: true, debounceMs: 80 });
    assert.deepEqual(received, [{ path: "a.md", kind: "modify" }]);
    dispose();
    flush();
    assert.equal(closed, 1);
    assert.equal(stopped, 1);
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});
