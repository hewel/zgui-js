import {
  createContext,
  createDelay,
  createEffect,
  createMemo,
  createSignal,
  flush,
  getSelection,
  onCleanup,
  scrollIntoView,
  setSelection,
  setWindowTitle,
  useContext,
} from "@hewel/zgui";
import type { Accessor, NativeNode, TextSelectionRange } from "@hewel/zgui";
import { toast } from "@hewel/zgui/components";
import type { ColorScheme, ThemePreset, TreeNodeData } from "@hewel/zgui/components";
import {
  createHostQuery,
  createKvState,
  readClipboardImage,
  registerImageBytes,
  writeClipboardText,
} from "@hewel/zgui-services";
import type { BookmarkInfo, ImageHandle, WatchEvent } from "@hewel/zgui-services";
import { buildOutlineTree, parseMarkdown, toggleTaskAtLine } from "./markdown.js";
import type { OutlineNodeMeta, ParsedDocument } from "./markdown.js";
import { INITIAL_DOCS, buildPipelineSvg, buildStressMarkdown, encodeUtf8Bytes } from "./samples.js";
import type { MarkdownDoc } from "./samples.js";
import {
  DEFAULT_PREFS,
  createStudioStorage,
  sanitizeFtsQuery,
  validateStudioPrefs,
} from "./storage.js";
import type { SearchHit, StudioPrefs } from "./storage.js";
import { createWorkspaceController } from "./workspace.js";
import type { WorkspaceNodeMeta } from "./workspace.js";

export type ViewMode = "split" | "editor" | "preview";

export interface StudioState {
  readonly scheme: Accessor<ColorScheme>;
  readonly setScheme: (next: ColorScheme) => void;
  readonly preset: Accessor<ThemePreset>;
  readonly setPreset: (next: ThemePreset) => void;
  readonly fontSize: Accessor<number>;
  readonly setFontSize: (next: number) => void;
  readonly splitSizes: Accessor<readonly number[]>;
  readonly setSplitSizes: (next: readonly number[]) => void;
  readonly viewMode: Accessor<ViewMode>;
  readonly setViewMode: (next: ViewMode) => void;

  readonly docs: Accessor<readonly MarkdownDoc[]>;
  readonly activeDocId: Accessor<string>;
  readonly setActiveDocId: (id: string) => void;
  readonly activeDoc: Accessor<MarkdownDoc>;
  readonly hasUnsavedChanges: Accessor<boolean>;
  readonly parsedResult: Accessor<{ readonly parsed: ParsedDocument; readonly parseMs: number }>;
  readonly outlineTree: Accessor<readonly TreeNodeData<OutlineNodeMeta>[]>;
  readonly stats: Accessor<{
    readonly chars: number;
    readonly words: number;
    readonly lines: number;
    readonly readMin: number;
  }>;

  readonly workspaceName: Accessor<string | null>;
  readonly workspaceTree: Accessor<readonly TreeNodeData<WorkspaceNodeMeta>[]>;
  readonly workspaceExpanded: Accessor<readonly string[]>;
  readonly setWorkspaceExpanded: (ids: readonly string[]) => void;
  readonly bookmarks: Accessor<readonly BookmarkInfo[]>;
  readonly externalEventLog: Accessor<string>;

  readonly selectionRange: Accessor<TextSelectionRange>;
  readonly setSelectionRange: (range: TextSelectionRange) => void;
  readonly editorEl: Accessor<NativeNode | null>;
  readonly setEditorEl: (node: NativeNode | null) => void;
  readonly highlightedLine: Accessor<number | null>;
  readonly pipelineImg: Accessor<string | null>;
  readonly pastedImg: Accessor<string | null>;
  readonly statusMsg: Accessor<string>;
  readonly setStatusMsg: (msg: string) => void;
  readonly lastCopiedSnippet: Accessor<string>;

  readonly indexedCount: Accessor<number>;
  readonly searchQuery: Accessor<string>;
  readonly setSearchQuery: (q: string) => void;
  readonly searchHits: Accessor<readonly SearchHit[]>;
  readonly searchLoading: Accessor<boolean>;
  readonly searchError: Accessor<Error | undefined>;
  readonly refetchSearch: () => void;
  readonly prefsHydrated: Accessor<boolean>;
  readonly prefsSaving: Accessor<boolean>;
  readonly prefsError: Accessor<Error | undefined>;
  readonly flushPrefs: () => Promise<void>;

