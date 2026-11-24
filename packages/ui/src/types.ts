import "./runtime-globals.js";
export type { JSX } from "./jsx-runtime.js";
import type { NativeEventProps } from "./events.js";
export type {
  DroppedFileHandle,
  NativeAnimationEvent,
  NativeContextMenuEvent,
  NativeDragEvent,
  NativeDropEvent,
  NativeEvent,
  NativePointerEvent,
  NativePointerBoundaryEvent,
  NativePointerDownEvent,
  NativeWheelEvent,
  NativeKeyEvent,
  NativeValueEvent,
  NativeFocusEvent,
  NativeTransitionEvent,
} from "./events.js";
import type { NativeStyle } from "./styles.js";
export type { NativeStyle, NativeStyleName } from "./styles.js";

declare const nativeNode: unique symbol;
/**
 * Borrowed opaque handle, never a native numeric ID. Permanently removed branches
 * invalidate their handles at the end of the synchronous host turn. A retained
 * ref does not retain native ownership; cross-turn detached-node reuse is unsupported.
 */
export interface NativeNode {
  readonly [nativeNode]: true;
}
/** A native node or reactive accessor resolving to one (e.g. a `createSignal<NativeNode | null>(null)` wired to `ref={setNode}`). */
export type NativeNodeTarget = NativeNode | Accessor<NativeNode | null | undefined>;
export interface NativeWindowRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}
export interface NativeViewport extends NativeWindowRect {
  readonly scale: number;
}

/** Physical output geometry. Use `id`, not the display `name`, to target the output. */
export interface Monitor extends NativeViewport {
  readonly id: string;
  readonly name: string;
  /** Compositor-reported logical width in CSS/logical pixels (falls back to `width / scale`). */
  readonly logicalWidth: number;
  /** Compositor-reported logical height in CSS/logical pixels (falls back to `height / scale`). */
  readonly logicalHeight: number;
  readonly refreshRateMilliHz: number | null;
}
export type LayerAnchor = "top" | "right" | "bottom" | "left";
export interface LayerSurfacePlacement {
  readonly output?: string | undefined;
  readonly layer?: "background" | "bottom" | "top" | "overlay";
  readonly anchor?: readonly LayerAnchor[];
  readonly width?: number;
  readonly height?: number;
  readonly margin?: Readonly<Partial<Record<LayerAnchor, number>>>;
  /** -1 ignores other exclusive zones; 0 reserves nothing; a positive value reserves that distance. */
  readonly exclusiveZone?: number;
  readonly keyboard?: "none" | "on-demand" | "exclusive";
}
/** Rectangles use surface-local logical coordinates. Nodes must belong to this surface. */
export type LayerInputRegion =
  | "full"
  | "empty"
  | readonly NativeWindowRect[]
  | readonly NativeNode[];
/** Native rounded-node background blur target set with an accumulated effective-opacity cutoff. */
export interface BlurRegionNodes {
  readonly nodes: readonly NativeNode[];
  /** Accumulated effective-opacity cutoff in `[0, 1]`. Default `0.5`; `0` skips only `<= 0` and `visibility: hidden`. */
  readonly minOpacity?: number | undefined;
}
/**
 * Declarative background blur region for an owned non-lock surface.
 * Rectangles use surface-local logical coordinates. Nodes must belong to this surface.
 * `[card]` is exact shorthand for `{ nodes: [card], minOpacity: 0.5 }`.
 * Omitted, `[]`, or `{ nodes: [] }` defaults to `"none"`.
 */
export type BlurRegion =
  | "none"
  | "full"
  | readonly NativeWindowRect[]
  | readonly NativeNode[]
  | BlurRegionNodes;
export interface LayerSurfaceProps extends LayerSurfacePlacement {
  readonly name: string;
  readonly inputRegion?: LayerInputRegion;
  readonly blurRegion?: BlurRegion | undefined;
  readonly children?: NativeChild;
}

/** Physical point on an anchor rectangle, or direction in which the popup extends. */
export type PopupPosition =
  | "center"
  | LayerAnchor
  | "top-left"
  | "top-right"
  | "bottom-left"
  | "bottom-right";
export interface PopupSurfacePlacement {
  readonly anchor?: PopupPosition;
  readonly gravity?: PopupPosition;
  readonly slide?: boolean;
  readonly flip?: boolean;
  readonly resize?: boolean;
  readonly reactive?: boolean;
}

/** Outer drawable bleed around a surface's visual XDG window geometry, in whole nonnegative CSS pixels. */
export type SurfaceInsets =
  | number
  | {
      readonly top?: number | undefined;
      readonly right?: number | undefined;
      readonly bottom?: number | undefined;
      readonly left?: number | undefined;
    };

