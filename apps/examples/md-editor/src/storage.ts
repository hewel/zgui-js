import { openDatabase, openKvStore } from "@hewel/zgui-services";
import type {
  HostRequest,
  JsonValue,
  KvStateStorage,
  KvStore,
  SqliteDatabase,
} from "@hewel/zgui-services";
import type { ColorScheme, ThemePreset } from "@hewel/zgui/components";
import type { MarkdownDoc } from "./samples.js";

export type StudioPrefs = {
  readonly scheme: ColorScheme;
  readonly preset: ThemePreset;
  readonly fontSize: number;
  readonly splitSizes: readonly number[];
  readonly lastWorkspaceBookmark: string | null;
};

export interface SearchHit {
  readonly docId: string;
  readonly title: string;
  readonly path: string;
  readonly excerpt: string;
}

export const DEFAULT_PREFS: StudioPrefs = {
  scheme: "dark",
  preset: "base",
  fontSize: 15,
  splitSizes: [22, 39, 39],
  lastWorkspaceBookmark: null,
};

const VALID_SCHEMES = new Set<ColorScheme>(["light", "dark", "system"]);
const VALID_PRESETS = new Set<ThemePreset>(["base", "ocean", "midnight", "ember", "terminal"]);

export function validateStudioPrefs(raw: unknown): StudioPrefs {
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) {
    throw new TypeError("Studio preferences must be a JSON object");
  }
  const record = raw as Record<string, unknown>;
  if (typeof record.scheme !== "string" || !VALID_SCHEMES.has(record.scheme as ColorScheme)) {
    throw new TypeError("Studio preferences scheme is invalid");
  }
  if (typeof record.preset !== "string" || !VALID_PRESETS.has(record.preset as ThemePreset)) {
    throw new TypeError("Studio preferences preset is invalid");
  }
  if (
    typeof record.fontSize !== "number" ||
    !Number.isFinite(record.fontSize) ||
    record.fontSize < 12 ||
    record.fontSize > 22
  ) {
    throw new TypeError("Studio preferences fontSize must be between 12 and 22");
  }
  if (
    !Array.isArray(record.splitSizes) ||
    record.splitSizes.length !== 3 ||
    !record.splitSizes.every(v => typeof v === "number" && Number.isFinite(v) && v >= 0)
  ) {
    throw new TypeError("Studio preferences splitSizes must be 3 non-negative finite numbers");
  }
  const rawBookmark = record.lastWorkspaceBookmark;
  if (rawBookmark !== null && (typeof rawBookmark !== "string" || rawBookmark.length === 0)) {
    throw new TypeError(
      "Studio preferences lastWorkspaceBookmark must be a non-empty string or null",
    );
  }
  return {
    scheme: record.scheme as ColorScheme,
    preset: record.preset as ThemePreset,
    fontSize: Math.round(record.fontSize),
    splitSizes: record.splitSizes as number[],
    lastWorkspaceBookmark: rawBookmark,
  };
}

