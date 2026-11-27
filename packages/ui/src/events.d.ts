import type { NativeNode } from "./index.js";

/** Snapshot data is retainable; cancellation methods expire when the handler returns. */
export interface NativeEvent {
  readonly type: string;
  /** Monotonic milliseconds in the native clock's origin. */
  readonly timestamp: number;
  readonly phase: "capture" | "target" | "bubble";
  readonly target: NativeNode;
  readonly currentTarget: NativeNode;
  readonly modifiers: {
    readonly shift: boolean;
    readonly control: boolean;
    readonly alt: boolean;
    readonly meta: boolean;
    readonly capsLock: boolean;
    readonly numLock: boolean;
  };
  preventDefault(): void;
  stopPropagation(): void;
  stopImmediatePropagation(): void;
}
export interface NativePointerEvent extends NativeEvent {
  readonly position: { readonly x: number; readonly y: number };
  readonly pointerId: string;
  readonly pointerKind: "mouse" | "touch" | "pen" | "unknown";
  readonly primary: boolean;
  readonly button: string | null;
  readonly buttonIndex?: number;
  readonly pressure: number | null;
  readonly coalesced: readonly {
    readonly position: { readonly x: number; readonly y: number };
    readonly pressure: number | null;
    readonly timestamp: number;
  }[];
  releasePointer(): void;
}
export type NativePointerBoundaryEvent = Omit<NativePointerEvent, "releasePointer">;
export type NativeContextMenuEvent = Omit<NativePointerEvent, "releasePointer">;
export interface NativePointerDownEvent extends NativePointerEvent {
  activateNow(): void;
  capturePointer(): void;
}
export interface NativeWheelEvent extends NativeEvent {
  readonly type: "wheel";
  readonly position: NativePointerEvent["position"];
  readonly pointerId: NativePointerEvent["pointerId"];
  readonly pointerKind: NativePointerEvent["pointerKind"];
  /** Positive deltas move the scroll offset right and down. */
  readonly delta: { readonly x: number; readonly y: number; readonly unit: "pixels" | "lines" };
  /** Scroll gesture phase, separate from the event's propagation phase. */
  readonly scrollPhase: "discrete" | "started" | "moved" | "ended" | "momentum" | "unknown";
}
export interface NativeKeyEvent extends NativeEvent {
  readonly key: string | null;
  readonly character: string | null;
  readonly keyWithoutModifiers: string | null;
  readonly deadKey: string | null;
  readonly code: string | null;
  readonly physicalCode: number | null;
  readonly location: string;
  readonly repeat: boolean;
  /** Whether a platform IME composition (preedit) was active when this key event arrived. */
  readonly isComposing: boolean;
}
export interface NativeValueEvent extends NativeEvent {
  readonly value: string;
  /** Selection offsets in JavaScript UTF-16 code units (matching `value.slice(start, end)`). */
  readonly selection: { readonly start: number; readonly end: number };
  readonly valueChange: "input" | "committed";
}
/** onFocus/onBlur subscribe to native bubbling focusin/focusout. */
export interface NativeFocusEvent extends NativeEvent {
  readonly cause: string;
}
export interface NativeAnimationEvent extends NativeEvent {
  readonly animationName: string;
  readonly elapsedTime: number;
  readonly pseudoElement: string;
}
export interface NativeTransitionEvent extends NativeEvent {
  readonly propertyName: string;
  readonly elapsedTime: number;
  readonly pseudoElement: string;
}

interface DroppedRequest<T> {
  readonly promise: Promise<T>;
  readonly cancel: () => void;
}

interface DroppedStat {
  readonly isFile: boolean;
  readonly isDir: boolean;
  readonly size: number;
  readonly modifiedMs: number | null;
}

interface DroppedDirectoryEntry {
  readonly name: string;
  readonly isFile: boolean;
  readonly isDir: boolean;
  readonly size: number;
}

/** Read-only file capability handle minted by the host on an accepted external file drop. */
export interface DroppedFileHandle {
  readonly kind: "file";
  readonly name: string;
  readonly mode: "read";
  readonly readText: (options?: { readonly maxBytes?: number }) => DroppedRequest<string>;
  readonly readBytes: (options?: { readonly maxBytes?: number }) => DroppedRequest<Uint8Array>;
  readonly stat: () => DroppedRequest<DroppedStat>;
  readonly close: () => void;
}