/** An independently mounted compositor popup. Its anchor uses parent-local logical coordinates. */
export type PopupSurfaceProps = {
  readonly name: string;
  readonly parent: string;
  readonly width: number;
  /** Logical height in CSS pixels, or `"auto"` (default) to size from laid-out content. */
  readonly height?: number | "auto";
  /** Minimum logical height in CSS pixels when `height` is `"auto"` (default `1`). */
  readonly minHeight?: number;
  /** Maximum logical height in CSS pixels when `height` is `"auto"` (default `600`). */
  readonly maxHeight?: number;
  readonly placement?: PopupSurfacePlacement;
  readonly offset?: NativePoint;
  readonly grab?: boolean;
  /** Outer drawable bleed around the visual popup geometry, in whole nonnegative CSS pixels. */
  readonly geometryInsets?: SurfaceInsets | undefined;
  readonly inputRegion?: LayerInputRegion;
  readonly blurRegion?: BlurRegion | undefined;
  readonly onDismiss?: () => void;
  readonly children?: NativeChild;
} & (
  | { readonly anchorRect: NativeWindowRect; readonly anchorNode?: never }
  | { readonly anchorNode: NativeNode | null | undefined; readonly anchorRect?: never }
);

/** Window chrome requested from the compositor for a managed toplevel window. */
export type WindowDecorations = "full" | "none" | "no-title-bar";

/** Reactive runtime state snapshot for a toplevel window (`renderApp` or `WindowSurface`). */
export interface WindowState {
  readonly title: string;
  readonly fullscreen: boolean;
  readonly maximized: boolean;
  readonly minimized: boolean;
  readonly decorations: WindowDecorations;
}

/** Synchronous close-request event passed to `onCloseRequest` handlers. */
export interface CloseRequestEvent {
  readonly defaultPrevented: boolean;
  preventDefault(): void;
}

/** Options accepted by `closeWindow(options)`. */
export interface CloseWindowOptions {
  /** Bypass registered `onCloseRequest` guards and close immediately. */
  readonly force?: boolean | undefined;
}

/** An independently mounted compositor-managed toplevel window (xdg_toplevel). */
export interface WindowSurfaceProps {
  readonly name: string;
  readonly title: string;
  readonly appId?: string;
  readonly width: number;
  readonly height: number;
  readonly minWidth?: number;
  readonly minHeight?: number;
  readonly maxWidth?: number;
  readonly maxHeight?: number;
  readonly resizable?: boolean;
  readonly decorations?: WindowDecorations;
  readonly transparent?: boolean;
  readonly blurRegion?: BlurRegion | undefined;
  readonly onClose?: () => void;
  readonly children?: NativeChild;
}

/** The compositor's last confirmed session-lock state. */
export type SessionLockState =
  | "unlocked"
  | "pending"
  | "locked"
  | "refused"
  | "finished"
  | "unlocking";

/** The package or selected backend does not grant session-lock authority. */
export interface SessionLockCapabilityError extends Error {
  readonly code: "E_CAPABILITY";
}

/** An authorized, generation-bound request. Leaving its owner never unlocks the session. */
export interface SessionLockRequest {
  /** Process-local request identity, stable even after disposal. */
  readonly id: number;
  readonly state: Accessor<SessionLockState>;
  unlock(): void;
  dispose(): void;
}

/** Content for a named output's compositor-owned lock surface. */
export interface SessionLockSurfaceProps {
  readonly name: string;
  readonly output: string;
  readonly children?: NativeChild;
}
export interface NativePoint {
  readonly x: number;
  readonly y: number;
}
export interface NativeSize {
  readonly width: number;
  readonly height: number;
}
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
export interface NativeFocusOptions {
  readonly reveal?: boolean;
}
export type ScrollBehavior = "instant" | "smooth";
export type ScrollBlock = "nearest" | "start" | "center" | "end";
export interface ScrollToOptions {
  readonly top?: number | undefined;
  readonly left?: number | undefined;
  readonly behavior?: ScrollBehavior | undefined;
}
export interface ScrollIntoViewOptions {
  readonly block?: ScrollBlock | undefined;
  readonly behavior?: ScrollBehavior | undefined;
}
export interface NativeFocusTrapOptions {
  readonly wrap?: boolean;
  readonly autoFocus?: boolean;
  readonly restore?: boolean;
}
export type NativeChild =
  | NativeNode
  | string
  | number
  | boolean
  | null
  | undefined
  | readonly NativeChild[]
  | (() => NativeChild);
