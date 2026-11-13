import assert from "node:assert/strict";
import { test } from "node:test";

test("services defer host lookup and preserve the host request, receiver and synchronous errors", async () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "__zguiServices");
  try {
    Object.defineProperty(globalThis, "__zguiServices", {
      configurable: true,
      get() { throw new Error("Host must not be accessed during module evaluation"); },
    });
    const { readTextAsset } = await import("../../packages/services/src/index.js");
    let cancellations = 0;
    const request = { promise: Promise.resolve("text"), cancel() { cancellations++; } };
    const failure = new Error("undeclared asset");
    const host = {
      readTextAsset(asset) {
        assert.equal(this, host);
        if (asset === "assets/missing.txt") throw failure;
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
  } finally {
    if (previous) Object.defineProperty(globalThis, "__zguiServices", previous);
    else delete globalThis.__zguiServices;
  }
});
