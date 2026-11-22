import {
  listBookmarks,
  openDirectoryDialog,
  openFileDialog,
  persistHandle,
  restoreBookmark,
  revokeBookmark,
  saveFileDialog,
  watchDirectory,
} from "@hewel/zgui-services";
import type { BookmarkInfo, DirHandle, FileHandle, WatchEvent } from "@hewel/zgui-services";
import type { TreeNodeData } from "@hewel/zgui/components";
import type { MarkdownDoc } from "./samples.js";

export interface WorkspaceNodeMeta {
  readonly kind: "dir" | "file";
  readonly relativePath: string;
  readonly docId?: string | undefined;
}

export interface ScannedWorkspace {
  readonly tree: readonly TreeNodeData<WorkspaceNodeMeta>[];
  readonly docs: readonly MarkdownDoc[];
  readonly expandedDirIds: readonly string[];
}

const MARKDOWN_EXT_RE = /\.(md|markdown|txt)$/i;

function joinRel(prefix: string, name: string): string {
  return prefix ? `${prefix}/${name}` : name;
}

export function workspaceDocId(relativePath: string): string {
  return `ws:${relativePath}`;
}

async function scanDirectoryRecursive(
  dir: DirHandle,
  prefix: string,
  docs: MarkdownDoc[],
  expandedDirIds: string[],
  depth = 0,
): Promise<TreeNodeData<WorkspaceNodeMeta>[]> {
  if (depth > 4) return [];
  const entries = await dir.readDir(prefix || undefined).promise;
  const sorted = [...entries].sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

  const nodes: TreeNodeData<WorkspaceNodeMeta>[] = [];
  for (const entry of sorted) {
    if (entry.name.startsWith(".")) continue;
    const rel = joinRel(prefix, entry.name);
    if (entry.isDir) {
      const id = `dir:${rel}`;
      if (depth === 0) expandedDirIds.push(id);
      const children = await scanDirectoryRecursive(dir, rel, docs, expandedDirIds, depth + 1);
      nodes.push({
        id,
        label: entry.name,
        badge: String(children.length),
        meta: { kind: "dir", relativePath: rel },
        children,
      });
    } else if (entry.isFile && MARKDOWN_EXT_RE.test(entry.name)) {
      let content = "";
      try {
        content = await dir.readText(rel).promise;
      } catch {
        continue;
      }
      const docId = workspaceDocId(rel);
      docs.push({
        id: docId,
        title: entry.name,
        badge: "Disk",
        content,
        source: "workspace",
        relativePath: rel,
        dirty: false,
      });
      nodes.push({
        id: docId,
        label: entry.name,
        badge: "MD",
        meta: { kind: "file", relativePath: rel, docId },
      });
    }
  }
  return nodes;
}

export async function scanWorkspaceDir(dir: DirHandle): Promise<ScannedWorkspace> {
  const docs: MarkdownDoc[] = [];
  const expandedDirIds: string[] = [];
  const tree = await scanDirectoryRecursive(dir, "", docs, expandedDirIds);
  return { tree, docs, expandedDirIds };
}

export interface WorkspaceController {
  readonly openLocalFile: () => Promise<{ doc: MarkdownDoc; handle: FileHandle } | null>;
  readonly saveDocument: (
    doc: MarkdownDoc,
  ) => Promise<{ savedName: string; relativePath?: string } | null>;
  readonly openFolder: (
    onExternalEvents: (events: readonly WatchEvent[]) => void,
  ) => Promise<{ dir: DirHandle; bookmarkToken: string | null; scanned: ScannedWorkspace } | null>;
  readonly restoreWorkspaceBookmark: (
    token: string,
    onExternalEvents: (events: readonly WatchEvent[]) => void,
  ) => Promise<{ dir: DirHandle; scanned: ScannedWorkspace } | null>;
  readonly refreshWorkspace: () => Promise<ScannedWorkspace | null>;
  readonly readWorkspaceFile: (relativePath: string) => Promise<string | null>;
  readonly listSavedBookmarks: () => Promise<readonly BookmarkInfo[]>;
  readonly revokeSavedBookmark: (token: string) => Promise<boolean>;
  readonly activeDirName: () => string | null;
  readonly close: () => void;
}

