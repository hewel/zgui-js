import { createSignal, getOwner, onCleanup } from "#solid";
import { bridgeFor, withCallbackBridge } from "./host.js";
import { flush } from "./renderer/index.js";
import type { Accessor, CloseRequestEvent, CloseWindowOptions, WindowDecorations, WindowState } from "./types.js";

/** Update the active toplevel window's title. Works in both `renderApp` and `WindowSurface`. */
export function setWindowTitle(title: string): void {
  bridgeFor().setWindowTitle(title);
}

/** Toggle fullscreen mode on the active toplevel window. */
export function setWindowFullscreen(fullscreen: boolean): void {
  bridgeFor().setWindowFullscreen(fullscreen);
}

/** Maximize or restore the active toplevel window. */
export function setWindowMaximized(maximized: boolean): void {
  bridgeFor().setWindowMaximized(maximized);
}

/** Minimize or restore the active toplevel window. */
export function setWindowMinimized(minimized: boolean): void {
  bridgeFor().setWindowMinimized(minimized);
}

/** Update window chrome decorations (`"full" | "none" | "no-title-bar"`). */
export function setWindowDecorations(decorations: WindowDecorations): void {
  bridgeFor().setWindowDecorations(decorations);
}

/** Initiate a platform-driven window move drag (e.g. from a custom frameless titlebar). */
export function beginWindowMoveDrag(): void {
  bridgeFor().beginWindowMoveDrag();
}

/** Request closing the active window. Pass `{ force: true }` to bypass `onCloseRequest` guards. */
export function closeWindow(options?: CloseWindowOptions): void {
  bridgeFor().closeWindow(options?.force === true);
}

/** Read a point-in-time snapshot of the active window's state. */
export function getWindowState(): WindowState {
  return bridgeFor().getWindowState();
}

/** Observe the active toplevel window's state reactively for the lifetime of the current Solid owner. */
export function useWindowState(): Accessor<WindowState> {
  if (!getOwner()) throw new Error("useWindowState requires a Solid owner");
  const bridge = bridgeFor();
  const [state, setState] = createSignal<WindowState>(bridge.getWindowState());
  const unsubscribe = bridge.subscribeWindowState(next => {
    withCallbackBridge(bridge, () => {
      setState(next);
      flush();
    });
  });
  onCleanup(unsubscribe);
  return state;
}

/**
 * Register a synchronous close-request handler on the active toplevel window.
 * Calling `event.preventDefault()` vetoes the close synchronously so the app can
 * display a confirmation dialog and later call `closeWindow({ force: true })`.
 */
export function onCloseRequest(handler: (event: CloseRequestEvent) => void): () => void {
  const bridge = bridgeFor();
  const unsubscribe = bridge.onCloseRequest(event => {
    withCallbackBridge(bridge, () => {
      handler(event);
      flush();
    });
  });
  if (getOwner()) onCleanup(unsubscribe);
  return unsubscribe;
}
