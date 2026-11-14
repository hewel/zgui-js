import "./runtime-globals.js";
export type { JSX } from "./jsx-runtime.js";
import type { NativeEventProps } from "./events.js";
export type { NativeEvent, NativePointerEvent, NativeKeyEvent, NativeValueEvent, NativeFocusEvent } from "./events.js";
import type { NativeStyle } from "./styles.js";
export type { NativeStyle, NativeStyleName } from "./styles.js";

declare const nativeNode: unique symbol;
/**
 * Borrowed opaque handle, never a native numeric ID. Permanently removed branches
 * invalidate their handles at the end of the synchronous host turn. A retained
 * ref does not retain native ownership; cross-turn detached-node reuse is unsupported.
 */
export interface NativeNode { readonly [nativeNode]: true; }
export type NativeChild = NativeNode | string | number | boolean | null | undefined
  | readonly NativeChild[] | (() => NativeChild);
export type Accessor<T> = () => T;
export type Setter<T> = {
  <U extends T>(value: (previous: T) => U): U;
  <U extends T>(value: Exclude<U, Function>): U;
  <U extends T>(...args: undefined extends T ? [] : [value: Exclude<U, Function> | ((previous: T) => U)]): undefined extends T ? undefined : U;
};
export type Signal<T> = [get: Accessor<T>, set: Setter<T>];
export type Component<P = {}> = (props: P) => NativeChild;
export type ParentProps<P = {}> = P & { children?: NativeChild };
export type ParentComponent<P = {}> = Component<ParentProps<P>>;
/**
 * A callback receives the live node during construction, possibly before attachment.
 * It is not called with null on disposal. Use onCleanup for subscriptions and never
 * use a saved ref after its branch is removed. A variable ref is assigned by the compiler.
 */
export type NativeRef = NativeNode | ((node: NativeNode) => void);
export interface NativeElementProps extends NativeEventProps {
  children?: NativeChild;
  ref?: NativeRef;
  class?: string | null;
  id?: string | null;
  /** Diagnostic identity, not a native/CSS attribute. At most 512 UTF-8 bytes; null clears. */
  testId?: string | null;
  style?: NativeStyle | null;
  [custom: `--${string}`]: string | null | undefined;
  title?: string | null;
  tabIndex?: number | null;
  disabled?: boolean | null;
  role?: "button" | "group" | "label" | "textInput" | "multilineTextInput" | null;
  ariaLabel?: string | null;
  ariaDescription?: string | null;
}
export type ButtonProps = Omit<NativeElementProps, "role">;
/** Uses native pointer/Enter/Space activation, disabled state and button semantics. */
export declare function Button(props: ButtonProps): NativeNode;
/** Native editor descendants are opaque and cannot be authored as JSX children. */
export interface NativeTextControlProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  /** Reactive native value; null clears, later undefined clears, initial undefined is absent. */
  value?: string | null;
  /** Initial-only; later changes and assignments after any explicit value are ignored. */
  defaultValue?: string | null;
  /** Native read-only editing state and accessibility semantics; focus remains available. */
  readOnly?: boolean | null;
}
export type TextFieldProps = Omit<NativeTextControlProps, "role">;
export type TextAreaProps = Omit<NativeTextControlProps, "role">;
/** Single-line native field. Defaults to tabIndex 0 and owns the textInput role. */
export declare function TextField(props: TextFieldProps): NativeNode;
/** Multiline native editor. Defaults to tabIndex 0 and owns the multilineTextInput role. */
export declare function TextArea(props: TextAreaProps): NativeNode;
/** Native replaced image; pixels are decoded and attached by the native image loader. */
export interface ImageProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  /** Exact manifest asset path in package-only previews; null/undefined clears the source. */
  src?: string | null;
}
export declare function Image(props: ImageProps): NativeNode;
export type NativeTag = "image" | "box" | "row" | "column" | "stack" | "text" | "label" | "scroll" | "control" | "field" | "editor";
export interface ReactiveOptions { name?: string; }
export interface MemoOptions<T> extends ReactiveOptions {
  equals?: false | ((previous: T, next: T) => boolean);
}
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
/**
 * Mount supported native children into a dedicated host root. The idempotent
 * disposer attempts reactive cleanup and native clear even if either throws.
 * If construction throws, the same cleanup runs before the mount error propagates.
 * Host teardown separately owns timer/request/callback registry cancellation.
 */
export declare function renderApp(code: () => NativeChild, root: NativeNode): () => void;