export function sanitizeFtsQuery(raw: string): string {
  const tokens = raw
    .trim()
    .split(/\s+/)
    .map(tok => tok.replace(/["'*^():-]/g, "").trim())
    .filter(tok => tok.length > 0);
  if (tokens.length === 0) return "";
  return tokens.map(tok => `"${tok}"*`).join(" AND ");
}

/**
 * Bridges legacy 5-key KV storage (`scheme`, `preset`, `fontSize`, `splitSizes`,
 * `lastWorkspaceBookmark`) strictly when the unified `"settings"` key is absent.
 */
export function createLegacyPrefsStorage(store: KvStateStorage): KvStateStorage {
  return {
    async get(key: string): Promise<JsonValue | null> {
      const existing = await store.get(key);
      if (existing !== null || key !== "settings") {
        return existing;
      }
      const [rawScheme, rawPreset, rawFont, rawSplit, rawBookmark] = await Promise.all([
        store.get("scheme"),
        store.get("preset"),
        store.get("fontSize"),
        store.get("splitSizes"),
        store.get("lastWorkspaceBookmark"),
      ]);
      if (
        rawScheme === null &&
        rawPreset === null &&
        rawFont === null &&
        rawSplit === null &&
        rawBookmark === null
      ) {
        return null;
      }
      const migrated: StudioPrefs = {
        scheme:
          typeof rawScheme === "string" && VALID_SCHEMES.has(rawScheme as ColorScheme)
            ? (rawScheme as ColorScheme)
            : DEFAULT_PREFS.scheme,
        preset:
          typeof rawPreset === "string" && VALID_PRESETS.has(rawPreset as ThemePreset)
            ? (rawPreset as ThemePreset)
            : DEFAULT_PREFS.preset,
        fontSize:
          typeof rawFont === "number" && Number.isFinite(rawFont) && rawFont >= 12 && rawFont <= 22
            ? Math.round(rawFont)
            : DEFAULT_PREFS.fontSize,
        splitSizes:
          Array.isArray(rawSplit) &&
          rawSplit.length === 3 &&
          rawSplit.every(v => typeof v === "number" && Number.isFinite(v) && v >= 0)
            ? (rawSplit as number[])
            : DEFAULT_PREFS.splitSizes,
        lastWorkspaceBookmark:
          typeof rawBookmark === "string" && rawBookmark.length > 0 ? rawBookmark : null,
      };
      return { v: 1, data: migrated };
    },
    set(key: string, value: JsonValue): Promise<void> {
      return store.set(key, value);
    },
  };
}

export interface StudioStorage {
  readonly init: () => Promise<void>;
  readonly isReady: () => boolean;
  readonly prefsStorage: KvStateStorage;
  readonly indexDocuments: (docs: readonly MarkdownDoc[]) => Promise<number>;
  readonly searchNotes: (rawQuery: string) => HostRequest<readonly SearchHit[]>;
  readonly close: () => void;
}

export function createStudioStorage(): StudioStorage {
  let db: SqliteDatabase | null = null;
  let kv: KvStore<JsonValue> | null = null;
  let closed = false;
  let ftsReady = false;
  let readyPromise: Promise<void> | null = null;

  const getHandles = (): { db: SqliteDatabase; kv: KvStore<JsonValue> } => {
    if (closed) throw new Error("StudioStorage is closed");
    if (!db) db = openDatabase("studio");
    if (!kv) kv = openKvStore("studio", "prefs");
    return { db, kv };
  };

  const ensureReady = (): Promise<void> => {
    if (!readyPromise) {
      const handles = getHandles();
      readyPromise = handles.db
        .execute(
          "CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(doc_id UNINDEXED, title, path, content)",
        )
        .promise.then(() => {
          if (!closed) ftsReady = true;
        })
        .catch((err: unknown) => {
          readyPromise = null;
          throw err;
        });
    }
    return readyPromise;
  };

  const prefsStorage = createLegacyPrefsStorage({
    get: key => getHandles().kv.get(key),
    set: (key, value) => getHandles().kv.set(key, value),
  });

  return {
    init: ensureReady,
    isReady: () => ftsReady && !closed,
    prefsStorage,

    async indexDocuments(docs: readonly MarkdownDoc[]): Promise<number> {
      await ensureReady();
      const { db: handle } = getHandles();
      const statements = [
        { sql: "DELETE FROM notes_fts" },
        ...docs.map(doc => ({
          sql: "INSERT INTO notes_fts(doc_id, title, path, content) VALUES (?, ?, ?, ?)",
          params: [doc.id, doc.title, doc.relativePath ?? doc.id, doc.content] as readonly string[],
        })),
      ];
      await handle.batch(statements).promise;
      return docs.length;
    },

    searchNotes(rawQuery: string): HostRequest<readonly SearchHit[]> {
      if (!ftsReady || closed) {
        throw new Error("StudioStorage FTS index is not ready");
      }
      const { db: handle } = getHandles();
      const ftsQuery = sanitizeFtsQuery(rawQuery);
      if (!ftsQuery) {
        return {
          promise: Promise.resolve([]),
          cancel() {},
        };
      }
      const request = handle.query<{
        doc_id: string;
        title: string;
        path: string;
        excerpt: string;
      }>(
        "SELECT doc_id, title, path, snippet(notes_fts, 3, '[', ']', '...', 10) AS excerpt FROM notes_fts WHERE notes_fts MATCH ? ORDER BY rank LIMIT 30",
        [ftsQuery],
      );
      return {
        promise: request.promise.then(rows =>
          rows.map(row => ({
            docId: String(row.doc_id ?? ""),
            title: String(row.title ?? ""),
            path: String(row.path ?? ""),
            excerpt: String(row.excerpt ?? ""),
          })),
        ),
        cancel: () => request.cancel(),
      };
    },

    close() {
      closed = true;
      ftsReady = false;
      kv?.close();
      kv = null;
      db?.close();
      db = null;
    },
  };
}
