import type { Accessor, Component, NativeChild, NativeElementProps, NativeNode, NativeTag } from "../index.js";

/** Compiler helpers. Application lifecycle uses renderApp, not raw render. */
export declare function render(code: () => NativeChild, root: NativeNode): () => void;
export declare function effect<T>(callback: (previous: T | undefined) => T, initial?: T): void;
export declare function memo<T>(callback: () => T, equal?: boolean): Accessor<T>;
export declare function createComponent<P>(component: Component<P>, props: P): NativeChild;
export declare function createElement(tag: NativeTag): NativeNode;
export declare function createTextNode(text: string): NativeNode;
export declare function insertNode(parent: NativeNode, node: NativeNode, anchor?: NativeNode | null): void;
export declare function insert(parent: NativeNode, value: NativeChild, marker?: NativeNode | null, initial?: NativeChild): void;
export declare function spread(node: NativeNode, props: NativeElementProps | Accessor<NativeElementProps>, skipChildren?: boolean): void;
export declare function setProp<T>(node: NativeNode, name: string, value: T, previous?: T): T;
export declare function mergeProps<T extends readonly object[]>(...sources: T): MergeSources<T>;
type MergeSources<T extends readonly object[]> = T extends readonly [infer Head extends object, ...infer Tail extends object[]]
  ? Omit<Head, keyof MergeSources<Tail>> & MergeSources<Tail> : {};
export declare function use<A, T>(callback: (node: NativeNode, argument: A) => T, node: NativeNode, argument: A): T;
