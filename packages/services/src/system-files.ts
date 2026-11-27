import { getOwner, onCleanup } from "#solid";
import type { DragHandle, NativeNode, OutboundDragEndResult } from "@hewel/zgui/dnd";
import type { DirHandle, FileAccessMode, OpenDirectoryDialogOptions } from "./files.js";
import type { GrantToken } from "./grants.js";
import type { HostRequest, ImageHandle } from "./index.js";

export type SystemEntryKind = "file" | "dir" | "symlink" | "other";

/** The entry itself, not the target of a final symlink. */
export interface SystemEntryInfo {
  readonly name: string;
  readonly relativePath: string;
  readonly kind: SystemEntryKind;
  readonly size: number;
  readonly modifiedMs: number | null;
  readonly mode: number;
  readonly isHidden: boolean;
  readonly symlinkTarget: string | null;
  readonly symlinkBroken: boolean;
  readonly symlinkTargetKind: Exclude<SystemEntryKind, "symlink"> | null;
  readonly mimeType: string;
}

export interface ReadSystemDirOptions {
  readonly cursor?: string;
  readonly pageSize?: number;
  readonly includeHidden?: boolean;
  readonly directoriesFirst?: boolean;
  readonly sortBy?: "name" | "size" | "modifiedMs" | "kind";
  readonly sortDirection?: "asc" | "desc";
  readonly numericCollation?: boolean;
  readonly locale?: string;
}

/** One page in the current sorted listing; a cursor is not a filesystem snapshot. */
export interface SystemDirPage {
  readonly entries: readonly SystemEntryInfo[];
  readonly nextCursor: string | null;
  readonly totalEntries: number;
}

/** Opaque session authority. Use the returned `relativePath` to address entries. */
export interface SystemDirHandle {
  readonly kind: "system-dir";
  readonly name: string;
  readonly mode: FileAccessMode;
  readonly lstat: (relativePath?: string) => HostRequest<SystemEntryInfo>;
  readonly readDir: (
    relativePath?: string,
    options?: ReadSystemDirOptions,
  ) => HostRequest<SystemDirPage>;
  readonly close: () => void;
}

/** A path beneath a live handle. An omitted path names the directory itself. */
export interface SystemEntryRef {
  readonly dir: SystemDirHandle;
  readonly relativePath?: string;
}

export interface FileJobProgress {
  readonly bytesCompleted: number;
  readonly bytesTotal: number;
  readonly itemsCompleted: number;
  readonly itemsTotal: number;
  readonly currentPath: string;
}

export interface FileConflictInfo {
  readonly sourcePath: string;
  readonly destinationPath: string;
}

export type FileConflictResolution =
  | { readonly action: "overwrite" | "skip" | "autoRename"; readonly applyToAll?: boolean }
  | { readonly action: "cancel" };

export interface FileJobOptions {
  readonly onConflict?: (
    conflict: FileConflictInfo,
  ) => FileConflictResolution | Promise<FileConflictResolution>;
  readonly onProgress?: (progress: FileJobProgress) => void;
}

/** Copy and Move name a destination directory and preserve each source basename. */
export type FileJobSpec =
  | {
      readonly operation: "copy" | "move";
      readonly sources: readonly SystemEntryRef[];
      readonly destination: SystemEntryRef;
    }
  | { readonly operation: "trash"; readonly entries: readonly SystemEntryRef[] }
  | { readonly operation: "restoreFromTrash"; readonly ids: readonly string[] }
  | {
      readonly operation: "removePermanent";
      readonly entries: readonly SystemEntryRef[];
      readonly recursive?: boolean;
    }
  | { readonly operation: "mkdir"; readonly entry: SystemEntryRef }
  | { readonly operation: "createSymlink"; readonly entry: SystemEntryRef; readonly target: string }
  | { readonly operation: "setPermissions"; readonly entry: SystemEntryRef; readonly mode: number };

export interface TrashEntry {
  readonly id: string;
  readonly name: string;
  readonly originalPath: string;
  readonly deletionDate: string;
}

export interface SystemVolume {
  readonly mountPoint: string;
  readonly source: string;
  readonly fileSystem: string;
  readonly readOnly: boolean;
  readonly removable: boolean;
}

export type ThumbnailSize = "normal" | "large" | "x-large" | "xx-large";
export type ThumbnailOptions = SystemEntryRef & {
  readonly relativePath: string;
  readonly size?: ThumbnailSize;
};
export type OpenExternalTarget = ({ readonly url: string } | SystemEntryRef) & {
  readonly activationToken?: string;
};

