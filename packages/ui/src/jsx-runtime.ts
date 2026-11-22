import type {
  ImageProps,
  NativeChild,
  NativeElementProps,
  NativeFieldProps,
  NativeTextControlProps,
  VectorProps,
} from "./index.js";

export declare namespace JSX {
  type Element = NativeChild;
  interface ElementChildrenAttribute {
    children: unknown;
  }
  interface IntrinsicElements {
    box: NativeElementProps;
    row: NativeElementProps;
    column: NativeElementProps;
    stack: NativeElementProps;
    text: NativeElementProps;
    label: NativeElementProps;
    scroll: NativeElementProps;
    control: NativeElementProps;
    field: NativeFieldProps;
    editor: NativeTextControlProps;
    image: ImageProps;
    vector: VectorProps;
  }
}
