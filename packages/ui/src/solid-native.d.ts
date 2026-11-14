// Solid's public declarations assume DOM JSX. This package-local type condition
// describes the pinned runtime's native renderer contract without browser globals.
import type { Accessor, MemoOptions, NativeChild, ReactiveOptions, Signal } from "./types.js";

export declare function createRoot<T>(callback: (dispose: () => void) => T): T;
export declare function createSignal<T>(): Signal<T | undefined>;
export declare function createSignal<T>(initial: T, options?: MemoOptions<T>): Signal<T>;
export declare function createMemo<T>(compute: (previous: T | undefined) => T): Accessor<T>;
export declare function createMemo<T>(compute: (previous: T) => T, initial: T, options?: MemoOptions<T>): Accessor<T>;
export declare function createMemo<T, I = T>(compute: (previous: T | I) => T, initial: I, options?: MemoOptions<T>): Accessor<T>;
export declare function createEffect<T>(effect: (previous: T | undefined) => T): void;
export declare function createEffect<T>(effect: (previous: T) => T, initial: T, options?: ReactiveOptions): void;
export declare function createEffect<T, I = T>(effect: (previous: T | I) => T, initial: I, options?: ReactiveOptions): void;
export declare function onMount(callback: () => void): void;
export declare function onCleanup<T extends () => void>(callback: T): T;
export declare function batch<T>(callback: () => T): T;
export declare function untrack<T>(callback: () => T): T;
/** Identity-keyed mapping: reordering the same item objects preserves their native nodes. */
export declare function For<T>(props: {
  each: readonly T[] | null | undefined | false;
  fallback?: NativeChild;
  children: (item: T, index: Accessor<number>) => NativeChild;
}): NativeChild;
/** Position-keyed mapping: each item accessor changes while its index stays fixed. */
export declare function Index<T>(props: {
  each: readonly T[] | null | undefined | false;
  fallback?: NativeChild;
  children: (item: Accessor<T>, index: number) => NativeChild;
}): NativeChild;
/**
 * Conditional children must be constructed lazily, e.g. <Show when={visible()}>
 * <text>Fresh branch</text></Show>. Do not pass cached JSX that was previously removed.
 */
export declare function Show<T>(props: {
  when: T | null | undefined | false;
  keyed?: false;
  fallback?: NativeChild;
  children: NativeChild | ((item: Accessor<NonNullable<T>>) => NativeChild);
}): NativeChild;
export declare function Show<T>(props: {
  when: T | null | undefined | false;
  keyed: true;
  fallback?: NativeChild;
  children: NativeChild | ((item: NonNullable<T>) => NativeChild);
}): NativeChild;
