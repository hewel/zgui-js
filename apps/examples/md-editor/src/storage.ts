import { openDatabase, openKvStore } from "@hewel/zgui-services";
import type { JsonValue, KvStore, SqliteDatabase } from "@hewel/zgui-services";
import type { ColorScheme, ThemePreset } from "@hewel/zgui/components";
import type { MarkdownDoc } from "./samples.js";

export interface StudioPrefs {
  readonly scheme: ColorScheme;
  readonly preset: ThemePreset;
  readonly fontSize: number;
  readonly splitSizes: readonly number[];
  readonly lastWorkspaceBookmark: string | null;
}

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

function sanitizeFtsQuery(raw: string): string {
  const tokens = raw
    .trim()
    .split(/\s+/)
    .map(tok => tok.replace(/["'*^():-]/g, "").trim())
    .filter(tok => tok.length > 0);
  if (tokens.length === 0) return "";
  return tokens.map(tok => `"${tok}"*`).join(" AND ");
}

export interface StudioStorage {
  readonly init: () => Promise<void>;
  readonly loadPrefs: () => Promise<StudioPrefs>;
  readonly savePrefs: (prefs: StudioPrefs) => Promise<void>;
  readonly indexDocuments: (docs: readonly MarkdownDoc[]) => Promise<number>;
  readonly searchNotes: (query: string) => Promise<readonly SearchHit[]>;
  readonly close: () => void;
}

export function createStudioStorage(): StudioStorage {
  let db: SqliteDatabase | null = null;
  let kv: KvStore<JsonValue> | null = null;
  let closed = false;
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
      readyPromise = handles.db.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(doc_id UNINDEXED, title, path, content)",
      ).promise.then(() => undefined);
    }
    return readyPromise;
  };

  return {
    init: ensureReady,

    async loadPrefs(): Promise<StudioPrefs> {
      await ensureReady();
      const { kv: store } = getHandles();
      const [rawScheme, rawPreset, rawFont, rawSplit, rawBookmark] = await Promise.all([
        store.get("scheme"),
        store.get("preset"),
        store.get("fontSize"),
        store.get("splitSizes"),
        store.get("lastWorkspaceBookmark"),
      ]);
      const scheme = typeof rawScheme === "string" && VALID_SCHEMES.has(rawScheme as ColorScheme)
        ? (rawScheme as ColorScheme)
        : DEFAULT_PREFS.scheme;
      const preset = typeof rawPreset === "string" && VALID_PRESETS.has(rawPreset as ThemePreset)
        ? (rawPreset as ThemePreset)
        : DEFAULT_PREFS.preset;
      const fontSize = typeof rawFont === "number" && Number.isFinite(rawFont) && rawFont >= 12 && rawFont <= 22
        ? Math.round(rawFont)
        : DEFAULT_PREFS.fontSize;
      let splitSizes = DEFAULT_PREFS.splitSizes;
      if (
        Array.isArray(rawSplit)
        && rawSplit.length === 3
        && rawSplit.every(v => typeof v === "number" && Number.isFinite(v) && v >= 0)
      ) {
        splitSizes = rawSplit as number[];
      }
      const lastWorkspaceBookmark = typeof rawBookmark === "string" && rawBookmark.length > 0
        ? rawBookmark
        : null;
      return { scheme, preset, fontSize, splitSizes, lastWorkspaceBookmark };
    },

    async savePrefs(prefs: StudioPrefs): Promise<void> {
      await ensureReady();
      const { kv: store } = getHandles();
      await Promise.all([
        store.set("scheme", prefs.scheme),
        store.set("preset", prefs.preset),
        store.set("fontSize", prefs.fontSize),
        store.set("splitSizes", [...prefs.splitSizes]),
        store.set("lastWorkspaceBookmark", prefs.lastWorkspaceBookmark),
      ]);
    },

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

    async searchNotes(rawQuery: string): Promise<readonly SearchHit[]> {
      await ensureReady();
      const { db: handle } = getHandles();
      const ftsQuery = sanitizeFtsQuery(rawQuery);
      if (!ftsQuery) return [];
      const rows = await handle.query<{
        doc_id: string;
        title: string;
        path: string;
        excerpt: string;
      }>(
        "SELECT doc_id, title, path, snippet(notes_fts, 3, '[', ']', '...', 10) AS excerpt FROM notes_fts WHERE notes_fts MATCH ? ORDER BY rank LIMIT 30",
        [ftsQuery],
      ).promise;
      return rows.map(row => ({
        docId: String(row.doc_id ?? ""),
        title: String(row.title ?? ""),
        path: String(row.path ?? ""),
        excerpt: String(row.excerpt ?? ""),
      }));
    },

    close() {
      closed = true;
      kv?.close();
      kv = null;
      db?.close();
      db = null;
    },
  };
}
