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
