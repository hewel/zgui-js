import assert from "node:assert/strict";
import { test } from "node:test";
import { createClock, createHost, moduleFor } from "./native-fixture.mjs";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function flushMicrotasks(rounds = 8) {
  for (let i = 0; i < rounds; i += 1) {
    await Promise.resolve();
  }
}

function trackUnhandledRejections(t) {
  const unhandled = [];
  const handler = reason => {
    unhandled.push(reason);
  };
  process.on("unhandledRejection", handler);
  t.after(() => {
    process.off("unhandledRejection", handler);
    assert.deepEqual(unhandled, [], `unexpected unhandled rejections: ${unhandled.join(", ")}`);
  });
  return unhandled;
}

test("createKvState validates admission, initial snapshot isolation, and JSON restrictions", async t => {
  trackUnhandledRejections(t);
  const h = createHost();
  const clock = createClock(t, h);
  const mod = await moduleFor(
    t,
    `
    import { renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { createKvState } from "@hewel/zgui-services";
    import type { JsonValue, KvState, KvStateStorage } from "@hewel/zgui-services";

    export function createOutsideOwner(kv: KvStateStorage) {
      return createKvState(kv, "k", { count: 0 }, { validate: raw => raw as { count: number } });
    }

    export function createInOwner(
      root: NativeNode,
      kv: KvStateStorage,
      key: string,
      initial: unknown,
      options: Record<string, unknown>,
    ) {
      let state!: KvState<JsonValue>;
      const dispose = renderApp(() => {
        state = createKvState(
          kv,
          key,
          initial as JsonValue,
          options as { validate: (raw: unknown) => JsonValue; version?: number; debounceMs?: number },
        );
        return <box />;
      }, root);
      return { state, dispose };
    }
  `,
  );
  globalThis.__zgui = h.host;
  t.after(() => {
    delete globalThis.__zgui;
  });

  let getCalls = 0;
  const noopStorage = {
    get: () => {
      getCalls += 1;
      return Promise.resolve(null);
    },
    set: () => Promise.resolve(),
  };

  // Requires active Solid owner.
  assert.throws(() => mod.createOutsideOwner(noopStorage), /active Solid owner/);
  assert.equal(getCalls, 0);

  // Invalid key, version, debounceMs, validate fail synchronously before I/O.
  const badCases = [
    ["", { a: 1 }, { validate: r => r }],
    ["k", { a: 1 }, { validate: r => r, version: 1.5 }],
    ["k", { a: 1 }, { validate: r => r, version: Number.MAX_SAFE_INTEGER + 1 }],
    ["k", { a: 1 }, { validate: r => r, debounceMs: -1 }],
    ["k", { a: 1 }, { validate: r => r, debounceMs: 1.2 }],
    ["k", { a: 1 }, { validate: r => r, debounceMs: 0x1_0000_0000 }],
    ["k", { a: 1 }, {}],
    ["k", { a: Number.NaN }, { validate: r => r }],
    ["k", { a: Number.POSITIVE_INFINITY }, { validate: r => r }],
    ["k", { fn: () => {} }, { validate: r => r }],
    ["k", new Date(), { validate: r => r }],
    ["k", new Map(), { validate: r => r }],
  ];
  for (const [key, initial, opts] of badCases) {
    assert.throws(() => mod.createInOwner(h.makeNode("root"), noopStorage, key, initial, opts));
  }
  const cyclic = { self: null };
  cyclic.self = cyclic;
  assert.throws(() =>
    mod.createInOwner(h.makeNode("root"), noopStorage, "k", cyclic, { validate: r => r }),
  );
  assert.equal(getCalls, 0, "failed admission must not trigger any storage I/O");

  // Caller mutation after construction cannot corrupt owned frozen snapshot.
  const mutableInitial = { nested: { count: 1 }, list: [1, 2] };
  const { state, dispose } = mod.createInOwner(
    h.makeNode("root"),
    noopStorage,
    "k",
    mutableInitial,
    {
      validate: r => r,
    },
  );
  t.after(() => dispose());
  mutableInitial.nested.count = 999;
  mutableInitial.list.push(3);
  assert.deepEqual(state.value(), { nested: { count: 1 }, list: [1, 2] });
  assert.equal(Object.isFrozen(state.value()), true);
  assert.equal(Object.isFrozen(state.value().nested), true);
  assert.equal(Object.isFrozen(state.value().list), true);
  assert.equal(getCalls, 1);
  assert.equal(clock.timers.size, 0);
});

