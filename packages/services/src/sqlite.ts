import { getOwner, onCleanup } from "#solid";
import type { HostRequest, JsonValue } from "./index";

/** A primitive value that can cross the SQLite parameter and result boundary. */
export type SqlValue = null | number | string | Uint8Array;

/** Positional array or named object SQL bindings. */
export type SqlParams = readonly SqlValue[] | Readonly<Record<string, SqlValue>>;

/** One row returned from a SQLite query. */
export type SqlRow = Readonly<Record<string, SqlValue>>;

/** Options bounding a SQLite query result set. */
export interface SqlQueryOptions {
  /** Maximum rows to return (1..=10000). Exceeding rejects with `E_LIMIT`. */
  readonly maxRows?: number;
}

/** Result metadata for a mutating SQL statement. */
export interface SqlExecuteResult {
  readonly rowsAffected: number;
  readonly lastInsertRowid: number | null;
}

/** One parameterized statement inside an atomic `batch(...)`. */
export interface SqlBatchStatement {
  readonly sql: string;
  readonly params?: SqlParams;
}

/** Options for an interactive SQLite write transaction. */
export interface SqlTransactionOptions {
  /** Host-enforced hard timeout in milliseconds (default 5000, max 30000). */
  readonly timeoutMs?: number;
}

/** Handle passed to the callback of `SqliteDatabase.transaction(...)`. */
export interface SqliteTransaction {
  query<T extends SqlRow = SqlRow>(
    sql: string,
    params?: SqlParams,
    options?: SqlQueryOptions,
  ): Promise<readonly T[]>;
  execute(sql: string, params?: SqlParams): Promise<SqlExecuteResult>;
}

/** An app-scoped asynchronous SQLite database handle. */
export interface SqliteDatabase {
  readonly name: string;
  /** Runs a read-only parameterized query on the concurrent reader pool. */
  query<T extends SqlRow = SqlRow>(
    sql: string,
    params?: SqlParams,
    options?: SqlQueryOptions,
  ): HostRequest<readonly T[]>;
  /** Runs a parameterized write statement on the serialized writer connection. */
  execute(sql: string, params?: SqlParams): HostRequest<SqlExecuteResult>;
  /** Executes statements atomically inside a single `BEGIN IMMEDIATE ... COMMIT` batch. */
  batch(statements: readonly SqlBatchStatement[]): HostRequest<readonly SqlExecuteResult[]>;
  /** Runs an interactive write transaction with host-enforced timeout and auto-rollback. */
  transaction<T>(
    fn: (tx: SqliteTransaction) => Promise<T> | T,
    options?: SqlTransactionOptions,
  ): Promise<T>;
  /** Closes the database handle and aborts any open transactions. Idempotent. */
  close(): void;
}

/** Key/value pair returned by `KvStore.list(...)`. */
export interface KvEntry<T extends JsonValue = JsonValue> {
  readonly key: string;
  readonly value: T;
}

/** Thin typed key-value wrapper backed by an app-scoped SQLite database. */
export interface KvStore<T extends JsonValue = JsonValue> {
  readonly database: string;
  readonly namespace: string;
  get(key: string): Promise<T | null>;
  set(key: string, value: T): Promise<void>;
  delete(key: string): Promise<boolean>;
  list(prefix?: string): Promise<readonly KvEntry<T>[]>;
  clear(): Promise<void>;
  close(): void;
}

export interface RawSqliteTransaction {
  readonly beginPromise: Promise<void>;
  readonly timeoutPromise: Promise<never>;
  query<T extends SqlRow = SqlRow>(
    sql: string,
    params?: SqlParams,
    options?: SqlQueryOptions,
  ): HostRequest<readonly T[]>;
  execute(sql: string, params?: SqlParams): HostRequest<SqlExecuteResult>;
  commit(): HostRequest<void>;
  rollback(): HostRequest<void>;
  abort(): void;
}

export interface RawSqliteDatabase {
  readonly name: string;
  query<T extends SqlRow = SqlRow>(
    sql: string,
    params?: SqlParams,
    options?: SqlQueryOptions,
  ): HostRequest<readonly T[]>;
  execute(sql: string, params?: SqlParams): HostRequest<SqlExecuteResult>;
  batch(statements: readonly SqlBatchStatement[]): HostRequest<readonly SqlExecuteResult[]>;
  __beginTransaction(options?: SqlTransactionOptions): RawSqliteTransaction;
  close(): void;
}

interface HostSqliteBinding {
  openDatabase(name: string): RawSqliteDatabase;
}

function hostSqlite(): HostSqliteBinding {
  return (globalThis as unknown as { readonly __zguiServices: HostSqliteBinding }).__zguiServices;
}

/**
 * Open a manifest-declared app-scoped SQLite database.
 * Live databases persist under `$XDG_DATA_HOME/<appId>/<name>.sqlite3` in WAL mode;
 * preview databases always run in-memory with zero host disk access.
 * Automatically closes on Solid owner cleanup when called inside an active owner.
 */