export type Accessor<T> = () => T;
export type Setter<T> = {
  <U extends T>(value: (previous: T) => U): U;
  <U extends T>(value: Exclude<U, Function>): U;
  <U extends T>(
    ...args: undefined extends T ? [] : [value: Exclude<U, Function> | ((previous: T) => U)]
  ): undefined extends T ? undefined : U;
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
export type NativeRole =
  | "alert"
  | "alertDialog"
  | "button"
  | "caption"
  | "cell"
  | "checkBox"
  | "columnHeader"
  | "comboBox"
  | "complementary"
  | "dialog"
  | "figure"
  | "form"
  | "genericContainer"
  | "grid"
  | "gridCell"
  | "group"
  | "heading"
  | "image"
  | "label"
  | "link"
  | "list"
  | "listBox"
  | "listBoxOption"
  | "listItem"
  | "main"
  | "menu"
  | "menuBar"
  | "menuItem"
  | "menuItemCheckBox"
  | "menuItemRadio"
  | "multilineTextInput"
  | "navigation"
  | "passwordInput"
  | "progressIndicator"
  | "radioButton"
  | "radioGroup"
  | "region"
  | "row"
  | "rowGroup"
  | "rowHeader"
  | "scrollBar"
  | "scrollView"
  | "slider"
  | "splitter"
  | "status"
  | "switch"
  | "tab"
  | "tabList"
  | "tabPanel"
  | "table"
  | "textInput"
  | "toolbar"
  | "tooltip";
export type NativeRelation = NativeNode | readonly NativeNode[] | null | undefined;
export interface NativeElementProps extends NativeEventProps {
  children?: NativeChild;
  ref?: NativeRef | undefined;
  class?: string | null | undefined;
  id?: string | null | undefined;
  /** Diagnostic identity, not a native/CSS attribute. At most 512 UTF-8 bytes; null clears. */
  testId?: string | null | undefined;
  /** Opt this container into native read-only cross-block `<text>` document selection. */
  documentSelection?: boolean | null | undefined;
  style?: NativeStyle | null | undefined;
  [custom: `--${string}`]: string | null | undefined;
  [data: `data-${string}`]: string | null | undefined;
  title?: string | null | undefined;
  tabIndex?: number | null | undefined;
  disabled?: boolean | null | undefined;
  /** Isolates this subtree from pointer hit-testing, focus, keyboard, and AccessKit while keeping layout, paint, and animations running. */
  inert?: boolean | null | undefined;
  checked?: boolean | "mixed" | null | undefined;
  expanded?: boolean | null | undefined;
  selected?: boolean | null | undefined;
  open?: boolean | null | undefined;
  invalid?: boolean | null | undefined;
  required?: boolean | null | undefined;
  ariaHidden?: boolean | null | undefined;
  ariaLevel?: number | null | undefined;
  ariaModal?: boolean | null | undefined;
  ariaHasPopup?: "menu" | "listbox" | "tree" | "grid" | "dialog" | null | undefined;
  ariaPopupFor?: NativeNode | null | undefined;
  ariaActiveDescendant?: NativeNode | null | undefined;
  orientation?: "horizontal" | "vertical" | null | undefined;
  role?: NativeRole | null | undefined;
  ariaLabel?: string | null | undefined;
  ariaDescription?: string | null | undefined;
  ariaKeyShortcuts?: string | null | undefined;
  ariaLabelledBy?: NativeRelation;
  ariaDescribedBy?: NativeRelation;
  ariaControls?: NativeRelation;
  ariaOwns?: NativeRelation;
  ariaRadioGroup?: NativeRelation;
  ariaBusy?: boolean | null | undefined;
  ariaLive?: "off" | "polite" | "assertive" | null | undefined;
  ariaValueMin?: number | null | undefined;
  ariaValueMax?: number | null | undefined;
  ariaValueNow?: number | null | undefined;
  ariaValueStep?: number | null | undefined;
}
export type ButtonProps = Omit<NativeElementProps, "role">;
/** Caret or selected text range in JavaScript UTF-16 code units. */
export interface TextSelectionRange {
  readonly start: number;
  readonly end: number;
}
/** One directional endpoint of a read-only cross-block document selection, in UTF-16 code units. */
export interface DocumentSelectionPoint {
  readonly node: NativeNode;
  readonly offset: number;
}
/** Snapshot of an active opt-in read-only cross-block document selection. */
export interface DocumentSelection {
  readonly container: NativeNode;
  readonly anchor: DocumentSelectionPoint;
  readonly focus: DocumentSelectionPoint;
  readonly text: string;
  readonly isCollapsed: boolean;
}
/** Native editor descendants are opaque and cannot be authored as JSX children. */
export interface NativeTextControlProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  /** Reactive native value; null clears, later undefined clears, initial undefined is absent. */
  value?: string | null | undefined;
  /** Initial-only; later changes and assignments after any explicit value are ignored. */
  defaultValue?: string | null | undefined;
  /** Native read-only editing state and accessibility semantics; focus remains available. */
  readOnly?: boolean | null | undefined;
  placeholder?: string | null | undefined;
}
export interface NativeFieldProps extends NativeTextControlProps {
  secure?: boolean | null | undefined;
}
export type TextFieldProps = Omit<NativeFieldProps, "role">;
export type TextAreaProps = Omit<NativeTextControlProps, "role">;
/** Native replaced image; pixels are decoded and attached by the native image loader. */
export interface ImageProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  /** Exact manifest asset path in package-only previews; null/undefined clears the source. */
  src?: string | null | undefined;
}

