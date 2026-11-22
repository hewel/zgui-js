export type Accessor<T> = () => T;

export interface Setter<T> {
  <U extends T>(value: Exclude<U, Function>): U;
  <U extends T>(fn: (prev: T) => U): U;
}

export type Signal<T> = [get: Accessor<T>, set: Setter<T>];

export declare function createSignal<T>(
  initial: Exclude<T, Function>,
  options?: { readonly equals?: false | ((prev: T, next: T) => boolean) | undefined },
): Signal<T>;

export declare function createEffect<T>(
  compute: (previous: T | undefined) => T,
  apply: (value: T, previous: T | undefined) => (() => void) | void,
): void;

export declare function getOwner(): object | null;
export declare function onCleanup<T extends () => void>(callback: T): T;
export declare function untrack<T>(callback: () => T): T;

declare global {
  function setTimeout(callback: () => void, delay?: number): number;
  function clearTimeout(handle: number): void;
}