export function openDatabase(name: string): SqliteDatabase {
  const raw = hostSqlite().openDatabase(name);
  const db: SqliteDatabase = {
    get name() {
      return raw.name;
    },
    query<T extends SqlRow = SqlRow>(sql: string, params?: SqlParams, options?: SqlQueryOptions) {
      return raw.query<T>(sql, params, options);
    },
    execute(sql, params) {
      return raw.execute(sql, params);
    },
    batch(statements) {
      return raw.batch(statements);
    },
    async transaction<T>(
      fn: (tx: SqliteTransaction) => Promise<T> | T,
      options?: SqlTransactionOptions,
    ): Promise<T> {
      const rawTx = raw.__beginTransaction(options);
      let settled = false;
      let timedOut = false;
      const onTimeout = rawTx.timeoutPromise.catch((err: unknown) => {
        timedOut = true;
        throw err;
      });
      // Observe timeout promise so normal completion never reports an unhandled rejection.
      void onTimeout.catch(() => {});
      await rawTx.beginPromise;
      const tx: SqliteTransaction = {
        query<R extends SqlRow = SqlRow>(
          sql: string,
          params?: SqlParams,
          queryOptions?: SqlQueryOptions,
        ) {
          return rawTx.query<R>(sql, params, queryOptions).promise;
        },
        execute(sql, params) {
          return rawTx.execute(sql, params).promise;
        },
      };
      try {
        const result = await Promise.race([Promise.resolve().then(() => fn(tx)), onTimeout]);
        if (timedOut) {
          await onTimeout;
        }
        await Promise.race([rawTx.commit().promise, onTimeout]);
        settled = true;
        return result;
      } catch (error) {
        if (!settled && !timedOut) {
          try {
            await rawTx.rollback().promise;
          } catch {
            rawTx.abort();
          }
          settled = true;
        } else if (!settled) {
          rawTx.abort();
          settled = true;
        }
        throw error;
      }
    },
    close() {
      raw.close();
    },
  };
  if (getOwner()) {
    onCleanup(() => db.close());
  }
  return db;
}

function escapeLikePrefix(prefix: string): string {
  return prefix.replace(/[\\%_]/g, ch => `\\${ch}`);
}

/**
 * Open a thin typed key-value store backed by a manifest-declared SQLite database.
 * Shares the same WAL engine and owner-scoped cleanup as `openDatabase`.
 */
export function openKvStore<T extends JsonValue = JsonValue>(
  dbName = "main",
  namespace = "default",
): KvStore<T> {
  if (typeof namespace !== "string" || namespace.length === 0) {
    throw new TypeError("KV store namespace must be a non-empty string");
  }
  const db = openDatabase(dbName);
  let schemaReady: Promise<void> | null = null;
  const ensureSchema = (): Promise<void> => {
    if (!schemaReady) {
      schemaReady = db
        .execute(
          "CREATE TABLE IF NOT EXISTS _zgui_kv (namespace TEXT NOT NULL, key TEXT NOT NULL, value_json TEXT NOT NULL, PRIMARY KEY (namespace, key))",
        )
        .promise.then(() => undefined)
        .catch((err: unknown) => {
          schemaReady = null;
          throw err;
        });
    }
    return schemaReady;
  };

  return {
    database: db.name,
    namespace,
    async get(key: string): Promise<T | null> {
      await ensureSchema();
      const rows = await db.query<{ readonly value_json: string }>(
        "SELECT value_json FROM _zgui_kv WHERE namespace = ?1 AND key = ?2",
        [namespace, key],
      ).promise;
      const first = rows[0];
      if (!first) return null;
      return JSON.parse(first.value_json) as T;
    },
    async set(key: string, value: T): Promise<void> {
      await ensureSchema();
      const encoded = JSON.stringify(value);
      if (encoded === undefined) {
        throw new TypeError("KV value must be JSON-serializable");
      }
      await db.execute(
        "INSERT INTO _zgui_kv (namespace, key, value_json) VALUES (?1, ?2, ?3) ON CONFLICT(namespace, key) DO UPDATE SET value_json = excluded.value_json",
        [namespace, key, encoded],
      ).promise;
    },
    async delete(key: string): Promise<boolean> {
      await ensureSchema();
      const res = await db.execute("DELETE FROM _zgui_kv WHERE namespace = ?1 AND key = ?2", [
        namespace,
        key,
      ]).promise;
      return res.rowsAffected > 0;
    },
    async list(prefix?: string): Promise<readonly KvEntry<T>[]> {
      await ensureSchema();
      const rows =
        prefix !== undefined && prefix.length > 0
          ? await db.query<{ readonly key: string; readonly value_json: string }>(
              "SELECT key, value_json FROM _zgui_kv WHERE namespace = ?1 AND key LIKE ?2 ESCAPE '\\' ORDER BY key ASC",
              [namespace, `${escapeLikePrefix(prefix)}%`],
            ).promise
          : await db.query<{ readonly key: string; readonly value_json: string }>(
              "SELECT key, value_json FROM _zgui_kv WHERE namespace = ?1 ORDER BY key ASC",
              [namespace],
            ).promise;
      return rows.map(row => ({
        key: row.key,
        value: JSON.parse(row.value_json) as T,
      }));
    },
    async clear(): Promise<void> {
      await ensureSchema();
      await db.execute("DELETE FROM _zgui_kv WHERE namespace = ?1", [namespace]).promise;
    },
    close() {
      db.close();
    },
  };
}
