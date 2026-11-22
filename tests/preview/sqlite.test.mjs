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

const sqliteAppSource = `
import { createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { openDatabase, openKvStore } from "@hewel/zgui-services";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  return renderApp(() => {
    const [capDenied, setCapDenied] = createSignal("PENDING");
    const [seededCount, setSeededCount] = createSignal("PENDING");
    const [ftsHit, setFtsHit] = createSignal("PENDING");
    const [txSummary, setTxSummary] = createSignal("PENDING");
    const [kvSummary, setKvSummary] = createSignal("PENDING");

    // 1. Undeclared DB must throw E_CAPABILITY.
    try {
      const bad = openDatabase("secret");
      bad.close();
      setCapDenied("UNEXPECTED_ALLOW");
    } catch (err) {
      const code = err instanceof Error && "code" in err ? String(err.code) : "UNKNOWN";
      setCapDenied(\`DENIED:\${code}\`);
    }

    const db = openDatabase("main");
    const kv = openKvStore<{ readonly count: number }>("main", "prefs");

    void (async () => {
      // 2. Verify fixture seed statements loaded in :memory: DB.
      const seeded = await db.query<{ readonly id: number; readonly title: string }>(
        "SELECT id, title FROM docs ORDER BY id ASC",
      ).promise;
      setSeededCount(\`\${seeded.length}:\${seeded.map(r => r.title).join(",")}\`);

      // 3. Atomic batch + rollback on constraint error.
      let batchFailed = false;
      try {
        await db.batch([
          { sql: "INSERT INTO docs (id, title) VALUES (?1, ?2)", params: [10, "temp"] },
          { sql: "INSERT INTO docs (id, title) VALUES (?1, ?2)", params: [1, "duplicate-pk"] },
        ]).promise;
      } catch {
        batchFailed = true;
      }
      const afterFailedBatch = await db.query<{ readonly cnt: number }>(
        "SELECT COUNT(*) AS cnt FROM docs WHERE id = 10",
      ).promise;

      // 4. Interactive transaction commit + JS-throw auto-rollback.
      await db.transaction(async tx => {
        await tx.execute("INSERT INTO docs (id, title) VALUES (:id, :title)", {
          id: 20,
          title: "committed-tx",
        });
      });

      let txRolledBack = false;
      try {
        await db.transaction(async tx => {
          await tx.execute("INSERT INTO docs (id, title) VALUES (?1, ?2)", [30, "aborted-tx"]);
          throw new Error("intentional-abort");
        });
      } catch {
        txRolledBack = true;
      }
      const txRows = await db.query<{ readonly title: string }>(
        "SELECT title FROM docs WHERE id IN (20, 30) ORDER BY id",
      ).promise;
      setTxSummary(
        \`batchRb=\${batchFailed && afterFailedBatch[0]?.cnt === 0},txRb=\${txRolledBack},rows=\${txRows.map(r => r.title).join(",")}\`,
      );

      // 5. FTS5 full-text search.
      await db.execute("INSERT INTO docs_fts (title, body) VALUES (?1, ?2)", [
        "Markdown Studio",
        "Local-first desktop workspace with WAL SQLite and FTS5",
      ]).promise;
      const matches = await db.query<{ readonly title: string }>(
        "SELECT title FROM docs_fts WHERE docs_fts MATCH ?1",
        ["FTS5"],
      ).promise;
      setFtsHit(matches.map(r => r.title).join(","));

      // 6. Thin typed KV wrapper backed by the same SQLite engine.
      await kv.set("editor.font", { count: 14 });
      await kv.set("editor.tab", { count: 2 });
      await kv.set("window.width", { count: 1280 });
      const font = await kv.get("editor.font");
      const editorKeys = await kv.list("editor.");
      const deleted = await kv.delete("editor.tab");
      const remaining = await kv.list();
      setKvSummary(
        \`font=\${font?.count},prefix=\${editorKeys.length},del=\${deleted},rem=\${remaining.map(e => e.key).join(",")}\`,
      );
    })();

    return (
      <column style="width: 100%; height: 100%; padding: 12px; gap: 6px">
        <text testId="cap-denied">{capDenied()}</text>
        <text testId="seeded">{seededCount()}</text>
        <text testId="tx-summary">{txSummary()}</text>
        <text testId="fts-hit">{ftsHit()}</text>
        <text testId="kv-summary">{kvSummary()}</text>
      </column>
    );
  }, root);
}
`;

const leakSqliteSource = `
import { renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import { openDatabase } from "@hewel/zgui-services";

export const hostApiVersion = 1;

export function mount(root: NativeNode) {
  // Open outside Solid owner so onCleanup does not close it -> teardown must catch leak.
  const leaked = openDatabase("main");
  void leaked;
  return renderApp(() => (
    <column style="width: 100%; height: 100%; padding: 12px">
      <text testId="status">OPEN</text>
    </column>
  ), root);
}
`;

test("SQLite :memory: preview runs fixture seeds, atomic batch, transactions, FTS5, KV store, and clean teardown (#60)", async t => {
  const temp = await createTempWorkspace("preview-sqlite-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "sqlite-preview",
    sqliteAppSource,
    { rawSource: true, capabilities: { sqlite: { databases: ["main"] } } },
  );
  const fixture = path.join(temp.dir, "fixture.json");
  await writeJson(fixture, {
    schemaVersion: 1,
    id: "sqlite-preview",
    services: {
      sqlite: {
        main: {
          statements: [
            "CREATE TABLE docs (id INTEGER PRIMARY KEY, title TEXT NOT NULL)",
            "INSERT INTO docs (id, title) VALUES (1, 'Alpha'), (2, 'Beta')",
            "CREATE VIRTUAL TABLE docs_fts USING fts5(title, body)",
          ],
        },
      },
    },
    events: [],
  });

  const { out, result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    fixture,
    name: "sqlite-preview",
    requiredCapabilities: [],
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      { id: "cap-done", op: "capture", captureId: "done", surface: "main" },
    ],
  });

  const tree = await readTree(out, "done");
  assert.equal(getNode(tree, "cap-denied")?.text, "DENIED:E_CAPABILITY");
  assert.equal(getNode(tree, "seeded")?.text, "2:Alpha,Beta");
  assert.equal(getNode(tree, "tx-summary")?.text, "batchRb=true,txRb=true,rows=committed-tx");
  assert.equal(getNode(tree, "fts-hit")?.text, "Markdown Studio");
  assert.equal(
    getNode(tree, "kv-summary")?.text,
    "font=14,prefix=2,del=true,rem=editor.font,window.width",
  );
  assert.equal(result.teardown.clean, true);
});

test("unclosed SQLite database handle fails teardown with E_TEARDOWN (#60)", async t => {
  const temp = await createTempWorkspace("preview-sqlite-leak-");
  t.after(() => temp.cleanup());

  const bundle = await buildFixtureApp(
    path.join(temp.dir, "app"),
    "sqlite-leak",
    leakSqliteSource,
    { rawSource: true, capabilities: { sqlite: { databases: ["main"] } } },
  );

  const { result } = await runScenario({
    projectDir: temp.dir,
    bundle,
    name: "sqlite-leak",
    requiredCapabilities: [],
    allowFailure: true,
    steps: [
      { id: "settle-0", op: "clock.advance", byMs: 32 },
      { id: "cap-open", op: "capture", captureId: "open", surface: "main" },
    ],
  });

  assert.equal(result.exitCode, 9);
  assert.equal(result.firstError?.code, "E_TEARDOWN");
  assert.equal(result.teardown.clean, false);
  assert.equal(result.teardown.resources.sqliteDatabases, 1);
});
