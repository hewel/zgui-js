// Solid 2.x's public declarations include DOM/hydration types. This package-local
// condition preserves native renderer node branding without acquiring browser globals.
import type {
  Accessor,
  MemoOptions,
  NativeChild,
  ReactiveOptions,
  Signal,
  SignalOptions,
} from "./types.js";

export type Store<T> = T;
export type StoreSetter<T> = (fn: (draft: T) => T | void) => void;
export type StoreReturn<T> = [get: Store<T>, set: StoreSetter<T>];
export type Owner = object;

export declare function createRoot<T>(callback: (dispose: () => void) => T): T;
export declare function createSignal<T>(): Signal<T | undefined>;
export declare function createSignal<T>(
  initial: Exclude<T, Function>,
  options?: SignalOptions<T>,
): Signal<T>;
export declare function createSignal<T>(
  compute: (previous: T | undefined) => T | PromiseLike<T>,
  options?: SignalOptions<T>,
): Signal<T>;
export declare function createMemo<T>(
  compute: (previous: T | undefined) => T | PromiseLike<T>,
  options?: MemoOptions<T>,
): Accessor<T>;
export declare function createEffect<T>(
  compute: (previous: T | undefined) => T | PromiseLike<T>,
  apply:
    | ((value: T, previous: T | undefined) => (() => void) | void)
    | {
        effect: (value: T, previous: T | undefined) => (() => void) | void;
        error: (err: unknown, cleanup: () => void) => void;
      },
  options?: ReactiveOptions,
): void;
export declare function onSettled(callback: () => (() => void) | void): void;
export declare function onCleanup<T extends () => void>(callback: T): T;
export declare function getOwner(): Owner | null;
export declare function runWithOwner<T>(owner: Owner | null, callback: () => T): T;
export declare function flush(): void;
export declare function flush<T>(callback: () => T): T;
export declare function untrack<T>(callback: () => T): T;

/** Identity-keyed mapping by default; pass `keyed={false}` for index-stabilized rows. */
export declare function For<T>(props: {
  each: readonly T[] | null | undefined | false;
  fallback?: NativeChild;
  keyed?: true;
  children: (item: T, index: Accessor<number>) => NativeChild;
}): NativeChild;
export declare function For<T>(props: {
  each: readonly T[] | null | undefined | false;
  fallback?: NativeChild;
  keyed: false;
  children: (item: Accessor<T>, index: number) => NativeChild;
}): NativeChild;
export declare function For<T>(props: {
  each: readonly T[] | null | undefined | false;
  fallback?: NativeChild;
  keyed: (item: T) => unknown;
  children: (item: Accessor<T>, index: Accessor<number>) => NativeChild;
}): NativeChild;

export declare function Repeat(props: {
  count: number;
  from?: number;
  fallback?: NativeChild;
  children: ((index: number) => NativeChild) | NativeChild;
}): NativeChild;

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

export declare function Switch(props: {
  fallback?: NativeChild;
  children: NativeChild;
}): NativeChild;
export declare function Match<T>(props: {
  when: T | null | undefined | false;
  keyed?: boolean;
  children: NativeChild | ((item: Accessor<NonNullable<T>>) => NativeChild);
}): NativeChild;

export declare function Loading(props: {
  fallback?: NativeChild;
  on?: unknown;
  children: NativeChild;
}): NativeChild;

export declare function Errored(props: {
  fallback: NativeChild | ((err: Accessor<unknown>, reset: () => void) => NativeChild);
  children: NativeChild;
}): NativeChild;

export interface Context<T> {
  (props: { value: T; children?: NativeChild }): NativeChild;
  id: symbol;
  defaultValue: T | undefined;
}
export declare function createContext<T>(): Context<T>;
export declare function createContext<T>(defaultValue: T, options?: { name?: string }): Context<T>;
export declare function useContext<T>(context: Context<T>): T;
export declare function createUniqueId(): string;

export declare function omit<T extends object, K extends readonly (keyof T)[]>(
  props: T,
  ...keys: K
): { [P in keyof T as Exclude<P, K[number]>]: T[P] };
type MergeSources<T extends readonly object[]> = T extends readonly [
  infer Head extends object,
  ...infer Tail extends object[],
]
  ? Omit<Head, keyof MergeSources<Tail>> & MergeSources<Tail>
  : {};
export declare function merge<T extends readonly object[]>(...sources: T): MergeSources<T>;

export declare function createStore<T extends object = {}>(
  initialValue: T,
  options?: ReactiveOptions & { shallow?: boolean },
): StoreReturn<T>;
export declare function createProjection<T extends object = {}>(
  derive: (draft: T) => void | T,
  seed: T,
  options?: ReactiveOptions & { key?: string | ((item: any) => any) | null },
): Store<T>;
export declare function reconcile<T>(
  value: T,
  key?: string | ((item: any) => any) | null,
): (state: T) => T;
export declare function snapshot<T>(value: T): T;
export declare function isPending(fn: () => unknown): boolean;
export declare function latest<T>(fn: () => T): T;
export declare const $PROXY: unique symbol;
export declare function resetErrorHalt(): void;
export declare function refresh<T>(target: T): Promise<unknown>;
export declare function action<Args extends unknown[], Y, R>(
  genFn: (...args: Args) => Generator<Y, R, any> | AsyncGenerator<Y, R, any>,
): (...args: Args) => Promise<R>;