type TrashListSpec = { readonly operation: "listTrash" };
interface SystemFilesBinding {
  readonly openSystemDirectory: (path: string) => SystemDirHandle;
  readonly openSystemDirectoryDialog: (
    options?: OpenDirectoryDialogOptions,
  ) => HostRequest<SystemDirHandle | null>;
  readonly openDroppedSystemDirectory: (
    directory: Pick<DirHandle, "kind" | "name" | "mode" | "close">,
  ) => SystemDirHandle;
  readonly persistSystemGrant: (handle: SystemDirHandle) => HostRequest<GrantToken>;
  readonly restoreSystemGrant: (token: GrantToken) => HostRequest<SystemDirHandle>;
  readonly startFileJob: (
    spec: FileJobSpec | TrashListSpec,
    options?: FileJobOptions,
  ) => HostRequest<void | readonly TrashEntry[]>;
  readonly listVolumes: () => HostRequest<readonly SystemVolume[]>;
  readonly watchVolumes: (callback: (volumes: readonly SystemVolume[]) => void) => () => void;
  readonly loadCachedThumbnail: (options: ThumbnailOptions) => HostRequest<ImageHandle | null>;
  readonly openExternal: (target: OpenExternalTarget) => HostRequest<void>;
}
export interface OutboundSystemDragOptions {
  readonly entries: readonly SystemEntryRef[];
  readonly allowedEffects?: readonly ("copy" | "move")[];
  readonly onEnd?: (result: OutboundDragEndResult) => void;
}
const runtime = globalThis as typeof globalThis & { readonly __zguiServices: SystemFilesBinding };
const dragRuntime = globalThis as typeof globalThis & {
  readonly __zguiDnd: {
    readonly startOutboundSystemDrag: (
      sourceNode: NativeNode,
      options: OutboundSystemDragOptions,
    ) => DragHandle;
  };
};

function ownedDirectory<T extends SystemDirHandle>(directory: T): T {
  if (getOwner()) onCleanup(() => directory.close());
  return directory;
}

function ownedDirectoryRequest<T extends SystemDirHandle | null>(
  request: HostRequest<T>,
): HostRequest<T> {
  if (getOwner()) {
    let disposed = false;
    let directory: T | undefined;
    void request.promise.then(
      handle => {
        directory = handle;
        if (disposed) handle?.close();
      },
      () => {},
    );
    onCleanup(() => {
      disposed = true;
      if (directory === undefined) request.cancel();
      else directory?.close();
    });
  }
  return request;
}

/** Open an exact manifest-declared root. A Solid owner closes it on cleanup. */
export function openSystemDirectory(path: string): SystemDirHandle {
  return ownedDirectory(runtime.__zguiServices.openSystemDirectory(path));
}

/** Ask the user to select a root during a live gesture. */
export function openSystemDirectoryDialog(
  options?: OpenDirectoryDialogOptions,
): HostRequest<SystemDirHandle | null> {
  return ownedDirectoryRequest(runtime.__zguiServices.openSystemDirectoryDialog(options));
}

/** Redeem a read-only directory from an accepted external drop, not a workspace handle. */
export function openDroppedSystemDirectory(
  directory: Pick<DirHandle, "kind" | "name" | "mode" | "close">,
): SystemDirHandle {
  return ownedDirectory(runtime.__zguiServices.openDroppedSystemDirectory(directory));
}

/** Persist this handle as a system-dir grant, subject to the sealed authority ceiling. */
export function persistSystemGrant(handle: SystemDirHandle): HostRequest<GrantToken> {
  return runtime.__zguiServices.persistSystemGrant(handle);
}

/** Restore system-dir authority without promoting workspace grants. */
export function restoreSystemGrant(token: GrantToken): HostRequest<SystemDirHandle> {
  return ownedDirectoryRequest(runtime.__zguiServices.restoreSystemGrant(token));
}

/** Start background work. With no conflict handler, a collision cancels the job. */
export function startFileJob(spec: FileJobSpec, options?: FileJobOptions): HostRequest<void>;
export function startFileJob(spec: TrashListSpec): HostRequest<readonly TrashEntry[]>;
export function startFileJob(
  spec: FileJobSpec | TrashListSpec,
  options?: FileJobOptions,
): HostRequest<void | readonly TrashEntry[]> {
  return runtime.__zguiServices.startFileJob(spec, options);
}

/** List Trash entries whose original locations lie within active system roots. */
export function listTrash(): HostRequest<readonly TrashEntry[]> {
  return startFileJob({ operation: "listTrash" });
}

/** Inventory mounted local, removable and user-space volumes. */
export function listVolumes(): HostRequest<readonly SystemVolume[]> {
  return runtime.__zguiServices.listVolumes();
}

/** Deliver the initial volume inventory and later changes. A Solid owner stops watching on cleanup. */
export function watchVolumes(callback: (volumes: readonly SystemVolume[]) => void): () => void {
  const stop = runtime.__zguiServices.watchVolumes(callback);
  if (getOwner()) onCleanup(stop);
  return stop;
}

/** Load an existing fresh thumbnail, or return null on a cache miss. Release a hit when no longer needed. */
export function loadCachedThumbnail(options: ThumbnailOptions): HostRequest<ImageHandle | null> {
  return runtime.__zguiServices.loadCachedThumbnail(options);
}

/** Ask the system handler to open a granted entry or HTTP(S)/mailto URL. Requires a gesture or token. */
export function openExternal(target: OpenExternalTarget): HostRequest<void> {
  return runtime.__zguiServices.openExternal(target);
}

/** Offer authorized paths during a live pointer/touch press without reading file contents. */
export function startOutboundSystemDrag(
  sourceNode: NativeNode,
  options: OutboundSystemDragOptions,
): DragHandle {
  const handle = dragRuntime.__zguiDnd.startOutboundSystemDrag(sourceNode, options);
  if (getOwner()) onCleanup(() => handle.cancel());
  return handle;
}