test("createKvState handles hydration miss, mismatch, null data, malformed envelopes, read errors, and early-edit dominance", async t => {
  trackUnhandledRejections(t);
  const h = createHost();
  const clock = createClock(t, h);
  const mod = await moduleFor(
    t,
    `
    import { renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { createKvState } from "@hewel/zgui-services";
    import type { JsonValue, KvState, KvStateStorage } from "@hewel/zgui-services";

    export function mountKv<T extends JsonValue>(
      root: NativeNode,
      kv: KvStateStorage,
      key: string,
      initial: T,
      validate: (raw: unknown) => T,
      version?: number,
      debounceMs?: number,
    ) {
      let state!: KvState<T>;
      const dispose = renderApp(() => {
        state = createKvState(kv, key, initial, { validate, version, debounceMs });
        return <box />;
      }, root);
      return { state, dispose };
    }
  `,
  );
  globalThis.__zgui = h.host;
  t.after(() => {
    delete globalThis.__zgui;
  });

  // 1) Missing key -> keeps defaults, no error, no write, clean flush resolves without writing.
  {
    const getDef = deferred();
    const writes = [];
    const kv = {
      get: () => getDef.promise,
      set: (k, v) => {
        writes.push([k, v]);
        return Promise.resolve();
      },
    };
    const { state, dispose } = mod.mountKv(h.makeNode("root"), kv, "prefs", { a: 1 }, r => r);
    t.after(() => dispose());
    assert.deepEqual(state.value(), { a: 1 });
    assert.equal(state.hydrated(), false);
    const cleanFlush = state.flush();
    getDef.resolve(null);
    await cleanFlush;
    assert.equal(state.hydrated(), true);
    assert.equal(state.error(), undefined);
    assert.deepEqual(state.value(), { a: 1 });
    assert.equal(writes.length, 0);
  }

  // 2) Well-formed version mismatch -> keeps defaults, no error, no write.
  {
    const writes = [];
    const kv = {
      get: () => Promise.resolve({ v: 2, data: { a: 99 } }),
      set: (k, v) => {
        writes.push([k, v]);
        return Promise.resolve();
      },
    };
    const { state, dispose } = mod.mountKv(h.makeNode("root"), kv, "prefs", { a: 1 }, r => r, 1);
    t.after(() => dispose());
    await flushMicrotasks();
    assert.equal(state.hydrated(), true);
    assert.equal(state.error(), undefined);
    assert.deepEqual(state.value(), { a: 1 });
    assert.equal(writes.length, 0);
  }

  // 3) Stored data: null is valid when validate accepts null.
  {
    const kv = {
      get: () => Promise.resolve({ v: 1, data: null }),
      set: () => Promise.resolve(),
    };
    const { state, dispose } = mod.mountKv(h.makeNode("root"), kv, "nullable", "default", r =>
      r === null || typeof r === "string"
        ? r
        : (() => {
            throw new TypeError("bad");
          })(),
    );
    t.after(() => dispose());
    await flushMicrotasks();
    assert.equal(state.hydrated(), true);
    assert.equal(state.error(), undefined);
    assert.equal(state.value(), null);
  }

  // 4) Malformed envelope, rejected read, and throwing validate surface error() without auto-repair;
  // clean flush after read-only failure still resolves without misreporting a write failure!
  for (const scenario of [
    { name: "malformed", resolveWith: { v: 1, data: { a: 2 }, extra: true } },
    { name: "missing-data", resolveWith: { v: 1 } },
    { name: "bad-validate", resolveWith: { v: 1, data: "wrong-type" } },
    { name: "read-reject", rejectWith: new Error("disk read failed") },
  ]) {
    const writes = [];
    const kv = {
      get: () =>
        scenario.rejectWith
          ? Promise.reject(scenario.rejectWith)
          : Promise.resolve(scenario.resolveWith),
      set: (k, v) => {
        writes.push([k, v]);
        return Promise.resolve();
      },
    };
    const validate = raw => {
      if (raw === null || typeof raw !== "object" || Array.isArray(raw)) {
        throw new TypeError("expected object");
      }
      return raw;
    };
    const { state, dispose } = mod.mountKv(h.makeNode("root"), kv, "prefs", { a: 1 }, validate);
    t.after(() => dispose());
    await state.flush(); // Must resolve (clean flush never misreports hydration failure as write failure)
    assert.equal(state.hydrated(), true, `${scenario.name}: hydrated`);
    assert.ok(state.error() instanceof Error, `${scenario.name}: error surfaced`);
    assert.deepEqual(state.value(), { a: 1 }, `${scenario.name}: defaults kept`);
    assert.equal(writes.length, 0, `${scenario.name}: no auto-repair write`);
  }

  // 5) Early edit before hydration finishes wins as a whole record, waits for hydration, and still writes after read failure!
  {
    const getDef = deferred();
    const setDef = deferred();
    const writes = [];
    const kv = {
      get: () => getDef.promise,
      set: (k, v) => {
        writes.push([k, v]);
        return setDef.promise;
      },
    };
    const { state, dispose } = mod.mountKv(
      h.makeNode("root"),
      kv,
      "prefs",
      { scheme: "dark", preset: "base" },
      r => r,
      1,
      50,
    );
    t.after(() => dispose());

    // Two same-turn functional updaters compose on last accepted local snapshot (NOT stale batched value()).
    h.runTurn(() => {
      state.set(prev => ({ ...prev, preset: "ocean" }));
      state.set(prev => ({ ...prev, scheme: "light" }));
    });
    assert.deepEqual(state.value(), { scheme: "light", preset: "ocean" });

    // Even if debounce elapses before hydration completes, write MUST wait for hydration!
    clock.advance(50);
    assert.equal(writes.length, 0, "writes must wait for hydration attempt to finish");

    // Hydration rejects (read failure) -> early edit still wins and proceeds to write!
    getDef.reject(new Error("read failed"));
    await flushMicrotasks();
    assert.equal(state.hydrated(), true);
    assert.deepEqual(state.value(), { scheme: "light", preset: "ocean" });
    assert.equal(writes.length, 1);
    assert.deepEqual(writes[0], ["prefs", { v: 1, data: { scheme: "light", preset: "ocean" } }]);
    assert.equal(state.saving(), true);

    setDef.resolve();
    await flushMicrotasks();
    assert.equal(state.saving(), false);
    assert.equal(state.error(), undefined);
  }
});

