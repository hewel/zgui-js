import "./runtime-globals.js";
export type { JSX } from "./jsx-runtime.js";
import type { NativeEventProps } from "./events.js";
export type { NativeAnimationEvent, NativeContextMenuEvent, NativeEvent, NativePointerEvent, NativePointerBoundaryEvent, NativePointerDownEvent, NativeKeyEvent, NativeValueEvent, NativeFocusEvent, NativeTransitionEvent } from "./events.js";
import type { NativeStyle } from "./styles.js";
export type { NativeStyle, NativeStyleName } from "./styles.js";

declare const nativeNode: unique symbol;
/**
 * Borrowed opaque handle, never a native numeric ID. Permanently removed branches
 * invalidate their handles at the end of the synchronous host turn. A retained
 * ref does not retain native ownership; cross-turn detached-node reuse is unsupported.
 */
export interface NativeNode { readonly [nativeNode]: true; }
export interface NativeWindowRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}
export interface NativeViewport extends NativeWindowRect { readonly scale: number; }
export interface NativePoint { readonly x: number; readonly y: number; }
export interface NativeSize { readonly width: number; readonly height: number; }
export interface NativeScrollPosition {
  readonly offset: NativePoint;
  readonly contentSize: NativeSize;
  readonly scrollport: NativeSize;
}
export interface NativeGeometry {
  readonly borderBox: NativeWindowRect;
  readonly borderSize: NativeSize;
  readonly contentSize: NativeSize;
  readonly scrollPosition: NativeScrollPosition;
  readonly viewport: NativeViewport;
}
export type NativeGeometryKind = keyof NativeGeometry;
export type NativeOverlayLayer = "content" | "popover" | "modal" | "toast";
export type NativeDismissReason = "outside-press" | "escape";
export interface NativeDismissLayerOptions {
  readonly layer?: NativeOverlayLayer;
  readonly dismissOnOutsidePress?: boolean;
  readonly dismissOnEscape?: boolean;
  readonly exclude?: NativeNode | null;
  readonly leaving?: boolean;
}
export interface NativeDismissLayer {
  update(options?: NativeDismissLayerOptions): void;
  dispose(): void;
}
export interface NativeFocusOptions { readonly reveal?: boolean; }
export interface NativeFocusTrapOptions {
  readonly wrap?: boolean;
  readonly autoFocus?: boolean;
  readonly restore?: boolean;
}
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
export type NativeRole = "alert" | "alertDialog" | "button" | "caption" | "cell" | "checkBox" | "columnHeader" | "comboBox" | "complementary" | "dialog" | "figure" | "form" | "genericContainer" | "grid" | "gridCell" | "group" | "heading" | "image" | "label" | "link" | "list" | "listBox" | "listBoxOption" | "listItem" | "main" | "menu" | "menuBar" | "menuItem" | "menuItemCheckBox" | "menuItemRadio" | "multilineTextInput" | "navigation" | "passwordInput" | "progressIndicator" | "radioButton" | "radioGroup" | "region" | "row" | "rowGroup" | "rowHeader" | "scrollBar" | "scrollView" | "slider" | "splitter" | "status" | "switch" | "tab" | "tabList" | "tabPanel" | "table" | "textInput" | "toolbar" | "tooltip";
export type NativeRelation = NativeNode | readonly NativeNode[] | null;
export interface NativeElementProps extends NativeEventProps {
  children?: NativeChild;
  ref?: NativeRef;
  class?: string | null;
  id?: string | null;
  /** Diagnostic identity, not a native/CSS attribute. At most 512 UTF-8 bytes; null clears. */
  testId?: string | null;
  style?: NativeStyle | null;
  [custom: `--${string}`]: string | null | undefined;
  [data: `data-${string}`]: string | null | undefined;
  title?: string | null;
  tabIndex?: number | null;
  disabled?: boolean | null;
  checked?: boolean | "mixed" | null;
  expanded?: boolean | null;
  selected?: boolean | null;
  open?: boolean | null;
  invalid?: boolean | null;
  required?: boolean | null;
  ariaHidden?: boolean | null;
  ariaLevel?: number | null;
  ariaModal?: boolean | null;
  ariaHasPopup?: "menu" | "listbox" | "tree" | "grid" | "dialog" | null;
  ariaPopupFor?: NativeNode | null;
  orientation?: "horizontal" | "vertical" | null;
  role?: NativeRole | null;
  ariaLabel?: string | null;
  ariaDescription?: string | null;
  ariaKeyShortcuts?: string | null;
  ariaLabelledBy?: NativeRelation;
  ariaDescribedBy?: NativeRelation;
  ariaControls?: NativeRelation;
  ariaOwns?: NativeRelation;
  ariaRadioGroup?: NativeRelation;
  ariaBusy?: boolean | null;
  ariaLive?: "off" | "polite" | "assertive" | null;
  ariaValueMin?: number | null;
  ariaValueMax?: number | null;
  ariaValueNow?: number | null;
  ariaValueStep?: number | null;
}
export type ButtonProps = Omit<NativeElementProps, "role">;
/** Native editor descendants are opaque and cannot be authored as JSX children. */
export interface NativeTextControlProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  /** Reactive native value; null clears, later undefined clears, initial undefined is absent. */
  value?: string | null;
  /** Initial-only; later changes and assignments after any explicit value are ignored. */
  defaultValue?: string | null;
  /** Native read-only editing state and accessibility semantics; focus remains available. */
  readOnly?: boolean | null;
  placeholder?: string | null;
}
export interface NativeFieldProps extends NativeTextControlProps {
  secure?: boolean | null;
}
export type TextFieldProps = Omit<NativeFieldProps, "role">;
export type TextAreaProps = Omit<NativeTextControlProps, "role">;
/** Native replaced image; pixels are decoded and attached by the native image loader. */
export interface ImageProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  /** Exact manifest asset path in package-only previews; null/undefined clears the source. */
  src?: string | null;
}
export interface VectorProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  d?: string | null;
  viewBox?: string | null;
}
export type NativeTag = "image" | "vector" | "box" | "row" | "column" | "stack" | "text" | "label" | "scroll" | "control" | "field" | "editor";
export interface ReactiveOptions { name?: string; }
export interface MemoOptions<T> extends ReactiveOptions {
  equals?: false | ((previous: T, next: T) => boolean);
}
