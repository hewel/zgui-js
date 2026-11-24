import { getOwner, onCleanup } from "#solid";
import type { HostRequest } from "./index.js";

/** Access level granted to a file or directory capability handle. */
export type FileAccessMode = "read" | "readwrite";

/** Opaque per-app persistent grant token returned by `persistHandle`. */
export type BookmarkToken = string;

/** Metadata returned by `stat(...)`. */
export interface FileStat {
  readonly isFile: boolean;
  readonly isDir: boolean;
  readonly size: number;
  readonly modifiedMs: number | null;
}

/** One entry returned by `DirHandle.readDir(...)`. */
export interface DirEntry {
  readonly name: string;
  readonly isFile: boolean;
  readonly isDir: boolean;
  readonly size: number;
}

/** Options bounding an asynchronous file read. */
export interface FileReadOptions {
  /** Maximum bytes to read (1..=16 MiB). Exceeding rejects with `TooLarge`. */
  readonly maxBytes?: number;
}

/** Options for directory creation. */
export interface MkdirOptions {
  readonly recursive?: boolean;
}

/** Options for entry removal. */
export interface RemoveOptions {
  readonly recursive?: boolean;
}

/** File-picker extension filter group. */
export interface FileDialogFilter {
  readonly name: string;
  readonly extensions: readonly string[];
}

/** Platform-neutral options for `openFileDialog`. */
export interface OpenFileDialogOptions {
  readonly title?: string;
  readonly readOnly?: boolean;
  readonly filters?: readonly FileDialogFilter[];
}

/** Platform-neutral options for `saveFileDialog`. */
export interface SaveFileDialogOptions {
  readonly title?: string;
  readonly suggestedName?: string;
  readonly filters?: readonly FileDialogFilter[];
}

/** Platform-neutral options for `openDirectoryDialog`. */
export interface OpenDirectoryDialogOptions {
  readonly title?: string;
  readonly readOnly?: boolean;
}

/** Metadata describing a persisted per-app bookmark. */
export interface BookmarkInfo {
  readonly token: BookmarkToken;
  readonly kind: "file" | "dir";
  readonly label: string;
  readonly mode: FileAccessMode;
  readonly createdMs: number;
}

/** Coalesced directory watch event kind. */
export type WatchEventKind = "create" | "modify" | "remove" | "rename" | "other";

/** One coalesced filesystem change relative to the watched `DirHandle`. */
export interface WatchEvent {
  readonly path: string;
  readonly kind: WatchEventKind;
}

/** Options for debounced `watchDirectory`. */
export interface WatchDirectoryOptions {
  readonly recursive?: boolean;
  /** Debounce window in milliseconds (10..=5000, default 100). */
  readonly debounceMs?: number;
}

/** Opaque session-scoped handle to a single authorized file. */
export interface FileHandle {
  readonly kind: "file";
  readonly name: string;
  readonly mode: FileAccessMode;
  readText(options?: FileReadOptions): HostRequest<string>;
  readBytes(options?: FileReadOptions): HostRequest<Uint8Array>;
  /** Atomically writes UTF-8 text via temp file + fsync + rename. */
  writeText(text: string): HostRequest<void>;
  /** Atomically writes raw bytes via temp file + fsync + rename. */
  writeBytes(bytes: Uint8Array): HostRequest<void>;
  stat(): HostRequest<FileStat>;
  /** Releases the session capability handle. Idempotent. */
  close(): void;
}

/** Opaque session-scoped handle to an authorized directory root. */
export interface DirHandle {
  readonly kind: "dir";
  readonly name: string;
  readonly mode: FileAccessMode;
  readText(relativePath: string, options?: FileReadOptions): HostRequest<string>;
  readBytes(relativePath: string, options?: FileReadOptions): HostRequest<Uint8Array>;
  /** Atomically writes UTF-8 text beneath this directory root. */
  writeText(relativePath: string, text: string): HostRequest<void>;
  /** Atomically writes raw bytes beneath this directory root. */
  writeBytes(relativePath: string, bytes: Uint8Array): HostRequest<void>;
  stat(relativePath?: string): HostRequest<FileStat>;
  readDir(relativePath?: string): HostRequest<readonly DirEntry[]>;
  mkdir(relativePath: string, options?: MkdirOptions): HostRequest<void>;
  rename(fromRelativePath: string, toRelativePath: string): HostRequest<void>;
  remove(relativePath: string, options?: RemoveOptions): HostRequest<void>;
  /** Releases the session capability handle. Idempotent. */
  close(): void;
}