test("createKvState serializes writes, coalesces pending snapshots, shares flush barriers, handles failures and disposal", async t => {
  trackUnhandledRejections(t);
  const h = createHost();
  const clock = createClock(t, h);
  const mod = await moduleFor(
    t,
    `
    import { renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { createKvState } from "@hewel/zgui-services";
    import type { JsonValue, KvState, KvStateStorage } from "@hewel/zgui-services";

    export function mountKv(root: NativeNode, kv: KvStateStorage, debounceMs = 100) {
      let state!: KvState<{ count: number }>;
      const dispose = renderApp(() => {
        state = createKvState(
          kv,
          "counter",
          { count: 0 },
          {
            validate: raw => {
              if (!raw || typeof raw !== "object" || typeof (raw as { count?: unknown }).count !== "number") {
                throw new TypeError("bad counter");
              }
              const count = (raw as { count: number }).count;
              if (count < 0) throw new RangeError("negative");
              return { count };
            },
            version: 3,
            debounceMs,
          },
        );
        return <box />;
      }, root);
      return { state, dispose };
    }
  `,
  );
  globalThis.__zgui = h.host;
  t.after(() => {
    delete globalThis.__zgui;
  });

  const writes = [];
  let closeCalls = 0;
  const kv = {
    get() {
      assert.equal(this, kv, "kv.get preserves storage receiver");
      return Promise.resolve({ v: 3, data: { count: 10 } });
    },
    set(key, value) {
      assert.equal(this, kv, "kv.set preserves storage receiver");
      const d = deferred();
      writes.push({ key, value, d });
      return d.promise;
    },
    close() {
      closeCalls += 1;
    },
  };

  const { state, dispose } = mod.mountKv(h.makeNode("root"), kv, 100);
  await flushMicrotasks();
  assert.deepEqual(state.value(), { count: 10 });
  assert.equal(state.hydrated(), true);

  // Throwing updater/validator leaves state and queue untouched.
  assert.throws(() => state.set({ count: -5 }), RangeError);
  assert.throws(
    () =>
      state.set(() => {
        throw new Error("updater boom");
      }),
    /updater boom/,
  );
  assert.deepEqual(state.value(), { count: 10 });
  assert.equal(clock.timers.size, 0);

  // Trailing debounce + at most 1 in-flight write + coalescing latest pending snapshot.
  state.set(p => ({ count: p.count + 1 })); // 11
  clock.advance(50);
  state.set(p => ({ count: p.count + 1 })); // 12 (resets trailing debounce)
  clock.advance(50);
  assert.equal(writes.length, 0, "trailing debounce not elapsed yet");
  clock.advance(50);
  assert.equal(writes.length, 1);
  assert.deepEqual(writes[0].value, { v: 3, data: { count: 12 } });
  assert.equal(state.saving(), true);

  // While write 0 (count=12) is in flight, queue count=13 then count=14 and call concurrent flushes.
  state.set({ count: 13 });
  const flushA = state.flush();
  state.set({ count: 14 });
  const flushB = state.flush();
  assert.equal(writes.length, 1, "at most one kv.set in flight");

  // Write 0 completes -> coalesces count=13 and count=14 into ONE follow-up write (count=14)!
  writes[0].d.resolve();
  await flushMicrotasks();
  assert.equal(writes.length, 2);
  assert.deepEqual(writes[1].value, { v: 3, data: { count: 14 } });
  assert.equal(state.saving(), true);

  // Write 1 fails -> retains dirty snapshot (14), publishes error, rejects waiting barriers, stops auto-retries!
  writes[1].d.reject(new Error("disk full"));
  await assert.rejects(flushA, /disk full/);
  await assert.rejects(flushB, /disk full/);
  assert.equal(state.saving(), false);
  assert.equal(state.error()?.message, "disk full");
  assert.deepEqual(state.value(), { count: 14 });
  clock.advance(10_000);
  assert.equal(writes.length, 2, "must not auto-retry after write failure");

  // Explicit flush() retries dirty snapshot and clears error() on success!
  const retryFlush = state.flush();
  assert.equal(writes.length, 3);
  assert.deepEqual(writes[2].value, { v: 3, data: { count: 14 } });
  writes[2].d.resolve();
  await retryFlush;
  assert.equal(state.error(), undefined);
  assert.equal(state.saving(), false);

  // reset() queues captured initial snapshot ({ count: 0 }) at current version.
  state.reset();
  assert.deepEqual(state.value(), { count: 0 });
  const resetFlush = state.flush();
  assert.equal(writes.length, 4);
  assert.deepEqual(writes[3].value, { v: 3, data: { count: 0 } });

  // Disposal cancels debounce, rejects waiting & future barriers, observes in-flight rejection, never closes storage!
  dispose();
  await assert.rejects(resetFlush, /disposed/);
  await assert.rejects(() => state.flush(), /disposed/);
  assert.equal(state.saving(), false);
  assert.equal(closeCalls, 0, "createKvState must never close borrowed storage");

  // Late rejection of already-started write after disposal must NOT cause unhandled rejection!
  writes[3].d.reject(new Error("late failure after dispose"));
  await flushMicrotasks();
});