/** Affine matrix coefficients in the order [a, b, c, d, e, f]. */
export type CanvasTransform = readonly [
  a: number,
  b: number,
  c: number,
  d: number,
  e: number,
  f: number,
];

type CanvasPathSegment =
  | { kind: "moveTo"; x: number; y: number }
  | { kind: "lineTo"; x: number; y: number }
  | { kind: "quadTo"; cx: number; cy: number; x: number; y: number }
  | { kind: "cubicTo"; c1x: number; c1y: number; c2x: number; c2y: number; x: number; y: number }
  | { kind: "close" };

/** End cap shape for stroked canvas paths. */
export type CanvasLineCap = "butt" | "round" | "square";

/** Corner join shape for stroked canvas paths. */
export type CanvasLineJoin = "miter" | "round" | "bevel";

/** One gradient color stop; offset must be in [0, 1] in non-decreasing order. */
export interface CanvasColorStop {
  readonly offset: number;
  readonly color: string;
}

/** Linear gradient brush along the segment from (x0, y0) to (x1, y1) in canvas-local coordinates. */
export interface CanvasLinearGradient {
  readonly kind: "linear";
  readonly x0: number;
  readonly y0: number;
  readonly x1: number;
  readonly y1: number;
  readonly stops: readonly CanvasColorStop[];
  readonly repeating?: boolean;
}

/** Radial gradient brush centered at (cx, cy) in canvas-local coordinates. */
export type CanvasRadialGradient = {
  readonly kind: "radial";
  readonly cx: number;
  readonly cy: number;
  readonly stops: readonly CanvasColorStop[];
  readonly repeating?: boolean;
} & (
  | { readonly radius: number; readonly radiusX?: never; readonly radiusY?: never }
  | { readonly radius?: never; readonly radiusX: number; readonly radiusY: number }
);

/** Paint brush for a canvas fill or stroke: hex color string or gradient descriptor. */
export type CanvasBrush = string | CanvasLinearGradient | CanvasRadialGradient;

/** Stroke configuration for a canvas command. */
export interface CanvasStroke {
  readonly color: CanvasBrush;
  readonly width: number;
  readonly lineCap?: CanvasLineCap;
  readonly lineJoin?: CanvasLineJoin;
  readonly lineDash?: readonly number[];
  readonly lineDashOffset?: number;
}

/** One painted shape; supply a fill brush, a stroke with positive width, or both. */
export type CanvasCommand = {
  fill?: CanvasBrush;
  stroke?: CanvasStroke;
} & (
  | { kind: "rect"; x: number; y: number; width: number; height: number }
  | { kind: "roundedRect"; x: number; y: number; width: number; height: number; radius: number }
  | { kind: "circle"; x: number; y: number; radius: number }
  /** Angles are radians; the sweep magnitude cannot exceed one full turn. */
  | { kind: "arc"; x: number; y: number; radius: number; startAngle: number; sweepAngle: number }
  /** Each subpath starts with moveTo, including after close. */
  | { kind: "path"; segments: readonly CanvasPathSegment[] }
);

declare const canvasScene: unique symbol;
/** A session-owned drawing that can be shared by multiple vectors. */
export interface CanvasScene {
  readonly [canvasScene]: true;
  readonly token: number;
  /** Commands replace geometry in paint order; null retains it, and [] clears it. Omit transform to retain the matrix. */
  update(commands: readonly CanvasCommand[] | null, transform?: CanvasTransform): void;
  /** Unbinds all vectors and invalidates further updates. Safe to call more than once. */
  dispose(): void;
}

/** Childless drawing. Sources are selected in the order canvas, svg, d. */
export interface VectorProps extends Omit<NativeElementProps, "children"> {
  children?: never;
  d?: string | null;
  viewBox?: string | null;
  svg?: string | null;
  canvas?: CanvasScene | null;
}
export type NativeTag =
  | "image"
  | "vector"
  | "box"
  | "row"
  | "column"
  | "stack"
  | "text"
  | "label"
  | "scroll"
  | "control"
  | "field"
  | "editor";
export interface ReactiveOptions {
  name?: string;
}
export interface MemoOptions<T> extends ReactiveOptions {
  equals?: false | ((previous: T, next: T) => boolean);
}
export interface SignalOptions<T> extends MemoOptions<T> {
  ownedWrite?: boolean;
  unobserved?: () => void;
}
