import type { NativeChild, NativeElementProps } from "./index.js";

export namespace JSX {
  type Element = NativeChild;
  interface ElementChildrenAttribute { children: unknown; }
  interface IntrinsicElements {
    box: NativeElementProps;
    row: NativeElementProps;
    column: NativeElementProps;
    stack: NativeElementProps;
    text: NativeElementProps;
    label: NativeElementProps;
    scroll: NativeElementProps;
    control: NativeElementProps;
  }
}