  readonly prefsWindowOpen: Accessor<boolean>;
  readonly setPrefsWindowOpen: (open: boolean) => void;
  readonly closeDialogOpen: Accessor<boolean>;
  readonly setCloseDialogOpen: (open: boolean) => void;
  readonly guardNavigation: Accessor<boolean>;
  readonly setGuardNavigation: (enabled: boolean) => void;
  readonly pendingLeavePath: Accessor<string | null>;
  readonly setPendingLeavePath: (path: string | null) => void;

  readonly updateActiveContent: (next: string, note?: string) => void;
  readonly wrapSelection: (before: string, after: string, fallbackText: string) => void;
  readonly appendBlock: (snippet: string, label: string) => void;
  readonly toggleTask: (lineIndex: number) => void;
  readonly registerHeadingNode: (lineIndex: number, node: NativeNode) => void;
  readonly jumpToHeading: (lineIndex: number, title: string) => void;
  readonly copyCodeBlock: (code: string, lang: string) => Promise<void>;
  readonly pasteClipboardImage: () => Promise<void>;
  readonly clearPastedImage: () => void;
  readonly openLocalFile: () => Promise<void>;
  readonly saveActiveDocument: () => Promise<boolean>;
  readonly openWorkspaceFolder: () => Promise<void>;
  readonly restoreBookmarkByToken: (token: string) => Promise<void>;
  readonly revokeBookmarkByToken: (token: string) => Promise<void>;
  readonly refreshBookmarks: () => Promise<void>;
  readonly loadStressDoc: () => void;
  readonly resetPrefs: () => Promise<void>;
}

export const StudioContext = createContext<StudioState | null>(null);

export function useStudio(): StudioState {
  const ctx = useContext(StudioContext);
  if (!ctx) throw new Error("useStudio() must be called inside StudioContext");
  return ctx;
}

function lineStartOffsetUtf16(content: string, lineIndex: number): number {
  if (lineIndex <= 0) return 0;
  let offset = 0;
  let line = 0;
  while (line < lineIndex && offset < content.length) {
    const nl = content.indexOf("\n", offset);
    if (nl < 0) return content.length;
    offset = nl + 1;
    line += 1;
  }
  return offset;
}