interface HostFilesBinding {
  openFileDialog(options?: OpenFileDialogOptions): HostRequest<FileHandle | null>;
  saveFileDialog(options?: SaveFileDialogOptions): HostRequest<FileHandle | null>;
  openDirectoryDialog(options?: OpenDirectoryDialogOptions): HostRequest<DirHandle | null>;
  openAuthorizedDirectory(path: string): DirHandle;
  persistHandle(handle: FileHandle | DirHandle): HostRequest<BookmarkToken>;
  restoreBookmark(token: BookmarkToken): HostRequest<FileHandle | DirHandle>;
  listBookmarks(): HostRequest<readonly BookmarkInfo[]>;
  revokeBookmark(token: BookmarkToken): HostRequest<boolean>;
  watchDirectory(
    dir: DirHandle,
    options: WatchDirectoryOptions | undefined,
    callback: (events: readonly WatchEvent[]) => void,
  ): () => void;
}

function hostFiles(): HostFilesBinding {
  return (globalThis as unknown as { readonly __zguiServices: HostFilesBinding }).__zguiServices;
}

/** Open a native file picker dialog (requires an active user interaction gesture). */
export function openFileDialog(options?: OpenFileDialogOptions): HostRequest<FileHandle | null> {
  return hostFiles().openFileDialog(options);
}

/** Open a native save-file dialog (requires an active user interaction gesture). */
export function saveFileDialog(options?: SaveFileDialogOptions): HostRequest<FileHandle | null> {
  return hostFiles().saveFileDialog(options);
}

/** Open a native directory picker dialog (requires an active user interaction gesture). */
export function openDirectoryDialog(
  options?: OpenDirectoryDialogOptions,
): HostRequest<DirHandle | null> {
  return hostFiles().openDirectoryDialog(options);
}

/**
 * Open a directory pre-declared in `manifest.json` (`capabilities.files.directories`).
 * Automatically closes on Solid owner cleanup when called inside an active owner.
 */
export function openAuthorizedDirectory(path: string): DirHandle {
  const dir = hostFiles().openAuthorizedDirectory(path);
  if (getOwner()) {
    onCleanup(() => dir.close());
  }
  return dir;
}

/** Persist an authorized `FileHandle` or `DirHandle` into the app's bookmark store. */
export function persistHandle(handle: FileHandle | DirHandle): HostRequest<BookmarkToken> {
  return hostFiles().persistHandle(handle);
}

/** Restore a previously persisted bookmark token into a live `FileHandle` or `DirHandle`. */
export function restoreBookmark(token: BookmarkToken): HostRequest<FileHandle | DirHandle> {
  return hostFiles().restoreBookmark(token);
}

/** List persisted bookmark metadata owned by this application. */
export function listBookmarks(): HostRequest<readonly BookmarkInfo[]> {
  return hostFiles().listBookmarks();
}

/** Revoke a persisted bookmark token. Resolves to `true` if the token existed. */
export function revokeBookmark(token: BookmarkToken): HostRequest<boolean> {
  return hostFiles().revokeBookmark(token);
}

/**
 * Watch an authorized directory for debounced, coalesced change batches.
 * Automatically stops watching on Solid owner cleanup when called inside an active owner.
 */
export function watchDirectory(
  dir: DirHandle,
  callback: (events: readonly WatchEvent[]) => void,
): () => void;
export function watchDirectory(
  dir: DirHandle,
  options: WatchDirectoryOptions,
  callback: (events: readonly WatchEvent[]) => void,
): () => void;
export function watchDirectory(
  dir: DirHandle,
  optionsOrCallback: WatchDirectoryOptions | ((events: readonly WatchEvent[]) => void),
  maybeCallback?: (events: readonly WatchEvent[]) => void,
): () => void {
  const options = typeof optionsOrCallback === "function" ? undefined : optionsOrCallback;
  const callback = typeof optionsOrCallback === "function" ? optionsOrCallback : maybeCallback;
  if (typeof callback !== "function") {
    throw new TypeError("watchDirectory requires a callback function");
  }
  const stop = hostFiles().watchDirectory(dir, options, callback);
  if (getOwner()) {
    onCleanup(stop);
  }
  return stop;
}