/** Read-only, confined directory capability minted on an accepted external drop. */
export interface DroppedDirectoryHandle {
  readonly kind: "dir";
  readonly name: string;
  readonly mode: "read";
  readonly readText: (
    relativePath: string,
    options?: { readonly maxBytes?: number },
  ) => DroppedRequest<string>;
  readonly readBytes: (
    relativePath: string,
    options?: { readonly maxBytes?: number },
  ) => DroppedRequest<Uint8Array>;
  readonly stat: (relativePath?: string) => DroppedRequest<DroppedStat>;
  readonly readDir: (relativePath?: string) => DroppedRequest<readonly DroppedDirectoryEntry[]>;
  readonly close: () => void;
}

/** Hit-tested drag hover event (`onDragEnter`, `onDragOver`, `onDragLeave`). External drags expose MIME types and fileCount only — never raw paths or handles. */
export interface NativeDragEvent {
  readonly dragId: number;
  readonly source: "internal" | "external";
  readonly types: readonly string[];
  readonly fileCount: number;
  readonly clientX: number;
  readonly clientY: number;
  readonly localX: number;
  readonly localY: number;
  readonly targetWidth: number;
  readonly targetHeight: number;
  /** Synchronously negotiate acceptance in external `onDragEnter` or in `onDragOver`. Only accept Move if the destination takes responsibility for the data. */
  accept(effect?: "copy" | "move" | "none"): void;
  stopPropagation(): void;
}
/** Hit-tested drop event delivered only to an accepting target. */
export interface NativeDropEvent {
  readonly dragId: number;
  readonly source: "internal" | "external";
  readonly types: readonly string[];
  readonly effect: "copy" | "move" | "none";
  readonly fileCount: number;
  readonly clientX: number;
  readonly clientY: number;
  readonly localX: number;
  readonly localY: number;
  readonly targetWidth: number;
  readonly targetHeight: number;
  /** Opaque session-local payload for internal drags (`undefined` for external drops). */
  readonly payload: unknown;
  /** Read-only Wave 1 `FileHandle`s minted exclusively for accepted external drops. */
  readonly files: readonly DroppedFileHandle[];
  /** Read-only directories scoped to their dropped roots; empty for internal or file-only drops. */
  readonly directories: readonly DroppedDirectoryHandle[];
  stopPropagation(): void;
}
interface EventMap {
  onClick: NativeEvent;
  onPointerDown: NativePointerDownEvent;
  onPointerMove: NativePointerEvent;
  onPointerEnter: NativePointerBoundaryEvent;
  onPointerLeave: NativePointerBoundaryEvent;
  onPointerUp: NativePointerEvent;
  onPointerCancel: NativePointerEvent;
  onWheel: NativeWheelEvent;
  onContextMenu: NativeContextMenuEvent;
  onKeyDown: NativeKeyEvent;
  onKeyUp: NativeKeyEvent;
  onInput: NativeValueEvent;
  onChange: NativeValueEvent;
  onFocus: NativeFocusEvent;
  onBlur: NativeFocusEvent;
  onAnimationStart: NativeAnimationEvent;
  onAnimationIteration: NativeAnimationEvent;
  onAnimationEnd: NativeAnimationEvent;
  onAnimationCancel: NativeAnimationEvent;
  onTransitionRun: NativeTransitionEvent;
  onTransitionStart: NativeTransitionEvent;
  onTransitionEnd: NativeTransitionEvent;
  onTransitionCancel: NativeTransitionEvent;
}
interface DndEventMap {
  onDragEnter: NativeDragEvent;
  onDragOver: NativeDragEvent;
  onDragLeave: NativeDragEvent;
  onDrop: NativeDropEvent;
}
export type NativeEventProps = {
  [Name in keyof EventMap]?: ((event: EventMap[Name]) => void) | null;
} & {
  [Name in keyof EventMap as `${Name}Capture`]?: ((event: EventMap[Name]) => void) | null;
} & {
  [Name in keyof DndEventMap]?: ((event: DndEventMap[Name]) => void) | null;
};
