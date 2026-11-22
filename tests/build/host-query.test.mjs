import assert from "node:assert/strict";
import { test } from "node:test";
import { createHost, moduleFor } from "./native-fixture.mjs";

function makeRequest(label = "req") {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  let cancelCount = 0;
  const request = {
    label,
    promise,
    get cancelCount() {
      return cancelCount;
    },
    cancel() {
      assert.equal(this, request, "cancel() must be invoked with HostRequest as receiver");
      cancelCount += 1;
      const err = Object.assign(new Error(`Cancelled ${label}`), { code: "Cancelled" });
      reject(err);
    },
  };
  return { request, resolve, reject };
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

test("createHostQuery requires owner, tracks only source/refetch, treats 0/empty-string as enabled and null/undefined/false as disabled", async t => {
  trackUnhandledRejections(t);
  const h = createHost();
  const mod = await moduleFor(
    t,
    `
    import { createSignal, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { createHostQuery } from "@hewel/zgui-services";
    import type { HostQuery, HostRequest } from "@hewel/zgui-services";

    export function createOutsideOwner() {
      return createHostQuery(
        () => "a",
        () => ({ promise: Promise.resolve("x"), cancel() {} }),
      );
    }

    const [source, setSource] = createSignal<string | number | null | undefined | false>(false);
    const [incidental, setIncidental] = createSignal(0);
    export { setSource, setIncidental };
    export let query!: HostQuery<unknown>;
    export const fetches: Array<string | number> = [];
    export const errors: Error[] = [];
    let nextFetcher: (input: string | number) => HostRequest<unknown> = () => ({
      promise: Promise.resolve(undefined),
      cancel() {},
    });
    export function setFetcher(fn: (input: string | number) => HostRequest<unknown>) {
      nextFetcher = fn;
    }

    export function mount(root: NativeNode, keepPreviousData = true, initialValue: unknown = "init") {
      return renderApp(() => {
        query = createHostQuery(
          source,
          input => {
            incidental(); // Incidental reactive read MUST NOT be tracked!
            fetches.push(input);
            return nextFetcher(input);
          },
          {
            initialValue,
            keepPreviousData,
            onError: err => {
              incidental(); // Incidental reactive read MUST NOT be tracked!
              errors.push(err);
            },
          },
        );
        return <box />;
      }, root);
    }
  `,
  );
  globalThis.__zgui = h.host;
  t.after(() => {
    delete globalThis.__zgui;
  });

  assert.throws(() => mod.createOutsideOwner(), /active Solid owner/);

  const requests = [];
  mod.setFetcher(input => {
    const r = makeRequest(String(input));
    requests.push(r);
    return r.request;
  });

  const dispose = mod.mount(h.makeNode("root"));
  t.after(() => dispose());

  assert.equal(mod.query.data(), "init");
  assert.equal(mod.query.loading(), false);
  assert.equal(mod.query.error(), undefined);
  assert.equal(mod.fetches.length, 0);

  // 0 and "" are ENABLED!
  h.runTurn(() => mod.setSource(0));
  assert.deepEqual(mod.fetches, [0]);
  assert.equal(mod.query.loading(), true);
  requests[0].resolve("zero-result");
  await flushMicrotasks();
  assert.equal(mod.query.data(), "zero-result");
  assert.equal(mod.query.loading(), false);

  h.runTurn(() => mod.setSource(""));
  assert.deepEqual(mod.fetches, [0, ""]);
  assert.equal(mod.query.loading(), true);
  assert.equal(mod.query.data(), "zero-result", "keepPreviousData defaults to true");

  // Callable payload T is stored as data, NOT invoked as Solid updater!
  let callableRuns = 0;
  const payloadFn = () => {
    callableRuns += 1;
    return "should-not-run";
  };
  requests[1].resolve(payloadFn);
  await flushMicrotasks();
  assert.equal(mod.query.data(), payloadFn);
  assert.equal(callableRuns, 0);
  assert.equal(mod.query.loading(), false);

  // Incidental signal update does NOT re-trigger query!
  h.runTurn(() => mod.setIncidental(1));
  assert.equal(mod.fetches.length, 2);

  // Same-turn source change + refetch coalesces into ONE launch!
  h.runTurn(() => {
    mod.setSource("coalesced");
    mod.query.refetch();
  });
  assert.deepEqual(mod.fetches, [0, "", "coalesced"]);
  assert.equal(requests.length, 3);
  assert.equal(mod.query.loading(), true);

  // Disabling (null/undefined/false) cancels active request, resets data to initialValue regardless of keepPreviousData, clears error & loading.
  h.runTurn(() => mod.setSource(null));
  assert.equal(requests[2].request.cancelCount, 1);
  assert.equal(
    mod.query.data(),
    "init",
    "disabling resets data to initialValue even when keepPreviousData=true",
  );
  assert.equal(mod.query.loading(), false);
  assert.equal(mod.query.error(), undefined);
  await flushMicrotasks();
  assert.equal(mod.errors.length, 0, "own cancellation must not surface error");
});

test("createHostQuery handles out-of-order cancellation, keepPreviousData: false, current Cancelled error, sync throws, bare Promise rejection, and disposal", async t => {
  trackUnhandledRejections(t);
  const h = createHost();
  const mod = await moduleFor(
    t,
    `
    import { createSignal, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { createHostQuery } from "@hewel/zgui-services";
    import type { HostQuery, HostRequest } from "@hewel/zgui-services";

    const [source, setSource] = createSignal<string | false>("first");
    export { setSource };
    export let query!: HostQuery<string>;
    export const errors: Error[] = [];
    let activeFetcher: (input: string) => HostRequest<string> = () => ({
      promise: Promise.resolve(""),
      cancel() {},
    });
    export function setFetcher(fn: (input: string) => HostRequest<string>) {
      activeFetcher = fn;
    }

    export function mount(root: NativeNode, keepPreviousData: boolean) {
      return renderApp(() => {
        query = createHostQuery(
          source,
          input => activeFetcher(input),
          {
            initialValue: "empty",
            keepPreviousData,
            onError: err => {
              errors.push(err);
            },
          },
        );
        return <box />;
      }, root);
    }
  `,
  );
  globalThis.__zgui = h.host;
  t.after(() => {
    delete globalThis.__zgui;
  });

  // Create requests whose cancel() does NOT synchronously reject, to test late out-of-order settlement!
  const makeSlowRequest = label => {
    let resolve;
    let reject;
    const promise = new Promise((res, rej) => {
      resolve = res;
      reject = rej;
    });
    let cancelCalls = 0;
    const request = {
      label,
      promise,
      get cancelCalls() {
        return cancelCalls;
      },
      cancel() {
        assert.equal(this, request);
        cancelCalls += 1;
      },
    };
    return { request, resolve, reject };
  };

  const issued = [];
  mod.setFetcher(input => {
    const r = makeSlowRequest(input);
    issued.push(r);
    return r.request;
  });

  const dispose = mod.mount(h.makeNode("root"), false);

  // Initial enabled source launched via Solid effect.
  assert.equal(issued.length, 1);
  assert.equal(mod.query.loading(), true);
  assert.equal(mod.query.data(), "empty");

  // Supersede first with second BEFORE first settles.
  h.runTurn(() => mod.setSource("second"));
  assert.equal(issued[0].request.cancelCalls, 1);
  assert.equal(issued.length, 2);
  assert.equal(mod.query.loading(), true);

  // Superseded first resolves LATE -> discarded AND must NOT clear loading() of second!
  issued[0].resolve("stale-first");
  await flushMicrotasks();
  assert.equal(mod.query.data(), "empty", "stale first resolution discarded");
  assert.equal(
    mod.query.loading(),
    true,
    "stale settlement must never clear loading for active request",
  );

  // Second succeeds.
  issued[1].resolve("fresh-second");
  await flushMicrotasks();
  assert.equal(mod.query.data(), "fresh-second");
  assert.equal(mod.query.loading(), false);
  assert.equal(mod.query.error(), undefined);

  // With keepPreviousData: false, starting third restores initialValue immediately,
  // and if third rejects with an EXTERNAL Cancelled error (NOT cancelled by createHostQuery!),
  // it MUST surface in error() and onError (never blindly swallow code === "Cancelled")!
  h.runTurn(() => mod.setSource("third"));
  assert.equal(mod.query.data(), "empty", "keepPreviousData: false restores initialValue at start");
  assert.equal(mod.query.loading(), true);
  const externalCancelled = Object.assign(new Error("Upstream cancelled"), { code: "Cancelled" });
  issued[2].reject(externalCancelled);
  await flushMicrotasks();
  assert.equal(mod.query.loading(), false);
  assert.equal(mod.query.error(), externalCancelled);
  assert.equal(mod.query.error().code, "Cancelled");
  assert.equal(mod.query.data(), "empty");
  assert.deepEqual(mod.errors, [externalCancelled]);

  // Bare Promise fetcher is rejected with TypeError.
  mod.setFetcher(() => Promise.resolve("bare-promise"));
  h.runTurn(() => mod.setSource("bare"));
  assert.equal(mod.query.loading(), false);
  assert.ok(mod.query.error() instanceof TypeError);
  assert.equal(mod.errors.length, 2);

  // Synchronous non-Error throw from fetcher is wrapped in Error and invokes onError once.
  mod.setFetcher(() => {
    throw "sync-string-boom";
  });
  h.runTurn(() => mod.setSource("sync-boom"));
  assert.equal(mod.query.loading(), false);
  assert.equal(mod.query.error()?.message, "sync-string-boom");
  assert.equal(mod.errors.length, 3);

  // Disposal invalidates, cancels pending handle, clears loading, makes later refetch() inert.
  mod.setFetcher(input => {
    const r = makeSlowRequest(input);
    issued.push(r);
    return r.request;
  });
  h.runTurn(() => mod.setSource("pending-before-dispose"));
  const lastIssued = issued.at(-1);
  assert.equal(mod.query.loading(), true);
  dispose();
  assert.equal(lastIssued.request.cancelCalls, 1);
  assert.equal(mod.query.loading(), false);

  // Late rejection & refetch after disposal stay completely inert.
  lastIssued.reject(new Error("late after dispose"));
  mod.query.refetch();
  await flushMicrotasks();
  assert.equal(issued.length, 4, "refetch after disposal is inert");
  assert.equal(mod.errors.length, 3, "no onError after disposal");
});

test("md-editor searchNotes gates on FTS readiness and cancels superseded slow queries via createHostQuery", async t => {
  trackUnhandledRejections(t);
  const h = createHost();
  const queryRequests = [];
  const initDef = makeRequest("fts-init");

  const prevServices = globalThis.__zguiServices;
  globalThis.__zguiServices = {
    openDatabase(name) {
      return {
        name,
        execute() {
          return initDef.request;
        },
        query(_sql, params) {
          const req = makeRequest(String(params?.[0] ?? ""));
          queryRequests.push(req);
          return req.request;
        },
        batch() {
          return { promise: Promise.resolve([]), cancel() {} };
        },
        __beginTransaction() {
          throw new Error("unused");
        },
        close() {},
      };
    },
  };
  t.after(() => {
    if (prevServices === undefined) delete globalThis.__zguiServices;
    else globalThis.__zguiServices = prevServices;
  });

  const mod = await moduleFor(
    t,
    `
    import { createSignal, onCleanup, renderApp } from "@hewel/zgui";
    import type { NativeNode } from "@hewel/zgui";
    import { createHostQuery } from "@hewel/zgui-services";
    import type { HostQuery } from "@hewel/zgui-services";
    import { createStudioStorage, sanitizeFtsQuery } from ${JSON.stringify(
      `${process.cwd()}/apps/examples/md-editor/src/storage.ts`,
    )};
    import type { SearchHit, StudioStorage } from ${JSON.stringify(
      `${process.cwd()}/apps/examples/md-editor/src/storage.ts`,
    )};

    const [ready, setReady] = createSignal(false);
    const [text, setText] = createSignal("Wayland");
    export { setReady, setText };
    export let storage!: StudioStorage;
    export let search!: HostQuery<readonly SearchHit[]>;

    export function mount(root: NativeNode) {
      return renderApp(() => {
        storage = createStudioStorage();
        onCleanup(() => storage.close());
        search = createHostQuery(
          () => (ready() && sanitizeFtsQuery(text()).length > 0 ? text() : false),
          q => storage.searchNotes(q),
          { initialValue: [] as readonly SearchHit[] },
        );
        return <box />;
      }, root);
    }
  `,
  );
  globalThis.__zgui = h.host;
  t.after(() => {
    delete globalThis.__zgui;
  });

  const dispose = mod.mount(h.makeNode("root"));
  t.after(() => dispose());

  // Before FTS init completes: gated off, zero SQL queries issued, direct searchNotes throws.
  assert.equal(queryRequests.length, 0);
  assert.throws(() => mod.storage.searchNotes("Wayland"), /not ready/);
  assert.deepEqual(mod.search.data(), []);

  // Complete FTS init and flip ready -> launches query for "Wayland".
  const initPromise = mod.storage.init();
  initDef.resolve({ rowsAffected: 0, lastInsertRowid: null });
  await initPromise;
  h.runTurn(() => mod.setReady(true));
  assert.equal(queryRequests.length, 1);
  assert.equal(mod.search.loading(), true);

  // User types a refined query before slow first query resolves -> cancels first, launches second.
  h.runTurn(() => mod.setText("SQLite"));
  assert.equal(queryRequests[0].request.cancelCount, 1);
  assert.equal(queryRequests.length, 2);

  queryRequests[1].resolve([
    { doc_id: "d2", title: "SQLite Note", path: "sqlite.md", excerpt: "[SQLite]" },
  ]);
  await flushMicrotasks();
  assert.equal(mod.search.loading(), false);
  assert.deepEqual(mod.search.data(), [
    { docId: "d2", title: "SQLite Note", path: "sqlite.md", excerpt: "[SQLite]" },
  ]);

  // Clear disables query and resets hits to [].
  h.runTurn(() => mod.setText(""));
  assert.deepEqual(mod.search.data(), []);
  assert.equal(mod.search.loading(), false);
});