export function createStudioState(): StudioState {
  const storage = createStudioStorage();
  const workspace = createWorkspaceController();

  const prefs = createKvState<StudioPrefs>(storage.prefsStorage, "settings", DEFAULT_PREFS, {
    validate: validateStudioPrefs,
    version: 1,
    debounceMs: 180,
  });

  const scheme = () => prefs.value().scheme;
  const preset = () => prefs.value().preset;
  const fontSize = () => prefs.value().fontSize;
  const splitSizes = () => prefs.value().splitSizes;
  const lastBookmark = () => prefs.value().lastWorkspaceBookmark;
  const [viewMode, setViewMode] = createSignal<ViewMode>("split");

  const [docs, setDocs] = createSignal<readonly MarkdownDoc[]>(INITIAL_DOCS);
  const [activeDocId, setActiveDocId] = createSignal<string>(INITIAL_DOCS[0]!.id);
  const [workspaceName, setWorkspaceName] = createSignal<string | null>(null);
  const [workspaceTree, setWorkspaceTree] = createSignal<
    readonly TreeNodeData<WorkspaceNodeMeta>[]
  >([]);
  const [workspaceExpanded, setWorkspaceExpanded] = createSignal<readonly string[]>([]);
  const [bookmarks, setBookmarks] = createSignal<readonly BookmarkInfo[]>([]);
  const [externalEventLog, setExternalEventLog] = createSignal("none");

  const [selectionRange, setSelectionRange] = createSignal<TextSelectionRange>({
    start: 0,
    end: 0,
  });
  const [editorEl, setEditorEl] = createSignal<NativeNode | null>(null);
  const [highlightedLine, setHighlightedLine] = createSignal<number | null>(null);
  const [pipelineImg, setPipelineImg] = createSignal<string | null>(null);
  const [pastedImg, setPastedImg] = createSignal<string | null>(null);
  const [statusMsg, setStatusMsg] = createSignal("Ready — local-first Markdown Studio");
  const [lastCopiedSnippet, setLastCopiedSnippet] = createSignal("");

  const [ftsReady, setFtsReady] = createSignal(false);
  const [indexedCount, setIndexedCount] = createSignal(0);
  const [searchQuery, setSearchQuery] = createSignal("");

  const [prefsWindowOpen, setPrefsWindowOpen] = createSignal(false);
  const [closeDialogOpen, setCloseDialogOpen] = createSignal(false);
  const [guardNavigation, setGuardNavigation] = createSignal(true);
  const [pendingLeavePath, setPendingLeavePath] = createSignal<string | null>(null);

  const headingNodes = new Map<number, NativeNode>();
  const highlightTimer = createDelay();
  const reindexTimer = createDelay();
  let pastedHandle: ImageHandle | null = null;
  let disposed = false;

  const searchHostQuery = createHostQuery(
    () => {
      if (!ftsReady()) return false;
      const sanitized = sanitizeFtsQuery(searchQuery());
      return sanitized.length > 0 ? searchQuery() : false;
    },
    query => storage.searchNotes(query),
    {
      initialValue: [] as readonly SearchHit[],
      onError: err => {
        if (!disposed) {
          setStatusMsg(`Search failed: ${err.message}`);
        }
      },
    },
  );
  const searchHits = () => searchHostQuery.data() ?? [];

  onCleanup(() => {
    disposed = true;
    pastedHandle?.release();
    pastedHandle = null;
    workspace.close();
    storage.close();
    headingNodes.clear();
  });

  createEffect(
    () => prefs.error(),
    err => {
      if (!err || disposed) return;
      setStatusMsg(`Preferences error: ${err.message}`);
      toast.error("Preferences persistence failed", { description: err.message });
    },
  );

  createEffect(
    () => scheme() === "dark",
    isDark => {
      const svg = buildPipelineSvg(isDark);
      const handle = registerImageBytes(encodeUtf8Bytes(svg));
      setPipelineImg(handle.src);
      return () => handle.release();
    },
  );

  const activeDoc = createMemo(() => docs().find(d => d.id === activeDocId()) ?? docs()[0]!);
  const hasUnsavedChanges = createMemo(() => docs().some(d => d.dirty === true));

  createEffect(
    () => ({ title: activeDoc().title, dirty: activeDoc().dirty === true }),
    ({ title, dirty }) => {
      setWindowTitle(`${title}${dirty ? " *" : ""} — Markdown Studio`);
    },
  );

  const parsedResult = createMemo(() => {
    const t0 = Date.now();
    const parsed = parseMarkdown(activeDoc().content);
    const parseMs = Date.now() - t0;
    return { parsed, parseMs };
  });

  const outlineTree = createMemo(() => buildOutlineTree(parsedResult().parsed.headings));

  const stats = createMemo(() => {
    const text = activeDoc().content;
    const lines = text ? text.split("\n").length : 0;
    const words = text.trim() ? text.trim().split(/\s+/).length : 0;
    const readMin = Math.max(1, Math.ceil(words / 220));
    return { chars: text.length, words, lines, readMin };
  });

  const setScheme = (next: ColorScheme) => {
    prefs.set(prev => ({ ...prev, scheme: next }));
  };
  const setPreset = (next: ThemePreset) => {
    prefs.set(prev => ({ ...prev, preset: next }));
  };
  const setFontSize = (next: number) => {
    prefs.set(prev => ({ ...prev, fontSize: next }));
  };
  const setSplitSizes = (next: readonly number[]) => {
    prefs.set(prev => ({ ...prev, splitSizes: [...next] }));
  };
  const setLastBookmark = (next: string | null) => {
    prefs.set(prev => ({ ...prev, lastWorkspaceBookmark: next }));
  };

  const reindexAll = async (list: readonly MarkdownDoc[]) => {
    if (disposed) return;
    try {
      const count = await storage.indexDocuments(list);
      if (disposed) return;
      const wasReady = ftsReady();
      setIndexedCount(count);
      if (!wasReady) {
        setFtsReady(true);
      } else if (sanitizeFtsQuery(searchQuery()).length > 0) {
        searchHostQuery.refetch();
      }
    } catch {
      // Ignore if disposed during shutdown
    }
  };

  const scheduleReindex = () => {
    reindexTimer.schedule(120, () => {
      void reindexAll(docs());
    });
  };

  const refreshBookmarks = async () => {
    try {
      const list = await workspace.listSavedBookmarks();
      if (!disposed) setBookmarks(list);
    } catch {
      // Ignore on unsupported environment
    }
  };

  const mergeWorkspaceDocs = (scannedDocs: readonly MarkdownDoc[], selectFirst: boolean) => {
    setDocs(prev => {
      const nonWorkspace = prev.filter(d => d.source !== "workspace");
      const dirtyById = new Map(
        prev.filter(d => d.source === "workspace" && d.dirty).map(d => [d.id, d]),
      );
      const mergedWs = scannedDocs.map(doc => dirtyById.get(doc.id) ?? doc);
      return [...mergedWs, ...nonWorkspace];
    });
    if (selectFirst && scannedDocs.length > 0) {
      setActiveDocId(scannedDocs[0]!.id);
    }
    void reindexAll(docs());
  };

  const handleExternalEvents = (events: readonly WatchEvent[]) => {
    if (disposed || events.length === 0) return;
    const summary = events.map(e => `${e.kind}:${e.path}`).join(",");
    setExternalEventLog(summary);
    void (async () => {
      const scanned = await workspace.refreshWorkspace();
      if (!scanned || disposed) return;
      setWorkspaceTree(scanned.tree);
      const current = activeDoc();
      const touchedActive =
        current.source === "workspace" &&
        current.relativePath &&
        events.some(e => e.path === current.relativePath);
      if (touchedActive && current.dirty && current.relativePath) {
        const rel = current.relativePath;
        toast.warning(`External edit detected in ${current.title}`, {
          description: "Local unsaved edits were preserved.",
          action: {
            label: "Reload",
            onClick: () => {
              void workspace.readWorkspaceFile(rel).then(fresh => {
                if (fresh === null || disposed) return;
                setDocs(prev =>
                  prev.map(d => (d.id === current.id ? { ...d, content: fresh, dirty: false } : d)),
                );
                setStatusMsg(`Reloaded ${current.title} from disk`);
                scheduleReindex();
              });
            },
          },
        });
      } else if (touchedActive && current.relativePath) {
        const fresh = await workspace.readWorkspaceFile(current.relativePath);
        if (fresh !== null && !disposed) {
          setDocs(prev =>
            prev.map(d => (d.id === current.id ? { ...d, content: fresh, dirty: false } : d)),
          );
          toast.info(`Reloaded ${current.title}`, { description: "Updated from disk." });
        }
      }
      mergeWorkspaceDocs(scanned.docs, false);
      setStatusMsg(`Workspace updated (${summary})`);
    })();
  };

  let restoredHydratedBookmark = false;
  createEffect(
    () => prefs.hydrated(),
    hydrated => {
      if (!hydrated || restoredHydratedBookmark || disposed) return;
      restoredHydratedBookmark = true;
      const bookmark = prefs.value().lastWorkspaceBookmark;
      if (!bookmark || workspaceName()) return;
      void workspace
        .restoreWorkspaceBookmark(bookmark, handleExternalEvents)
        .then(restored => {
          if (restored && !disposed) {
            setWorkspaceName(restored.dir.name);
            setWorkspaceTree(restored.scanned.tree);
            setWorkspaceExpanded(restored.scanned.expandedDirIds);
            mergeWorkspaceDocs(restored.scanned.docs, true);
            setStatusMsg(`Restored workspace "${restored.dir.name}"`);
          }
        })
        .catch(() => {
          // Ignore unavailable/revoked bookmark on startup
        });
    },
  );

  // Defer SQLite FTS & bookmark bootstrap to next turn so initial mount stays well inside entry budget
  const bootstrapTimer = createDelay();
  bootstrapTimer.schedule(0, () => {
    if (disposed) return;
    void (async () => {
      try {
        await storage.init();
        if (disposed) return;
        await reindexAll(docs());
        await refreshBookmarks();
      } catch {
        // Fallback gracefully to in-memory defaults
      }
    })();
  });

  const updateActiveContent = (nextContent: string, note?: string) => {
    const id = activeDoc().id;
    setDocs(prev => prev.map(d => (d.id === id ? { ...d, content: nextContent, dirty: true } : d)));
    if (note) setStatusMsg(note);
    scheduleReindex();
  };

  const wrapSelection = (before: string, after: string, fallbackText: string) => {
    const content = activeDoc().content;
    const sel = getSelection(editorEl) ?? selectionRange();
    const start = Math.max(0, Math.min(content.length, sel.start));
    const end = Math.max(start, Math.min(content.length, sel.end));
    const selected = content.slice(start, end) || fallbackText;
    const next = `${content.slice(0, start)}${before}${selected}${after}${content.slice(end)}`;
    const nextStart = start + before.length;
    const nextEnd = nextStart + selected.length;
    updateActiveContent(next, `Formatted UTF-16 range ${start}:${end} -> ${nextStart}:${nextEnd}`);
    setSelectionRange({ start: nextStart, end: nextEnd });
    flush();
    setSelection(editorEl, nextStart, nextEnd);
  };

  const appendBlock = (snippet: string, label: string) => {
    const content = activeDoc().content.trimEnd();
    updateActiveContent(`${content}\n\n${snippet}\n`, `Appended ${label}`);
  };

  const toggleTask = (lineIndex: number) => {
    const updated = toggleTaskAtLine(activeDoc().content, lineIndex);
    updateActiveContent(updated, `Toggled task at line ${lineIndex + 1}`);
  };

  const registerHeadingNode = (lineIndex: number, node: NativeNode) => {
    headingNodes.set(lineIndex, node);
  };

  const jumpToHeading = (lineIndex: number, title: string) => {
    setHighlightedLine(lineIndex);
    const target = headingNodes.get(lineIndex);
    if (target) {
      scrollIntoView(target, { block: "start", behavior: "smooth" });
    }
    const offset = lineStartOffsetUtf16(activeDoc().content, lineIndex);
    setSelectionRange({ start: offset, end: offset });
    setSelection(editorEl, offset, offset);
    setStatusMsg(`Jumped to "${title}" (line ${lineIndex + 1})`);
    highlightTimer.schedule(1800, () => setHighlightedLine(null));
  };

  const copyCodeBlock = async (code: string, lang: string) => {
    try {
      await writeClipboardText(code).promise;
      if (disposed) return;
      setLastCopiedSnippet(code);
      setStatusMsg(`Copied ${lang} snippet (${code.length} chars)`);
      toast.success("Copied code block", {
        description: `${lang} (${code.split("\n").length} lines)`,
      });
    } catch (err) {
      toast.error("Clipboard copy failed", {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const pasteClipboardImage = async () => {
    try {
      const handle = await readClipboardImage().promise;
      if (disposed) {
        handle?.release();
        return;
      }
      if (!handle) {
        toast.info("Clipboard has no PNG image");
        return;
      }
      pastedHandle?.release();
      pastedHandle = handle;
      setPastedImg(handle.src);
      setStatusMsg("Pasted PNG image from clipboard");
      toast.success("Pasted clipboard image", { description: "Rendered via host ImageHandle." });
    } catch (err) {
      toast.error("Paste image failed", {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const clearPastedImage = () => {
    pastedHandle?.release();
    pastedHandle = null;
    setPastedImg(null);
    setStatusMsg("Cleared pasted image preview");
  };

  const openLocalFile = async () => {
    try {
      const opened = await workspace.openLocalFile();
      if (!opened || disposed) return;
      setDocs(prev => {
        const filtered = prev.filter(d => d.id !== opened.doc.id);
        return [opened.doc, ...filtered];
      });
      setActiveDocId(opened.doc.id);
      setStatusMsg(`Opened local file "${opened.doc.title}"`);
      toast.success(`Opened ${opened.doc.title}`);
      void reindexAll(docs());
    } catch (err) {
      toast.error("Open file failed", {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const saveActiveDocument = async (): Promise<boolean> => {
    const doc = activeDoc();
    try {
      const saved = await workspace.saveDocument(doc);
      if (!saved || disposed) return false;
      setDocs(prev =>
        prev.map(d =>
          d.id === doc.id
            ? {
                ...d,
                title: saved.savedName,
                relativePath: saved.relativePath ?? d.relativePath,
                dirty: false,
              }
            : d,
        ),
      );
      setStatusMsg(`Saved "${saved.savedName}"`);
      toast.success(`Saved ${saved.savedName}`);
      void reindexAll(docs());
      return true;
    } catch (err) {
      toast.error("Save failed", { description: err instanceof Error ? err.message : String(err) });
      return false;
    }
  };

  const openWorkspaceFolder = async () => {
    try {
      const opened = await workspace.openFolder(handleExternalEvents);
      if (!opened || disposed) return;
      setWorkspaceName(opened.dir.name);
      setWorkspaceTree(opened.scanned.tree);
      setWorkspaceExpanded(opened.scanned.expandedDirIds);
      mergeWorkspaceDocs(opened.scanned.docs, true);
      if (opened.bookmarkToken) {
        setLastBookmark(opened.bookmarkToken);
      }
      await refreshBookmarks();
      setStatusMsg(`Opened workspace "${opened.dir.name}" (${opened.scanned.docs.length} files)`);
      toast.success(`Workspace "${opened.dir.name}" opened`, {
        description: `${opened.scanned.docs.length} markdown files indexed in SQLite FTS5.`,
      });
    } catch (err) {
      toast.error("Open folder failed", {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const restoreBookmarkByToken = async (token: string) => {
    try {
      const restored = await workspace.restoreWorkspaceBookmark(token, handleExternalEvents);
      if (!restored || disposed) {
        toast.warning("Bookmark could not be restored");
        return;
      }
      setWorkspaceName(restored.dir.name);
      setWorkspaceTree(restored.scanned.tree);
      setWorkspaceExpanded(restored.scanned.expandedDirIds);
      mergeWorkspaceDocs(restored.scanned.docs, true);
      setLastBookmark(token);
      toast.success(`Restored "${restored.dir.name}"`);
    } catch (err) {
      toast.error("Restore bookmark failed", {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const revokeBookmarkByToken = async (token: string) => {
    try {
      const ok = await workspace.revokeSavedBookmark(token);
      if (ok && lastBookmark() === token) {
        setLastBookmark(null);
      }
      await refreshBookmarks();
      if (ok) toast.info("Revoked workspace bookmark");
    } catch (err) {
      toast.error("Revoke bookmark failed", {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const loadStressDoc = () => {
    const existing = docs().find(d => d.id === "stress");
    if (!existing) {
      const stressDoc: MarkdownDoc = {
        id: "stress",
        title: "120-Block Stress Doc",
        badge: "Stress",
        source: "builtin",
        content: buildStressMarkdown(),
      };
      setDocs(prev => [...prev, stressDoc]);
    }
    setActiveDocId("stress");
    setStatusMsg("Loaded 120-block Markdown stress document");
    scheduleReindex();
  };

  const resetPrefs = async () => {
    prefs.reset();
    toast.info("Preferences reset to defaults");
    try {
      await prefs.flush();
    } catch {
      // Surfaced via prefs.error() effect
    }
  };

  return {
    scheme,
    setScheme,
    preset,
    setPreset,
    fontSize,
    setFontSize,
    splitSizes,
    setSplitSizes,
    viewMode,
    setViewMode,
    docs,
    activeDocId,
    setActiveDocId,
    activeDoc,
    hasUnsavedChanges,
    parsedResult,
    outlineTree,
    stats,
    workspaceName,
    workspaceTree,
    workspaceExpanded,
    setWorkspaceExpanded,
    bookmarks,
    externalEventLog,
    selectionRange,
    setSelectionRange,
    editorEl,
    setEditorEl,
    highlightedLine,
    pipelineImg,
    pastedImg,
    statusMsg,
    setStatusMsg,
    lastCopiedSnippet,
    indexedCount,
    searchQuery,
    setSearchQuery,
    searchHits,
    searchLoading: searchHostQuery.loading,
    searchError: searchHostQuery.error,
    refetchSearch: searchHostQuery.refetch,
    prefsHydrated: prefs.hydrated,
    prefsSaving: prefs.saving,
    prefsError: prefs.error,
    flushPrefs: prefs.flush,
    prefsWindowOpen,
    setPrefsWindowOpen,
    closeDialogOpen,
    setCloseDialogOpen,
    guardNavigation,
    setGuardNavigation,
    pendingLeavePath,
    setPendingLeavePath,
    updateActiveContent,
    wrapSelection,
    appendBlock,
    toggleTask,
    registerHeadingNode,
    jumpToHeading,
    copyCodeBlock,
    pasteClipboardImage,
    clearPastedImage,
    openLocalFile,
    saveActiveDocument,
    openWorkspaceFolder,
    restoreBookmarkByToken,
    revokeBookmarkByToken,
    refreshBookmarks,
    loadStressDoc,
    resetPrefs,
  };
}