test("md-editor createLegacyPrefsStorage bridges 5 legacy keys strictly on missing settings key and propagates errors", async t => {
  trackUnhandledRejections(t);
  const { createLegacyPrefsStorage, validateStudioPrefs, DEFAULT_PREFS } =
    await import("../../apps/examples/md-editor/src/storage.ts");

  // 1) Missing settings + present legacy 5 keys -> synthesizes v1 envelope.
  const legacyMap = new Map([
    ["scheme", "light"],
    ["preset", "ocean"],
    ["fontSize", 18],
    ["splitSizes", [20, 40, 40]],
    ["lastWorkspaceBookmark", "bm-1"],
  ]);
  const bridged = createLegacyPrefsStorage({
    get: async k => (legacyMap.has(k) ? legacyMap.get(k) : null),
    set: async (k, v) => {
      legacyMap.set(k, v);
    },
  });
  const migrated = await bridged.get("settings");
  assert.deepEqual(migrated, {
    v: 1,
    data: {
      scheme: "light",
      preset: "ocean",
      fontSize: 18,
      splitSizes: [20, 40, 40],
      lastWorkspaceBookmark: "bm-1",
    },
  });
  assert.deepEqual(validateStudioPrefs(migrated.data), migrated.data);

  // 2) Once unified settings key exists, legacy keys are NOT consulted.
  let legacyReads = 0;
  const unifiedMap = new Map([
    ["settings", { v: 1, data: { ...DEFAULT_PREFS, preset: "terminal" } }],
    ["preset", "ember"],
  ]);
  const bridgedUnified = createLegacyPrefsStorage({
    get: async k => {
      if (k !== "settings") legacyReads += 1;
      return unifiedMap.has(k) ? unifiedMap.get(k) : null;
    },
    set: async () => {
      throw new Error("write boom");
    },
  });
  const unified = await bridgedUnified.get("settings");
  assert.equal(legacyReads, 0);
  assert.equal(unified.data.preset, "terminal");
  await assert.rejects(() => bridgedUnified.set("settings", unified), /write boom/);
});
