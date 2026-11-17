import type { NativeNode } from "./index.js";

/** Snapshot data is retainable; cancellation methods expire when the handler returns. */
export interface NativeEvent {
  readonly type: string;
  /** Monotonic milliseconds in the native clock's origin. */
  readonly timestamp: number;
  readonly phase: "capture" | "target" | "bubble";
  readonly target: NativeNode;
  readonly currentTarget: NativeNode;
  readonly modifiers: { readonly shift: boolean; readonly control: boolean; readonly alt: boolean; readonly meta: boolean };
  preventDefault(): void;
  stopPropagation(): void;
  stopImmediatePropagation(): void;
}
export interface NativePointerEvent extends NativeEvent {
  readonly position: { readonly x: number; readonly y: number };
  readonly pointerId: string;
  readonly pointerKind: "mouse" | "touch" | "pen" | "unknown";
  readonly primary: boolean;
  readonly button: string | null;
  readonly buttonIndex?: number;
  readonly pressure: number | null;
  readonly coalesced: readonly { readonly position: { readonly x: number; readonly y: number }; readonly pressure: number | null; readonly timestamp: number }[];
  releasePointer(): void;
}
export type NativePointerBoundaryEvent = Omit<NativePointerEvent, "releasePointer">;
export type NativeContextMenuEvent = Omit<NativePointerEvent, "releasePointer">;
export interface NativePointerDownEvent extends NativePointerEvent {
  activateNow(): void;
  capturePointer(): void;
}
export interface NativeKeyEvent extends NativeEvent {
  readonly key: string | null;
  readonly character: string | null;
  readonly keyWithoutModifiers: string | null;
  readonly deadKey: string | null;
  readonly code: string | null;
  readonly physicalCode: number | null;
  readonly location: string;
  readonly repeat: boolean;
}
export interface NativeValueEvent extends NativeEvent {
  readonly value: string;
  /** Offsets are UTF-8 bytes, not JavaScript UTF-16 string indices. */
  readonly selection: { readonly start: number; readonly end: number };
  readonly valueChange: "input" | "committed";
}
/** onFocus/onBlur subscribe to native bubbling focusin/focusout. */
export interface NativeFocusEvent extends NativeEvent { readonly cause: string; }
export interface NativeAnimationEvent extends NativeEvent {
  readonly animationName: string;
  readonly elapsedTime: number;
  readonly pseudoElement: string;
}
export interface NativeTransitionEvent extends NativeEvent {
  readonly propertyName: string;
  readonly elapsedTime: number;
  readonly pseudoElement: string;
}
interface EventMap {
  onClick: NativeEvent;
  onPointerDown: NativePointerDownEvent;
  onPointerMove: NativePointerEvent;
  onPointerEnter: NativePointerBoundaryEvent;
  onPointerLeave: NativePointerBoundaryEvent;
  onPointerUp: NativePointerEvent;
  onPointerCancel: NativePointerEvent;
  onContextMenu: NativeContextMenuEvent;
  onKeyDown: NativeKeyEvent;
  onKeyUp: NativeKeyEvent;
  onInput: NativeValueEvent;
  onChange: NativeValueEvent;
  onFocus: NativeFocusEvent;
  onBlur: NativeFocusEvent;
  onAnimationStart: NativeAnimationEvent;
  onAnimationIteration: NativeAnimationEvent;
  onAnimationEnd: NativeAnimationEvent;
  onAnimationCancel: NativeAnimationEvent;
  onTransitionRun: NativeTransitionEvent;
  onTransitionStart: NativeTransitionEvent;
  onTransitionEnd: NativeTransitionEvent;
  onTransitionCancel: NativeTransitionEvent;
}
export type NativeEventProps = {
  [Name in keyof EventMap]?: ((event: EventMap[Name]) => void) | null;
} & {
  [Name in keyof EventMap as `${Name}Capture`]?: ((event: EventMap[Name]) => void) | null;
};
