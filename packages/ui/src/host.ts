import type { NativeNode } from "./types.js";

/** Installed by UiBridge::install; resolving it must remain lazy during preflight. */
export interface NativeHost {
  readonly __zgui: {
    createElement(tag: string): NativeNode;
    createTextNode(text: string): NativeNode;
    replaceText(node: NativeNode, text: string): void;
    isTextNode(node: NativeNode): boolean;
    insertNode(parent: NativeNode, node: NativeNode, anchor?: NativeNode | null): void;
    removeNode(parent: NativeNode, node: NativeNode): void;
    getParentNode(node: NativeNode): NativeNode | null | undefined;
    getFirstChild(node: NativeNode): NativeNode | null | undefined;
    getNextSibling(node: NativeNode): NativeNode | null | undefined;
    setProperty(node: NativeNode, name: string, value: unknown, previous?: unknown): void;
    clearRoot(root: NativeNode): void;
  };
}