export function createWorkspaceController(): WorkspaceController {
  let activeDir: DirHandle | null = null;
  let activeWatcherStop: (() => void) | null = null;
  const standaloneFiles = new Map<string, FileHandle>();

  const stopWatcher = () => {
    if (activeWatcherStop) {
      activeWatcherStop();
      activeWatcherStop = null;
    }
  };

  const replaceActiveDir = (
    nextDir: DirHandle,
    onExternalEvents: (events: readonly WatchEvent[]) => void,
  ) => {
    stopWatcher();
    if (activeDir && activeDir !== nextDir) {
      activeDir.close();
    }
    activeDir = nextDir;
    activeWatcherStop = watchDirectory(
      nextDir,
      { recursive: true, debounceMs: 60 },
      onExternalEvents,
    );
  };

  return {
    async openLocalFile() {
      const handle = await openFileDialog({
        title: "Open Markdown File",
        filters: [{ name: "Markdown", extensions: ["md", "markdown", "txt"] }],
      }).promise;
      if (!handle) return null;
      const content = await handle.readText().promise;
      const id = `file:${handle.name}`;
      const previous = standaloneFiles.get(id);
      if (previous && previous !== handle) previous.close();
      standaloneFiles.set(id, handle);
      const doc: MarkdownDoc = {
        id,
        title: handle.name,
        badge: "File",
        content,
        source: "file",
        relativePath: handle.name,
        dirty: false,
      };
      return { doc, handle };
    },

    async saveDocument(doc) {
      if (doc.source === "workspace" && activeDir && doc.relativePath) {
        await activeDir.writeText(doc.relativePath, doc.content).promise;
        return { savedName: doc.title, relativePath: doc.relativePath };
      }
      const existingHandle = standaloneFiles.get(doc.id);
      if (existingHandle && existingHandle.mode === "readwrite") {
        await existingHandle.writeText(doc.content).promise;
        return { savedName: existingHandle.name, relativePath: existingHandle.name };
      }
      const suggested = doc.title.endsWith(".md") ? doc.title : `${doc.id}.md`;
      const savedHandle = await saveFileDialog({
        title: "Save Markdown File",
        suggestedName: suggested,
        filters: [{ name: "Markdown", extensions: ["md", "markdown"] }],
      }).promise;
      if (!savedHandle) return null;
      await savedHandle.writeText(doc.content).promise;
      const prev = standaloneFiles.get(doc.id);
      if (prev && prev !== savedHandle) prev.close();
      standaloneFiles.set(doc.id, savedHandle);
      return { savedName: savedHandle.name, relativePath: savedHandle.name };
    },

    async openFolder(onExternalEvents) {
      const dir = await openDirectoryDialog({ title: "Open Markdown Workspace Folder" }).promise;
      if (!dir) return null;
      let bookmarkToken: string | null = null;
      try {
        bookmarkToken = await persistHandle(dir).promise;
      } catch {
        bookmarkToken = null;
      }
      replaceActiveDir(dir, onExternalEvents);
      const scanned = await scanWorkspaceDir(dir);
      return { dir, bookmarkToken, scanned };
    },

    async restoreWorkspaceBookmark(token, onExternalEvents) {
      const restored = await restoreBookmark(token).promise;
      if (!restored) return null;
      if (restored.kind !== "dir") {
        restored.close();
        return null;
      }
      replaceActiveDir(restored, onExternalEvents);
      const scanned = await scanWorkspaceDir(restored);
      return { dir: restored, scanned };
    },

    async refreshWorkspace() {
      if (!activeDir) return null;
      return scanWorkspaceDir(activeDir);
    },

    async readWorkspaceFile(relativePath) {
      if (!activeDir) return null;
      try {
        return await activeDir.readText(relativePath).promise;
      } catch {
        return null;
      }
    },

    async listSavedBookmarks() {
      return listBookmarks().promise;
    },

    async revokeSavedBookmark(token) {
      return revokeBookmark(token).promise;
    },

    activeDirName: () => activeDir?.name ?? null,

    close() {
      stopWatcher();
      if (activeDir) {
        activeDir.close();
        activeDir = null;
      }
      for (const handle of standaloneFiles.values()) {
        handle.close();
      }
      standaloneFiles.clear();
    },
  };
}
